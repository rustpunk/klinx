//! Pipeline template model and instantiation.
//!
//! Templates are valid Clinker pipeline YAML files with `_template` metadata.
//! No template engine, no variables — Klinx copies the file, strips the
//! `_template` block, opens it as a new tab, and the user edits it.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Source of a template.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TemplateSource {
    /// Bundled in the Klinx binary (embedded at compile time).
    Bundled,
    /// Found in the workspace's `templates/` directory.
    Workspace,
}

/// Parsed `_template` metadata block from a pipeline YAML file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TemplateMetadata {
    /// Display name for the template.
    pub name: String,
    /// Short description of what the template does.
    pub description: String,
    /// Category for grouping (e.g., "transform", "join", "etl").
    #[serde(default)]
    pub category: Option<String>,
    /// Tags for search/filtering.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Author identifier.
    #[serde(default)]
    pub author: Option<String>,
    /// Template version.
    #[serde(default)]
    pub version: Option<String>,
    /// Guide annotation hints (YAML path → hint text).
    #[serde(default)]
    pub hints: HashMap<String, String>,
}

/// A resolved template ready for display in the gallery.
#[derive(Clone, Debug, PartialEq)]
pub struct Template {
    /// Parsed metadata from `_template` block.
    pub metadata: TemplateMetadata,
    /// Full YAML content (including `_template` block).
    pub raw_yaml: String,
    /// Where this template came from.
    pub source: TemplateSource,
    /// Format category for filtering (derived from input type in the YAML).
    pub format_category: String,
}

// ── Bundled templates (embedded at compile time) ────────────────────────

const BUNDLED_TEMPLATES: &[(&str, &str)] = &[
    (
        "csv_transform",
        include_str!("templates/csv_transform.yaml"),
    ),
    ("csv_join", include_str!("templates/csv_join.yaml")),
    ("csv_dedup", include_str!("templates/csv_dedup.yaml")),
    ("json_flatten", include_str!("templates/json_flatten.yaml")),
    ("xml_extract", include_str!("templates/xml_extract.yaml")),
    ("full_etl", include_str!("templates/full_etl.yaml")),
];

// ── Parsing ─────────────────────────────────────────────────────────────

/// Intermediate struct for deserializing just the `_template` block.
#[derive(Deserialize)]
struct TemplateYaml {
    _template: TemplateMetadata,
}

/// Parse a template from YAML content.
///
/// Extracts the `_template` metadata block. Returns `None` if the YAML
/// doesn't contain a `_template` block.
pub fn parse_template(yaml: &str, source: TemplateSource) -> Option<Template> {
    let parsed: TemplateYaml = clinker_plan::yaml::from_str(yaml).ok()?;
    let format_category = detect_format_category(yaml);

    Some(Template {
        metadata: parsed._template,
        raw_yaml: yaml.to_string(),
        source,
        format_category,
    })
}

/// Detect the primary format from the YAML content.
fn detect_format_category(yaml: &str) -> String {
    for line in yaml.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("type:") {
            let value = trimmed.strip_prefix("type:").unwrap().trim();
            return match value {
                "csv" => "csv",
                "json" | "jsonl" => "json",
                "xml" => "xml",
                _ => "other",
            }
            .to_string();
        }
    }
    "other".to_string()
}

// ── Template loading ────────────────────────────────────────────────────

/// Load all bundled templates.
pub fn load_bundled_templates() -> Vec<Template> {
    BUNDLED_TEMPLATES
        .iter()
        .filter_map(|(_name, yaml)| parse_template(yaml, TemplateSource::Bundled))
        .collect()
}

/// Discover workspace templates from the `templates/` directory.
pub fn load_workspace_templates(workspace_root: &Path) -> Vec<Template> {
    let templates_dir = workspace_root.join("templates");
    if !templates_dir.is_dir() {
        return Vec::new();
    }

    let Ok(entries) = fs::read_dir(&templates_dir) else {
        return Vec::new();
    };

    entries
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|ext| ext == "yaml" || ext == "yml")
        })
        .filter_map(|e| {
            let content = fs::read_to_string(e.path()).ok()?;
            parse_template(&content, TemplateSource::Workspace)
        })
        .collect()
}

/// Load all templates (bundled + workspace).
pub fn load_all_templates(workspace_root: Option<&Path>) -> Vec<Template> {
    let mut templates = load_bundled_templates();
    if let Some(root) = workspace_root {
        templates.extend(load_workspace_templates(root));
    }
    templates
}

// ── Instantiation ───────────────────────────────────────────────────────

/// Strip the `_template` block from YAML, producing a valid pipeline YAML.
///
/// This is a text-level operation — it removes the `_template:` block
/// (from `_template:` to the next top-level key) without parsing/re-serializing
/// the entire YAML, preserving formatting and comments.
pub fn strip_template_block(yaml: &str) -> String {
    let mut result = String::with_capacity(yaml.len());
    let mut in_template_block = false;

    for line in yaml.lines() {
        if line.starts_with("_template:") {
            in_template_block = true;
            continue;
        }

        if in_template_block {
            // Check if this line is a new top-level key (not indented)
            if !line.is_empty() && !line.starts_with(' ') && !line.starts_with('\t') {
                in_template_block = false;
                result.push_str(line);
                result.push('\n');
            }
            // Skip indented lines that are part of _template block
            continue;
        }

        result.push_str(line);
        result.push('\n');
    }

    // Trim leading blank lines
    result.trim_start_matches('\n').to_string()
}

/// All available format categories for gallery filtering.
pub const FORMAT_CATEGORIES: &[&str] = &["All", "CSV", "JSON", "XML", "Multi"];

/// Filter label matches a template's format category.
pub fn format_filter_matches(filter: &str, category: &str) -> bool {
    match filter {
        "All" => true,
        "CSV" => category == "csv",
        "JSON" => category == "json",
        "XML" => category == "xml",
        "Multi" => category == "other",
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_bundled_templates() {
        let templates = load_bundled_templates();
        assert!(
            templates.len() >= 6,
            "expected at least 6 bundled templates, got {}",
            templates.len()
        );

        let names: Vec<&str> = templates.iter().map(|t| t.metadata.name.as_str()).collect();
        assert!(names.contains(&"CSV Transform"));
        assert!(names.contains(&"Multi-Source Join"));
        assert!(names.contains(&"JSON Flatten"));
        assert!(names.contains(&"XML Extract"));
        assert!(names.contains(&"Full ETL Pipeline"));
    }

    #[test]
    fn test_bundled_template_bodies_parse_against_engine() {
        // Templates ship as valid pipeline YAML with a `_template` header. The
        // metadata tests only cover the header — this gate compiles the body
        // each template instantiates into, catching engine schema drift (e.g.
        // an option that moved from a source to an output struct) that would
        // otherwise surface only when a user instantiates the template.
        for (name, yaml) in BUNDLED_TEMPLATES {
            let body = strip_template_block(yaml);
            if let Err(errors) = crate::sync::parse_yaml(&body) {
                panic!(
                    "bundled template `{name}` body does not parse against the \
                     current engine: {errors:?}"
                );
            }
        }
    }

    /// Classification of a vendored example `.yaml` by the engine parser that
    /// owns its document shape. Each variant routes to a different parse entry
    /// point in [`test_vendored_examples_parse_against_engine`].
    enum ExampleKind {
        /// A full pipeline document (`pipeline:` + `nodes:`). Parsed via
        /// `parse_config` and additionally compiled via `PipelineConfig::compile`.
        Pipeline,
        /// A `*.comp.yaml` composition (`_compose:` + body `nodes:`), parsed via
        /// `CompositionFile::parse`.
        Composition,
        /// A `*.channel.yaml` per-tenant binding (`channel:` + `config`/`vars`),
        /// parsed via `ChannelBinding::from_yaml_bytes`.
        Channel,
        /// A `*.schema.yaml` source-schema overlay (`_schema:` + `fields:`),
        /// parsed via `clinker_schema::parse_schema`.
        Schema,
        /// A `channel.yaml` channel *manifest* (`_channel:` with `id`/`active`).
        /// This is a Klinx authoring convention with no engine parser — the
        /// engine's channel crate scans only `*.channel.yaml` bindings — so it
        /// is discovered for the coverage count but never parse-gated.
        Manifest,
    }

    /// Route a vendored example path to the engine parser that owns its shape,
    /// keyed off the filename suffix the IDE itself uses to classify documents.
    fn classify_example(path: &Path) -> ExampleKind {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if name == "channel.yaml" {
            ExampleKind::Manifest
        } else if name.ends_with(".comp.yaml") {
            ExampleKind::Composition
        } else if name.ends_with(".channel.yaml") {
            ExampleKind::Channel
        } else if name.ends_with(".schema.yaml") {
            ExampleKind::Schema
        } else {
            ExampleKind::Pipeline
        }
    }

    #[test]
    fn test_vendored_examples_parse_against_engine() {
        use std::num::NonZeroU32;

        use clinker_core_types::span::FileId;
        use clinker_plan::config::composition::CompositionFile;
        use clinker_plan::config::{CompileContext, parse_config};

        // Klinx ships an on-disk sample workspace at repo-root `examples/` so
        // users can Open Workspace → Open File on real pipeline YAML. This gate
        // walks the whole subtree and validates every document through the same
        // engine parser the IDE uses for that document kind, so a future engine
        // `rev` bump that drifts any schema breaks the build here rather than
        // silently shipping a sample that fails to open. `CARGO_MANIFEST_DIR` is
        // `<repo>/crates/klinx`; the workspace lives two levels up.
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/pipelines")
            .canonicalize()
            .expect("examples/pipelines should exist relative to crates/klinx");

        // Discover by walking — never a hardcoded file list — so a newly added
        // example is auto-covered without touching this test.
        fn collect(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
            for entry in fs::read_dir(dir)
                .expect("read_dir on examples subtree")
                .flatten()
            {
                let path = entry.path();
                if path.is_dir() {
                    collect(&path, out);
                } else if path.extension().and_then(|e| e.to_str()) == Some("yaml") {
                    out.push(path);
                }
            }
        }

        let mut files = Vec::new();
        collect(&root, &mut files);
        files.sort();

        // A non-windowed aggregate's empty-group fold and the workspace
        // composition scan both resolve paths relative to the workspace root;
        // pin it to the vendored tree so `compile` matches how the IDE would
        // compile these files in place.
        let ctx = CompileContext::new(root.clone());

        let mut failures = Vec::new();
        let mut counts = std::collections::BTreeMap::<&str, usize>::new();
        for path in &files {
            let label = path
                .strip_prefix(&root)
                .unwrap_or(path.as_path())
                .display()
                .to_string();
            let read = || {
                fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
            };
            match classify_example(path) {
                ExampleKind::Pipeline => {
                    *counts.entry("pipeline").or_default() += 1;
                    let yaml = read();
                    match parse_config(&yaml) {
                        Err(e) => failures.push(format!("{label}: parse: {e}")),
                        Ok(cfg) => {
                            if let Err(diags) = cfg.compile(&ctx) {
                                failures.push(format!("{label}: compile: {diags:?}"));
                            }
                        }
                    }
                }
                ExampleKind::Composition => {
                    *counts.entry("composition").or_default() += 1;
                    let yaml = read();
                    let file_id = FileId::new(NonZeroU32::new(1).expect("1 is non-zero"));
                    if let Err(e) = CompositionFile::parse(&yaml, file_id, path.clone()) {
                        failures.push(format!("{label}: composition parse: {e}"));
                    }
                }
                ExampleKind::Channel => {
                    *counts.entry("channel").or_default() += 1;
                    let bytes =
                        fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
                    if let Err(e) =
                        clinker_channel::ChannelBinding::from_yaml_bytes(&bytes, path.clone())
                    {
                        failures.push(format!("{label}: channel parse: {e}"));
                    }
                }
                ExampleKind::Schema => {
                    *counts.entry("schema").or_default() += 1;
                    let yaml = read();
                    if let Err(e) = clinker_schema::parse_schema(&yaml, path) {
                        failures.push(format!("{label}: schema parse: {e}"));
                    }
                }
                // Manifests have no engine parser; count for coverage only.
                ExampleKind::Manifest => *counts.entry("manifest").or_default() += 1,
            }
        }

        // A path mistake (wrong relative offset, renamed dir) must fail loudly
        // rather than vacuously pass on an empty set. Per-kind floors track the
        // vendored workspace's current contents; new examples only push these up.
        let floors = [
            ("pipeline", 9),
            ("composition", 5),
            ("channel", 3),
            ("schema", 2),
            ("manifest", 3),
        ];
        for (kind, floor) in floors {
            let got = counts.get(kind).copied().unwrap_or(0);
            assert!(
                got >= floor,
                "expected at least {floor} {kind} example(s) under {}, found {got} \
                 — check path resolution",
                root.display(),
            );
        }

        assert!(
            failures.is_empty(),
            "vendored example(s) failed to validate against the pinned engine:\n{}",
            failures.join("\n")
        );
    }

    #[test]
    fn test_strip_template_block() {
        let yaml = r#"_template:
  name: "Test"
  description: "A test template"
  tags: ["csv"]

pipeline:
  name: test

inputs:
  - name: source
    type: csv
    path: ./data/input.csv
"#;
        let stripped = strip_template_block(yaml);
        assert!(!stripped.contains("_template"));
        assert!(stripped.contains("pipeline:"));
        assert!(stripped.contains("inputs:"));
    }

    #[test]
    fn test_detect_format_category() {
        assert_eq!(detect_format_category("  type: csv\n"), "csv");
        assert_eq!(detect_format_category("  type: json\n"), "json");
        assert_eq!(detect_format_category("  type: xml\n"), "xml");
    }

    #[test]
    fn test_template_metadata_parsing() {
        let yaml = include_str!("templates/csv_transform.yaml");
        let template = parse_template(yaml, TemplateSource::Bundled).unwrap();
        assert_eq!(template.metadata.name, "CSV Transform");
        assert_eq!(template.metadata.tags, vec!["csv", "filter", "map"]);
        assert!(!template.metadata.hints.is_empty());
        assert_eq!(template.format_category, "csv");
    }

    #[test]
    fn test_format_filter_matches() {
        assert!(format_filter_matches("All", "csv"));
        assert!(format_filter_matches("CSV", "csv"));
        assert!(!format_filter_matches("CSV", "json"));
        assert!(format_filter_matches("JSON", "json"));
    }
}
