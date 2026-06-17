use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use dioxus::html::geometry::WheelDelta;
use dioxus::prelude::*;

use crate::pipeline_view::layout_model::{CanvasLayoutEngine, apply_canvas_layout};
use crate::pipeline_view::{
    CanvasConnectorPath, FIELD_ROW_HEIGHT, FieldEdge, FieldEdgeKind, FieldRow, NODE_WIDTH,
    RoleEdge, StagePortSide, StageView, derive_body_view, derive_partial_pipeline_view,
    derive_pipeline_view, derive_resolved_pipeline_view, field_lineage_full, fit_transform,
    group_endpoints_by_node, layout_bounds, lineage_closure,
};
use crate::state::{ChannelViewMode, use_app_state};

use super::connector::{
    Connector, ConnectorEndpoints, ConnectorObstacle, FieldConnector, obstacle_aware_channel_paths,
};
use super::node::{CanvasNode, FieldDisplayInfo};
use super::{CanvasHover, HoverTarget, LineageTarget, PinnedField};

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
#[derive(Clone, Debug, Default, PartialEq)]
struct FieldDisplayState {
    visible_limit: usize,
    query: String,
}

#[derive(Clone, Debug, PartialEq)]
struct ProjectedStage {
    stage: StageView,
    display: FieldDisplayInfo,
}

fn project_stage_fields(
    stage: &StageView,
    state: &FieldDisplayState,
    temporary_fields: &HashSet<String>,
) -> ProjectedStage {
    let query = state.query.trim();
    let visible_limit = state.visible_limit.max(FIELD_ROW_CAP);
    let mut fields: Vec<FieldRow> = stage
        .fields
        .iter()
        .filter(|field| field_matches_query(field, query))
        .cloned()
        .collect();
    let matching_count = fields.len();
    let capped = matching_count > visible_limit;
    if capped {
        fields.truncate(visible_limit);
    }

    let hidden_count = matching_count.saturating_sub(fields.len());
    let next_count = hidden_count.min(FIELD_ROW_CAP);
    let mut visible_names: HashSet<String> =
        fields.iter().map(|field| field.name.clone()).collect();
    let mut appended_temporary_fields = Vec::new();
    for field in &stage.fields {
        if temporary_fields.contains(&field.name) && !visible_names.contains(&field.name) {
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
            searchable: stage.fields.len() > FIELD_ROW_CAP || !query.is_empty(),
            can_reduce: visible_limit > FIELD_ROW_CAP && matching_count > FIELD_ROW_CAP,
        },
    }
}

fn field_matches_query(field: &FieldRow, query: &str) -> bool {
    let query = query.trim();
    query.is_empty()
        || text_matches_query(&field.name, query)
        || field
            .ty
            .as_ref()
            .is_some_and(|ty| text_matches_query(ty, query))
        || text_matches_query(field_kind_label(field), query)
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

fn field_matches_by_node(stages: &[StageView], query: &str) -> HashMap<usize, HashSet<String>> {
    let query = query.trim();
    if query.is_empty() {
        return HashMap::new();
    }

    stages
        .iter()
        .enumerate()
        .filter_map(|(index, stage)| {
            let matches: HashSet<String> = stage
                .fields
                .iter()
                .filter(|field| field_matches_query(field, query))
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
    let mut field_display_states = use_signal(HashMap::<String, FieldDisplayState>::new);
    let mut global_field_query = use_signal(String::new);

    let view_mode = *state.channel_view_mode.read();
    let drill_stack = state.composition_drill_stack.read();

    // A composition document (`*.comp.yaml`) renders its pre-derived body DAG —
    // it has no pipeline config, channel overlay, or drill stack of its own.
    let pipeline_view = if let Some(comp_view) = state.composition_view.read().clone() {
        comp_view
    } else if let Some(frame) = drill_stack.last() {
        let compiled_guard = state.compiled_plan.read();
        match compiled_guard
            .as_ref()
            .and_then(|plan| plan.body_of(frame.body_id))
        {
            Some(body) => derive_body_view(body),
            None => crate::pipeline_view::PipelineView::default(),
        }
    } else {
        // Top-level: dispatch on view mode
        match view_mode {
            ChannelViewMode::Resolved => {
                let compiled_guard = state.compiled_plan.read();
                match compiled_guard.as_ref() {
                    Some(plan) => derive_resolved_pipeline_view(plan),
                    None => match &*(state.pipeline).read() {
                        Some(config) => derive_pipeline_view(config),
                        None => crate::pipeline_view::PipelineView::default(),
                    },
                }
            }
            ChannelViewMode::Raw => match &*(state.pipeline).read() {
                Some(config) => derive_pipeline_view(config),
                None => match &*(state.partial_pipeline).read() {
                    Some(partial) => derive_partial_pipeline_view(partial),
                    None => crate::pipeline_view::PipelineView::default(),
                },
            },
        }
    };
    let pipeline_view =
        apply_canvas_layout(pipeline_view, CanvasLayoutEngine::PortAwareSugiyama).view;
    drop(drill_stack);
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
    let pinned_selection = pinned_field.0.read().clone();
    let hover_target = hovered_field.0.read().clone();
    let (closure, role_closure): (HashSet<usize>, HashSet<usize>) = match &pinned_selection {
        Some(LineageTarget::Field(node, field)) => {
            let field_closure = field_lineage_full(&field_edges, *node, field);
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
                field_closure.extend(field_lineage_full(
                    &field_edges,
                    edge.from_node,
                    &edge.from_field,
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
    let global_query = global_field_query.read().clone();
    let global_matches_by_node = field_matches_by_node(&raw_stages, &global_query);
    let mut temporary_fields_by_node = lineage_fields_by_node.clone();
    merge_field_name_sets(
        &mut temporary_fields_by_node,
        &lineage_role_source_fields_by_node,
    );
    merge_field_name_sets(&mut temporary_fields_by_node, &global_matches_by_node);

    let projected: Vec<ProjectedStage> = {
        let display_states = field_display_states.read();
        raw_stages
            .iter()
            .enumerate()
            .map(|(index, stage)| {
                let state = display_states.get(&stage.id).cloned().unwrap_or_default();
                let temporary_fields = temporary_fields_by_node.remove(&index).unwrap_or_default();
                project_stage_fields(stage, &state, &temporary_fields)
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
    let connections: Vec<(
        StageView,
        StageView,
        Option<usize>,
        Option<CanvasConnectorPath>,
    )> = connections_model
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
            (
                stages[c.from].clone(),
                stages[c.to].clone(),
                c.from_branch,
                dynamic_path.or(layout_path),
            )
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
        // Bind the fingerprint so the reads above are retained as subscriptions.
        let _ = (
            &comp_ids,
            &drill_key,
            mode,
            pipeline_present,
            plan_present,
            partial_present,
        );

        let mut hovered = hovered_field;
        hovered.force_clear();
        let mut pinned = pinned_field;
        pinned.0.set(None);
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
    let node_cards: Vec<(usize, StageView, FieldDisplayInfo, String, String)> = stages
        .iter()
        .cloned()
        .zip(field_displays.iter().cloned())
        .enumerate()
        .map(|(index, (stage, display))| {
            let query_stage_id = stage.id.clone();
            let expand_stage_id = stage.id.clone();
            (index, stage, display, query_stage_id, expand_stage_id)
        })
        .collect();

    // Channel view mode toggle state
    let tab_mgr = use_context::<crate::state::TabManagerState>();
    let has_channel = tab_mgr
        .channel_state
        .read()
        .as_ref()
        .is_some_and(|cs| cs.active_channel.is_some());
    let is_resolved = view_mode == ChannelViewMode::Resolved;

    rsx! {
        div {
            class: "klinx-canvas-column",

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
                        for (from, to, from_branch, path) in connections {
                            Connector {
                                key: "{from.id}-{to.id}-{from_branch:?}",
                                from,
                                to,
                                from_branch,
                                path,
                            }
                        }
                    }
                }

                // Node cards
                for (index, stage, display, query_stage_id, expand_stage_id) in node_cards {
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
                            let mut states = field_display_states.write();
                            let entry = states.entry(query_stage_id.clone()).or_default();
                            entry.query = query;
                            entry.visible_limit = FIELD_ROW_CAP;
                        },
                        on_field_toggle: move |_| {
                            let mut states = field_display_states.write();
                            let entry = states.entry(expand_stage_id.clone()).or_default();
                            if display.hidden_count > 0 {
                                entry.visible_limit = entry
                                    .visible_limit
                                    .max(FIELD_ROW_CAP)
                                    .saturating_add(FIELD_ROW_CAP);
                            } else if display.can_reduce {
                                entry.visible_limit = FIELD_ROW_CAP;
                            }
                        },
                        // Dim cards outside the revealed field's lineage closure.
                        dimmed: hover_active && !participating_nodes.contains(&index),
                        highlighted_fields: highlighted_by_node.remove(&index).unwrap_or_default(),
                        highlighted_role_ports: highlighted_role_ports_by_node.remove(&index).unwrap_or_default(),
                    }
                }

                // Active field-level cables — ONLY the hovered/pinned field's
                // lineage closure, never the whole field-edge set. The overlay
                // is above default cables but below cards, so field rows mask
                // any stroke through their interiors and keep pointer control.
                if hover_active {
                    svg {
                        class: "klinx-canvas-svg klinx-canvas-svg--active",
                        width: "{svg_w}",
                        height: "{svg_h}",
                        for (ei, anchors) in active_field_edges {
                            FieldConnector {
                                key: "field-{ei}",
                                start: anchors.start,
                                end: anchors.end,
                                kind_attr: anchors.kind_attr,
                                kind: anchors.kind,
                                path: anchors.path,
                                spotlight: spotlight_edges.contains(&ei),
                            }
                        }
                        for (ei, anchors) in active_role_edges {
                            FieldConnector {
                                key: "role-{ei}",
                                start: anchors.start,
                                end: anchors.end,
                                kind_attr: anchors.kind_attr,
                                kind: anchors.kind,
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
        CanvasPoint, FieldKind, FieldRow, NODE_WIDTH, RouteBranch, StageKind, StageView,
        derive_pipeline_view,
    };

    use super::super::connector::{
        ConnectorEndpoints, ConnectorObstacle, obstacle_aware_channel_paths,
    };
    use super::{
        FIELD_ROW_CAP, FieldDisplayState, field_matches_by_node, project_stage_fields,
        rendered_card_height, text_matches_query,
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
                })
                .collect(),
            branches: Vec::<RouteBranch>::new(),
            role_ports: Vec::new(),
        }
    }

    #[test]
    fn wide_schema_projection_caps_default_rows() {
        let stage = wide_stage(120);
        let projected =
            project_stage_fields(&stage, &FieldDisplayState::default(), &HashSet::new());

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
        let filtered = project_stage_fields(
            &stage,
            &FieldDisplayState {
                visible_limit: FIELD_ROW_CAP,
                query: "customer".to_string(),
            },
            &HashSet::new(),
        );

        assert_eq!(filtered.stage.fields.len(), 1);
        assert_eq!(filtered.stage.fields[0].name, "field_099");
        assert_eq!(filtered.display.matching_count, 1);
        assert_eq!(filtered.display.hidden_count, 0);
        assert_eq!(filtered.display.next_count, 0);
        assert_eq!(filtered.stage.field_index("field_099"), Some(0));

        let paged = project_stage_fields(
            &stage,
            &FieldDisplayState {
                visible_limit: FIELD_ROW_CAP * 2,
                query: String::new(),
            },
            &HashSet::new(),
        );

        assert_eq!(paged.stage.fields.len(), FIELD_ROW_CAP * 2);
        assert_eq!(paged.display.hidden_count, 120 - FIELD_ROW_CAP * 2);
        assert_eq!(paged.display.next_count, FIELD_ROW_CAP);
        assert!(paged.display.can_reduce);
        assert_eq!(paged.stage.fields[FIELD_ROW_CAP].name, "field_024");
        assert_eq!(paged.stage.field_index("field_099"), None);

        let fully_visible = project_stage_fields(
            &stage,
            &FieldDisplayState {
                visible_limit: 120,
                query: String::new(),
            },
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
        let projected = project_stage_fields(&stage, &FieldDisplayState::default(), &temporary);

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

        let expanded = project_stage_fields(
            &stage,
            &FieldDisplayState {
                visible_limit: 120,
                query: String::new(),
            },
            &temporary,
        );
        assert_eq!(expanded.stage.fields.len(), 120);
        assert!(expanded.display.temporary_fields.is_empty());
    }

    #[test]
    fn field_search_accepts_wildcards() {
        assert!(text_matches_query("customer_id", "cust*"));
        assert!(text_matches_query("customer_id", "*_id"));
        assert!(text_matches_query("field_007", "field_00?"));
        assert!(!text_matches_query("field_010", "field_00?"));

        let stage = wide_stage(12);
        let matches = field_matches_by_node(&[stage], "field_00?");
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
                project_stage_fields(stage, &FieldDisplayState::default(), &HashSet::new())
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
}
