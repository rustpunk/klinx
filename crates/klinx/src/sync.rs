/// YAML text ↔ PipelineConfig sync. Single-model sync only (no composition
/// or channel overlay reconciliation).
use std::collections::HashMap;
use std::num::NonZeroU32;
use std::path::Path;

use clinker_core::config::composition::CompositionFile;
use clinker_core::config::{PipelineConfig, parse_config};
use clinker_core::span::FileId;

use crate::pipeline_view::{PipelineView, derive_composition_view};

/// Tracks which view most recently edited the pipeline model.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum EditSource {
    Yaml,
    Inspector,
    #[default]
    None,
}

/// Parse a YAML string into a `PipelineConfig`.
///
/// On failure the raw engine error is passed through [`crate::parse_diagnostics`]
/// so a recognizable mistake (e.g. a mapping key that lost its colon) gains a
/// hint pointing at the true culprit line before it reaches the editor.
pub fn parse_yaml(yaml: &str) -> Result<PipelineConfig, Vec<String>> {
    parse_config(yaml).map_err(|e| crate::parse_diagnostics::refine(yaml, vec![e.to_string()]))
}

/// Compatibility shim — same as `parse_yaml`.
pub fn parse_yaml_raw_path(yaml: &str) -> Result<PipelineConfig, Vec<String>> {
    parse_yaml(yaml)
}

/// True when the document is a composition (`*.comp.yaml`) rather than a
/// pipeline: it has a top-level `_compose:` key. Pipelines use `pipeline:`; the
/// two are mutually exclusive at the document root. Detected by content (not
/// filename) so the live editor classifies correctly regardless of the tab path.
/// `_compose:` only appears unindented as the root key — any occurrence inside a
/// nested value (e.g. a `cxl` block) is indented and so won't match.
pub fn is_composition_yaml(yaml: &str) -> bool {
    yaml.lines().any(|line| line.starts_with("_compose:"))
}

/// Parse a composition document, returning its canvas DAG view (best-effort) and
/// any validation errors.
///
/// Used instead of [`parse_yaml`] for `_compose:` documents so they validate as
/// compositions — opening a `.comp.yaml` no longer fails with the spurious
/// "missing required key: pipeline". Two outputs, decoupled to match the two
/// user-facing needs:
/// - **view**: the body DAG to render. On a clean parse it comes from the typed
///   nodes; if the `_compose:` signature fails strict validation (e.g. schema
///   drift) we still recover the body graph via [`body_dag_view`] so the canvas
///   isn't blank.
/// - **errors**: the strict composition validation errors for the editor,
///   refined through [`crate::parse_diagnostics`] just like the pipeline path.
pub fn parse_composition(yaml: &str) -> (Option<PipelineView>, Vec<String>) {
    let file_id = FileId::new(NonZeroU32::new(1).expect("1 is non-zero"));
    match CompositionFile::parse(yaml, file_id, std::path::PathBuf::new()) {
        Ok(comp) => (Some(derive_composition_view(&comp)), Vec::new()),
        Err(e) => (
            body_dag_view(yaml),
            crate::parse_diagnostics::refine(yaml, vec![e.to_string()]),
        ),
    }
}

/// Best-effort body DAG when strict composition parse fails: reframe the
/// top-level `nodes:` block (identical taxonomy to a pipeline's) under a
/// synthetic `pipeline:` header and run the tolerant partial parser, so the
/// graph still renders while the `_compose:` signature errors are surfaced
/// separately. Returns `None` when the document has no `nodes:` block to recover.
fn body_dag_view(yaml: &str) -> Option<PipelineView> {
    let nodes_at = yaml.lines().position(|line| line.starts_with("nodes:"))?;
    let mut doc = String::from("pipeline:\n  name: composition\n");
    for line in yaml.lines().skip(nodes_at) {
        doc.push_str(line);
        doc.push('\n');
    }
    match clinker_core::partial::parse_partial_config(&doc) {
        Ok(partial) => Some(crate::pipeline_view::derive_partial_pipeline_view(&partial)),
        Err(_) => None,
    }
}

/// Result of parsing a pipeline YAML.
pub struct ResolvedPipeline {
    pub resolved: PipelineConfig,
}

/// Parse YAML to a ResolvedPipeline. `_workspace_root` is accepted for API
/// compatibility but unused.
pub fn parse_and_resolve_yaml(
    yaml: &str,
    _workspace_root: Option<&Path>,
) -> Result<ResolvedPipeline, Vec<String>> {
    let resolved = parse_yaml(yaml)?;
    Ok(ResolvedPipeline { resolved })
}

#[allow(clippy::large_enum_variant)]
pub enum ParseResult {
    Complete(ResolvedPipeline),
    Partial(clinker_core::partial::PartialPipelineConfig),
    Failed(Vec<String>),
}

pub fn try_parse_yaml(yaml: &str, workspace_root: Option<&Path>) -> ParseResult {
    if let Ok(resolved) = parse_and_resolve_yaml(yaml, workspace_root) {
        return ParseResult::Complete(resolved);
    }
    match clinker_core::partial::parse_partial_config(yaml) {
        Ok(mut partial) => {
            partial.errors = crate::parse_diagnostics::refine(yaml, partial.errors);
            ParseResult::Partial(partial)
        }
        Err(e) => ParseResult::Failed(crate::parse_diagnostics::refine(yaml, vec![e])),
    }
}

/// Compute YAML line ranges for each named stage (best-effort).
pub fn compute_yaml_ranges(yaml: &str, config: &PipelineConfig) -> HashMap<String, (usize, usize)> {
    let mut ranges = HashMap::new();
    let lines: Vec<&str> = yaml.lines().collect();

    let mut stage_names: Vec<String> = Vec::new();
    for input in config.source_configs() {
        stage_names.push(input.name.clone());
    }
    for transform in config.transform_views() {
        stage_names.push(transform.name.to_string());
    }
    for output in config.output_configs() {
        stage_names.push(output.name.clone());
    }

    for name in &stage_names {
        let name_pattern = format!("name: {name}");
        if let Some(start_idx) = lines.iter().position(|line| line.contains(&name_pattern)) {
            let mut block_start = start_idx;
            if start_idx > 0 {
                let line = lines[start_idx].trim_start();
                if !(line.starts_with("- name:") || line.starts_with("name:")) {
                    for i in (0..start_idx).rev() {
                        let prev = lines[i].trim_start();
                        if prev.starts_with("- ") {
                            block_start = i;
                            break;
                        }
                        if !prev.is_empty() {
                            break;
                        }
                    }
                }
            }

            let base_indent = lines[block_start].len() - lines[block_start].trim_start().len();
            let mut end_idx = start_idx;
            #[allow(clippy::needless_range_loop)]
            for i in (start_idx + 1)..lines.len() {
                let line = lines[i];
                if line.trim().is_empty() {
                    continue;
                }
                let indent = line.len() - line.trim_start().len();
                if indent <= base_indent {
                    break;
                }
                end_idx = i;
            }

            ranges.insert(name.clone(), (block_start + 1, end_idx + 1));
        }
    }

    ranges
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A current-schema composition (engine fixture shape): one `combine` body
    /// node behind two input ports.
    const VALID_COMP: &str = r#"_compose:
  name: combine_enrich
  inputs:
    orders:
      schema:
        - { name: order_id, type: string }
        - { name: product_id, type: string }
    products:
      schema:
        - { name: product_id, type: string }
        - { name: name, type: string }
  outputs:
    enriched: enrich_combine
  config_schema: {}

nodes:
  - type: combine
    name: enrich_combine
    input:
      orders: orders
      products: products
    config:
      where: "orders.product_id == products.product_id"
      match: first
      on_miss: null_fields
      cxl: |
        emit order_id = orders.order_id
        emit product_name = products.name
      propagate_ck: driver
"#;

    #[test]
    fn detects_composition_by_root_key() {
        assert!(is_composition_yaml(VALID_COMP));
        assert!(is_composition_yaml("_compose:\n  name: x\nnodes: []\n"));
        assert!(!is_composition_yaml("pipeline:\n  name: x\nnodes: []\n"));
        // An indented `_compose:` (e.g. inside a cxl block) is not the root key.
        assert!(!is_composition_yaml(
            "pipeline:\n  cxl: |\n    _compose: nope\n"
        ));
    }

    #[test]
    fn valid_composition_renders_body_dag_without_errors() {
        let (view, errors) = parse_composition(VALID_COMP);
        assert!(
            errors.is_empty(),
            "valid composition should not error: {errors:?}"
        );
        let view = view.expect("composition should yield a DAG view");
        assert_eq!(view.stages.len(), 1, "one body node (enrich_combine)");
    }

    #[test]
    fn composition_is_not_misparsed_as_pipeline() {
        // The spurious "missing required key: pipeline" must not appear for a
        // composition: it is routed to the composition parser instead.
        let (_view, errors) = parse_composition(VALID_COMP);
        assert!(!errors.iter().any(|e| e.contains("pipeline")));
    }
}
