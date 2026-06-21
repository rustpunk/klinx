/// Per-tab data model: each open pipeline stores its state as plain data.
///
/// Signals live in `AppShell` (one set for the active tab). When switching
/// tabs, the active tab's signal values are snapshotted into the departing
/// `TabEntry`, and the arriving `TabEntry`'s snapshot is loaded into the
/// signals. This avoids Dioxus scope-ownership issues where signals created
/// in child components get dropped when the component unmounts.
use std::fmt;
use std::path::PathBuf;

use clinker_exec::partial::PartialPipelineConfig;
use clinker_plan::config::PipelineConfig;
use uuid::Uuid;

use crate::file_ops::compute_hash;
use crate::pipeline_view::PipelineView;
use crate::sync::{EditSource, is_composition_yaml, parse_composition, parse_yaml_raw_path};

/// Scaffold YAML for new untitled pipelines.
const SCAFFOLD_YAML: &str = r#"source:
  format: csv
  path: ""
stages: []
sink:
  format: csv
  path: ""
"#;

/// Stable identity for a tab — survives reordering and state changes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TabId(Uuid);

impl TabId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl fmt::Display for TabId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Snapshot of a tab's editable state (plain data, no signals).
#[derive(Clone)]
pub struct TabSnapshot {
    pub yaml_text: String,
    pub pipeline: Option<PipelineConfig>,
    /// Partial pipeline from graceful degradation (when full parse fails).
    pub partial_pipeline: Option<PartialPipelineConfig>,
    /// Derived DAG view when the document is a composition (`*.comp.yaml`);
    /// `None` for pipelines. Mutually exclusive with `pipeline`.
    pub composition_view: Option<PipelineView>,
    pub parse_errors: Vec<String>,
    pub edit_source: EditSource,
    pub selected_stage: Option<String>,
}

/// Parse a document as either a pipeline or a composition (by content), giving
/// back whichever model parsed plus any errors. Shared by the tab constructors;
/// the live editor uses the equivalent routing in `use_pipeline_sync`.
fn parse_document(yaml: &str) -> (Option<PipelineConfig>, Option<PipelineView>, Vec<String>) {
    if is_composition_yaml(yaml) {
        let (view, errors) = parse_composition(yaml);
        (None, view, errors)
    } else {
        match parse_yaml_raw_path(yaml) {
            Ok(config) => (Some(config), None, Vec::new()),
            Err(errors) => (None, None, errors),
        }
    }
}

/// One open pipeline tab with its file info and state snapshot.
pub struct TabEntry {
    pub id: TabId,
    /// `None` for unsaved / untitled tabs.
    pub file_path: Option<PathBuf>,
    /// Display name for untitled tabs.
    untitled_name: Option<String>,
    /// Blake3 hash of the YAML at last save/open. `None` for never-saved tabs.
    content_hash: Option<[u8; 32]>,
    /// The tab's current state (updated on tab-switch-away, read on switch-to).
    pub snapshot: TabSnapshot,
}

impl TabEntry {
    /// Create a new untitled tab with scaffold YAML.
    pub fn new_untitled(existing_tabs: &[TabEntry]) -> Self {
        let untitled_count = existing_tabs
            .iter()
            .filter(|t| t.file_path.is_none())
            .count();

        let name = if untitled_count == 0 {
            "untitled.yaml".to_string()
        } else {
            format!("untitled-{}.yaml", untitled_count + 1)
        };

        Self {
            id: TabId::new(),
            file_path: None,
            untitled_name: Some(name),
            content_hash: None,
            snapshot: TabSnapshot {
                yaml_text: SCAFFOLD_YAML.to_string(),
                pipeline: parse_yaml_raw_path(SCAFFOLD_YAML).ok(),
                partial_pipeline: None,
                composition_view: None,
                parse_errors: Vec::new(),
                edit_source: EditSource::None,
                selected_stage: None,
            },
        }
    }

    /// Create a tab from a file on disk.
    pub fn from_file(path: PathBuf, yaml: String) -> Self {
        let hash = compute_hash(&yaml);
        let (config, composition_view, errors) = parse_document(&yaml);

        Self {
            id: TabId::new(),
            file_path: Some(path),
            untitled_name: None,
            content_hash: Some(hash),
            snapshot: TabSnapshot {
                yaml_text: yaml,
                pipeline: config,
                partial_pipeline: None,
                composition_view,
                parse_errors: errors,
                edit_source: EditSource::None,
                selected_stage: None,
            },
        }
    }

    /// Create a new untitled tab pre-loaded with given YAML content.
    ///
    /// Used for template instantiation — the tab opens dirty (unsaved)
    /// with the template content ready for editing.
    pub fn new_from_yaml(existing_tabs: &[TabEntry], yaml: String) -> Self {
        let untitled_count = existing_tabs
            .iter()
            .filter(|t| t.file_path.is_none())
            .count();

        let name = if untitled_count == 0 {
            "untitled.yaml".to_string()
        } else {
            format!("untitled-{}.yaml", untitled_count + 1)
        };

        let (config, composition_view, errors) = parse_document(&yaml);

        Self {
            id: TabId::new(),
            file_path: None,
            untitled_name: Some(name),
            content_hash: None,
            snapshot: TabSnapshot {
                yaml_text: yaml,
                pipeline: config,
                partial_pipeline: None,
                composition_view,
                parse_errors: errors,
                edit_source: EditSource::None,
                selected_stage: None,
            },
        }
    }

    /// Whether the tab has unsaved changes.
    pub fn is_dirty(&self) -> bool {
        let Some(saved_hash) = self.content_hash else {
            return true;
        };
        let current = compute_hash(&self.snapshot.yaml_text);
        current != saved_hash
    }

    /// Mark the current YAML as saved.
    pub fn mark_saved(&mut self, path: PathBuf, yaml: &str) {
        self.content_hash = Some(compute_hash(yaml));
        self.file_path = Some(path);
    }

    /// Display name for the tab label.
    pub fn display_name(&self) -> String {
        match &self.file_path {
            Some(p) => p
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "untitled.yaml".to_string()),
            None => self
                .untitled_name
                .clone()
                .unwrap_or_else(|| "untitled.yaml".to_string()),
        }
    }

    /// Full file path as a string (for tooltips).
    pub fn full_path(&self) -> Option<String> {
        self.file_path.as_ref().map(|p| p.display().to_string())
    }
}

/// File path of the active tab, if any.
///
/// Shared by the compile effect's active-file memo (`use_compiled_plan`) and
/// session persistence (`workspace::save_full_session`), which both need the
/// active tab's `file_path` and previously duplicated this lookup.
pub fn active_tab_file_path(tabs: &[TabEntry], active: Option<TabId>) -> Option<&PathBuf> {
    let id = active?;
    tabs.iter()
        .find(|t| t.id == id)
        .and_then(|t| t.file_path.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_tab_file_path_finds_the_active_tabs_path() {
        let path = PathBuf::from("/ws/flow.yaml");
        let file_tab = TabEntry::from_file(path.clone(), String::new());
        let active = file_tab.id;
        let tabs = vec![TabEntry::new_untitled(&[]), file_tab];
        assert_eq!(active_tab_file_path(&tabs, Some(active)), Some(&path));
    }

    #[test]
    fn active_tab_file_path_is_none_when_no_tab_is_active() {
        let tabs = vec![TabEntry::new_untitled(&[])];
        assert_eq!(active_tab_file_path(&tabs, None), None);
    }

    #[test]
    fn active_tab_file_path_is_none_for_an_unsaved_active_tab() {
        let untitled = TabEntry::new_untitled(&[]);
        let active = untitled.id;
        let tabs = vec![untitled];
        assert_eq!(active_tab_file_path(&tabs, Some(active)), None);
    }
}
