/// App-level reactive state and context types.
///
/// `AppState` is the per-tab context consumed by all downstream components.
/// Its shape is unchanged from the single-pipeline era — components don't
/// know about tabs.
///
/// `TabManagerState` is the global context for tab/file operations.
///
/// Navigation uses a two-level model:
/// - `NavigationContext` — top-level activity (Pipeline, Channels, Git, Docs, Runs)
/// - `PipelineLayoutMode` — view within Pipeline context only (Canvas, Hybrid, Editor)
use std::path::PathBuf;
use std::sync::Arc;

use clinker_exec::partial::PartialPipelineConfig;
use clinker_plan::config::PipelineConfig;
use clinker_plan::plan::CompiledPlan;
use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

use crate::pipeline_view::{
    PipelineView, derive_body_view, derive_partial_pipeline_view, derive_pipeline_view,
    derive_resolved_pipeline_view,
};
use crate::sync::EditSource;
use crate::tab::{TabEntry, TabId};
use crate::workspace::Workspace;

/// Visual theme — switches the entire UI between sub-aesthetics.
///
/// Oxide is the default dark Rustpunk theme (warm charred surfaces).
/// Enamel is a light industrial theme (porcelain-fused-to-steel data plates).
/// Arc is the dark high-contrast "electrified" sub-aesthetic (cold blue-black
/// ground under glowing ember/patina neons).
/// The active theme drives CSS custom property overrides via `data-theme` on the root element.
#[derive(Clone, Copy, PartialEq, Debug, Default, Serialize, Deserialize)]
pub enum KilnTheme {
    #[default]
    Oxide,
    Enamel,
    Arc,
}

impl KilnTheme {
    /// CSS `data-theme` attribute value.
    pub fn as_data_attr(self) -> &'static str {
        match self {
            Self::Oxide => "oxide",
            Self::Enamel => "enamel",
            Self::Arc => "arc",
        }
    }

    /// Cycle to the next theme (Oxide → Enamel → Arc → Oxide).
    pub fn toggle(self) -> Self {
        match self {
            Self::Oxide => Self::Enamel,
            Self::Enamel => Self::Arc,
            Self::Arc => Self::Oxide,
        }
    }

    /// Parse from persisted string, defaulting to Oxide.
    pub fn from_str_or_default(s: &str) -> Self {
        match s {
            "enamel" => Self::Enamel,
            "arc" => Self::Arc,
            _ => Self::Oxide,
        }
    }
}

/// Which left-side panel is currently open (280px slide-in slot).
///
/// Only one panel can be open at a time. Explorer, Search, Schemas, and
/// Compositions share the same slot. `None` means the slot is collapsed.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum LeftPanel {
    #[default]
    None,
    /// Workspace file explorer (discovered tree / raw filesystem).
    Explorer,
    Search,
    Schemas,
    Compositions,
}

impl LeftPanel {
    /// Toggle `target` against the current panel: show `target`, or collapse the
    /// slot if `target` is already showing. Shared by the keyboard, command
    /// palette, and activity-bar toggles so the open/close rule lives once.
    pub fn toggled(self, target: LeftPanel) -> LeftPanel {
        if self == target {
            LeftPanel::None
        } else {
            target
        }
    }
}

/// Top-level navigation context — the activity the user is performing.
///
/// Each context is a distinct page with its own layout, content, and purpose.
/// Switching contexts changes *what you're doing*. State is preserved per
/// context when switching away.
#[derive(Clone, Copy, PartialEq, Debug, Default, Serialize, Deserialize)]
pub enum NavigationContext {
    /// Pipeline editing — canvas, YAML editor, inspector, compositions.
    #[default]
    Pipeline,
    /// Channel management — identity card, pipeline override grid, health.
    Channels,
    /// Version control — staged/unstaged files, diff view, commit form.
    Git,
    /// Documentation — Klinx user guide, CXL reference, pipeline authoring docs.
    Docs,
    /// Run history — chronological run list, filterable, expandable entries.
    Runs,
}

impl NavigationContext {
    /// CSS data attribute value for the content area.
    pub fn as_data_attr(self) -> &'static str {
        match self {
            Self::Pipeline => "pipeline",
            Self::Channels => "channels",
            Self::Git => "git",
            Self::Docs => "docs",
            Self::Runs => "runs",
        }
    }

    /// Full display label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Pipeline => "Pipeline",
            Self::Channels => "Channels",
            Self::Git => "Version Control",
            Self::Docs => "Technical Guide",
            Self::Runs => "Run History",
        }
    }

    /// Short label for the activity bar (max 4 chars).
    pub fn short_label(self) -> &'static str {
        match self {
            Self::Pipeline => "Pipe",
            Self::Channels => "Chan",
            Self::Git => "Git",
            Self::Docs => "Guide",
            Self::Runs => "Runs",
        }
    }

    /// Unicode icon character for the activity bar.
    pub fn icon_char(self) -> &'static str {
        match self {
            Self::Pipeline => "◇",
            Self::Channels => "◈",
            Self::Git => "⟠",
            Self::Docs => "◆",
            Self::Runs => "◎",
        }
    }

    /// Keyboard shortcut hint for display.
    pub fn keyboard_hint(self) -> &'static str {
        match self {
            Self::Pipeline => "Ctrl+Shift+E",
            Self::Channels => "Ctrl+Shift+C",
            Self::Git => "Ctrl+Shift+G",
            Self::Docs => "",
            Self::Runs => "Ctrl+Shift+R",
        }
    }

    /// All contexts in display order.
    pub const ALL: [NavigationContext; 5] = [
        Self::Pipeline,
        Self::Channels,
        Self::Git,
        Self::Docs,
        Self::Runs,
    ];
}

/// Pipeline view mode — how you see the pipeline content.
///
/// Only applies within the Pipeline context. Switching layout modes changes
/// *how you see* the pipeline, not what you're doing. All three modes share
/// the same state (selected transformation, cursor position, inspector).
#[derive(Clone, Copy, PartialEq, Debug, Default, Serialize, Deserialize)]
pub enum PipelineLayoutMode {
    /// Canvas takes full width, YAML sidebar hidden.
    Canvas,
    /// Canvas ~62% + YAML sidebar ~38% (360px). Primary authoring mode.
    #[default]
    Hybrid,
    /// YAML editor takes full width, canvas and inspector hidden.
    Editor,
    /// Pipeline autodoc — full scrollable documentation view (Blueprint aesthetic).
    Schematics,
}

impl PipelineLayoutMode {
    /// CSS data attribute value for layout switching.
    pub fn as_data_attr(self) -> &'static str {
        match self {
            Self::Canvas => "canvas",
            Self::Hybrid => "hybrid",
            Self::Editor => "editor",
            Self::Schematics => "schematics",
        }
    }

    /// Display label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Canvas => "Canvas",
            Self::Hybrid => "Hybrid",
            Self::Editor => "Editor",
            Self::Schematics => "Schematics",
        }
    }

    /// All layout modes in display order.
    pub const ALL: [PipelineLayoutMode; 4] =
        [Self::Canvas, Self::Hybrid, Self::Editor, Self::Schematics];
}

/// Canvas data source mode — switches between raw pipeline config and
/// channel-resolved compiled plan.
///
/// `Raw` shows the pipeline as authored (PipelineConfig from YAML).
/// `Resolved` shows the pipeline after channel overlay application
/// (CompiledPlan with provenance-tracked config values).
///
/// Orthogonal to composition drill depth — a user can be drilled into
/// a composition at depth 2 and toggle between Raw and Resolved.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ChannelViewMode {
    /// Show pipeline config from YAML (state.pipeline).
    #[default]
    Raw,
    /// Show resolved config from channel overlay (compiled_plan).
    Resolved,
}

/// How a field-lineage reveal treats the off-path graph (#123).
///
/// A reveal (hovering or selecting a field column) computes that column's lineage
/// closure. This mode decides what happens to the nodes/edges OUTSIDE that
/// closure while the reveal is active:
///
/// - `Highlight` keeps the FULL graph and DIMS the off-path cards/cables, so the
///   surrounding context stays visible. This is the long-standing default and the
///   only behavior before #123 — formalized behind the mode without changing its
///   effect.
/// - `Filter` HIDES off-path cards (and any edge with a hidden endpoint) so only
///   the connecting paths remain, keeping a large lineage closure readable. Every
///   connecting path stays intact — an edge is drawn only when BOTH endpoints
///   survive, so no dangling partial path is left behind.
///
/// Stored as a per-tab signal on [`AppState`] so the canvas reveal logic and the
/// toolbar toggle share one source of truth. PR5 (#152) reuses this exact state
/// for its persistent focus toggle, so the mode lives here rather than as
/// canvas-local component state.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum LineageRevealMode {
    /// Keep the full graph; dim everything outside the active lineage closure.
    #[default]
    Highlight,
    /// Hide everything outside the active lineage closure; keep connecting paths.
    Filter,
}

impl LineageRevealMode {
    /// CSS `data-reveal-mode` attribute value for the canvas container.
    pub fn as_data_attr(self) -> &'static str {
        match self {
            Self::Highlight => "highlight",
            Self::Filter => "filter",
        }
    }

    /// Short display label for the toolbar toggle.
    pub fn label(self) -> &'static str {
        match self {
            Self::Highlight => "HIGHLIGHT",
            Self::Filter => "FILTER",
        }
    }

    /// The opposite mode — the toolbar toggle flips between the two.
    pub fn toggled(self) -> Self {
        match self {
            Self::Highlight => Self::Filter,
            Self::Filter => Self::Highlight,
        }
    }
}

/// A frame in the composition drill-in stack.
///
/// Each frame represents one level of drill-in. The body_id indexes into
/// `CompileArtifacts::composition_bodies` to get the sub-canvas nodes.
#[derive(Clone, Debug, PartialEq)]
pub struct CompositionDrillFrame {
    /// Body ID for the composition's bound body (not an alias prefix string).
    pub body_id: clinker_plan::plan::composition_body::CompositionBodyId,
    /// Display label for the breadcrumb (composition alias or name).
    pub alias: String,
    /// Path to the `.comp.yaml` file (for display in breadcrumb tooltip).
    pub use_path: std::path::PathBuf,
}

/// A selected output field on the currently visible canvas.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectedField {
    pub stage_id: String,
    pub field_name: String,
}

impl SelectedField {
    pub fn new(stage_id: impl Into<String>, field_name: impl Into<String>) -> Self {
        Self {
            stage_id: stage_id.into(),
            field_name: field_name.into(),
        }
    }
}

/// Per-tab reactive state — consumed by canvas, inspector, YAML sidebar, etc.
///
/// Downstream components call `use_context::<AppState>()` and get the
/// active tab's signals transparently.
#[derive(Clone, Copy)]
pub struct AppState {
    pub active_context: Signal<NavigationContext>,
    pub pipeline_layout: Signal<PipelineLayoutMode>,
    pub run_log_expanded: Signal<bool>,
    pub selected_stages: Signal<std::collections::HashSet<String>>,
    pub selected_field: Signal<Option<SelectedField>>,
    /// Raw YAML text shown in the sidebar editor.
    pub yaml_text: Signal<String>,
    /// Parsed pipeline config (None if YAML is invalid).
    pub pipeline: Signal<Option<PipelineConfig>>,
    /// Partial pipeline from graceful degradation.
    pub partial_pipeline: Signal<Option<PartialPipelineConfig>>,
    /// Derived DAG view for a composition document (`*.comp.yaml`). `Some` only
    /// when the active tab is a composition; the canvas renders this in place of
    /// the pipeline view. Kept as a derived view (not a `CompositionFile`) so the
    /// canvas stays uniform — see [`crate::pipeline_view::derive_composition_view`].
    pub composition_view: Signal<Option<PipelineView>>,
    /// Parse error messages (empty when YAML is valid).
    ///
    /// Raw, immediate parse output: updated on every debounced parse (~150ms)
    /// and consumed by the canvas, inspector, and the title-bar validity LED.
    /// The YAML error bar renders `visible_errors`, not this — see #43.
    pub parse_errors: Signal<Vec<String>>,
    /// Parse errors as displayed in the YAML sidebar's error bar.
    ///
    /// Decoupled from `parse_errors` to stop the bar flickering while the user
    /// is mid-keystroke (issue #43). For typing (`EditSource::Yaml`) it tracks
    /// `parse_errors` only after a ~500ms idle settle; for non-typing
    /// transitions (tab switch, file open, inspector edits) it tracks
    /// `parse_errors` immediately. Holds its last value while typing, so the
    /// bar never pops up or churns between keystrokes.
    pub visible_errors: Signal<Vec<String>>,
    /// Which view last edited the model (sync loop prevention).
    pub edit_source: Signal<EditSource>,
    /// Schema validation warnings for the current pipeline.
    pub schema_warnings: Signal<Vec<SchemaWarning>>,
    /// Canvas data source mode (Raw = pipeline config, Resolved = compiled plan).
    pub channel_view_mode: Signal<ChannelViewMode>,
    /// How a field-lineage reveal treats the off-path graph: dim it
    /// (`Highlight`, the default) or hide it (`Filter`). Shared by the canvas
    /// reveal logic and the toolbar toggle; reused by PR5's persistent focus
    /// toggle (#123).
    pub lineage_reveal_mode: Signal<LineageRevealMode>,
    /// Composition drill-in stack. Empty = top-level pipeline view.
    /// Each frame holds a body ID for rendering the sub-canvas.
    pub composition_drill_stack: Signal<Vec<CompositionDrillFrame>>,
    /// Compiled plan with channel overlay applied. None when no channel is
    /// loaded or in Raw mode. Wrapped in Arc because CompiledPlan is not Clone.
    pub compiled_plan: Signal<Option<Arc<CompiledPlan>>>,
}

/// Read the current `AppState` from context.
///
/// The context holds a `Signal<AppState>` which is updated when the active
/// tab changes. This helper reads through the signal so callers get the
/// current tab's state.
pub fn use_app_state() -> AppState {
    let sig = use_context::<Signal<AppState>>();
    *sig.read()
}

/// Derive the DAG view currently shown by the canvas.
///
/// This is intentionally layout-free: callers that render cards/connectors can
/// apply their layout engine afterward, while inspectors can read the same
/// stage and lineage model without depending on canvas geometry.
pub fn current_pipeline_view(state: AppState) -> PipelineView {
    if let Some(comp_view) = state.composition_view.read().clone() {
        return comp_view;
    }

    let drill_stack = state.composition_drill_stack.read();
    if let Some(frame) = drill_stack.last() {
        let compiled_guard = state.compiled_plan.read();
        return compiled_guard
            .as_ref()
            .and_then(|plan| plan.body_of(frame.body_id))
            .map(derive_body_view)
            .unwrap_or_default();
    }
    drop(drill_stack);

    match *state.channel_view_mode.read() {
        ChannelViewMode::Resolved => {
            let compiled_guard = state.compiled_plan.read();
            match compiled_guard.as_ref() {
                Some(plan) => derive_resolved_pipeline_view(plan),
                None => match &*state.pipeline.read() {
                    Some(config) => derive_pipeline_view(config),
                    None => PipelineView::default(),
                },
            }
        }
        ChannelViewMode::Raw => match &*state.pipeline.read() {
            Some(config) => derive_pipeline_view(config),
            None => match &*state.partial_pipeline.read() {
                Some(partial) => derive_partial_pipeline_view(partial),
                None => PipelineView::default(),
            },
        },
    }
}

/// Global tab management context — used by tab bar, title bar, keyboard handlers.
#[derive(Clone, Copy)]
pub struct TabManagerState {
    pub tabs: Signal<Vec<TabEntry>>,
    pub active_tab_id: Signal<Option<TabId>>,
    pub workspace: Signal<Option<Workspace>>,
    /// Which left panel is currently open (Search or Schemas).
    pub left_panel: Signal<LeftPanel>,
    /// Workspace schema index — populated on workspace load, refreshed on file changes.
    pub schema_index: Signal<SchemaIndex>,
    /// Whether the template gallery overlay is visible.
    pub show_template_gallery: Signal<bool>,
    /// Git repository status — branch, ahead/behind, file changes.
    pub git_state: Signal<Option<klinx_git::RepoStatus>>,
    /// Whether the command palette overlay is visible.
    pub show_command_palette: Signal<bool>,
    /// Whether the settings overlay is visible.
    pub show_settings: Signal<bool>,
    /// Whether the activity bar is visible (focus mode toggle).
    pub activity_bar_visible: Signal<bool>,
    /// Navigation history stack for back-navigation (capped at 50).
    pub nav_history: Signal<Vec<NavigationContext>>,
    /// Channel state discovered from clinker.toml (None if no clinker.toml found).
    pub channel_state: Signal<Option<ChannelState>>,
    /// Active visual theme (Oxide dark / Enamel light).
    pub theme: Signal<KilnTheme>,
}

use clinker_schema::{SchemaIndex, SchemaWarning};

// ── Channel state ──────────────────────────────────────────────────────

/// Discovered channel workspace state based on the ChannelBinding model.
///
/// Populated when a workspace has `.channel.yaml` files in its channels
/// directory. None when no workspace is loaded or no channels are found.
#[derive(Clone, Debug, PartialEq)]
pub struct ChannelState {
    /// Discovered channel binding summaries.
    pub channels: Vec<ChannelBindingSummary>,
    /// Currently selected channel name (None = run base pipeline).
    pub active_channel: Option<String>,
    /// Recently selected channel names (most recent first, max 10).
    pub recent_channels: Vec<String>,
}

/// Summary of a discovered `.channel.yaml` binding file.
#[derive(Clone, Debug, PartialEq)]
pub struct ChannelBindingSummary {
    /// Channel name (from the binding's `name` field).
    pub name: String,
    /// Path to the `.channel.yaml` file on disk.
    pub source_path: PathBuf,
    /// Display string for the channel's target (pipeline or composition path).
    pub target: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #123: the reveal mode defaults to `Highlight` — the long-standing behavior
    /// before the mode existed — so formalizing it behind the mode does not change
    /// the out-of-the-box reveal.
    #[test]
    fn lineage_reveal_mode_defaults_to_highlight() {
        assert_eq!(LineageRevealMode::default(), LineageRevealMode::Highlight);
    }

    /// #123: the toolbar toggle flips between exactly the two modes and is its own
    /// inverse, so a double-click returns to the start. PR5 reuses this toggle for
    /// its persistent focus control, so the round-trip contract is load-bearing.
    #[test]
    fn lineage_reveal_mode_toggle_is_involution() {
        assert_eq!(
            LineageRevealMode::Highlight.toggled(),
            LineageRevealMode::Filter
        );
        assert_eq!(
            LineageRevealMode::Filter.toggled(),
            LineageRevealMode::Highlight
        );
        for mode in [LineageRevealMode::Highlight, LineageRevealMode::Filter] {
            assert_eq!(mode.toggled().toggled(), mode, "toggling twice is identity");
        }
    }

    /// #123: each mode carries a DISTINCT, slug-safe `data-reveal-mode` attribute
    /// and a distinct label, so CSS keyed off the attribute and the toolbar label
    /// can tell the two apart.
    #[test]
    fn lineage_reveal_mode_attrs_and_labels_are_distinct() {
        assert_ne!(
            LineageRevealMode::Highlight.as_data_attr(),
            LineageRevealMode::Filter.as_data_attr()
        );
        assert_ne!(
            LineageRevealMode::Highlight.label(),
            LineageRevealMode::Filter.label()
        );
        for mode in [LineageRevealMode::Highlight, LineageRevealMode::Filter] {
            let attr = mode.as_data_attr();
            assert!(
                !attr.is_empty() && attr.chars().all(|c| c.is_ascii_lowercase()),
                "{mode:?} data attr must be a lowercase slug, got {attr:?}",
            );
        }
    }

    /// The default theme is Oxide — the dark Rustpunk surface the app boots into.
    #[test]
    fn kiln_theme_defaults_to_oxide() {
        assert_eq!(KilnTheme::default(), KilnTheme::Oxide);
    }

    /// The quick toggle cycles through all three themes and returns to the start
    /// after one full lap, so the status-bar click and Ctrl+Shift+T reach Arc.
    #[test]
    fn kiln_theme_toggle_cycles_all_three() {
        assert_eq!(KilnTheme::Oxide.toggle(), KilnTheme::Enamel);
        assert_eq!(KilnTheme::Enamel.toggle(), KilnTheme::Arc);
        assert_eq!(KilnTheme::Arc.toggle(), KilnTheme::Oxide);
        for theme in [KilnTheme::Oxide, KilnTheme::Enamel, KilnTheme::Arc] {
            assert_eq!(
                theme.toggle().toggle().toggle(),
                theme,
                "three toggles is identity",
            );
        }
    }

    /// Each theme carries a DISTINCT, slug-safe `data-theme` attribute so the CSS
    /// keyed off `[data-theme="…"]` can tell the three surfaces apart.
    #[test]
    fn kiln_theme_attrs_are_distinct_slugs() {
        let attrs = [
            KilnTheme::Oxide.as_data_attr(),
            KilnTheme::Enamel.as_data_attr(),
            KilnTheme::Arc.as_data_attr(),
        ];
        for attr in attrs {
            assert!(
                !attr.is_empty() && attr.chars().all(|c| c.is_ascii_lowercase()),
                "data attr must be a lowercase slug, got {attr:?}",
            );
        }
        let unique: std::collections::HashSet<_> = attrs.iter().collect();
        assert_eq!(unique.len(), attrs.len(), "data attrs must be distinct");
    }

    /// Persisted theme strings round-trip through `from_str_or_default`, and
    /// unknown values fall back to Oxide.
    #[test]
    fn kiln_theme_from_str_round_trips() {
        for theme in [KilnTheme::Oxide, KilnTheme::Enamel, KilnTheme::Arc] {
            assert_eq!(KilnTheme::from_str_or_default(theme.as_data_attr()), theme);
        }
        assert_eq!(
            KilnTheme::from_str_or_default("bogus"),
            KilnTheme::Oxide,
            "unknown theme falls back to Oxide",
        );
    }

    /// The theme survives a serde round-trip so it can persist in workspace.json.
    #[test]
    fn kiln_theme_serde_round_trips() {
        for theme in [KilnTheme::Oxide, KilnTheme::Enamel, KilnTheme::Arc] {
            let json = serde_json::to_string(&theme).unwrap();
            let restored: KilnTheme = serde_json::from_str(&json).unwrap();
            assert_eq!(theme, restored);
        }
    }
}
