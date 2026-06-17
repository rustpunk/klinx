/// YAML text ↔ PipelineConfig sync. Single-model sync only (no composition
/// or channel overlay reconciliation).
use std::collections::HashMap;
use std::num::NonZeroU32;
use std::path::Path;

use clinker_core_types::span::FileId;
use clinker_plan::config::composition::CompositionFile;
use clinker_plan::config::{PipelineConfig, parse_config};

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
    match clinker_exec::partial::parse_partial_config(&doc) {
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
    Partial(clinker_exec::partial::PartialPipelineConfig),
    Failed(Vec<String>),
}

pub fn try_parse_yaml(yaml: &str, workspace_root: Option<&Path>) -> ParseResult {
    if let Ok(resolved) = parse_and_resolve_yaml(yaml, workspace_root) {
        return ParseResult::Complete(resolved);
    }
    match clinker_exec::partial::parse_partial_config(yaml) {
        Ok(mut partial) => {
            partial.errors = crate::parse_diagnostics::refine(yaml, partial.errors);
            ParseResult::Partial(partial)
        }
        Err(e) => ParseResult::Failed(crate::parse_diagnostics::refine(yaml, vec![e])),
    }
}

/// Source range for one node block in the authoritative YAML document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct YamlNodeRange {
    /// Inclusive 1-based line where the node block starts.
    pub start_line: usize,
    /// Inclusive 1-based line where the node block ends.
    pub end_line: usize,
    /// Byte offset where the node block starts.
    pub start_byte: usize,
    /// Byte offset one past the node block.
    pub end_byte: usize,
}

/// Compute YAML source ranges for each named node.
///
/// The engine stores every parsed node as a spanned [`PipelineNode`]. Use those
/// spans rather than scanning only source/transform/output helper lists so all
/// node variants get a range and duplicate-like names cannot collide.
pub fn compute_yaml_node_ranges(
    yaml: &str,
    config: &PipelineConfig,
) -> HashMap<String, YamlNodeRange> {
    let mut starts = Vec::with_capacity(config.nodes.len());
    for node in &config.nodes {
        let Some(offset) = node.referenced.span().byte_offset() else {
            continue;
        };
        let offset = offset as usize;
        if offset > yaml.len() {
            continue;
        }
        starts.push((node.value.name().to_string(), line_start(yaml, offset)));
    }

    if starts.windows(2).any(|window| window[0].1 >= window[1].1) {
        return HashMap::new();
    }

    let Some((_, first_start)) = starts.first() else {
        return HashMap::new();
    };
    let Some((_, last_start)) = starts.last() else {
        return HashMap::new();
    };
    let nodes_end = find_nodes_block_end(yaml, *last_start);
    let mut ranges = HashMap::with_capacity(starts.len());

    for (index, (name, start_byte)) in starts.iter().enumerate() {
        let end_byte = starts
            .get(index + 1)
            .map(|(_, next_start)| *next_start)
            .unwrap_or(nodes_end)
            .min(yaml.len());
        if *start_byte < *first_start || *start_byte > end_byte {
            continue;
        }
        ranges.insert(
            name.clone(),
            YamlNodeRange {
                start_line: line_number_at(yaml, *start_byte),
                end_line: line_number_at(yaml, end_byte.saturating_sub(1)),
                start_byte: *start_byte,
                end_byte,
            },
        );
    }

    ranges
}

/// Compute YAML line ranges for each named stage (best-effort).
pub fn compute_yaml_ranges(yaml: &str, config: &PipelineConfig) -> HashMap<String, (usize, usize)> {
    compute_yaml_node_ranges(yaml, config)
        .into_iter()
        .map(|(name, range)| (name, (range.start_line, range.end_line)))
        .collect()
}

/// Replace one node block inside a full YAML document.
///
/// The caller supplies a previously-derived range; this helper never parses or
/// serializes the whole config. It preserves all bytes outside the selected
/// block and normalizes the replacement to end with one newline when the
/// original range ended before more document content.
pub fn splice_yaml_node_block(yaml: &str, range: YamlNodeRange, replacement: &str) -> String {
    if range.start_byte > range.end_byte || range.end_byte > yaml.len() {
        return yaml.to_string();
    }

    let mut normalized = replacement.to_string();
    if range.end_byte < yaml.len() && !normalized.ends_with('\n') {
        normalized.push('\n');
    }

    let mut out = String::with_capacity(
        yaml.len().saturating_sub(range.end_byte - range.start_byte) + normalized.len(),
    );
    out.push_str(&yaml[..range.start_byte]);
    out.push_str(&normalized);
    out.push_str(&yaml[range.end_byte..]);
    out
}

fn line_start(text: &str, offset: usize) -> usize {
    text[..offset].rfind('\n').map_or(0, |idx| idx + 1)
}

fn line_number_at(text: &str, offset: usize) -> usize {
    text[..offset.min(text.len())]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1
}

fn find_nodes_block_end(text: &str, last_node_start: usize) -> usize {
    let last_indent = text[last_node_start..]
        .lines()
        .next()
        .map(line_indent)
        .unwrap_or(0);
    let search = &text[last_node_start..];
    let mut cursor = last_node_start;
    for line in search.lines() {
        let line_start = cursor;
        cursor += line.len() + 1;
        if line_start == last_node_start || line.trim().is_empty() {
            continue;
        }
        let indent = line_indent(line);
        if indent < last_indent {
            return line_start;
        }
    }
    text.len()
}

fn line_indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
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
        // 2 input ports (orders, products) + 1 body node (enrich_combine) +
        // 1 output port (enriched) = 4 boundary-and-body stages.
        assert_eq!(
            view.stages.len(),
            4,
            "2 input ports + 1 body node + 1 output port"
        );
    }

    #[test]
    fn composition_is_not_misparsed_as_pipeline() {
        // The spurious "missing required key: pipeline" must not appear for a
        // composition: it is routed to the composition parser instead.
        let (_view, errors) = parse_composition(VALID_COMP);
        assert!(!errors.iter().any(|e| e.contains("pipeline")));
    }

    /// Every bundled example composition must parse cleanly against the pinned
    /// engine schema and render the full contract — guards the example workspace
    /// against silent schema drift (the reason the originals stopped rendering).
    ///
    /// The exact stage count is derived per file from the parsed signature +
    /// body (`inputs + body nodes + outputs`) rather than hardcoded, so adding
    /// an example composition keeps the assertion specific without edits here.
    #[test]
    fn bundled_example_compositions_parse_and_render() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/pipelines/compositions");
        let mut checked = 0;
        for entry in std::fs::read_dir(&dir).expect("compositions dir exists") {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("yaml") {
                continue;
            }
            let yaml = std::fs::read_to_string(&path).unwrap();
            assert!(is_composition_yaml(&yaml), "{path:?} not detected as comp");
            let (view, errors) = parse_composition(&yaml);
            assert!(errors.is_empty(), "{path:?} must parse cleanly: {errors:?}");
            let view = view.expect("composition view");

            // Expected = input ports + body nodes + output ports, computed from
            // the same parse the renderer consumes.
            let comp = CompositionFile::parse(
                &yaml,
                FileId::new(NonZeroU32::new(1).expect("nonzero")),
                std::path::PathBuf::new(),
            )
            .expect("composition re-parses for count check");
            let expected =
                comp.signature.inputs.len() + comp.nodes.len() + comp.signature.outputs.len();
            assert_eq!(
                view.stages.len(),
                expected,
                "{path:?} must render every input port, body node, and output port"
            );
            checked += 1;
        }
        assert!(
            checked >= 5,
            "expected the bundled example comps, got {checked}"
        );
    }

    const ALL_VARIANTS_PIPELINE: &str = r#"
pipeline:
  name: variant_ranges
nodes:
  - type: source
    name: src
    config:
      name: src
      type: csv
      path: ./in.csv
      schema:
        - { name: x, type: int }
  - type: transform
    name: clean
    input: src
    config:
      cxl: |
        emit x2 = x + 1
  - type: aggregate
    name: rollup
    input: clean
    config:
      group_by: [x2]
      cxl: |
        emit count = 1
  - type: route
    name: split
    input: clean
    config:
      conditions:
        high: "x2 > 10"
      default: low
  - type: merge
    name: joined
    inputs: [split.high, split.low]
  - type: combine
    name: combined
    input:
      left: joined
      right: rollup
    config:
      where: "left.x2 == right.x2"
      match: first
      on_miss: skip
      cxl: |
        emit count = right.count
      propagate_ck: driver
  - type: composition
    name: sub
    input: combined
    use: compositions/clean_names.comp.yaml
  - type: reshape
    name: shaped
    input: sub
    config:
      partition_by: [x2]
      rules:
        - name: fill
          when: "x2 > 0"
          synthesize:
            copy_from: trigger
  - type: cull
    name: pruned
    input: shaped
    config:
      partition_by: [x2]
      removed_to: dropped
      rules:
        - name: drop_small
          drop_group_when: "count(*) < 2"
  - type: envelope
    name: framed
    body: pruned
    config:
      strategy: preserve
  - type: output
    name: out
    input: framed
    config:
      name: out
      type: csv
      path: ./out.csv
"#;

    #[test]
    fn yaml_ranges_cover_every_pipeline_node_variant() {
        let config = parse_config(ALL_VARIANTS_PIPELINE).expect("fixture parses");
        let ranges = compute_yaml_node_ranges(ALL_VARIANTS_PIPELINE, &config);

        for name in [
            "src", "clean", "rollup", "split", "joined", "combined", "sub", "shaped", "pruned",
            "framed", "out",
        ] {
            let range = ranges
                .get(name)
                .unwrap_or_else(|| panic!("range for {name}"));
            let block = &ALL_VARIANTS_PIPELINE[range.start_byte..range.end_byte];
            assert!(
                block.contains(&format!("name: {name}")),
                "block for {name} should contain its name: {block}"
            );
        }
    }

    #[test]
    fn yaml_ranges_do_not_confuse_duplicate_like_names() {
        let yaml = r#"
pipeline:
  name: duplicate_like
nodes:
  - type: source
    name: src
    config:
      name: src
      type: csv
      path: ./src.csv
      schema:
        - { name: id, type: string }
  - type: source
    name: src_extra
    config:
      name: src_extra
      type: csv
      path: ./src_extra.csv
      schema:
        - { name: id, type: string }
"#;
        let config = parse_config(yaml).expect("fixture parses");
        let ranges = compute_yaml_node_ranges(yaml, &config);
        let src = ranges.get("src").expect("src range");
        let src_block = &yaml[src.start_byte..src.end_byte];

        assert!(src_block.contains("name: src"));
        assert!(!src_block.contains("name: src_extra"));
    }

    #[test]
    fn splice_yaml_node_block_preserves_outside_comments_and_grows_or_shrinks() {
        let yaml = r#"
pipeline:
  name: splice
# nodes stay below
nodes:
  - type: source
    name: src
    config:
      name: src
      type: csv
      path: ./in.csv
      schema:
        - { name: id, type: string }
  - type: output
    name: out
    input: src
    config:
      name: out
      type: csv
      path: ./out.csv
# tail comment
"#;
        let config = parse_config(yaml).expect("fixture parses");
        let ranges = compute_yaml_node_ranges(yaml, &config);
        let range = *ranges.get("src").expect("src range");
        let grown = splice_yaml_node_block(
            yaml,
            range,
            "  - type: source\n    name: src\n    config:\n      name: src\n      type: csv\n      path: ./changed.csv\n      schema:\n        - { name: id, type: string }",
        );

        assert!(grown.contains("# nodes stay below"));
        assert!(grown.contains("path: ./changed.csv"));
        assert!(grown.contains("name: out"));
        assert!(grown.contains("# tail comment"));

        let reparsed = parse_config(&grown).expect("grown YAML still parses");
        let out_range = *compute_yaml_node_ranges(&grown, &reparsed)
            .get("src")
            .expect("src range after grow");
        let shrunk = splice_yaml_node_block(
            &grown,
            out_range,
            "  - type: source\n    name: src\n    config:\n      name: src\n      type: csv\n      path: ./short.csv",
        );
        assert!(shrunk.contains("path: ./short.csv"));
        assert!(!shrunk.contains("schema:"));
        assert!(shrunk.contains("name: out"));
    }

    #[test]
    fn splice_yaml_node_block_allows_temporarily_invalid_draft() {
        let config = parse_config(ALL_VARIANTS_PIPELINE).expect("fixture parses");
        let range = *compute_yaml_node_ranges(ALL_VARIANTS_PIPELINE, &config)
            .get("clean")
            .expect("clean range");
        let edited = splice_yaml_node_block(
            ALL_VARIANTS_PIPELINE,
            range,
            "  - type: transform\n    name: clean\n    input: src\n    config:\n      cxl: |\n        emit x2 =",
        );

        assert!(edited.contains("emit x2 ="));
        assert!(edited.contains("name: rollup"));
    }
}
