use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use dioxus::html::geometry::WheelDelta;
use dioxus::prelude::*;

use crate::pipeline_view::layout_model::{CanvasLayoutEngine, apply_canvas_layout};
use crate::pipeline_view::{
    CanvasConnectorPath, EdgeNature, FIELD_ROW_HEIGHT, FieldEdge, FieldEdgeKind, FieldRow,
    NODE_WIDTH, Precision, RoleEdge, StagePortSide, StageView, field_lineage_full,
    field_lineage_full_capped, fit_transform, group_endpoints_by_node, layout_bounds,
    lineage_closure, lineage_keep_nodes,
};
use crate::state::{ChannelViewMode, LineageRevealMode, current_pipeline_view, use_app_state};

use super::connector::{
    Connector, ConnectorEndpoints, ConnectorObstacle, FieldConnector, obstacle_aware_channel_paths,
};
use super::node::{
    CanvasNode, FieldDisplayInfo, GlobalNodeDisplayMode, NodeDisplayAction, ResolvedNodeDisplayMode,
};
use super::{CanvasHover, HoverTarget, LineageTarget, PinnedField};

/// One node-level connector ready to render: its endpoint node indices (for the
/// Filter-mode keep-set check, #123), the resolved endpoint [`StageView`]s, the
/// source branch port (if it leaves a Route), and the routed path.
#[derive(Clone, PartialEq)]
struct CanvasConnection {
    from_index: usize,
    to_index: usize,
    from: StageView,
    to: StageView,
    from_branch: Option<usize>,
    path: Option<CanvasConnectorPath>,
}

/// Resolved world-space endpoints + accent for one field-lineage cable.
///
/// A [`FieldEdge`] names `(node, field)` endpoints; this is the geometry the
/// SVG cable actually draws — the producer row's RIGHT anchor to the consumer
/// row's LEFT anchor, plus the producer's `data-stage-kind` for the accent and
/// the edge's [`FieldEdgeKind`] for the relationship-type stroke colour.
#[derive(Clone, PartialEq)]
struct FieldEdgeAnchors {
    start: (f32, f32),
    end: (f32, f32),
    kind_attr: String,
    kind: FieldEdgeKind,
    /// The edge's precision tier (#148), threaded to the cable so an
    /// `Approximate` over-approximation reads as a hatched/ghosted ribbon (#152).
    precision: Precision,
    path: Option<CanvasConnectorPath>,
}

/// Resolve a [`FieldEdge`] to drawable anchor geometry, or `None` if either
/// endpoint field is absent from its stage's rows (defensive — a well-formed
/// edge set always resolves, but the canvas renders pre-validation input).
fn resolve_edge_anchors(
    stages: &[StageView],
    edge: &FieldEdge,
    path: Option<&CanvasConnectorPath>,
) -> Option<FieldEdgeAnchors> {
    let from = stages.get(edge.from_node)?;
    let to = stages.get(edge.to_node)?;
    let fi = from.field_index(&edge.from_field)?;
    let ti = to.field_index(&edge.to_field)?;
    Some(FieldEdgeAnchors {
        start: from.field_anchor_out(fi),
        end: to.field_anchor_in(ti),
        kind_attr: from.kind.kind_attr().to_string(),
        kind: edge.kind,
        precision: edge.precision,
        path: path.filter(|path| path.points.len() >= 2).cloned(),
    })
}

fn resolve_role_edge_anchors(
    stages: &[StageView],
    edge: &RoleEdge,
    path: Option<&CanvasConnectorPath>,
) -> Option<FieldEdgeAnchors> {
    let from = stages.get(edge.from_node)?;
    let to = stages.get(edge.to_node)?;
    let fi = from.field_index(&edge.from_field)?;
    let ti = to.role_port_index(StagePortSide::Input, &edge.to_port)?;
    Some(FieldEdgeAnchors {
        start: from.field_anchor_out(fi),
        end: to.role_port_anchor_in(ti),
        kind_attr: from.kind.kind_attr().to_string(),
        kind: edge.kind,
        // A role-port edge feeds a semantic influence input (e.g. an Aggregate
        // group-by key), so it is an INDIRECT influence relationship — graded
        // `Approximate` like the influence field edges it parallels (#148/#152).
        precision: Precision::Approximate,
        path: path.filter(|path| path.points.len() >= 2).cloned(),
    })
}

// ── Canvas transform constants ───────────────────────────────────────────────
const ZOOM_MIN: f32 = 0.25;
const ZOOM_MAX: f32 = 4.0;
/// Zoom factor applied per scroll-wheel "tick" (for Line/Page delta modes).
const ZOOM_STEP_LINE: f32 = 1.10;
/// Zoom factor per pixel of scroll delta (for Pixel mode).
const ZOOM_STEP_PIXEL: f32 = 0.001;
/// Screen-space padding kept around the node graph when fitting it to view.
const FIT_MARGIN: f32 = 60.0;
/// Fallback viewport dimensions used before the panel reports its real size
/// (the `onmounted` measurement is async). Sized to a typical canvas pane so a
/// fit triggered on the very first frame still produces a sane transform.
const DEFAULT_VIEWPORT_W: f32 = 1000.0;
const DEFAULT_VIEWPORT_H: f32 = 700.0;
/// Default number of field rows rendered per node before the user loads more or
/// filters. This bounds card height and per-row connector work for wide schemas.
const FIELD_ROW_CAP: usize = 24;
/// Rows shown in Preview mode before explicit per-node expansion.
const PREVIEW_FIELD_ROW_CAP: usize = 6;
const SMALL_GRAPH_NODE_LIMIT: usize = 8;
const LARGE_GRAPH_NODE_LIMIT: usize = 30;
const SMALL_GRAPH_FIELD_LIMIT: usize = 12;
const WIDE_SCHEMA_FIELD_LIMIT: usize = FIELD_ROW_CAP;
const AUTO_COMPACT_ZOOM: f32 = 0.55;
const AUTO_PREVIEW_ZOOM: f32 = 0.95;
/// Default per-direction hop cap for a SELECTED field's transitive lineage reveal
/// in FILTER mode (#123). Bounds the closure walk so a runaway lineage on a large
/// graph stays readable, while sitting comfortably ABOVE the deepest closure in any
/// bundled example (deepest 7 hops; generous headroom) so the default Filter reveal
/// is unclipped on real examples. Only a pathologically deep closure is clipped, and
/// then the "expand further" affordance raises it by [`HOP_CAP_STEP`]; a cap at/beyond
/// the lineage depth equals the uncapped walk. Highlight mode is never capped (see
/// [`reveal_closure_cap`]); hover reveals are already 1-hop and never capped. The
/// `default_hop_cap_does_not_clip_example_pipelines` guard locks this against the
/// bundled examples.
const DEFAULT_HOP_CAP: usize = 16;
/// How far each "expand further" click raises the active hop cap (#123).
const HOP_CAP_STEP: usize = 8;

/// The hop cap to apply to a selected/pinned reveal's closure for `mode` (#123).
///
/// Filter mode HIDES off-path cards, so a bounded closure is the readability win
/// (and the EXPAND+ affordance un-clips it) → `Some(hop_cap)`. Highlight mode only
/// DIMS (never hides), so it keeps the FULL, uncapped closure → `None`, preserving
/// the pre-#123 selected-field reveal at ANY lineage depth (the "#123: Highlight
/// preserves current selected-field behavior" acceptance criterion, honored for
/// every pipeline, not just ones shallower than the cap).
fn reveal_closure_cap(mode: LineageRevealMode, hop_cap: usize) -> Option<usize> {
    match mode {
        LineageRevealMode::Filter => Some(hop_cap),
        LineageRevealMode::Highlight => None,
    }
}

/// Whether a ribbon cable of [`EdgeNature`] `nature` should be DRAWN given the two
/// independent ribbon toggles (#152).
///
/// The dual ribbons are independent: `show_value` gates the solid DIRECT "value
/// lineage" ribbon, `show_influence` gates the ghosted INDIRECT "influence halo".
/// Toggling one never affects the other. This gates only the DRAWN overlay set;
/// the dim/focus still keys off the full closure, so hiding a ribbon thins the
/// drawn cables without un-dimming the off-path cards the path still traverses.
fn ribbon_edge_visible(nature: EdgeNature, show_value: bool, show_influence: bool) -> bool {
    match nature {
        EdgeNature::Direct => show_value,
        EdgeNature::Indirect => show_influence,
    }
}
#[derive(Clone, Debug, Default, PartialEq)]
struct FieldDisplayState {
    visible_limit: usize,
    query: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct GraphDisplayProfile {
    node_count: usize,
    max_field_count: usize,
}

impl GraphDisplayProfile {
    fn from_stages(stages: &[StageView]) -> Self {
        Self {
            node_count: stages.len(),
            max_field_count: stages
                .iter()
                .map(|stage| stage.fields.len())
                .max()
                .unwrap_or(0),
        }
    }
}

#[derive(Clone, Debug, Default)]
struct FieldRankSignals {
    produced_or_derived_by_node: HashMap<usize, HashSet<String>>,
    operator_relevant_by_node: HashMap<usize, HashSet<String>>,
    /// Per-node field names that are an endpoint of a `GroupBy` influence edge
    /// (#147) — the Aggregate group-by grain. Replaces the retired
    /// `FieldRow::is_aggregate_grain` flag for preview ranking and search, so the
    /// grain is represented exactly once (the edge) and read from it here.
    aggregate_grain_by_node: HashMap<usize, HashSet<String>>,
}

fn build_field_rank_signals(
    stages: &[StageView],
    field_edges: &[FieldEdge],
    role_edges: &[RoleEdge],
) -> FieldRankSignals {
    let mut signals = FieldRankSignals::default();
    for edge in field_edges {
        if edge.kind == FieldEdgeKind::GroupBy {
            signals
                .aggregate_grain_by_node
                .entry(edge.from_node)
                .or_default()
                .insert(edge.from_field.clone());
            signals
                .aggregate_grain_by_node
                .entry(edge.to_node)
                .or_default()
                .insert(edge.to_field.clone());
        }
        if matches!(edge.kind, FieldEdgeKind::Derive | FieldEdgeKind::Access) {
            signals
                .produced_or_derived_by_node
                .entry(edge.from_node)
                .or_default()
                .insert(edge.from_field.clone());
            signals
                .produced_or_derived_by_node
                .entry(edge.to_node)
                .or_default()
                .insert(edge.to_field.clone());
        }

        if stages
            .get(edge.from_node)
            .is_some_and(|stage| stage_kind_prioritizes_operator_fields(&stage.kind))
        {
            signals
                .operator_relevant_by_node
                .entry(edge.from_node)
                .or_default()
                .insert(edge.from_field.clone());
        }
        if stages
            .get(edge.to_node)
            .is_some_and(|stage| stage_kind_prioritizes_operator_fields(&stage.kind))
        {
            signals
                .operator_relevant_by_node
                .entry(edge.to_node)
                .or_default()
                .insert(edge.to_field.clone());
        }
    }

    for edge in role_edges {
        signals
            .produced_or_derived_by_node
            .entry(edge.from_node)
            .or_default()
            .insert(edge.from_field.clone());
        signals
            .operator_relevant_by_node
            .entry(edge.from_node)
            .or_default()
            .insert(edge.from_field.clone());
    }

    signals
}

fn stage_kind_prioritizes_operator_fields(kind: &crate::pipeline_view::StageKind) -> bool {
    matches!(
        kind,
        crate::pipeline_view::StageKind::Aggregate
            | crate::pipeline_view::StageKind::Route
            | crate::pipeline_view::StageKind::Merge
            | crate::pipeline_view::StageKind::Combine
            | crate::pipeline_view::StageKind::Output
            | crate::pipeline_view::StageKind::OutputPort
    )
}

fn resolve_node_display_mode(
    global: GlobalNodeDisplayMode,
    node_override: Option<ResolvedNodeDisplayMode>,
    profile: GraphDisplayProfile,
    zoom: f32,
    has_auto_focus: bool,
) -> ResolvedNodeDisplayMode {
    if let Some(mode) = node_override {
        return mode;
    }

    match global.resolved() {
        Some(mode) => mode,
        None => {
            let base = if zoom < AUTO_COMPACT_ZOOM {
                ResolvedNodeDisplayMode::Compact
            } else if zoom < AUTO_PREVIEW_ZOOM {
                ResolvedNodeDisplayMode::Preview
            } else if profile.node_count >= LARGE_GRAPH_NODE_LIMIT {
                ResolvedNodeDisplayMode::Compact
            } else if profile.max_field_count > WIDE_SCHEMA_FIELD_LIMIT {
                ResolvedNodeDisplayMode::Preview
            } else if profile.node_count <= SMALL_GRAPH_NODE_LIMIT
                && profile.max_field_count <= SMALL_GRAPH_FIELD_LIMIT
            {
                ResolvedNodeDisplayMode::Schema
            } else {
                ResolvedNodeDisplayMode::Preview
            };

            if has_auto_focus && matches!(base, ResolvedNodeDisplayMode::Compact) {
                ResolvedNodeDisplayMode::Preview
            } else {
                base
            }
        }
    }
}

fn default_limit_for_mode(mode: ResolvedNodeDisplayMode, query: &str) -> usize {
    if !query.trim().is_empty() && matches!(mode, ResolvedNodeDisplayMode::Compact) {
        return FIELD_ROW_CAP;
    }

    match mode {
        ResolvedNodeDisplayMode::Compact => 0,
        ResolvedNodeDisplayMode::Preview => PREVIEW_FIELD_ROW_CAP,
        ResolvedNodeDisplayMode::Schema => FIELD_ROW_CAP,
        ResolvedNodeDisplayMode::Full => usize::MAX,
    }
}

fn page_size_for_mode(mode: ResolvedNodeDisplayMode) -> usize {
    match mode {
        ResolvedNodeDisplayMode::Compact | ResolvedNodeDisplayMode::Preview => {
            PREVIEW_FIELD_ROW_CAP
        }
        ResolvedNodeDisplayMode::Schema | ResolvedNodeDisplayMode::Full => FIELD_ROW_CAP,
    }
}

#[derive(Clone, Debug, PartialEq)]
struct ProjectedStage {
    stage: StageView,
    display: FieldDisplayInfo,
}

#[derive(Clone, Copy)]
struct FieldProjectionContext<'a> {
    stage_index: usize,
    mode: ResolvedNodeDisplayMode,
    global_mode: GlobalNodeDisplayMode,
    override_mode: Option<ResolvedNodeDisplayMode>,
    temporary_fields: &'a HashSet<String>,
    rank_signals: &'a FieldRankSignals,
}

fn project_stage_fields(
    stage: &StageView,
    state: &FieldDisplayState,
    context: FieldProjectionContext<'_>,
) -> ProjectedStage {
    let query = state.query.trim();
    let default_limit = default_limit_for_mode(context.mode, query);
    let visible_limit = if matches!(context.mode, ResolvedNodeDisplayMode::Full) {
        usize::MAX
    } else if state.visible_limit == 0 {
        default_limit
    } else {
        state.visible_limit.max(default_limit)
    };
    let grain_fields = context
        .rank_signals
        .aggregate_grain_by_node
        .get(&context.stage_index);
    let matching_fields: Vec<(usize, &FieldRow)> = stage
        .fields
        .iter()
        .enumerate()
        .filter(|(_, field)| {
            let is_grain = grain_fields.is_some_and(|fields| fields.contains(&field.name));
            field_matches_query(field, query, is_grain)
        })
        .collect();
    let matching_count = matching_fields.len();
    let mut fields = visible_fields_for_mode(
        context.stage_index,
        stage,
        &matching_fields,
        context.mode,
        visible_limit,
        context.rank_signals,
    );

    let hidden_count = matching_count.saturating_sub(fields.len());
    let next_count = hidden_count.min(page_size_for_mode(context.mode));
    let mut visible_names: HashSet<String> =
        fields.iter().map(|field| field.name.clone()).collect();
    let mut appended_temporary_fields = Vec::new();
    for field in &stage.fields {
        if context.temporary_fields.contains(&field.name) && !visible_names.contains(&field.name) {
            visible_names.insert(field.name.clone());
            appended_temporary_fields.push(field.name.clone());
            fields.push(field.clone());
        }
    }
    let mut projected = stage.clone();
    projected.fields = fields;

    ProjectedStage {
        stage: projected,
        display: FieldDisplayInfo {
            total_count: stage.fields.len(),
            matching_count,
            hidden_count,
            next_count,
            temporary_fields: appended_temporary_fields,
            query: state.query.clone(),
            searchable: stage.fields.len() > PREVIEW_FIELD_ROW_CAP
                || !query.is_empty()
                || (matches!(context.mode, ResolvedNodeDisplayMode::Compact)
                    && !stage.fields.is_empty()),
            can_reduce: state.visible_limit > default_limit && matching_count > default_limit,
            mode: context.mode,
            global_mode: context.global_mode,
            override_mode: context.override_mode,
        },
    }
}

fn visible_fields_for_mode(
    stage_index: usize,
    stage: &StageView,
    matching_fields: &[(usize, &FieldRow)],
    mode: ResolvedNodeDisplayMode,
    visible_limit: usize,
    rank_signals: &FieldRankSignals,
) -> Vec<FieldRow> {
    if visible_limit == 0 {
        return Vec::new();
    }

    let limit = visible_limit.min(matching_fields.len());
    match mode {
        ResolvedNodeDisplayMode::Compact | ResolvedNodeDisplayMode::Schema => matching_fields
            .iter()
            .take(limit)
            .map(|(_, field)| (*field).clone())
            .collect(),
        ResolvedNodeDisplayMode::Full => matching_fields
            .iter()
            .map(|(_, field)| (*field).clone())
            .collect(),
        ResolvedNodeDisplayMode::Preview => {
            let mut ranked = matching_fields.to_vec();
            ranked.sort_by_key(|(index, field)| {
                (
                    preview_rank(stage_index, stage, field, rank_signals),
                    *index,
                )
            });
            ranked
                .into_iter()
                .take(limit)
                .map(|(_, field)| field.clone())
                .collect()
        }
    }
}

fn preview_rank(
    stage_index: usize,
    stage: &StageView,
    field: &FieldRow,
    rank_signals: &FieldRankSignals,
) -> u8 {
    let is_aggregate_grain = rank_signals
        .aggregate_grain_by_node
        .get(&stage_index)
        .is_some_and(|fields| fields.contains(&field.name));
    if field.is_correlation_key || is_aggregate_grain {
        return 1;
    }
    if matches!(field.kind, crate::pipeline_view::FieldKind::Emitted)
        || rank_signals
            .produced_or_derived_by_node
            .get(&stage_index)
            .is_some_and(|fields| fields.contains(&field.name))
    {
        return 2;
    }
    if stage_kind_prioritizes_operator_fields(&stage.kind)
        && rank_signals
            .operator_relevant_by_node
            .get(&stage_index)
            .is_some_and(|fields| fields.contains(&field.name))
    {
        return 3;
    }
    if matches!(field.kind, crate::pipeline_view::FieldKind::Declared) {
        return 4;
    }
    5
}

/// Whether a field matches the search `query`. `is_aggregate_grain` is supplied
/// by the caller from the per-node `GroupBy`-edge grain set (#147), since the
/// grain is no longer a `FieldRow` flag; it keeps the "aggregate failure grain"
/// search term working without re-introducing the retired flag.
fn field_matches_query(field: &FieldRow, query: &str, is_aggregate_grain: bool) -> bool {
    let query = query.trim();
    query.is_empty()
        || text_matches_query(&field.name, query)
        || field
            .ty
            .as_ref()
            .is_some_and(|ty| text_matches_query(ty, query))
        || text_matches_query(field_kind_label(field), query)
        || (field.is_correlation_key && text_matches_query("source correlation key", query))
        || (is_aggregate_grain && text_matches_query("aggregate failure grain", query))
}

fn text_matches_query(value: &str, query: &str) -> bool {
    if query.contains('*') || query.contains('?') {
        wildcard_match(
            query.to_ascii_lowercase().as_bytes(),
            value.to_ascii_lowercase().as_bytes(),
        )
    } else {
        value
            .to_ascii_lowercase()
            .contains(&query.to_ascii_lowercase())
    }
}

fn wildcard_match(pattern: &[u8], value: &[u8]) -> bool {
    let (mut pi, mut vi) = (0, 0);
    let mut star = None;
    let mut match_after_star = 0;

    while vi < value.len() {
        if pi < pattern.len() && (pattern[pi] == b'?' || pattern[pi] == value[vi]) {
            pi += 1;
            vi += 1;
        } else if pi < pattern.len() && pattern[pi] == b'*' {
            star = Some(pi);
            match_after_star = vi;
            pi += 1;
        } else if let Some(star_index) = star {
            pi = star_index + 1;
            match_after_star += 1;
            vi = match_after_star;
        } else {
            return false;
        }
    }

    while pi < pattern.len() && pattern[pi] == b'*' {
        pi += 1;
    }

    pi == pattern.len()
}

fn field_kind_label(field: &FieldRow) -> &'static str {
    match field.kind {
        crate::pipeline_view::FieldKind::Declared => "declared",
        crate::pipeline_view::FieldKind::Emitted => "emitted",
        crate::pipeline_view::FieldKind::PassThrough => "passthrough",
    }
}

fn field_endpoint_names_by_node(
    edges: &[FieldEdge],
    closure: &HashSet<usize>,
) -> HashMap<usize, HashSet<String>> {
    let mut fields = HashMap::<usize, HashSet<String>>::new();
    for &edge_index in closure {
        let Some(edge) = edges.get(edge_index) else {
            continue;
        };
        fields
            .entry(edge.from_node)
            .or_default()
            .insert(edge.from_field.clone());
        fields
            .entry(edge.to_node)
            .or_default()
            .insert(edge.to_field.clone());
    }
    fields
}

fn role_source_field_names_by_node(
    edges: &[RoleEdge],
    closure: &HashSet<usize>,
) -> HashMap<usize, HashSet<String>> {
    let mut fields = HashMap::<usize, HashSet<String>>::new();
    for &edge_index in closure {
        let Some(edge) = edges.get(edge_index) else {
            continue;
        };
        fields
            .entry(edge.from_node)
            .or_default()
            .insert(edge.from_field.clone());
    }
    fields
}

fn role_endpoint_names_by_node(
    edges: &[RoleEdge],
    closure: &HashSet<usize>,
) -> HashMap<usize, HashSet<String>> {
    let mut ports = HashMap::<usize, HashSet<String>>::new();
    for &edge_index in closure {
        let Some(edge) = edges.get(edge_index) else {
            continue;
        };
        ports
            .entry(edge.to_node)
            .or_default()
            .insert(edge.to_port.clone());
    }
    ports
}

fn role_port_closure(edges: &[RoleEdge], node: usize, port: &str) -> HashSet<usize> {
    edges
        .iter()
        .enumerate()
        .filter_map(|(index, edge)| (edge.to_node == node && edge.to_port == port).then_some(index))
        .collect()
}

fn role_edges_from_fields(
    edges: &[RoleEdge],
    fields_by_node: &HashMap<usize, HashSet<String>>,
) -> HashSet<usize> {
    edges
        .iter()
        .enumerate()
        .filter_map(|(index, edge)| {
            fields_by_node
                .get(&edge.from_node)
                .is_some_and(|fields| fields.contains(&edge.from_field))
                .then_some(index)
        })
        .collect()
}

fn field_matches_by_node(
    stages: &[StageView],
    query: &str,
    aggregate_grain_by_node: &HashMap<usize, HashSet<String>>,
) -> HashMap<usize, HashSet<String>> {
    let query = query.trim();
    if query.is_empty() {
        return HashMap::new();
    }

    stages
        .iter()
        .enumerate()
        .filter_map(|(index, stage)| {
            let grain_fields = aggregate_grain_by_node.get(&index);
            let matches: HashSet<String> = stage
                .fields
                .iter()
                .filter(|field| {
                    let is_grain = grain_fields.is_some_and(|fields| fields.contains(&field.name));
                    field_matches_query(field, query, is_grain)
                })
                .map(|field| field.name.clone())
                .collect();
            (!matches.is_empty()).then_some((index, matches))
        })
        .collect()
}

fn merge_field_name_sets(
    target: &mut HashMap<usize, HashSet<String>>,
    source: &HashMap<usize, HashSet<String>>,
) {
    for (node, names) in source {
        target
            .entry(*node)
            .or_default()
            .extend(names.iter().cloned());
    }
}

fn append_highlights(
    target: &mut HashMap<usize, Vec<String>>,
    source: &HashMap<usize, HashSet<String>>,
) {
    for (node, names) in source {
        let entry = target.entry(*node).or_default();
        entry.extend(names.iter().cloned());
        entry.sort();
        entry.dedup();
    }
}

fn rendered_card_height(stage: &StageView, display: &FieldDisplayInfo) -> f32 {
    let footer_height =
        if !stage.fields.is_empty() && (display.hidden_count > 0 || display.can_reduce) {
            FIELD_ROW_HEIGHT
        } else {
            0.0
        };

    stage.card_height() + footer_height
}

// ── Drag state — non-reactive, stored in Rc<RefCell<>> to avoid a signal write
// (and therefore a re-render) on every pointer-move event. ───────────────────
#[derive(Default)]
struct DragState {
    /// Whether a pan drag is currently active.
    active: bool,
    /// Client-space X where the drag began.
    start_x: f32,
    /// Client-space Y where the drag began.
    start_y: f32,
    /// Pan X at drag start — restored as offset when computing current pan.
    start_pan_x: f32,
    /// Pan Y at drag start.
    start_pan_y: f32,
}

/// The infinite-canvas panel rendering the pipeline node graph.
///
/// Pan: left-click-drag anywhere on the canvas background.
/// Zoom: scroll wheel (zoom anchored to cursor position, range 25 %–400 %).
/// Fit-to-view: double-click the canvas, or the toolbar "FIT" button — frames
/// every node in the viewport with a margin. "RESET" re-fits the engine layout.
///
/// Visual layers (back to front):
///   1. Dot grid background (CSS radial-gradient, does NOT transform with content)
///   2. Noise + scanline overlays (CSS ::before / ::after, fixed to panel)
///   3. `klinx-canvas-viewport` div — receives CSS transform(translate + scale)
///      a. Default SVG connector overlay with centered visible channels
///      b. Active field-lineage SVG overlay above default channels
///      c. Node cards masking strokes that would otherwise cross card interiors
#[component]
pub fn CanvasPanel() -> Element {
    let state = use_app_state();
    let tab_mgr = use_context::<crate::state::TabManagerState>();
    let mut field_display_states = use_signal(HashMap::<String, FieldDisplayState>::new);
    let mut global_field_query = use_signal(String::new);
    let mut global_node_display_mode = use_signal(|| GlobalNodeDisplayMode::Auto);
    let mut node_display_overrides = use_signal(HashMap::<String, ResolvedNodeDisplayMode>::new);

    // ── Transform state (local — only the canvas needs these) ────────────────
    let mut pan_x = use_signal(|| 0.0_f32);
    let mut pan_y = use_signal(|| 0.0_f32);
    let mut zoom = use_signal(|| 1.0_f32);

    // ── Viewport pixel size — measured on mount/resize so Fit-to-View can frame
    // the graph against the real pane. Seeded with sane defaults because the
    // `onmounted` measurement is async and may not have run on first fit. ─────
    let mut viewport_w = use_signal(|| DEFAULT_VIEWPORT_W);
    let mut viewport_h = use_signal(|| DEFAULT_VIEWPORT_H);

    // ── Non-reactive drag state — hot path, no re-renders during drag ─────────
    let drag = use_hook(|| Rc::new(RefCell::new(DragState::default())));

    // Per-direction hop cap for the SELECTED column's transitive reveal (#123).
    // Bounds a deep lineage so a large closure stays readable; "expand further"
    // raises it, and the view-swap effect resets it so a cap never carries across
    // graphs. Canvas-local because it is a transient reveal control, not tab state.
    let mut hop_cap = use_signal(|| DEFAULT_HOP_CAP);

    // Dual value/influence ribbon toggles (#152). Two INDEPENDENT booleans gating
    // which of the two ribbons the active overlay draws: the solid DIRECT "value
    // lineage" and the ghosted INDIRECT "influence halo" (the #147 edge nature).
    // Both default ON so a fresh selection lights the whole path. Unlike `hop_cap`
    // (a per-graph bound the view-swap effect RESETS), these are view PREFERENCES —
    // like the reveal MODE, they intentionally PERSIST across graph/tab swaps, so the
    // view-swap effect leaves them alone; a hidden ribbon stays hidden until the user
    // re-enables it. They gate only the DRAWN cables — the dim/focus closure (and so
    // which cards dim) is untouched.
    let mut show_value_ribbon = use_signal(|| true);
    let mut show_influence_ribbon = use_signal(|| true);

    let view_mode = *state.channel_view_mode.read();
    // How the active reveal treats off-path nodes: Highlight dims them (today's
    // behavior), Filter hides them (#123). Read once per render and shared by the
    // dim/hide branch below and the toolbar toggle.
    let reveal_mode = *state.lineage_reveal_mode.read();
    let active_hop_cap = *hop_cap.read();
    // Read once per render; shared by the toolbar toggles and the overlay filter.
    let value_ribbon_on = *show_value_ribbon.read();
    let influence_ribbon_on = *show_influence_ribbon.read();
    let is_filter_mode = reveal_mode == LineageRevealMode::Filter;
    let closure_cap = reveal_closure_cap(reveal_mode, active_hop_cap);
    let pipeline_view = current_pipeline_view(state);
    let pipeline_view =
        apply_canvas_layout(pipeline_view, CanvasLayoutEngine::PortAwareSugiyama).view;
    let connections_model = pipeline_view.connections;
    let connection_paths = pipeline_view.connection_paths;
    let field_edges = pipeline_view.field_edges;
    let field_edge_paths = pipeline_view.field_edge_paths;
    let role_edges = pipeline_view.role_edges;
    let role_edge_paths = pipeline_view.role_edge_paths;
    let raw_stages = pipeline_view.stages;

    // ── Field-level lineage hover state ──────────────────────────────────────
    // The current pointer [`HoverTarget`], provided as a canvas-scoped context so
    // field rows can request lineage reveals. DEFAULT (`None`) draws node-level
    // connectors only; a cold `Field` hover waits briefly before revealing one
    // field's 1-hop closure, while warm row-to-row movement is immediate. It
    // never reveals the whole field-edge set at once (#72).
    //
    // D2 (Phase 2, perf): an applied hover writes this signal, re-rendering the
    // whole canvas. The cold-entry delay suppresses quick sweep-through flashes,
    // while the warm state keeps intentional row scanning responsive; a future
    // pass can scope the highlight to affected cards.
    let mut hovered_field = use_context_provider(|| {
        CanvasHover(
            Signal::new(HoverTarget::None),
            Signal::new(HoverTarget::None),
            Signal::new(0),
            Signal::new(false),
        )
    });
    // The pinned (clicked-to-select) column (#75) — sticky across pointer moves,
    // takes precedence over the hover in the reveal computation below. Cleared by
    // a canvas-background click and by the view-swap effect.
    let mut pinned_field = use_context_provider(|| PinnedField(Signal::new(None)));

    // The participating field-edge indices for the active reveal. A PIN (#75) wins
    // over the hover (#72) — a clicked column stays revealed across pointer moves:
    //  - pinned `(node, f)` → that column's FULL transitive pipeline lineage
    //    (`field_lineage_full`: every up- and down-stream edge), the click reveal.
    //  - else `Field(node, f)` → that field's DIRECT (1-hop) neighbourhood (hover).
    //  - else empty (the node-level DAG only).
    // Computed before field projection so hidden lineage endpoints can be appended
    // as temporary rows and then resolve to real connector anchors below.
    let selected_field = state.selected_field.read().clone();
    let selected_field_target = selected_field.as_ref().and_then(|selected| {
        raw_stages
            .iter()
            .position(|stage| stage.id == selected.stage_id)
            .map(|node| LineageTarget::Field(node, selected.field_name.clone()))
    });
    let pinned_selection = selected_field_target
        .clone()
        .or_else(|| pinned_field.0.read().clone());
    let hover_target = hovered_field.0.read().clone();
    let (closure, role_closure): (HashSet<usize>, HashSet<usize>) = match &pinned_selection {
        Some(LineageTarget::Field(node, field)) => {
            let field_closure = field_lineage_full_capped(&field_edges, *node, field, closure_cap);
            let mut fields_by_node = field_endpoint_names_by_node(&field_edges, &field_closure);
            fields_by_node
                .entry(*node)
                .or_default()
                .insert(field.clone());
            let role_closure = role_edges_from_fields(&role_edges, &fields_by_node);
            (field_closure, role_closure)
        }
        Some(LineageTarget::RolePort(node, port)) => {
            let role_closure = role_port_closure(&role_edges, *node, port);
            let mut field_closure = HashSet::new();
            for role_index in &role_closure {
                let Some(edge) = role_edges.get(*role_index) else {
                    continue;
                };
                field_closure.extend(field_lineage_full_capped(
                    &field_edges,
                    edge.from_node,
                    &edge.from_field,
                    closure_cap,
                ));
            }
            (field_closure, role_closure)
        }
        None => match &hover_target {
            HoverTarget::None => (HashSet::new(), HashSet::new()),
            HoverTarget::Field(node, field) => {
                let field_closure = lineage_closure(&field_edges, *node, field);
                let fields_by_node = HashMap::from([(*node, HashSet::from([field.clone()]))]);
                let role_closure = role_edges_from_fields(&role_edges, &fields_by_node);
                (field_closure, role_closure)
            }
            HoverTarget::RolePort(node, port) => {
                (HashSet::new(), role_port_closure(&role_edges, *node, port))
            }
        },
    };
    let spotlight_edges: HashSet<usize> = if pinned_selection.is_some() {
        match &hover_target {
            HoverTarget::Field(node, field) => lineage_closure(&field_edges, *node, field)
                .intersection(&closure)
                .copied()
                .collect(),
            HoverTarget::RolePort(_, _) | HoverTarget::None => HashSet::new(),
        }
    } else {
        HashSet::new()
    };
    let spotlight_role_edges: HashSet<usize> = if pinned_selection.is_some() {
        match &hover_target {
            HoverTarget::RolePort(node, port) => role_port_closure(&role_edges, *node, port)
                .intersection(&role_closure)
                .copied()
                .collect(),
            HoverTarget::None => HashSet::new(),
            HoverTarget::Field(_, _) => HashSet::new(),
        }
    } else {
        HashSet::new()
    };
    let lineage_fields_by_node = field_endpoint_names_by_node(&field_edges, &closure);
    let lineage_role_source_fields_by_node =
        role_source_field_names_by_node(&role_edges, &role_closure);
    // Built before search so the "aggregate failure grain" search term can read
    // the per-node `GroupBy`-edge grain set (#147 — the grain is no longer a row
    // flag).
    let rank_signals = build_field_rank_signals(&raw_stages, &field_edges, &role_edges);
    let global_query = global_field_query.read().clone();
    let global_matches_by_node = field_matches_by_node(
        &raw_stages,
        &global_query,
        &rank_signals.aggregate_grain_by_node,
    );
    let mut temporary_fields_by_node = lineage_fields_by_node.clone();
    merge_field_name_sets(
        &mut temporary_fields_by_node,
        &lineage_role_source_fields_by_node,
    );
    merge_field_name_sets(&mut temporary_fields_by_node, &global_matches_by_node);

    let display_profile = GraphDisplayProfile::from_stages(&raw_stages);
    let zoom_for_auto = *zoom.read();
    let global_display_mode = *global_node_display_mode.read();
    let selected_stage_ids = state.selected_stages.read().clone();
    let visible_stage_ids: HashSet<String> =
        raw_stages.iter().map(|stage| stage.id.clone()).collect();

    {
        let visible_stage_indices: HashMap<String, usize> = raw_stages
            .iter()
            .enumerate()
            .map(|(index, stage)| (stage.id.clone(), index))
            .collect();
        let visible_fields: HashMap<String, HashSet<String>> = raw_stages
            .iter()
            .map(|stage| {
                (
                    stage.id.clone(),
                    stage
                        .fields
                        .iter()
                        .map(|field| field.name.clone())
                        .collect(),
                )
            })
            .collect();
        use_effect(move || {
            let selected = state.selected_field.read().clone();
            let mut pinned = pinned_field;
            match selected {
                Some(selection)
                    if visible_fields
                        .get(&selection.stage_id)
                        .is_some_and(|fields| fields.contains(&selection.field_name)) =>
                {
                    if let Some(node) = visible_stage_indices.get(&selection.stage_id).copied() {
                        pinned
                            .0
                            .set(Some(LineageTarget::Field(node, selection.field_name)));
                    }
                }
                Some(_) | None => {
                    if matches!(&*pinned.0.peek(), Some(LineageTarget::Field(_, _))) {
                        pinned.0.set(None);
                    }
                }
            }
        });
    }

    let projected: Vec<ProjectedStage> = {
        let display_states = field_display_states.read();
        let display_overrides = node_display_overrides.read();
        raw_stages
            .iter()
            .enumerate()
            .map(|(index, stage)| {
                let display_state = display_states.get(&stage.id).cloned().unwrap_or_default();
                let temporary_fields = temporary_fields_by_node.remove(&index).unwrap_or_default();
                let override_mode = display_overrides.get(&stage.id).copied();
                let has_auto_focus =
                    selected_stage_ids.contains(stage.id.as_str()) || !temporary_fields.is_empty();
                let mode = resolve_node_display_mode(
                    global_display_mode,
                    override_mode,
                    display_profile,
                    zoom_for_auto,
                    has_auto_focus,
                );
                project_stage_fields(
                    stage,
                    &display_state,
                    FieldProjectionContext {
                        stage_index: index,
                        mode,
                        global_mode: global_display_mode,
                        override_mode,
                        temporary_fields: &temporary_fields,
                        rank_signals: &rank_signals,
                    },
                )
            })
            .collect()
    };
    let field_displays: Vec<FieldDisplayInfo> =
        projected.iter().map(|p| p.display.clone()).collect();
    let stages: Vec<StageView> = projected.into_iter().map(|p| p.stage).collect();
    let connector_obstacles = stages
        .iter()
        .zip(field_displays.iter())
        .map(|(stage, display)| ConnectorObstacle {
            x: stage.canvas_x,
            y: stage.canvas_y,
            width: NODE_WIDTH,
            height: rendered_card_height(stage, display),
        })
        .collect::<Vec<_>>();

    // Resolve each connection to its endpoint stages plus the source branch (if
    // it leaves a Route). The branch lets the connector anchor at the specific
    // branch port instead of the shared node-level port.
    let connection_endpoints: Vec<ConnectorEndpoints> = connections_model
        .iter()
        .map(|c| {
            let from = &stages[c.from];
            let to = &stages[c.to];
            let (sx, sy) = match c.from_branch {
                Some(i) => from.branch_anchor_out(i),
                None => from.port_out(),
            };
            let (tx, ty) = to.port_in();
            ConnectorEndpoints { sx, sy, tx, ty }
        })
        .collect();
    let connection_channel_paths =
        obstacle_aware_channel_paths(&connection_endpoints, &connector_obstacles);
    let connections: Vec<CanvasConnection> = connections_model
        .iter()
        .enumerate()
        .map(|(edge_index, c)| {
            let dynamic_path = connection_channel_paths
                .get(edge_index)
                .filter(|path| path.points.len() >= 2)
                .cloned();
            let layout_path = connection_paths
                .get(edge_index)
                .filter(|path| path.points.len() >= 2)
                .cloned();
            CanvasConnection {
                from_index: c.from,
                to_index: c.to,
                from: stages[c.from].clone(),
                to: stages[c.to].clone(),
                from_branch: c.from_branch,
                path: dynamic_path.or(layout_path),
            }
        })
        .collect();

    // D1: clear a stale hover when the active view swaps. A hovered
    // `(node_idx, field)` is only meaningful against the CURRENT view's
    // `field_edges`/`stages`; after a drill in/out, a Raw↔Resolved toggle, or a
    // composition switch, that index would re-run the closure against a
    // different graph and highlight the wrong rows.
    //
    // The effect reads the signals that SELECT which view is shown — the
    // composition document, the drill stack, the channel view mode, and the
    // underlying pipeline/plan signals — so it re-subscribes to each and re-runs
    // whenever any of them changes (i.e. on exactly the swaps above), resetting
    // the hover. It does NOT read pan/zoom/hover, so a pure interaction never
    // clears the highlight. Reading reactive signals *inside* the effect is what
    // drives re-runs (a captured plain value would not); the write lives in the
    // effect, never the render body.
    use_effect(move || {
        // Fingerprint the active view by reading its selecting signals. We read
        // the displayed stages' identities under each branch so a composition
        // switch (same Raw view, different document) also re-runs the effect.
        let comp_ids: Option<Vec<String>> = state
            .composition_view
            .read()
            .as_ref()
            .map(|v| v.stages.iter().map(|s| s.id.clone()).collect());
        let drill_key: Vec<u32> = state
            .composition_drill_stack
            .read()
            .iter()
            .map(|f| f.body_id.0)
            .collect();
        let mode = *state.channel_view_mode.read();
        let pipeline_present = state.pipeline.read().is_some();
        let plan_present = state.compiled_plan.read().is_some();
        let partial_present = state.partial_pipeline.read().is_some();
        // The active tab id distinguishes two DIFFERENT loaded pipelines that share
        // the same fingerprint otherwise (both top-level, no drill, same channel
        // mode) — without it, switching between two pipeline tabs leaves the present
        // booleans unchanged and the effect never re-runs, carrying a stale hover /
        // pin / hop cap onto a different graph (#123 acceptance: no stale state
        // pointing at another graph). Tab identity lives on `TabManagerState`, not
        // `AppState`.
        let active_tab = *tab_mgr.active_tab_id.read();
        // Bind the fingerprint so the reads above are retained as subscriptions.
        let _ = (
            &comp_ids,
            &drill_key,
            mode,
            pipeline_present,
            plan_present,
            partial_present,
            active_tab,
        );

        let mut hovered = hovered_field;
        hovered.force_clear();
        let mut pinned = pinned_field;
        pinned.0.set(None);
        let mut selected_field = state.selected_field;
        selected_field.set(None);
        // Reset the reveal depth so a cap raised on the old graph never carries
        // into the new one (#123). The reveal MODE is a persistent UI preference
        // and is intentionally NOT reset here.
        hop_cap.set(DEFAULT_HOP_CAP);

        if node_display_overrides
            .peek()
            .keys()
            .any(|stage_id| !visible_stage_ids.contains(stage_id))
        {
            node_display_overrides
                .write()
                .retain(|stage_id, _| visible_stage_ids.contains(stage_id));
        }
    });

    // An empty closure means no field cables and no dim; global field search only
    // highlights/reveals matching rows and does not recede the node-level DAG.
    let mut active_field_edges: Vec<(usize, FieldEdgeAnchors)> = Vec::new();
    let mut active_role_edges: Vec<(usize, FieldEdgeAnchors)> = Vec::new();
    let mut participating_nodes: HashSet<usize> = HashSet::new();
    // Resolve each participating edge to drawable anchors in ONE pass, collecting
    // the participating nodes (for the dim exemption) and the resolved edges. Only
    // edges whose anchors RESOLVE feed the highlight/dim/cable sets, so all three
    // derive from one set — a tinted cell can never land on a dimmed, cable-less
    // card.
    let mut resolved_edges: Vec<&FieldEdge> = Vec::with_capacity(closure.len());
    let mut closure_indices: Vec<usize> = closure.iter().copied().collect();
    closure_indices.sort_unstable();
    for ei in closure_indices {
        let edge = &field_edges[ei];
        if let Some(anchors) = resolve_edge_anchors(&stages, edge, field_edge_paths.get(ei)) {
            participating_nodes.insert(edge.from_node);
            participating_nodes.insert(edge.to_node);
            active_field_edges.push((ei, anchors));
            resolved_edges.push(edge);
        }
    }
    let mut role_closure_indices: Vec<usize> = role_closure.iter().copied().collect();
    role_closure_indices.sort_unstable();
    for ei in role_closure_indices {
        let edge = &role_edges[ei];
        if let Some(anchors) = resolve_role_edge_anchors(&stages, edge, role_edge_paths.get(ei)) {
            participating_nodes.insert(edge.from_node);
            participating_nodes.insert(edge.to_node);
            active_role_edges.push((ei, anchors));
        }
    }
    let active_field_endpoints: Vec<ConnectorEndpoints> = active_field_edges
        .iter()
        .map(|(_, anchors)| ConnectorEndpoints {
            sx: anchors.start.0,
            sy: anchors.start.1,
            tx: anchors.end.0,
            ty: anchors.end.1,
        })
        .collect();
    let active_field_channel_paths =
        obstacle_aware_channel_paths(&active_field_endpoints, &connector_obstacles);
    for ((_, anchors), path) in active_field_edges
        .iter_mut()
        .zip(active_field_channel_paths.into_iter())
    {
        if path.points.len() >= 2 {
            anchors.path = Some(path);
        }
    }
    let active_role_endpoints: Vec<ConnectorEndpoints> = active_role_edges
        .iter()
        .map(|(_, anchors)| ConnectorEndpoints {
            sx: anchors.start.0,
            sy: anchors.start.1,
            tx: anchors.end.0,
            ty: anchors.end.1,
        })
        .collect();
    let active_role_channel_paths =
        obstacle_aware_channel_paths(&active_role_endpoints, &connector_obstacles);
    for ((_, anchors), path) in active_role_edges
        .iter_mut()
        .zip(active_role_channel_paths.into_iter())
    {
        if path.points.len() >= 2 {
            anchors.path = Some(path);
        }
    }
    // Per-node lineage-endpoint field names (#87): node index → the field rows on
    // that card to tint, so a multi-field node shows *which row* is the
    // source/target — not just that the card participates (the whole-node dim
    // conveys the latter). `group_endpoints_by_node` returns sorted, de-duplicated
    // names so each card's prop is stable across renders, and an empty map when
    // nothing is hovered (each card then gets an empty Vec without a per-node scan).
    // `mut` because each card's names are MOVED out below via `.remove(&index)`
    // (each index is visited once, so this never loses a card's names).
    let mut highlighted_by_node = group_endpoints_by_node(resolved_edges);
    append_highlights(
        &mut highlighted_by_node,
        &lineage_role_source_fields_by_node,
    );
    append_highlights(&mut highlighted_by_node, &global_matches_by_node);
    let mut highlighted_role_ports_by_node =
        role_endpoint_names_by_node(&role_edges, &role_closure)
            .into_iter()
            .map(|(node, ports)| {
                let mut ports = ports.into_iter().collect::<Vec<_>>();
                ports.sort();
                (node, ports)
            })
            .collect::<HashMap<_, _>>();
    // Any field hovered with a non-empty closure dims the rest of the canvas.
    let hover_active = !active_field_edges.is_empty() || !active_role_edges.is_empty();

    // The node anchoring the active reveal — a pinned/selected column wins over a
    // hover, mirroring the closure dispatch above. Used as the Filter keep-set's
    // seed so the selected card is never hidden, even with no resolved edges.
    let anchor_node: Option<usize> = match &pinned_selection {
        Some(LineageTarget::Field(node, _) | LineageTarget::RolePort(node, _)) => Some(*node),
        None => match &hover_target {
            HoverTarget::Field(node, _) | HoverTarget::RolePort(node, _) => Some(*node),
            HoverTarget::None => None,
        },
    };

    // Filter mode (#123): the set of node indices to KEEP visible while a reveal is
    // active — the anchor plus every endpoint of a participating (resolved) field
    // edge. `lineage_keep_nodes` is the tested helper that guarantees no dangling
    // path (an edge is kept only when BOTH endpoints are kept). The role-edge
    // endpoints are folded in too, so a role port on a kept connecting path does
    // not vanish; both edge kinds share the node-index space. Computed ONLY when
    // Filter mode is actually suppressing a reveal, so Highlight mode pays nothing
    // and keeps byte-for-byte its prior behavior.
    let filter_keep_nodes: Option<HashSet<usize>> = if is_filter_mode && hover_active {
        let participating_field_edges = active_field_edges.iter().map(|(ei, _)| &field_edges[*ei]);
        let mut keep =
            lineage_keep_nodes(participating_field_edges, anchor_node.unwrap_or_default());
        for (ei, _) in &active_role_edges {
            let edge = &role_edges[*ei];
            keep.insert(edge.from_node);
            keep.insert(edge.to_node);
        }
        Some(keep)
    } else {
        None
    };

    // Whether raising the hop cap would reveal MORE of the SELECTED column's
    // lineage (#123). Only meaningful in Filter mode — Highlight keeps the full
    // uncapped closure, so there is never anything to expand there. True when a
    // column OR role port is pinned/selected and its uncapped closure strictly
    // exceeds the (capped) `closure` already computed above — i.e. the cap is
    // currently clipping. Reuses `closure`/`role_closure` rather than recomputing
    // the capped walk; the uncapped walk runs only in this Filter+pinned case, so
    // Highlight and idle hovers pay nothing. Hover reveals are 1-hop, never capped.
    let can_expand_lineage: bool = is_filter_mode
        && match &pinned_selection {
            Some(LineageTarget::Field(node, field)) => {
                field_lineage_full(&field_edges, *node, field).len() > closure.len()
            }
            Some(LineageTarget::RolePort(_, _)) => {
                // The role-port closure is the union of its source fields' lineage;
                // compare the uncapped union against the capped `closure`.
                let mut uncapped: HashSet<usize> = HashSet::new();
                for role_index in &role_closure {
                    if let Some(edge) = role_edges.get(*role_index) {
                        uncapped.extend(field_lineage_full(
                            &field_edges,
                            edge.from_node,
                            &edge.from_field,
                        ));
                    }
                }
                uncapped.len() > closure.len()
            }
            None => false,
        };

    // Bounding box of the current layout (None when empty). `LayoutBounds` is
    // Copy, so each fit handler captures it without cloning the stage list.
    let bounds = layout_bounds(&stages);

    // Fit-to-view shared by the FIT/RESET buttons and the double-click handler.
    // Signals are `Copy`, so each handler captures its own copy and calls this
    // free helper rather than sharing one `FnMut` closure (which can't be moved
    // into multiple handlers). No-op when there are no nodes to frame.
    let fit_to_view =
        move |mut pan_x: Signal<f32>, mut pan_y: Signal<f32>, mut zoom: Signal<f32>| {
            let Some(b) = bounds else { return };
            let (px, py, z) = fit_transform(
                b,
                *viewport_w.peek(),
                *viewport_h.peek(),
                FIT_MARGIN,
                ZOOM_MIN,
                ZOOM_MAX,
            );
            pan_x.set(px);
            pan_y.set(py);
            zoom.set(z);
        };

    // ── Event handler closures ────────────────────────────────────────────────

    let drag_down = {
        let drag = drag.clone();
        move |e: MouseEvent| {
            // Only initiate pan on left-button (button 0) or middle-button (1).
            // Right-click is reserved for a future context menu.
            if e.trigger_button() == Some(dioxus::html::input_data::MouseButton::Primary)
                || e.trigger_button() == Some(dioxus::html::input_data::MouseButton::Auxiliary)
            {
                let pos = e.client_coordinates();
                let mut d = drag.borrow_mut();
                d.active = true;
                d.start_x = pos.x as f32;
                d.start_y = pos.y as f32;
                d.start_pan_x = *pan_x.peek();
                d.start_pan_y = *pan_y.peek();
            }
        }
    };

    let drag_move = {
        let drag = drag.clone();
        move |e: MouseEvent| {
            let d = drag.borrow();
            if d.active {
                let pos = e.client_coordinates();
                let dx = pos.x as f32 - d.start_x;
                let dy = pos.y as f32 - d.start_y;
                pan_x.set(d.start_pan_x + dx);
                pan_y.set(d.start_pan_y + dy);
            }
        }
    };

    let drag_up = {
        let drag = drag.clone();
        move |_: MouseEvent| {
            drag.borrow_mut().active = false;
        }
    };

    let on_wheel = move |e: WheelEvent| {
        // Compute a zoom multiplier from the wheel delta.
        // Positive delta_y = scroll down = zoom out (< 1).
        // Negative delta_y = scroll up   = zoom in  (> 1).
        let factor = match e.delta() {
            WheelDelta::Pixels(data) => {
                let dy = data.y as f32;
                if dy == 0.0 {
                    return;
                }
                1.0 - dy * ZOOM_STEP_PIXEL
            }
            WheelDelta::Lines(data) => {
                let dy = data.y as f32;
                if dy == 0.0 {
                    return;
                }
                if dy < 0.0 {
                    ZOOM_STEP_LINE
                } else {
                    1.0 / ZOOM_STEP_LINE
                }
            }
            WheelDelta::Pages(data) => {
                let dy = data.y as f32;
                if dy == 0.0 {
                    return;
                }
                if dy < 0.0 {
                    ZOOM_STEP_LINE * ZOOM_STEP_LINE
                } else {
                    1.0 / (ZOOM_STEP_LINE * ZOOM_STEP_LINE)
                }
            }
        };

        let old_z = *zoom.peek();
        let new_z = (old_z * factor).clamp(ZOOM_MIN, ZOOM_MAX);
        if (new_z - old_z).abs() < 0.0001 {
            return;
        }

        // Anchor zoom to cursor position (cursor stays fixed in world space).
        let cursor = e.client_coordinates();
        let cx = cursor.x as f32;
        let cy = cursor.y as f32;
        let old_px = *pan_x.peek();
        let old_py = *pan_y.peek();
        let ratio = new_z / old_z;

        pan_x.set(cx - (cx - old_px) * ratio);
        pan_y.set(cy - (cy - old_py) * ratio);
        zoom.set(new_z);
    };

    // SVG overlay bounds — computed from stage bounding box with padding.
    let (svg_w, svg_h) = if stages.is_empty() {
        (1200.0_f32, 400.0_f32)
    } else {
        let max_x = stages
            .iter()
            .map(|s| s.canvas_x + NODE_WIDTH)
            .fold(0.0_f32, f32::max);
        // Use each card's own height: a field-bearing card extends below
        // `NODE_HEIGHT`, so its bottom-row cables (and their hover hit-areas)
        // would be clipped if the overlay were sized by the fixed header height.
        let max_y = stages
            .iter()
            .zip(field_displays.iter())
            .map(|(s, display)| s.canvas_y + rendered_card_height(s, display))
            .fold(0.0_f32, f32::max);
        let min_y = stages.iter().map(|s| s.canvas_y).fold(f32::MAX, f32::min);
        // Ensure SVG covers negative-Y nodes (secondary inputs above the chain).
        let _ = min_y; // min_y handled by SVG viewBox if needed; overflow:visible covers it.
        (max_x + 80.0, max_y + 80.0)
    };
    let node_cards: Vec<(usize, StageView, FieldDisplayInfo, String, String, String)> = stages
        .iter()
        .cloned()
        .zip(field_displays.iter().cloned())
        .enumerate()
        .map(|(index, (stage, display))| {
            let query_stage_id = stage.id.clone();
            let expand_stage_id = stage.id.clone();
            let display_stage_id = stage.id.clone();
            (
                index,
                stage,
                display,
                query_stage_id,
                expand_stage_id,
                display_stage_id,
            )
        })
        .collect();

    // Channel view mode toggle state (`tab_mgr` captured once at the top).
    let has_channel = tab_mgr
        .channel_state
        .read()
        .as_ref()
        .is_some_and(|cs| cs.active_channel.is_some());
    let is_resolved = view_mode == ChannelViewMode::Resolved;

    rsx! {
        div {
            class: "klinx-canvas-column",
            // Surface the active reveal mode as a data attribute (consistent with
            // `data-layout`/`data-context`), so CSS and PR5's focus toggle can key
            // off it without re-reading the signal (#123).
            "data-reveal-mode": reveal_mode.as_data_attr(),

            // ── Breadcrumb bar (composition drill-in navigation) ─────────
            {
                let stack = state.composition_drill_stack.read();
                if !stack.is_empty() {
                    let frames: Vec<_> = stack.iter().map(|f| f.alias.clone()).collect();
                    drop(stack);
                    rsx! {
                        super::breadcrumbs::BreadcrumbBar {
                            frames,
                        }
                    }
                } else {
                    drop(stack);
                    rsx! {}
                }
            }

            // ── Channel view mode toggle bar ─────────────────────────────
            div {
                class: "klinx-canvas-toolbar",

                button {
                    class: if is_resolved { "klinx-view-toggle klinx-view-toggle--active" } else { "klinx-view-toggle" },
                    disabled: !has_channel && !is_resolved,
                    title: if !has_channel && !is_resolved { "Select a channel to enable resolved view" } else if is_resolved { "Switch to Raw view" } else { "Switch to Resolved view" },
                    onclick: move |_| {
                        let mut mode = state.channel_view_mode;
                        let current = *mode.read();
                        mode.set(match current {
                            ChannelViewMode::Raw => ChannelViewMode::Resolved,
                            ChannelViewMode::Resolved => ChannelViewMode::Raw,
                        });
                    },
                    span { class: "klinx-view-toggle-label",
                        if is_resolved { "RESOLVED" } else { "RAW" }
                    }
                }

                // ── Lineage reveal-mode toggle (#123, focus surface for #152) ──
                // The persistent FOCUS control (#152): Highlight mode dims every
                // off-path card for the SELECTED field (persistent, not hover-only
                // — a selected field populates the same closure a hover would), so
                // the lit ribbon stands out against a receded background; Filter
                // mode instead HIDES off-path cards (#123, distinct). Reuses the
                // per-tab `lineage_reveal_mode` — no parallel dim mechanism — so a
                // field selected with Highlight active reads as a focused path.
                button {
                    class: if is_filter_mode { "klinx-view-toggle klinx-view-toggle--active" } else { "klinx-view-toggle" },
                    title: if is_filter_mode { "Filter mode: off-path cards are hidden. Click to focus (dim off-path) instead." } else { "Focus mode: off-path cards are dimmed for the selected field's path. Click to hide them instead." },
                    onclick: move |_| {
                        let mut mode = state.lineage_reveal_mode;
                        let next = mode.read().toggled();
                        mode.set(next);
                    },
                    span { class: "klinx-view-toggle-label", "{reveal_mode.label()}" }
                }

                // ── Dual value/influence ribbon toggles (#152) ───────────────
                // Two INDEPENDENT toggles for the whole-path ribbon: VALUE lights
                // the solid DIRECT "value lineage" cables; INFLUENCE lights the
                // ghosted INDIRECT "influence halo" (#147). Each gates only its own
                // ribbon in the overlay draw; toggling one never touches the other,
                // and neither changes the focus/dim closure. Both default ON.
                button {
                    class: if value_ribbon_on { "klinx-view-toggle klinx-view-toggle--active" } else { "klinx-view-toggle" },
                    title: if value_ribbon_on { "Value lineage ribbon shown (DIRECT cables). Click to hide it." } else { "Value lineage ribbon hidden. Click to show the DIRECT cables." },
                    onclick: move |_| {
                        let next = !*show_value_ribbon.peek();
                        show_value_ribbon.set(next);
                    },
                    span { class: "klinx-view-toggle-label", "VALUE" }
                }
                button {
                    class: if influence_ribbon_on { "klinx-view-toggle klinx-view-toggle--active" } else { "klinx-view-toggle" },
                    title: if influence_ribbon_on { "Influence halo shown (INDIRECT cables). Click to hide it." } else { "Influence halo hidden. Click to show the INDIRECT cables." },
                    onclick: move |_| {
                        let next = !*show_influence_ribbon.peek();
                        show_influence_ribbon.set(next);
                    },
                    span { class: "klinx-view-toggle-label", "INFLUENCE" }
                }

                // ── Expand-further affordance (#123) ─────────────────────────
                // Raises the active hop cap, revealing more of a deep selected
                // lineage. Shown only when the cap is currently clipping the
                // closure (more lineage exists past the bound).
                if can_expand_lineage {
                    button {
                        class: "klinx-view-toggle klinx-lineage-expand",
                        title: "Reveal more of this column's lineage (raise the hop limit)",
                        onclick: move |_| {
                            let next = hop_cap.peek().saturating_add(HOP_CAP_STEP);
                            hop_cap.set(next);
                        },
                        span { class: "klinx-view-toggle-label", "EXPAND +" }
                    }
                }

                // ── Extract as Composition button (enabled with 2+ nodes selected) ──
                {
                    let count = state.selected_stages.read().len();
                    rsx! {
                        button {
                            class: "klinx-view-toggle",
                            disabled: count < 2,
                            title: if count < 2 { "Select 2+ nodes to extract as composition" } else { "Extract selected nodes as a composition" },
                            onclick: move |_| {
                                // TODO: open extraction modal.
                            },
                            span { class: "klinx-view-toggle-label", "EXTRACT" }
                        }
                    }
                }

                // ── Fit to View — frame all nodes in the viewport ──────────
                button {
                    class: "klinx-view-toggle",
                    disabled: bounds.is_none(),
                    title: if bounds.is_none() { "No nodes to fit" } else { "Fit all nodes to the viewport" },
                    onclick: move |_| fit_to_view(pan_x, pan_y, zoom),
                    span { class: "klinx-view-toggle-label", "FIT" }
                }

                // ── Reset Layout — re-run the engine layout and re-fit. With no
                // persisted position overrides yet (issue #3 PR 3b), positions
                // are already recomputed every render, so reset == re-fit. ──
                button {
                    class: "klinx-view-toggle",
                    disabled: bounds.is_none(),
                    title: if bounds.is_none() { "No layout to reset" } else { "Reset to the engine-computed layout" },
                    onclick: move |_| fit_to_view(pan_x, pan_y, zoom),
                    span { class: "klinx-view-toggle-label", "RESET" }
                }

                div {
                    class: "klinx-display-mode-group",
                    for display_mode in GlobalNodeDisplayMode::ALL {
                        button {
                            class: if global_display_mode == display_mode { "klinx-view-toggle klinx-view-toggle--active" } else { "klinx-view-toggle" },
                            title: "{display_mode.title()}",
                            onclick: move |_| {
                                global_node_display_mode.set(display_mode);
                            },
                            span { class: "klinx-view-toggle-label", "{display_mode.label()}" }
                        }
                    }
                }

                div {
                    class: "klinx-global-field-search",
                    input {
                        class: "klinx-global-field-search-input",
                        r#type: "search",
                        value: "{global_query}",
                        placeholder: "find fields",
                        title: "Find fields across visible canvas nodes. Supports * and ? wildcards.",
                        oninput: move |e: FormEvent| {
                            global_field_query.set(e.value());
                        },
                    }
                }
            }

            div {
                class: "klinx-canvas-panel",
            // Measure the pane on mount (guaranteed) and on every resize so
            // Fit-to-View frames the graph against the real viewport size.
            onmounted: move |evt| {
                spawn(async move {
                    if let Ok(rect) = evt.get_client_rect().await {
                        viewport_w.set(rect.size.width.max(1.0) as f32);
                        viewport_h.set(rect.size.height.max(1.0) as f32);
                    }
                });
            },
            onresize: move |evt| {
                if let Ok(size) = evt.get_content_box_size() {
                    viewport_w.set(size.width.max(1.0) as f32);
                    viewport_h.set(size.height.max(1.0) as f32);
                }
            },
            // Events on the outer panel — pointer capture is a future enhancement.
            onmousedown: drag_down,
            onmousemove: drag_move,
            onmouseup: drag_up,
            // Cancel drag if pointer leaves the panel entirely.
            onmouseleave: move |_| { drag.borrow_mut().active = false; },
            onwheel: on_wheel,
            // Double-click empty canvas fits all nodes to view. Node cards
            // stop_propagation() on mousedown, so this fires only on the background.
            ondoubleclick: move |_| fit_to_view(pan_x, pan_y, zoom),
            // Clicking empty canvas deselects any selected node AND clears a pinned
            // field lineage (#75). Node and field-row clicks call stop_propagation(),
            // so this only fires on empty space.
            onclick: move |_| {
                let mut sel = state.selected_stages;
                sel.set(std::collections::HashSet::new());
                let mut pinned = pinned_field;
                pinned.0.set(None);
                let mut selected_field = state.selected_field;
                selected_field.set(None);
            },

            // ── Transformed viewport ──────────────────────────────────────
            div {
                class: "klinx-canvas-viewport",
                style: "transform: translate({pan_x}px, {pan_y}px) scale({zoom});",

                // Default SVG connector overlay. Its channels are populated
                // from the currently visible node-level connections and drawn
                // below cards so node bodies remain readable and clickable.
                svg {
                    class: "klinx-canvas-svg klinx-canvas-svg--base",
                    width: "{svg_w}",
                    height: "{svg_h}",
                    // Node-level connectors — the DEFAULT view. While a field's
                    // lineage is revealed, only THIS group recedes so the field
                    // cables in the active overlay read clearly against it.
                    g {
                        class: if hover_active { "klinx-canvas-edges klinx-canvas-edges--recede" } else { "klinx-canvas-edges" },
                        for conn in connections {
                            // Filter mode (#123): draw a node-level connector only
                            // when BOTH endpoints survive the keep-set, so a cable
                            // never dangles to a hidden card. Highlight mode keeps
                            // every connector (`keep` is None) and only recedes it.
                            if filter_keep_nodes.as_ref().is_none_or(|keep| {
                                keep.contains(&conn.from_index) && keep.contains(&conn.to_index)
                            }) {
                                Connector {
                                    key: "{conn.from.id}-{conn.to.id}-{conn.from_branch:?}",
                                    from: conn.from,
                                    to: conn.to,
                                    from_branch: conn.from_branch,
                                    path: conn.path,
                                }
                            }
                        }
                    }
                }

                // Node cards
                for (index, stage, display, query_stage_id, expand_stage_id, display_stage_id) in node_cards {
                    // Hand each card its pre-grouped lineage-endpoint field names
                    // (#87). `remove` MOVES the Vec out (each index is visited once,
                    // so this never loses a card's names), avoiding a clone and any
                    // per-node scan of a global set. A non-endpoint card gets a
                    // fresh empty Vec: cheap, and unchanged across renders so
                    // CanvasNode's PartialEq memoization holds.
                    CanvasNode {
                        key: "{stage.id}",
                        stage,
                        index,
                        field_display: display.clone(),
                        on_field_query: move |query: String| {
                            hovered_field.force_clear_if_node(index);
                            if pinned_field
                                .0
                                .peek()
                                .as_ref()
                                .is_some_and(|target| target.node() == index)
                            {
                                pinned_field.0.set(None);
                            }
                            if state
                                .selected_field
                                .peek()
                                .as_ref()
                                .is_some_and(|field| field.stage_id == query_stage_id)
                            {
                                let mut selected_field = state.selected_field;
                                selected_field.set(None);
                            }
                            let mut states = field_display_states.write();
                            let entry = states.entry(query_stage_id.clone()).or_default();
                            entry.query = query;
                            entry.visible_limit = FIELD_ROW_CAP;
                        },
                        on_field_toggle: move |_| {
                            let mut states = field_display_states.write();
                            let entry = states.entry(expand_stage_id.clone()).or_default();
                            if display.hidden_count > 0 {
                                let page_size = page_size_for_mode(display.mode);
                                entry.visible_limit = entry
                                    .visible_limit
                                    .max(page_size)
                                    .saturating_add(page_size);
                            } else if display.can_reduce {
                                entry.visible_limit = 0;
                            }
                        },
                        on_display_action: move |action: NodeDisplayAction| {
                            hovered_field.force_clear_if_node(index);
                            if pinned_field
                                .0
                                .peek()
                                .as_ref()
                                .is_some_and(|target| target.node() == index)
                            {
                                pinned_field.0.set(None);
                            }
                            if state
                                .selected_field
                                .peek()
                                .as_ref()
                                .is_some_and(|field| field.stage_id == display_stage_id)
                            {
                                let mut selected_field = state.selected_field;
                                selected_field.set(None);
                            }
                            let mut overrides = node_display_overrides.write();
                            match action {
                                NodeDisplayAction::ClearOverride => {
                                    overrides.remove(&display_stage_id);
                                }
                                NodeDisplayAction::CycleOverride => {
                                    let next = overrides
                                        .get(&display_stage_id)
                                        .copied()
                                        .map_or(
                                            Some(ResolvedNodeDisplayMode::Compact),
                                            ResolvedNodeDisplayMode::next_override,
                                        );
                                    match next {
                                        Some(mode) => {
                                            overrides.insert(display_stage_id.clone(), mode);
                                        }
                                        None => {
                                            overrides.remove(&display_stage_id);
                                        }
                                    }
                                }
                            }
                        },
                        // Highlight mode: dim cards outside the revealed field's
                        // lineage closure (today's behavior). In Filter mode no card
                        // is dimmed — off-path cards are HIDDEN instead (#123).
                        dimmed: hover_active && filter_keep_nodes.is_none() && !participating_nodes.contains(&index),
                        // Filter mode: hide cards outside the keep-set so only the
                        // connecting paths remain. `keep` is None outside Filter mode,
                        // so this is always false in Highlight mode.
                        hidden: filter_keep_nodes.as_ref().is_some_and(|keep| !keep.contains(&index)),
                        highlighted_fields: highlighted_by_node.remove(&index).unwrap_or_default(),
                        highlighted_role_ports: highlighted_role_ports_by_node.remove(&index).unwrap_or_default(),
                    }
                }

                // Active field-level cables — ONLY the hovered/pinned field's
                // lineage closure, never the whole field-edge set. The overlay
                // is above default cables but below cards, so field rows mask
                // any stroke through their interiors and keep pointer control.
                // The dual ribbon toggles (#152) filter WHICH cables draw here by
                // edge nature: a hidden ribbon thins the drawn set only — the
                // closure that drives the dim/focus is unchanged. Edges keep their
                // stable identity key (`field-{ei}`/`role-{ei}`), so toggling a
                // ribbon adds/removes whole cables without re-keying survivors.
                if hover_active {
                    svg {
                        class: "klinx-canvas-svg klinx-canvas-svg--active",
                        width: "{svg_w}",
                        height: "{svg_h}",
                        for (ei, anchors) in active_field_edges
                            .into_iter()
                            .filter(|(_, anchors)| {
                                ribbon_edge_visible(
                                    anchors.kind.nature(),
                                    value_ribbon_on,
                                    influence_ribbon_on,
                                )
                            })
                        {
                            FieldConnector {
                                key: "field-{ei}",
                                start: anchors.start,
                                end: anchors.end,
                                kind_attr: anchors.kind_attr,
                                kind: anchors.kind,
                                precision: anchors.precision,
                                path: anchors.path,
                                spotlight: spotlight_edges.contains(&ei),
                            }
                        }
                        for (ei, anchors) in active_role_edges
                            .into_iter()
                            .filter(|(_, anchors)| {
                                ribbon_edge_visible(
                                    anchors.kind.nature(),
                                    value_ribbon_on,
                                    influence_ribbon_on,
                                )
                            })
                        {
                            FieldConnector {
                                key: "role-{ei}",
                                start: anchors.start,
                                end: anchors.end,
                                kind_attr: anchors.kind_attr,
                                kind: anchors.kind,
                                precision: anchors.precision,
                                path: anchors.path,
                                spotlight: spotlight_role_edges.contains(&ei),
                            }
                        }
                    }
                }
            }
        }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use clinker_plan::config::parse_config;

    use crate::pipeline_view::layout_model::{CanvasLayoutEngine, apply_canvas_layout};
    use crate::pipeline_view::{
        CanvasPoint, FIELD_ROW_HEIGHT, FieldKind, FieldRow, NODE_WIDTH, RouteBranch, StageKind,
        StageView, derive_pipeline_view,
    };

    use super::super::connector::{
        ConnectorEndpoints, ConnectorObstacle, obstacle_aware_channel_paths,
    };
    use super::{
        FIELD_ROW_CAP, FieldDisplayState, FieldProjectionContext, FieldRankSignals,
        GlobalNodeDisplayMode, GraphDisplayProfile, PREVIEW_FIELD_ROW_CAP, ResolvedNodeDisplayMode,
        field_matches_by_node, preview_rank, project_stage_fields, rendered_card_height,
        resolve_node_display_mode, text_matches_query,
    };

    fn wide_stage(count: usize) -> StageView {
        StageView {
            id: "wide".to_string(),
            label: "wide".to_string(),
            kind: StageKind::Transform,
            subtitle: "wide schema".to_string(),
            canvas_x: 10.0,
            canvas_y: 20.0,
            cxl_source: None,
            description: None,
            error_message: None,
            fields: (0..count)
                .map(|i| FieldRow {
                    name: format!("field_{i:03}"),
                    kind: if i % 2 == 0 {
                        FieldKind::Declared
                    } else {
                        FieldKind::PassThrough
                    },
                    ty: Some(if i == 99 { "customer_id" } else { "int" }.to_string()),
                    is_correlation_key: false,
                    ..Default::default()
                })
                .collect(),
            branches: Vec::<RouteBranch>::new(),
            role_ports: Vec::new(),
        }
    }

    fn project_for_test(
        stage: &StageView,
        state: &FieldDisplayState,
        mode: ResolvedNodeDisplayMode,
        temporary_fields: &HashSet<String>,
    ) -> super::ProjectedStage {
        project_stage_fields(
            stage,
            state,
            FieldProjectionContext {
                stage_index: 0,
                mode,
                global_mode: GlobalNodeDisplayMode::Auto,
                override_mode: None,
                temporary_fields,
                rank_signals: &FieldRankSignals::default(),
            },
        )
    }

    #[test]
    fn wide_schema_projection_caps_default_rows() {
        let stage = wide_stage(120);
        let projected = project_for_test(
            &stage,
            &FieldDisplayState::default(),
            ResolvedNodeDisplayMode::Schema,
            &HashSet::new(),
        );

        assert_eq!(projected.stage.fields.len(), FIELD_ROW_CAP);
        assert_eq!(projected.display.total_count, 120);
        assert_eq!(projected.display.matching_count, 120);
        assert_eq!(projected.display.hidden_count, 120 - FIELD_ROW_CAP);
        assert_eq!(projected.display.next_count, FIELD_ROW_CAP);
        assert!(projected.display.searchable);
        assert_eq!(
            projected.stage.card_height(),
            stage.clone().tap_fields(FIELD_ROW_CAP).card_height(),
            "card height follows the displayed row set"
        );
        assert!(projected.stage.field_index("field_099").is_none());
    }

    #[test]
    fn wide_schema_projection_filters_and_expands_rows() {
        let stage = wide_stage(120);
        let filtered = project_for_test(
            &stage,
            &FieldDisplayState {
                visible_limit: FIELD_ROW_CAP,
                query: "customer".to_string(),
            },
            ResolvedNodeDisplayMode::Schema,
            &HashSet::new(),
        );

        assert_eq!(filtered.stage.fields.len(), 1);
        assert_eq!(filtered.stage.fields[0].name, "field_099");
        assert_eq!(filtered.display.matching_count, 1);
        assert_eq!(filtered.display.hidden_count, 0);
        assert_eq!(filtered.display.next_count, 0);
        assert_eq!(filtered.stage.field_index("field_099"), Some(0));

        let paged = project_for_test(
            &stage,
            &FieldDisplayState {
                visible_limit: FIELD_ROW_CAP * 2,
                query: String::new(),
            },
            ResolvedNodeDisplayMode::Schema,
            &HashSet::new(),
        );

        assert_eq!(paged.stage.fields.len(), FIELD_ROW_CAP * 2);
        assert_eq!(paged.display.hidden_count, 120 - FIELD_ROW_CAP * 2);
        assert_eq!(paged.display.next_count, FIELD_ROW_CAP);
        assert!(paged.display.can_reduce);
        assert_eq!(paged.stage.fields[FIELD_ROW_CAP].name, "field_024");
        assert_eq!(paged.stage.field_index("field_099"), None);

        let fully_visible = project_for_test(
            &stage,
            &FieldDisplayState {
                visible_limit: 120,
                query: String::new(),
            },
            ResolvedNodeDisplayMode::Schema,
            &HashSet::new(),
        );

        assert_eq!(fully_visible.stage.fields.len(), 120);
        assert_eq!(fully_visible.display.hidden_count, 0);
        assert_eq!(fully_visible.display.next_count, 0);
        assert!(fully_visible.display.can_reduce);
        assert_eq!(fully_visible.stage.field_index("field_099"), Some(99));
    }

    #[test]
    fn wide_schema_projection_appends_temporary_hidden_fields() {
        let stage = wide_stage(120);
        let temporary = HashSet::from(["field_099".to_string(), "field_010".to_string()]);
        let projected = project_for_test(
            &stage,
            &FieldDisplayState::default(),
            ResolvedNodeDisplayMode::Schema,
            &temporary,
        );

        assert_eq!(projected.stage.fields.len(), FIELD_ROW_CAP + 1);
        assert_eq!(
            projected
                .stage
                .fields
                .last()
                .map(|field| field.name.as_str()),
            Some("field_099")
        );
        assert_eq!(projected.display.temporary_fields, vec!["field_099"]);
        assert_eq!(projected.display.hidden_count, 120 - FIELD_ROW_CAP);

        let expanded = project_for_test(
            &stage,
            &FieldDisplayState {
                visible_limit: 120,
                query: String::new(),
            },
            ResolvedNodeDisplayMode::Schema,
            &temporary,
        );
        assert_eq!(expanded.stage.fields.len(), 120);
        assert!(expanded.display.temporary_fields.is_empty());
    }

    #[test]
    fn preview_projection_keeps_visible_active_rows_stable_and_appends_hidden_ones() {
        let mut stage = wide_stage(0);
        stage.kind = StageKind::Combine;
        stage.fields = vec![
            FieldRow {
                name: "filler".to_string(),
                kind: FieldKind::PassThrough,
                ..Default::default()
            },
            FieldRow {
                name: "declared".to_string(),
                kind: FieldKind::Declared,
                ..Default::default()
            },
            FieldRow {
                name: "join_key".to_string(),
                kind: FieldKind::PassThrough,
                ..Default::default()
            },
            FieldRow {
                name: "derived_total".to_string(),
                kind: FieldKind::Emitted,
                ..Default::default()
            },
            FieldRow {
                name: "correlation".to_string(),
                kind: FieldKind::Declared,
                is_correlation_key: true,
                ..Default::default()
            },
            FieldRow {
                name: "extra".to_string(),
                kind: FieldKind::PassThrough,
                ..Default::default()
            },
            FieldRow {
                name: "active_match".to_string(),
                kind: FieldKind::PassThrough,
                ..Default::default()
            },
        ];
        let mut rank_signals = FieldRankSignals::default();
        rank_signals
            .operator_relevant_by_node
            .entry(0)
            .or_default()
            .insert("join_key".to_string());
        let temporary = HashSet::from(["active_match".to_string(), "filler".to_string()]);

        let projected = project_stage_fields(
            &stage,
            &FieldDisplayState::default(),
            FieldProjectionContext {
                stage_index: 0,
                mode: ResolvedNodeDisplayMode::Preview,
                global_mode: GlobalNodeDisplayMode::Auto,
                override_mode: None,
                temporary_fields: &temporary,
                rank_signals: &rank_signals,
            },
        );

        let names = projected
            .stage
            .fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "correlation",
                "derived_total",
                "join_key",
                "declared",
                "filler",
                "extra",
                "active_match",
            ]
        );
        assert_eq!(projected.stage.fields.len(), PREVIEW_FIELD_ROW_CAP + 1);
        assert_eq!(projected.display.hidden_count, 1);
        assert_eq!(projected.display.temporary_fields, vec!["active_match"]);
    }

    #[test]
    fn preview_rank_prioritizes_aggregate_grain_rows() {
        let stage = wide_stage(0);
        let field = FieldRow {
            name: "invoice_date".to_string(),
            kind: FieldKind::PassThrough,
            ..Default::default()
        };
        // The grain is no longer a row flag (#147); it is read from the per-node
        // `GroupBy`-edge grain set on the rank signals.
        let mut signals = FieldRankSignals::default();
        signals
            .aggregate_grain_by_node
            .entry(0)
            .or_default()
            .insert("invoice_date".to_string());

        assert_eq!(
            preview_rank(0, &stage, &field, &signals),
            1,
            "aggregate failure-grain rows should stay visible like source CK rows"
        );
        // A field NOT in the grain set is not prioritized to rank 1 by grain.
        assert_ne!(
            preview_rank(0, &stage, &field, &FieldRankSignals::default()),
            1,
            "without a GroupBy edge the row is not treated as grain"
        );
    }

    #[test]
    fn compact_projection_hides_fields_but_temporarily_reveals_active_endpoint() {
        let stage = wide_stage(30);
        let temporary = HashSet::from(["field_029".to_string()]);
        let projected = project_for_test(
            &stage,
            &FieldDisplayState::default(),
            ResolvedNodeDisplayMode::Compact,
            &temporary,
        );

        assert_eq!(projected.stage.fields.len(), 1);
        assert_eq!(projected.stage.fields[0].name, "field_029");
        assert_eq!(projected.stage.field_index("field_029"), Some(0));
        assert_eq!(projected.display.hidden_count, 30);
        assert_eq!(projected.display.temporary_fields, vec!["field_029"]);
    }

    #[test]
    fn auto_mode_defaults_by_graph_size_schema_width_and_zoom() {
        let small = GraphDisplayProfile {
            node_count: 5,
            max_field_count: 12,
        };
        let wide = GraphDisplayProfile {
            node_count: 5,
            max_field_count: FIELD_ROW_CAP + 1,
        };
        let large = GraphDisplayProfile {
            node_count: 30,
            max_field_count: 8,
        };

        assert_eq!(
            resolve_node_display_mode(GlobalNodeDisplayMode::Auto, None, small, 1.0, false),
            ResolvedNodeDisplayMode::Schema
        );
        assert_eq!(
            resolve_node_display_mode(GlobalNodeDisplayMode::Auto, None, wide, 1.0, false),
            ResolvedNodeDisplayMode::Preview
        );
        assert_eq!(
            resolve_node_display_mode(GlobalNodeDisplayMode::Auto, None, large, 1.0, false),
            ResolvedNodeDisplayMode::Compact
        );
        assert_eq!(
            resolve_node_display_mode(GlobalNodeDisplayMode::Auto, None, large, 1.0, true),
            ResolvedNodeDisplayMode::Preview
        );
        assert_eq!(
            resolve_node_display_mode(GlobalNodeDisplayMode::Auto, None, small, 0.4, false),
            ResolvedNodeDisplayMode::Compact
        );
        assert_eq!(
            resolve_node_display_mode(GlobalNodeDisplayMode::Schema, None, large, 0.4, true),
            ResolvedNodeDisplayMode::Schema
        );
        assert_eq!(
            resolve_node_display_mode(
                GlobalNodeDisplayMode::Auto,
                Some(ResolvedNodeDisplayMode::Full),
                large,
                0.4,
                true,
            ),
            ResolvedNodeDisplayMode::Full
        );
    }

    #[test]
    fn compact_projection_keeps_branch_geometry_synced_with_temporary_fields() {
        let mut stage = wide_stage(4);
        stage.branches = vec![RouteBranch {
            name: "default".to_string(),
            predicate: None,
            is_default: true,
        }];

        let compact = project_for_test(
            &stage,
            &FieldDisplayState::default(),
            ResolvedNodeDisplayMode::Compact,
            &HashSet::new(),
        );
        assert!(compact.stage.fields.is_empty());
        assert_eq!(compact.stage.branches.len(), 1);
        let compact_branch_y = compact.stage.branch_anchor_out(0).1;

        let temporary = HashSet::from(["field_003".to_string()]);
        let revealed = project_for_test(
            &stage,
            &FieldDisplayState::default(),
            ResolvedNodeDisplayMode::Compact,
            &temporary,
        );

        assert_eq!(revealed.stage.fields.len(), 1);
        assert_eq!(
            revealed.stage.branch_anchor_out(0).1 - compact_branch_y,
            FIELD_ROW_HEIGHT,
            "branch anchors move by exactly one row when a temporary field appears"
        );
        assert_eq!(
            rendered_card_height(&revealed.stage, &revealed.display)
                - rendered_card_height(&compact.stage, &compact.display),
            FIELD_ROW_HEIGHT * 2.0,
            "rendered card height includes the temporary row plus its footer"
        );
    }

    #[test]
    fn field_search_accepts_wildcards() {
        assert!(text_matches_query("customer_id", "cust*"));
        assert!(text_matches_query("customer_id", "*_id"));
        assert!(text_matches_query("field_007", "field_00?"));
        assert!(!text_matches_query("field_010", "field_00?"));

        let stage = wide_stage(12);
        let matches =
            field_matches_by_node(&[stage], "field_00?", &std::collections::HashMap::new());
        assert_eq!(matches.get(&0).map(HashSet::len), Some(10));
    }

    #[test]
    fn dimmed_node_css_keeps_card_opaque() {
        let css = include_str!("../../../assets/klinx.css");
        let block = css_rule_block(css, ".klinx-node--dimmed").expect("dimmed node CSS rule");

        assert!(
            !block.contains("opacity:"),
            "dimmed cards must remain opaque so below-card connector strokes cannot show through"
        );
        assert!(
            block.contains("filter:"),
            "dimmed cards should recede visually without changing alpha"
        );
    }

    /// #123: Filter mode HIDES an off-path card (`display: none`) rather than
    /// dimming it — so the off-path card occupies no layout space and casts no
    /// below-card stroke. The hidden modifier is distinct from `--dimmed` (which
    /// recedes via `filter`), so Highlight mode's dim effect is untouched.
    #[test]
    fn filter_hidden_node_css_removes_card_from_layout() {
        let css = include_str!("../../../assets/klinx.css");
        let block =
            css_rule_block(css, ".klinx-node--hidden {").expect("hidden node CSS rule (#123)");
        assert!(
            block.contains("display:") && block.contains("none"),
            "a Filter-mode off-path card must be removed with display:none, got {block:?}"
        );

        // The dim rule must NOT use display:none — Highlight mode keeps the card in
        // layout (it only recedes). This guards the two modes staying distinct.
        let dimmed =
            css_rule_block(css, ".klinx-node--dimmed").expect("dimmed node CSS rule still present");
        assert!(
            !dimmed.contains("display:"),
            "Highlight-mode dimming must keep the card in layout, never display:none",
        );
    }

    /// #123: the central reveal-cap invariant — Highlight is NEVER capped (full
    /// closure at any depth, preserving the pre-#123 selected-field behavior), while
    /// Filter caps at the active hop cap (bounded + EXPAND+ recoverable). This is the
    /// fix for the Highlight-clips-deep-pipelines regression; a flip of the match arm
    /// would silently re-introduce it.
    #[test]
    fn highlight_reveal_is_never_capped_filter_is() {
        assert_eq!(
            super::reveal_closure_cap(crate::state::LineageRevealMode::Highlight, 16),
            None,
            "Highlight mode must use the full uncapped closure at any depth"
        );
        assert_eq!(
            super::reveal_closure_cap(crate::state::LineageRevealMode::Filter, 16),
            Some(16),
            "Filter mode bounds the closure at the active hop cap"
        );
    }

    /// #123 guard: the [`DEFAULT_HOP_CAP`] must NOT clip the lineage of any field in
    /// any bundled example pipeline. Highlight mode is now unconditionally UNCAPPED
    /// (`closure_cap = None`), so it preserves the pre-#123 reveal for every pipeline
    /// regardless of depth; the cap applies only in Filter mode. This guard keeps the
    /// FILTER-mode default honest on the bundled examples: a default-cap Filter reveal
    /// equals the full closure (so no example is silently hidden, and the EXPAND+
    /// affordance does not spuriously appear on a shallow graph). If a future example
    /// introduces a deeper closure, this fails loudly so the constant (or a
    /// degree-based strategy) is revisited rather than silently clipping a Filter view.
    #[test]
    fn default_hop_cap_does_not_clip_example_pipelines() {
        use crate::pipeline_view::{field_lineage_full, field_lineage_full_capped};
        let examples: &[&str] = &[
            include_str!("../../../../../examples/pipelines/audit_join.yaml"),
            include_str!("../../../../../examples/pipelines/customer_etl.yaml"),
            include_str!("../../../../../examples/pipelines/invoice_daily_rollup.yaml"),
            include_str!("../../../../../examples/pipelines/invoices.yaml"),
            include_str!("../../../../../examples/pipelines/long_field_support.yaml"),
            include_str!("../../../../../examples/pipelines/multi_source_session.yaml"),
            include_str!("../../../../../examples/pipelines/order_fulfillment.yaml"),
            include_str!("../../../../../examples/pipelines/tumbling_clicks.yaml"),
            include_str!("../../../../../examples/pipelines/layout_benchmark_order_lifecycle.yaml"),
            include_str!("../../../../../examples/pipelines/layout_benchmark_source_reuse.yaml"),
        ];
        let mut checked_fields = 0usize;
        for yaml in examples {
            let config = parse_config(yaml).expect("bundled example pipeline parses");
            let view = derive_pipeline_view(&config);
            for (node, stage) in view.stages.iter().enumerate() {
                for field in &stage.fields {
                    let full = field_lineage_full(&view.field_edges, node, &field.name);
                    let capped = field_lineage_full_capped(
                        &view.field_edges,
                        node,
                        &field.name,
                        Some(super::DEFAULT_HOP_CAP),
                    );
                    assert_eq!(
                        capped,
                        full,
                        "default hop cap {} clips {}.{}'s lineage — Highlight mode would regress",
                        super::DEFAULT_HOP_CAP,
                        stage.id,
                        field.name,
                    );
                    checked_fields += 1;
                }
            }
        }
        assert!(
            checked_fields > 0,
            "the guard must actually exercise some example fields",
        );
    }

    /// #147: INDIRECT influence edges read as ghosted, dashed cables distinct from
    /// the solid DIRECT value cables — the `--indirect` modifier (driven by
    /// `FieldEdgeKind::nature()`) ghosts (lower opacity) and the inner path is
    /// dashed.
    #[test]
    fn indirect_field_edge_css_is_ghosted_and_dashed() {
        let css = include_str!("../../../assets/klinx.css");

        let modifier = css_rule_block(css, ".klinx-field-edge--indirect {")
            .expect("indirect field-edge CSS rule");
        assert!(
            modifier.contains("opacity:"),
            "INDIRECT edges should be ghosted via reduced opacity, got {modifier:?}"
        );

        let dash = css_rule_block(css, ".klinx-field-edge--indirect path")
            .expect("indirect field-edge path dash rule");
        assert!(
            dash.contains("stroke-dasharray:"),
            "INDIRECT edge paths should be dashed, got {dash:?}"
        );

        // Each INDIRECT subtype keeps its own hue class so a reader can tell a
        // Filter from a JoinKey on inspection.
        for selector in [
            ".klinx-field-edge--filter {",
            ".klinx-field-edge--groupby {",
            ".klinx-field-edge--joinkey {",
            ".klinx-field-edge--conditional {",
        ] {
            let block = css_rule_block(css, selector)
                .unwrap_or_else(|| panic!("missing INDIRECT subtype CSS rule {selector}"));
            assert!(
                block.contains("stroke:"),
                "{selector} should set a stroke hue, got {block:?}"
            );
        }
    }

    /// #148: the lineage-precision node corner is HIDDEN by default and revealed
    /// ONLY on selection/hover — never an always-on overlay (avoid badge fatigue).
    /// Assert the base opacity is 0 and that BOTH the `:hover` and the `--selected`
    /// rules — checked INDEPENDENTLY (they are separate, not comma-joined rules) —
    /// RAISE opacity strictly above the base, so the reveal gating lives in the CSS,
    /// not the component.
    #[test]
    fn precision_corner_css_reveals_only_on_selection_or_hover() {
        let css = include_str!("../../../assets/klinx.css");

        let base = css_rule_block(css, ".klinx-node-precision-corner {")
            .expect("precision-corner base CSS rule");
        let base_opacity = css_opacity(base).expect("base rule sets opacity");
        assert_eq!(
            base_opacity, 0.0,
            "the corner must be hidden (opacity:0) by default, got {base_opacity}"
        );

        // Hover and selection are SEPARATE rules — assert each independently raises
        // opacity above the hidden base.
        for selector in [
            ".klinx-node:hover .klinx-node-precision-corner {",
            ".klinx-node--selected .klinx-node-precision-corner {",
        ] {
            let block = css_rule_block(css, selector)
                .unwrap_or_else(|| panic!("missing reveal rule {selector}"));
            let opacity = css_opacity(block)
                .unwrap_or_else(|| panic!("{selector} must set opacity, got {block:?}"));
            assert!(
                opacity > base_opacity,
                "{selector} must RAISE opacity above the base {base_opacity}, got {opacity}"
            );
        }

        // The hatch hue is driven by `data-precision`; each tier has its own rule.
        for selector in [
            ".klinx-node-precision-corner[data-precision=\"exact\"]",
            ".klinx-node-precision-corner[data-precision=\"approximate\"]",
            ".klinx-node-precision-corner[data-precision=\"unknown\"]",
        ] {
            let block = css_rule_block(css, selector)
                .unwrap_or_else(|| panic!("missing precision-corner tier rule {selector}"));
            assert!(
                block.contains("background-image:"),
                "{selector} should set a hatch background-image, got {block:?}"
            );
        }
    }

    /// Parse the `opacity: <f32>;` value out of a CSS rule block, if present.
    fn css_opacity(block: &str) -> Option<f32> {
        let after = block.split("opacity:").nth(1)?;
        let value = after.split(';').next()?.trim();
        value.parse::<f32>().ok()
    }

    fn css_rule_block<'a>(css: &'a str, selector: &str) -> Option<&'a str> {
        let start = css.find(selector)?;
        let open = css[start..].find('{')? + start;
        let close = css[open..].find('}')? + open;
        Some(&css[open + 1..close])
    }

    #[test]
    fn order_fulfillment_products_to_lookup_connector_avoids_transform_card() {
        let yaml = include_str!("../../../../../examples/pipelines/order_fulfillment.yaml");
        let config = parse_config(yaml).expect("order_fulfillment.yaml parses");
        let view = apply_canvas_layout(
            derive_pipeline_view(&config),
            CanvasLayoutEngine::PortAwareSugiyama,
        )
        .view;

        let projected = view
            .stages
            .iter()
            .map(|stage| {
                project_for_test(
                    stage,
                    &FieldDisplayState::default(),
                    ResolvedNodeDisplayMode::Schema,
                    &HashSet::new(),
                )
            })
            .collect::<Vec<_>>();
        let field_displays = projected
            .iter()
            .map(|projected| projected.display.clone())
            .collect::<Vec<_>>();
        let stages = projected
            .into_iter()
            .map(|projected| projected.stage)
            .collect::<Vec<_>>();
        let obstacles = stages
            .iter()
            .zip(field_displays.iter())
            .map(|(stage, display)| ConnectorObstacle {
                x: stage.canvas_x,
                y: stage.canvas_y,
                width: NODE_WIDTH,
                height: rendered_card_height(stage, display),
            })
            .collect::<Vec<_>>();

        let connection = view
            .connections
            .iter()
            .find(|connection| {
                stages[connection.from].id == "products"
                    && stages[connection.to].id == "product_lookup"
            })
            .expect("products connects directly to product_lookup");
        let from = &stages[connection.from];
        let to = &stages[connection.to];
        let (sx, sy) = match connection.from_branch {
            Some(branch_index) => from.branch_anchor_out(branch_index),
            None => from.port_out(),
        };
        let (tx, ty) = to.port_in();
        let paths =
            obstacle_aware_channel_paths(&[ConnectorEndpoints { sx, sy, tx, ty }], &obstacles);

        let transform_index = stages
            .iter()
            .position(|stage| stage.id == "normalize_fields")
            .expect("normalize_fields stage exists");
        let transform = &obstacles[transform_index];

        assert!(
            !path_intersects_obstacle(&paths[0].points, transform),
            "products -> product_lookup connector should avoid normalize_fields: {:?}",
            paths[0]
        );
    }

    fn path_intersects_obstacle(points: &[CanvasPoint], obstacle: &ConnectorObstacle) -> bool {
        points
            .windows(2)
            .any(|segment| segment_intersects_obstacle(segment[0], segment[1], obstacle))
    }

    fn segment_intersects_obstacle(
        from: CanvasPoint,
        to: CanvasPoint,
        obstacle: &ConnectorObstacle,
    ) -> bool {
        let seg_min_x = from.x.min(to.x);
        let seg_max_x = from.x.max(to.x);
        let seg_min_y = from.y.min(to.y);
        let seg_max_y = from.y.max(to.y);
        let left = obstacle.x;
        let right = obstacle.x + obstacle.width;
        let top = obstacle.y;
        let bottom = obstacle.y + obstacle.height;

        if (from.y - to.y).abs() < 0.5 {
            return from.y > top
                && from.y < bottom
                && open_ranges_overlap(seg_min_x, seg_max_x, left, right);
        }
        if (from.x - to.x).abs() < 0.5 {
            return from.x > left
                && from.x < right
                && open_ranges_overlap(seg_min_y, seg_max_y, top, bottom);
        }

        open_ranges_overlap(seg_min_x, seg_max_x, left, right)
            && open_ranges_overlap(seg_min_y, seg_max_y, top, bottom)
    }

    fn open_ranges_overlap(a_start: f32, a_end: f32, b_start: f32, b_end: f32) -> bool {
        a_start.max(b_start) < a_end.min(b_end)
    }

    trait StageViewTestExt {
        fn tap_fields(self, count: usize) -> Self;
    }

    impl StageViewTestExt for StageView {
        fn tap_fields(mut self, count: usize) -> Self {
            self.fields.truncate(count);
            self
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // #152 — whole-path lineage ribbon + focus mode
    //
    // These tests pin the ribbon overlay's contract against the SELECTED field's
    // transitive closure: it lights the full up+down path, dims off-path cards
    // (focus), draws the two ribbons independently, and — the headline acceptance
    // criterion — does so as a RENDER OVERLAY that never reflows the layout.
    // ─────────────────────────────────────────────────────────────────────────

    use super::{FieldEdgeAnchors, resolve_edge_anchors, ribbon_edge_visible};
    use crate::pipeline_view::{
        EdgeNature, PipelineView, Precision, field_lineage_full, field_lineage_full_capped,
    };

    /// A MID-graph field whose transitive closure spans BOTH edge natures and
    /// leaves at least one card off-path — the one fixture that exercises every
    /// ribbon contract. In the bundled `order_fulfillment.yaml`,
    /// `normalize_fields.product_code` has upstream producers AND downstream
    /// consumers (a whole-path ribbon, both directions), is carried as a value
    /// (DIRECT) while also being the Combine `JoinKey` (INDIRECT) — both ribbons —
    /// and its closure touches 6 of the graph's 7 cards, so a card stays off-path
    /// for the focus-dim test.
    const RIBBON_FIXTURE: &str =
        include_str!("../../../../../examples/pipelines/order_fulfillment.yaml");
    const RIBBON_SELECTED_STAGE: &str = "normalize_fields";
    const RIBBON_SELECTED_FIELD: &str = "product_code";

    /// Lay out the ribbon fixture exactly as `CanvasPanel` does (derive →
    /// port-aware Sugiyama), returning the laid-out view. The layout owns
    /// `canvas_x`/`canvas_y` and every `*_paths`; the ribbon reads them and must
    /// never write them.
    fn laid_out_ribbon_view() -> PipelineView {
        let config = parse_config(RIBBON_FIXTURE).expect("audit_join.yaml parses");
        apply_canvas_layout(
            derive_pipeline_view(&config),
            CanvasLayoutEngine::PortAwareSugiyama,
        )
        .view
    }

    fn node_index(view: &PipelineView, stage_id: &str) -> usize {
        view.stages
            .iter()
            .position(|stage| stage.id == stage_id)
            .unwrap_or_else(|| panic!("stage {stage_id} exists in the ribbon fixture"))
    }

    /// The participating field-edge index set for selecting `(stage, field)` —
    /// the full transitive up+down closure, mirroring the Highlight-mode reveal
    /// (uncapped) the panel computes for a pinned/selected column.
    fn selected_closure(view: &PipelineView, stage_id: &str, field: &str) -> HashSet<usize> {
        let node = node_index(view, stage_id);
        field_lineage_full(&view.field_edges, node, field)
    }

    /// Resolve a closure into the drawable overlay cables exactly as the render
    /// path does (`resolve_edge_anchors` against the laid-out stages, reading the
    /// precomputed `field_edge_paths`). Returns `(edge_index, anchors)` pairs for
    /// every edge whose endpoints resolve — the eligible ribbon set.
    fn resolve_overlay(
        view: &PipelineView,
        closure: &HashSet<usize>,
    ) -> Vec<(usize, FieldEdgeAnchors)> {
        let mut indices: Vec<usize> = closure.iter().copied().collect();
        indices.sort_unstable();
        indices
            .into_iter()
            .filter_map(|ei| {
                let edge = &view.field_edges[ei];
                resolve_edge_anchors(&view.stages, edge, view.field_edge_paths.get(ei))
                    .map(|anchors| (ei, anchors))
            })
            .collect()
    }

    /// Acceptance: selecting a field lights its FULL transitive up+down path. The
    /// eligible ribbon edge set equals the resolvable portion of the closure (no
    /// edge silently dropped), and that closure genuinely spans both upstream and
    /// downstream of the selected node.
    #[test]
    fn ribbon_covers_full_transitive_closure() {
        let view = laid_out_ribbon_view();
        let selected = node_index(&view, RIBBON_SELECTED_STAGE);
        let closure = selected_closure(&view, RIBBON_SELECTED_STAGE, RIBBON_SELECTED_FIELD);

        assert!(
            !closure.is_empty(),
            "selecting {RIBBON_SELECTED_STAGE}.{RIBBON_SELECTED_FIELD} must reveal a path"
        );

        // Every closure edge whose endpoints exist in the laid-out stages must be
        // an eligible ribbon cable — the overlay covers the whole closure, not a
        // subset. Both endpoint fields exist for every closure edge in this
        // fixture, so the eligible set equals the closure exactly.
        let overlay = resolve_overlay(&view, &closure);
        let drawn: HashSet<usize> = overlay.iter().map(|(ei, _)| *ei).collect();
        assert_eq!(
            drawn, closure,
            "the ribbon must cover the entire transitive closure"
        );

        // The path reaches BOTH upstream of and downstream of the selected node —
        // it is a whole-path ribbon, not a one-directional trace.
        let mut has_upstream = false;
        let mut has_downstream = false;
        for ei in &closure {
            let edge = &view.field_edges[*ei];
            if edge.to_node == selected {
                has_upstream = true;
            }
            if edge.from_node == selected {
                has_downstream = true;
            }
        }
        assert!(
            has_upstream && has_downstream,
            "a selected mid-graph field's ribbon must light both upstream and downstream"
        );
    }

    /// Acceptance (focus mode): for the SELECTED field, an off-path card is dimmed
    /// while in-path cards are not. The dim set is the complement of the closure's
    /// participating nodes — the exact `dimmed:` gate the panel applies in
    /// Highlight (focus) mode, independent of any hover.
    #[test]
    fn focus_mode_dims_off_path_cards_only() {
        let view = laid_out_ribbon_view();
        let closure = selected_closure(&view, RIBBON_SELECTED_STAGE, RIBBON_SELECTED_FIELD);
        let overlay = resolve_overlay(&view, &closure);

        let mut participating: HashSet<usize> = HashSet::new();
        for (ei, _) in &overlay {
            let edge = &view.field_edges[*ei];
            participating.insert(edge.from_node);
            participating.insert(edge.to_node);
        }
        assert!(
            !participating.is_empty(),
            "a selected field's path must light at least its own card"
        );

        // The selected card is in-path (not dimmed); at least one card is off-path
        // (dimmed). Together these prove the focus dim is a real partition, not a
        // dim-everything or dim-nothing.
        let selected = node_index(&view, RIBBON_SELECTED_STAGE);
        assert!(
            participating.contains(&selected),
            "the selected card must be in-path (never dimmed)"
        );
        let off_path: Vec<usize> = (0..view.stages.len())
            .filter(|node| !participating.contains(node))
            .collect();
        assert!(
            !off_path.is_empty(),
            "the fixture must have at least one off-path card to dim"
        );
        // The dim set (off-path) is exactly the complement of the bright
        // (participating) set the panel's `dimmed: !participating.contains(node)`
        // gate applies in focus mode: disjoint, covering, and — proven above — both
        // non-empty. A regression that dimmed an in-path card or lit an off-path one
        // would break this partition.
        assert!(
            off_path.iter().all(|n| !participating.contains(n)),
            "no card may be both on-path (bright) and off-path (dimmed)"
        );
        assert_eq!(
            off_path.len() + participating.len(),
            view.stages.len(),
            "every card is exactly one of on-path (bright) or off-path (dimmed)"
        );
        // Every bright card is genuinely incident to a revealed cable (not lit by
        // accident), and every dimmed card is genuinely absent from the overlay — so
        // the partition tracks real path incidence, not a blanket dim/bright.
        for node in 0..view.stages.len() {
            let incident_to_overlay = overlay.iter().any(|(ei, _)| {
                let edge = &view.field_edges[*ei];
                edge.from_node == node || edge.to_node == node
            });
            assert_eq!(
                participating.contains(&node),
                incident_to_overlay,
                "card {node}: bright iff it is incident to a revealed lineage cable"
            );
        }
    }

    /// THE headline acceptance criterion: highlight is a RENDER OVERLAY — selecting
    /// a field must NOT reflow the layout. We snapshot every node's `canvas_x`/
    /// `canvas_y` and every `*_paths` output, then run the exact selection-reveal
    /// computation (closure walk + `resolve_edge_anchors`, which reads the layout
    /// paths) and assert the layout outputs are byte-for-byte unchanged. The
    /// overlay produced real cables, so the no-op is genuine, not vacuous.
    #[test]
    fn selection_is_render_only_and_never_reflows_layout() {
        let view = laid_out_ribbon_view();

        // Snapshot the layout-owned outputs BEFORE the selection reveal.
        let positions_before: Vec<(f32, f32)> = view
            .stages
            .iter()
            .map(|stage| (stage.canvas_x, stage.canvas_y))
            .collect();
        let field_paths_before = view.field_edge_paths.clone();
        let connection_paths_before = view.connection_paths.clone();
        let role_paths_before = view.role_edge_paths.clone();

        // Simulate the selection reveal: compute the closure and resolve every
        // participating edge to anchors — the full render-path read of the layout.
        let closure = selected_closure(&view, RIBBON_SELECTED_STAGE, RIBBON_SELECTED_FIELD);
        let overlay = resolve_overlay(&view, &closure);
        assert!(
            !overlay.is_empty(),
            "the selection must resolve real cables, else the no-reflow check is vacuous"
        );

        // The layout outputs the ribbon READS must be identical AFTER — the reveal
        // wrote none of them.
        let positions_after: Vec<(f32, f32)> = view
            .stages
            .iter()
            .map(|stage| (stage.canvas_x, stage.canvas_y))
            .collect();
        assert_eq!(
            positions_before, positions_after,
            "selection must not move any node — the ribbon is a render overlay"
        );
        assert_eq!(
            field_paths_before, view.field_edge_paths,
            "selection must not rewrite field_edge_paths"
        );
        assert_eq!(
            connection_paths_before, view.connection_paths,
            "selection must not rewrite connection_paths"
        );
        assert_eq!(
            role_paths_before, view.role_edge_paths,
            "selection must not rewrite role_edge_paths"
        );
    }

    /// Acceptance: the value (DIRECT) and influence (INDIRECT) ribbons toggle
    /// INDEPENDENTLY. Against a closure that spans both natures: value-off drops
    /// every DIRECT cable and keeps every INDIRECT one; influence-off drops every
    /// INDIRECT and keeps every DIRECT; toggling one never affects the other.
    #[test]
    fn value_and_influence_ribbons_toggle_independently() {
        let view = laid_out_ribbon_view();
        let closure = selected_closure(&view, RIBBON_SELECTED_STAGE, RIBBON_SELECTED_FIELD);
        let overlay = resolve_overlay(&view, &closure);

        let direct: HashSet<usize> = overlay
            .iter()
            .filter(|(_, a)| a.kind.nature() == EdgeNature::Direct)
            .map(|(ei, _)| *ei)
            .collect();
        let indirect: HashSet<usize> = overlay
            .iter()
            .filter(|(_, a)| a.kind.nature() == EdgeNature::Indirect)
            .map(|(ei, _)| *ei)
            .collect();
        assert!(
            !direct.is_empty() && !indirect.is_empty(),
            "the fixture closure must contain both ribbon natures to test independence"
        );

        let drawn = |value_on: bool, influence_on: bool| -> HashSet<usize> {
            overlay
                .iter()
                .filter(|(_, a)| ribbon_edge_visible(a.kind.nature(), value_on, influence_on))
                .map(|(ei, _)| *ei)
                .collect::<HashSet<usize>>()
        };

        // Both on → the whole closure.
        assert_eq!(
            drawn(true, true),
            &direct | &indirect,
            "both ribbons on draws all"
        );
        // Value off → only INDIRECT survives; every DIRECT cable is gone.
        assert_eq!(
            drawn(false, true),
            indirect,
            "value ribbon off must draw only INDIRECT cables"
        );
        // Influence off → only DIRECT survives; every INDIRECT cable is gone.
        assert_eq!(
            drawn(true, false),
            direct,
            "influence ribbon off must draw only DIRECT cables"
        );
        // Both off → nothing.
        assert!(
            drawn(false, false).is_empty(),
            "both ribbons off draws nothing"
        );

        // Independence: flipping the influence toggle never changes the DIRECT set,
        // and flipping the value toggle never changes the INDIRECT set.
        assert_eq!(
            &drawn(true, true) & &direct,
            &drawn(true, false) & &direct,
            "toggling influence must not change which DIRECT cables draw"
        );
        assert_eq!(
            &drawn(true, true) & &indirect,
            &drawn(false, true) & &indirect,
            "toggling value must not change which INDIRECT cables draw"
        );
    }

    /// `ribbon_edge_visible` is the single gating predicate the overlay filters on.
    /// Pin its truth table so the independence guarantee can't silently invert.
    #[test]
    fn ribbon_edge_visible_truth_table() {
        for (nature, value_on, influence_on, expected) in [
            (EdgeNature::Direct, true, true, true),
            (EdgeNature::Direct, true, false, true),
            (EdgeNature::Direct, false, true, false),
            (EdgeNature::Direct, false, false, false),
            (EdgeNature::Indirect, true, true, true),
            (EdgeNature::Indirect, true, false, false),
            (EdgeNature::Indirect, false, true, true),
            (EdgeNature::Indirect, false, false, false),
        ] {
            assert_eq!(
                ribbon_edge_visible(nature, value_on, influence_on),
                expected,
                "{nature:?} value={value_on} influence={influence_on}"
            );
        }
    }

    /// Performance bound: the overlay's work is bounded by the SELECTED closure,
    /// never the whole field-edge set. On the large fixture, a single field's
    /// resolved ribbon must be a strict subset of all edges (the budget guarantee:
    /// a selection draws its path, not the entire graph). No wall-clock timing.
    #[test]
    fn overlay_is_bounded_by_selected_closure_on_large_fixture() {
        let config = parse_config(include_str!("../../../tests/fixtures/large_pipeline.yaml"))
            .expect("large_pipeline.yaml parses");
        let view = apply_canvas_layout(
            derive_pipeline_view(&config),
            CanvasLayoutEngine::PortAwareSugiyama,
        )
        .view;

        // Pick the first node/field that has any lineage, then bound its overlay.
        let mut checked = false;
        'outer: for (node, stage) in view.stages.iter().enumerate() {
            for field in &stage.fields {
                let closure = field_lineage_full(&view.field_edges, node, &field.name);
                if closure.is_empty() {
                    continue;
                }
                let overlay = resolve_overlay(&view, &closure);
                assert!(
                    overlay.len() <= closure.len(),
                    "overlay never exceeds the closure it resolves"
                );
                // On a large multi-source graph, no single field's closure can be
                // the ENTIRE edge set — the overlay is genuinely scoped.
                assert!(
                    closure.len() < view.field_edges.len(),
                    "a single field's ribbon must be a strict subset of all {} edges, got {}",
                    view.field_edges.len(),
                    closure.len()
                );
                checked = true;
                break 'outer;
            }
        }
        assert!(
            checked,
            "the large fixture must expose at least one field with lineage"
        );
    }

    /// The Highlight-mode (focus) reveal is uncapped, so a deep selected path is
    /// lit in full — focus never silently clips the closure it dims around. (Filter
    /// mode's cap is #123's; focus reuses Highlight, which `reveal_closure_cap`
    /// leaves uncapped.) Guards that focus mode and the full-path ribbon agree.
    #[test]
    fn focus_mode_reveal_is_uncapped() {
        let view = laid_out_ribbon_view();
        let node = node_index(&view, RIBBON_SELECTED_STAGE);
        let full = field_lineage_full(&view.field_edges, node, RIBBON_SELECTED_FIELD);
        let capped = field_lineage_full_capped(
            &view.field_edges,
            node,
            RIBBON_SELECTED_FIELD,
            super::reveal_closure_cap(
                crate::state::LineageRevealMode::Highlight,
                super::DEFAULT_HOP_CAP,
            ),
        );
        assert_eq!(
            capped, full,
            "focus (Highlight) mode must light the full transitive path uncapped"
        );
    }

    /// #152 CSS: the two ribbons read as visually distinct cables, and an
    /// Approximate edge is hatched. Asserted via `css_rule_block` so the rendered
    /// classes have backing rules.
    #[test]
    fn ribbon_and_precision_css_rules_exist() {
        let css = include_str!("../../../assets/klinx.css");

        // The DIRECT value ribbon is solid + continuous (no dash on its paths).
        let value = css_rule_block(css, ".klinx-field-edge--value {")
            .expect("value ribbon CSS rule (#152)");
        assert!(
            value.contains("opacity:"),
            "the value ribbon should be fully present (opacity set), got {value:?}"
        );
        let value_path = css_rule_block(css, ".klinx-field-edge--value path")
            .expect("value ribbon path rule (#152)");
        assert!(
            value_path.contains("stroke-dasharray:") && value_path.contains("none"),
            "the value ribbon must be CONTINUOUS (no dash), got {value_path:?}"
        );

        // The INDIRECT influence halo stays ghosted + dashed (the #147 contract the
        // dual-ribbon split builds on).
        let indirect =
            css_rule_block(css, ".klinx-field-edge--indirect {").expect("indirect halo CSS rule");
        assert!(
            indirect.contains("opacity:"),
            "the influence halo should be ghosted (reduced opacity), got {indirect:?}"
        );

        // Precision-aware (#148): the Approximate modifier hatches the cable.
        let approx = css_rule_block(css, ".klinx-field-edge--approximate path")
            .expect("approximate ribbon path rule (#152)");
        assert!(
            approx.contains("stroke-dasharray:"),
            "an Approximate edge must be hatched (dashed), got {approx:?}"
        );
    }

    /// The role-edge resolver tags every cable `Approximate` — a role port is an
    /// influence input, so the precision-aware ribbon styling applies to role
    /// cables too. Exercised against a group-by fixture that produces role edges.
    #[test]
    fn role_edge_cables_are_approximate() {
        let config = parse_config(include_str!(
            "../../../../../examples/pipelines/invoice_daily_rollup.yaml"
        ))
        .expect("invoice_daily_rollup.yaml parses");
        let view = apply_canvas_layout(
            derive_pipeline_view(&config),
            CanvasLayoutEngine::PortAwareSugiyama,
        )
        .view;
        assert!(
            !view.role_edges.is_empty(),
            "the group-by fixture must produce role edges to exercise the resolver"
        );

        let mut resolved_any = false;
        for (ei, edge) in view.role_edges.iter().enumerate() {
            if let Some(anchors) =
                super::resolve_role_edge_anchors(&view.stages, edge, view.role_edge_paths.get(ei))
            {
                assert_eq!(
                    anchors.precision,
                    Precision::Approximate,
                    "a role-port (influence) cable is graded Approximate"
                );
                resolved_any = true;
            }
        }
        assert!(
            resolved_any,
            "the group-by fixture must produce a resolved role cable"
        );
    }
}
