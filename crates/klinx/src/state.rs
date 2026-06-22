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

/// Resolve a composition node name to a drill/overlay frame against a compiled
/// plan, or `None` when the plan has no body assignment for that node.
///
/// Shared by the full-swap drill (`composition_drill_stack`) and the in-context
/// overlay (`composition_overlay_stack`, #171): both navigate into the SAME body
/// for a given call-site, so they resolve the frame identically and only differ
/// in which stack they push it onto. Kept as a free function so the resolution
/// (body-id lookup + `use_path` read) is unit-testable without a Dioxus runtime.
pub fn resolve_composition_frame(
    plan: &CompiledPlan,
    node_name: &str,
) -> Option<CompositionDrillFrame> {
    let &body_id = plan
        .artifacts()
        .composition_body_assignments
        .get(node_name)?;
    let use_path = plan
        .body_of(body_id)
        .map(|b| b.signature_path.clone())
        .unwrap_or_default();
    Some(CompositionDrillFrame {
        body_id,
        alias: node_name.to_string(),
        use_path,
    })
}

/// Move every composition frame from one navigation stack onto another,
/// preserving order and any frames already on the target, and empty the source.
///
/// The three composition view-mode stacks — full-swap `composition_drill_stack`,
/// lightbox `composition_overlay_stack`, and corner-inset `composition_pip_stack`
/// (#171 Phase 2) — are mutually exclusive for a given navigation: transitioning
/// between modes hands the frames from one to the next at the same depth. Every
/// transition (overlay→drill "OPEN FULL", overlay→pip "dock to corner",
/// pip→overlay "expand", pip→drill) is this same move, so it lives in one pure,
/// unit-testable transform over the two frame vectors.
pub fn move_composition_frames(
    from: &mut Vec<CompositionDrillFrame>,
    to: &mut Vec<CompositionDrillFrame>,
) {
    to.append(from);
}

/// Promote the in-context overlay frames to the full-swap drill stack (#171).
///
/// The overlay's "OPEN FULL" escape hatch: the user has navigated some depth
/// inside the overlay and wants the classic full-canvas drill at that same depth.
/// A thin alias over [`move_composition_frames`] kept for call-site clarity.
pub fn promote_overlay_to_drill(
    overlay: &mut Vec<CompositionDrillFrame>,
    drill: &mut Vec<CompositionDrillFrame>,
) {
    move_composition_frames(overlay, drill);
}

/// Toggle a composition node's in-place explode (#171 Phase 3): insert its name
/// if absent (explode), remove it if present (collapse). A free function over the
/// set so the toggle is one place and unit-testable without a Dioxus runtime.
pub fn toggle_composition_explode(set: &mut std::collections::HashSet<String>, node_name: &str) {
    if !set.remove(node_name) {
        set.insert(node_name.to_string());
    }
}

/// A composition-binding diagnostic surfaced to the user (#187).
///
/// A composition-binding or hard compile diagnostic surfaced from the compile
/// hook, so a degraded or failed compile is never a silent no-op —
/// indistinguishable from "this node has no composition body". The canvas and
/// inspector key off `node` to flag the offending node; the YAML error bar shows
/// the `node: None` (pipeline-level) entries. Two paths produce these:
///
/// - **Non-fatal binding failure (#187):** a `composition` node's `use:` fails to
///   bind (engine codes E101–E109). The engine drops that node's body from the
///   compiled DAG non-fatally (its gate keeps any `"E10"`-prefixed error and omits
///   the node) but the plan still compiles. Always node-attributed.
/// - **Hard compile failure (#189):** the compile returns `Err` (any
///   error-severity code not starting `"E10"` — e.g. E111 empty-body, E200 type
///   error, E153). There is no plan; the error-severity diagnostics are surfaced
///   so the resolved/compiled tooling does not go dark unexplained.
///
/// - `node` is the offending composition node's name; `None` for a pipeline-level
///   diagnostic not attributable to a single composition node (a general hard
///   compile error).
/// - `code` is the engine diagnostic code (e.g. `"E103"`); empty for a synthesized
///   fallback entry produced when a dropped node had no engine message naming it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompositionDiagnostic {
    pub node: Option<String>,
    pub code: String,
    pub message: String,
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
    /// Composition body OVERLAY stack (#171 Phase 1). Empty = no overlay open.
    ///
    /// Parallel to `composition_drill_stack`: the parent canvas keeps rendering
    /// its top-level view while the in-context "lightbox" overlay independently
    /// renders the body of `last().body_id` over a dimmed/blurred parent. The
    /// in-overlay breadcrumb pushes/truncates THIS stack; the overlay's "OPEN
    /// FULL" escape hatch moves these frames into `composition_drill_stack` and
    /// clears this one (the existing full-swap drill). Reuses
    /// `CompositionDrillFrame` verbatim — a frame means the same thing whether it
    /// is shown in the overlay or in the full-swap canvas.
    pub composition_overlay_stack: Signal<Vec<CompositionDrillFrame>>,
    /// Composition body PICTURE-IN-PICTURE stack (#171 Phase 2). Empty = no inset
    /// open.
    ///
    /// The non-modal sibling of `composition_overlay_stack`: rather than a modal
    /// lightbox over a dimmed parent, the body renders in a small pinned corner
    /// panel while the parent canvas stays **fully interactive**. The overlay's
    /// "dock to corner" button moves its frames here (and the inset's "expand"
    /// button moves them back) via [`move_composition_frames`]; "open full"
    /// promotes to `composition_drill_stack`. Reuses `CompositionDrillFrame`
    /// verbatim — a frame means the same body whether shown full-swap, in the
    /// lightbox, or in the inset.
    pub composition_pip_stack: Signal<Vec<CompositionDrillFrame>>,
    /// Compiled plan with channel overlay applied. None when no channel is
    /// loaded or in Raw mode. Wrapped in Arc because CompiledPlan is not Clone.
    pub compiled_plan: Signal<Option<Arc<CompiledPlan>>>,
    /// Composition-binding and hard compile diagnostics from the last compile
    /// (#187, #189). Empty when every `composition` node bound cleanly and the
    /// pipeline compiled (or there was nothing to compile). Populated by
    /// `use_compiled_plan`: on a successful compile the node-attributed binding
    /// failures ride alongside `compiled_plan`; on a hard compile failure the plan
    /// is cleared but the error-severity diagnostics are still set here. The canvas
    /// flags node-attributed entries and the inspector lists the reason; the YAML
    /// error bar shows the pipeline-level (`node: None`) entries. See
    /// [`CompositionDiagnostic`].
    pub composition_diagnostics: Signal<Vec<CompositionDiagnostic>>,
    /// Names of the `composition` nodes currently EXPLODED in place on the main
    /// canvas (#171 Phase 3). Unlike the single-active overlay/pip stacks, this is
    /// a SET: several compositions can be exploded at once — each renders its body
    /// as a mini-DAG embedded at the node's position, with siblings reflowing
    /// around the enlarged footprint. Empty = nothing exploded. Toggled per node
    /// by [`toggle_composition_explode`]; cleared on a view swap.
    pub composition_explode_set: Signal<std::collections::HashSet<String>>,
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

    /// #171 Phase 3: the explode toggle inserts a node on first click and removes
    /// it on the next, and several nodes can be exploded at once (a SET, unlike the
    /// single-active overlay/pip stacks).
    #[test]
    fn toggle_composition_explode_inserts_then_removes_and_allows_many() {
        use std::collections::HashSet;
        let mut set: HashSet<String> = HashSet::new();
        toggle_composition_explode(&mut set, "clean");
        assert!(set.contains("clean"), "first toggle explodes the node");
        toggle_composition_explode(&mut set, "clean");
        assert!(!set.contains("clean"), "second toggle collapses it");

        toggle_composition_explode(&mut set, "a");
        toggle_composition_explode(&mut set, "b");
        assert_eq!(
            set.len(),
            2,
            "independent nodes can be exploded simultaneously"
        );
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

    fn frame(alias: &str, id: u32) -> CompositionDrillFrame {
        CompositionDrillFrame {
            body_id: clinker_plan::plan::composition_body::CompositionBodyId(id),
            alias: alias.to_string(),
            use_path: std::path::PathBuf::from(format!("./{alias}.comp.yaml")),
        }
    }

    /// #171: "OPEN FULL" moves every overlay frame onto the drill stack in order
    /// and empties the overlay stack, so the full-swap canvas opens at exactly the
    /// depth the user reached inside the overlay.
    #[test]
    fn promote_overlay_moves_frames_in_order_and_clears_overlay() {
        let mut overlay = vec![frame("outer", 1), frame("inner", 2)];
        let mut drill = Vec::new();
        promote_overlay_to_drill(&mut overlay, &mut drill);
        assert!(overlay.is_empty(), "overlay stack is emptied");
        assert_eq!(
            drill.iter().map(|f| f.alias.as_str()).collect::<Vec<_>>(),
            vec!["outer", "inner"],
            "frames keep their drill-in order",
        );
    }

    /// #171: promotion APPENDS — any frames already on the drill stack are kept
    /// ahead of the promoted overlay frames (defensive; the stacks are mutually
    /// exclusive in practice, but the transform must not drop frames).
    #[test]
    fn promote_overlay_appends_to_existing_drill_frames() {
        let mut overlay = vec![frame("b", 2)];
        let mut drill = vec![frame("a", 1)];
        promote_overlay_to_drill(&mut overlay, &mut drill);
        assert_eq!(
            drill.iter().map(|f| f.alias.as_str()).collect::<Vec<_>>(),
            vec!["a", "b"],
        );
    }

    /// #171: an empty overlay promotes to a no-op — clicking "OPEN FULL" with no
    /// overlay open leaves the drill stack untouched.
    #[test]
    fn promote_empty_overlay_is_a_no_op() {
        let mut overlay: Vec<CompositionDrillFrame> = Vec::new();
        let mut drill = vec![frame("a", 1)];
        promote_overlay_to_drill(&mut overlay, &mut drill);
        assert_eq!(drill.len(), 1);
    }

    /// #171 Phase 2: "dock to corner" moves the lightbox overlay frames onto the
    /// PiP stack at the same depth and empties the overlay; "expand" round-trips
    /// them back. The generic [`move_composition_frames`] backs every transition.
    #[test]
    fn dock_and_expand_round_trip_between_overlay_and_pip() {
        let mut overlay = vec![frame("outer", 1), frame("inner", 2)];
        let mut pip: Vec<CompositionDrillFrame> = Vec::new();

        // Dock: overlay → pip.
        move_composition_frames(&mut overlay, &mut pip);
        assert!(overlay.is_empty(), "overlay is emptied on dock");
        assert_eq!(
            pip.iter().map(|f| f.alias.as_str()).collect::<Vec<_>>(),
            vec!["outer", "inner"],
            "frames keep their depth order in the inset",
        );

        // Expand: pip → overlay.
        move_composition_frames(&mut pip, &mut overlay);
        assert!(pip.is_empty(), "pip is emptied on expand");
        assert_eq!(
            overlay.iter().map(|f| f.alias.as_str()).collect::<Vec<_>>(),
            vec!["outer", "inner"],
            "frames return to the lightbox at the same depth",
        );
    }

    /// Compile a tiny `src → comp → out` pipeline whose `comp` node uses a
    /// one-transform composition body, returning the `CompiledPlan` so resolver
    /// tests have a real plan with a populated `composition_body_assignments`. The
    /// temporary workspace is removed before returning.
    fn compiled_plan_with_composition() -> CompiledPlan {
        use clinker_plan::config::{CompileContext, parse_config};

        let unique = format!(
            "klinx-resolve-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock after epoch")
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&root).expect("create temp composition workspace");
        std::fs::write(
            root.join("body.comp.yaml"),
            r#"_compose:
  name: passthrough
  inputs:
    src:
      schema:
        - { name: x, type: int }
  outputs:
    result: pass
nodes:
  - type: transform
    name: pass
    input: src
    config:
      cxl: |
        emit y = x + 1
"#,
        )
        .expect("write composition fixture");

        let pipeline = r#"
pipeline:
  name: resolve_drill
nodes:
  - type: source
    name: src
    config:
      name: src
      type: csv
      path: ./in.csv
      schema:
        - { name: x, type: int }
  - type: composition
    name: comp
    input: src
    use: ./body.comp.yaml
    inputs:
      src: src
  - type: output
    name: out
    input: comp
    config:
      name: out
      type: csv
      path: ./out.csv
"#;
        let config = parse_config(pipeline).expect("pipeline fixture parses");
        let plan = config
            .compile(&CompileContext::new(root.clone()))
            .expect("pipeline fixture compiles");
        let _ = std::fs::remove_dir_all(root);
        plan
    }

    /// #171: the shared resolver returns a frame for a real composition node —
    /// with that node's name as the alias and the body id the plan assigned it, so
    /// the overlay and the full-swap drill navigate into the SAME body.
    #[test]
    fn resolve_composition_frame_returns_frame_for_known_node() {
        let plan = compiled_plan_with_composition();
        let expected_body = *plan
            .artifacts()
            .composition_body_assignments
            .get("comp")
            .expect("the compiled plan assigns a body to `comp`");

        let resolved =
            resolve_composition_frame(&plan, "comp").expect("known composition node resolves");
        assert_eq!(
            resolved.alias, "comp",
            "the alias is the call-site node name"
        );
        assert_eq!(
            resolved.body_id, expected_body,
            "the frame points at the plan's assigned body id",
        );
    }

    /// #171: the resolver returns `None` for a node the plan has no body
    /// assignment for (a non-composition name, or a typo) — the `▶`/overlay then
    /// silently no-ops rather than pushing a bogus frame.
    #[test]
    fn resolve_composition_frame_returns_none_for_unknown_node() {
        let plan = compiled_plan_with_composition();
        assert!(
            resolve_composition_frame(&plan, "src").is_none(),
            "a non-composition node has no body assignment",
        );
        assert!(
            resolve_composition_frame(&plan, "no_such_node").is_none(),
            "an unknown node name resolves to None",
        );
    }
}
