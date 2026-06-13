/// Derives canvas-renderable stage data from a `PipelineConfig`.
///
/// The canvas dispatches on `PipelineNode` variants directly. Every variant —
/// Source, Transform, Aggregate, Route, Merge, Output, Composition — maps 1:1
/// to a [`StageKind`] via an exhaustive `match` in [`stage_kind_for_node`], so
/// adding a new variant to `PipelineNode` is a compile-time break. Composition
/// currently renders as a placeholder badge pending full sub-canvas rendering.
use clinker_core::config::composition::{CompositionFile, OutputAlias, PortDecl};
use clinker_core::config::node_header::NodeInput;
use clinker_core::config::{PipelineConfig, PipelineNode};
use clinker_core::yaml::Spanned;

pub const NODE_HEIGHT: f32 = 92.0;
pub const NODE_WIDTH: f32 = 160.0;

#[derive(Clone, Debug, PartialEq)]
pub enum StageKind {
    Source,
    Transform,
    Aggregate,
    Route,
    Merge,
    Combine,
    Output,
    Composition,
    InputPort,
    OutputPort,
    Error,
}

impl StageKind {
    pub fn kind_attr(&self) -> &'static str {
        match self {
            StageKind::Source => "source",
            StageKind::Transform => "transform",
            StageKind::Aggregate => "aggregate",
            StageKind::Route => "route",
            StageKind::Merge => "merge",
            StageKind::Combine => "combine",
            StageKind::Output => "output",
            StageKind::Composition => "composition",
            StageKind::InputPort => "input-port",
            StageKind::OutputPort => "output-port",
            StageKind::Error => "error",
        }
    }

    pub fn badge_label(&self) -> &'static str {
        match self {
            StageKind::Source => "SOURCE",
            StageKind::Transform => "TRANSFORM",
            StageKind::Aggregate => "AGGREGATE",
            StageKind::Route => "ROUTE",
            StageKind::Merge => "MERGE",
            StageKind::Combine => "COMBINE",
            StageKind::Output => "OUTPUT",
            StageKind::Composition => "COMPOSITION",
            StageKind::InputPort => "INPUT",
            StageKind::OutputPort => "OUTPUT",
            StageKind::Error => "ERROR",
        }
    }
}

/// Exhaustive compile-time variant dispatch: classify a [`PipelineNode`]
/// into its canvas [`StageKind`]. Adding a new variant to `PipelineNode`
/// without updating this match is a build error.
pub fn stage_kind_for_node(node: &PipelineNode) -> StageKind {
    match node {
        PipelineNode::Source { .. } => StageKind::Source,
        PipelineNode::Transform { .. } => StageKind::Transform,
        PipelineNode::Aggregate { .. } => StageKind::Aggregate,
        PipelineNode::Route { .. } => StageKind::Route,
        PipelineNode::Merge { .. } => StageKind::Merge,
        PipelineNode::Combine { .. } => StageKind::Combine,
        PipelineNode::Output { .. } => StageKind::Output,
        PipelineNode::Composition { .. } => StageKind::Composition,
    }
}

fn node_input_name(ni: &NodeInput) -> &str {
    match ni {
        NodeInput::Single(s) => s.as_str(),
        NodeInput::Port { node, .. } => node.as_str(),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct StageView {
    pub id: String,
    pub label: String,
    pub kind: StageKind,
    pub subtitle: String,
    pub canvas_x: f32,
    pub canvas_y: f32,
    pub cxl_source: Option<String>,
    pub description: Option<String>,
    pub error_message: Option<String>,
}

impl StageView {
    pub fn port_out(&self) -> (f32, f32) {
        (
            self.canvas_x + NODE_WIDTH,
            self.canvas_y + NODE_HEIGHT / 2.0,
        )
    }

    pub fn port_in(&self) -> (f32, f32) {
        (self.canvas_x, self.canvas_y + NODE_HEIGHT / 2.0)
    }
}

const NODE_GAP: f32 = 80.0;
const LEFT_MARGIN: f32 = 60.0;
const BASE_Y: f32 = 120.0;
const STAGGER_Y: f32 = 20.0;
const STACK_GAP: f32 = 24.0;
const INPUT_Y_OFFSET: f32 = 30.0;
/// Vertical midline every column is centered on. Columns with different node
/// counts share this line so the graph reads as a balanced horizontal flow
/// instead of a top-anchored ragged stack.
const COLUMN_CENTER_Y: f32 = 260.0;

#[derive(Clone, Debug, PartialEq)]
pub struct PipelineView {
    pub stages: Vec<StageView>,
    /// Explicit connections between stages: `(from_idx, to_idx)`.
    pub connections: Vec<(usize, usize)>,
}

/// Axis-aligned bounding box of a node layout in world coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutBounds {
    pub min_x: f32,
    pub min_y: f32,
    pub max_x: f32,
    pub max_y: f32,
}

impl LayoutBounds {
    pub fn width(&self) -> f32 {
        self.max_x - self.min_x
    }

    pub fn height(&self) -> f32 {
        self.max_y - self.min_y
    }
}

/// World-space bounding box enclosing every stage card, or `None` when there
/// are no stages. Each card spans [`NODE_WIDTH`] × [`NODE_HEIGHT`] from its
/// top-left `(canvas_x, canvas_y)`; the box is the union of all card rects.
pub fn layout_bounds(stages: &[StageView]) -> Option<LayoutBounds> {
    let first = stages.first()?;
    let mut b = LayoutBounds {
        min_x: first.canvas_x,
        min_y: first.canvas_y,
        max_x: first.canvas_x + NODE_WIDTH,
        max_y: first.canvas_y + NODE_HEIGHT,
    };
    for s in &stages[1..] {
        b.min_x = b.min_x.min(s.canvas_x);
        b.min_y = b.min_y.min(s.canvas_y);
        b.max_x = b.max_x.max(s.canvas_x + NODE_WIDTH);
        b.max_y = b.max_y.max(s.canvas_y + NODE_HEIGHT);
    }
    Some(b)
}

/// Pan/zoom that frames `bounds` inside a `viewport_w` × `viewport_h` pixel
/// viewport with `margin` pixels of padding on every side.
///
/// The canvas viewport applies `translate(pan_x, pan_y) scale(zoom)` with
/// `transform-origin: 0 0`, so a world point `p` maps to screen
/// `pan + p * zoom`. We pick the largest `zoom` (clamped to
/// `[zoom_min, zoom_max]`) at which the padded box fits both axes, then set
/// `pan` so the box's center lands at the viewport center. Returns
/// `(pan_x, pan_y, zoom)`. Degenerate viewports (≤ 0) yield identity pan at
/// the minimum-fit zoom so the caller never divides by zero.
pub fn fit_transform(
    bounds: LayoutBounds,
    viewport_w: f32,
    viewport_h: f32,
    margin: f32,
    zoom_min: f32,
    zoom_max: f32,
) -> (f32, f32, f32) {
    // Usable space after reserving margin on both sides of each axis.
    let avail_w = (viewport_w - 2.0 * margin).max(1.0);
    let avail_h = (viewport_h - 2.0 * margin).max(1.0);

    // A zero-extent box (single node, or a column of coincident points on one
    // axis) must not force an infinite zoom — treat its extent as one node.
    let box_w = bounds.width().max(NODE_WIDTH);
    let box_h = bounds.height().max(NODE_HEIGHT);

    let zoom = (avail_w / box_w)
        .min(avail_h / box_h)
        .clamp(zoom_min, zoom_max);

    // Center the box: viewport_center = pan + box_center * zoom.
    let box_cx = (bounds.min_x + bounds.max_x) / 2.0;
    let box_cy = (bounds.min_y + bounds.max_y) / 2.0;
    let pan_x = viewport_w / 2.0 - box_cx * zoom;
    let pan_y = viewport_h / 2.0 - box_cy * zoom;
    (pan_x, pan_y, zoom)
}

/// Compute `(canvas_x, canvas_y)` for every node from a column assignment and
/// a predecessor relation, applying a barycenter crossing-reduction pass.
///
/// `cols[i]` is node `i`'s pre-computed column (sources sit at the lowest
/// column). `predecessors[i]` lists the node indices that feed node `i`.
/// Callers own column assignment because the two derivation paths discover
/// edges differently (config headers vs. a petgraph); this function is the
/// shared geometry + ordering core so both canvases stay visually consistent.
///
/// Ordering: a single left-to-right sweep places the first column in
/// declaration order, then orders each later column by the mean row-index of
/// each node's predecessors (the barycenter heuristic). Reducing the average
/// vertical distance between connected nodes is what cuts edge crossings on
/// fan-in / fan-out DAGs. The sort is stable, so nodes that share a barycenter
/// (or have no resolved predecessors) keep their declaration order.
///
/// Spacing: within each column the ordered nodes are evenly stacked and the
/// whole column is centered on [`COLUMN_CENTER_Y`], so columns with different
/// node counts share a midline. The returned `Vec` is indexed by node index
/// (parallel to `cols`).
fn layout_positions(cols: &[usize], predecessors: &[Vec<usize>]) -> Vec<(f32, f32)> {
    use std::collections::BTreeMap;

    let n = cols.len();
    debug_assert_eq!(n, predecessors.len());
    if n == 0 {
        return Vec::new();
    }

    // Group node indices by column, preserving declaration order within a
    // column. BTreeMap keeps columns ascending so the sweep is left-to-right
    // and every predecessor's row is fixed before its consumers are ordered.
    let mut by_col: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (idx, &col) in cols.iter().enumerate() {
        by_col.entry(col).or_default().push(idx);
    }

    // Row index assigned to each node within its column, filled column by
    // column so a node's barycenter can read its predecessors' fixed rows.
    let mut row_of: Vec<usize> = vec![0; n];
    let mut positions: Vec<(f32, f32)> = vec![(0.0, 0.0); n];

    for (&col, members) in &by_col {
        let mut ordered: Vec<usize> = members.clone();
        ordered.sort_by(|&a, &b| {
            let ka = barycenter(a, predecessors, &row_of);
            let kb = barycenter(b, predecessors, &row_of);
            ka.partial_cmp(&kb).unwrap_or(std::cmp::Ordering::Equal)
        });

        let count = ordered.len();
        let total_h = count as f32 * NODE_HEIGHT + (count.saturating_sub(1)) as f32 * STACK_GAP;
        let top = COLUMN_CENTER_Y - total_h / 2.0;
        let x = LEFT_MARGIN + (col as f32) * (NODE_WIDTH + NODE_GAP);
        for (row, &idx) in ordered.iter().enumerate() {
            row_of[idx] = row;
            positions[idx] = (x, top + row as f32 * (NODE_HEIGHT + STACK_GAP));
        }
    }

    positions
}

/// Mean row-index of a node's predecessors, or `f32::MAX` when none resolve.
///
/// `f32::MAX` sorts predecessor-less nodes to the end of their column; paired
/// with a stable sort this preserves their declaration order.
fn barycenter(idx: usize, predecessors: &[Vec<usize>], row_of: &[usize]) -> f32 {
    let preds = &predecessors[idx];
    if preds.is_empty() {
        return f32::MAX;
    }
    let sum: usize = preds.iter().map(|&p| row_of[p]).sum();
    sum as f32 / preds.len() as f32
}

/// Walk `config.nodes` in declaration order and dispatch on `PipelineNode`
/// variant to produce a [`StageView`] for every node. Connections are derived
/// from each consumer's `input:` / `inputs:` header field. The match arms
/// here mirror [`stage_kind_for_node`]; both are compile-time exhaustive, so
/// adding a new `PipelineNode` variant is a build error.
///
/// Node placement is delegated to [`layout_positions`]: each node's column is
/// `1 + max(column of inputs)` (sources at column 0), then the shared
/// barycenter pass orders rows and assigns even, centered coordinates.
pub fn derive_pipeline_view(config: &PipelineConfig) -> PipelineView {
    derive_view_from_nodes(&config.nodes)
}

/// Derive a canvas view from a flat node list.
///
/// Shared by pipelines and compositions: both store the same
/// `Vec<Spanned<PipelineNode>>`, so column/edge derivation is identical. A
/// composition's body nodes render exactly like a pipeline's; references to the
/// composition's *input ports* (which are not nodes) simply produce no edge.
pub fn derive_view_from_nodes(nodes: &[Spanned<PipelineNode>]) -> PipelineView {
    use std::collections::HashMap;

    // Column = 1 + max column of inputs; sources sit in column 0.
    let mut name_to_idx: HashMap<String, usize> = HashMap::new();
    let mut cols: Vec<usize> = Vec::with_capacity(nodes.len());
    for (idx, spanned) in nodes.iter().enumerate() {
        let node = &spanned.value;
        name_to_idx.insert(node.name().to_string(), idx);
        let col = match node {
            PipelineNode::Source { .. } => 0,
            PipelineNode::Merge { header, .. } => header
                .inputs
                .iter()
                .filter_map(|ni| name_to_idx.get(node_input_name(&ni.value)).copied())
                .map(|i| cols[i] + 1)
                .max()
                .unwrap_or(1),
            PipelineNode::Combine { header, .. } => header
                .input
                .values()
                .filter_map(|ni| name_to_idx.get(node_input_name(&ni.value)).copied())
                .map(|i| cols[i] + 1)
                .max()
                .unwrap_or(1),
            PipelineNode::Transform { header, .. }
            | PipelineNode::Aggregate { header, .. }
            | PipelineNode::Route { header, .. }
            | PipelineNode::Output { header, .. }
            | PipelineNode::Composition { header, .. } => name_to_idx
                .get(node_input_name(&header.input.value))
                .copied()
                .map(|i| cols[i] + 1)
                .unwrap_or(1),
        };
        cols.push(col);
    }

    // Connections: resolve each consumer's input header reference. These also
    // form the predecessor relation the barycenter layout pass consumes.
    let mut connections: Vec<(usize, usize)> = Vec::new();
    let mut predecessors: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
    for (idx, spanned) in nodes.iter().enumerate() {
        match &spanned.value {
            PipelineNode::Source { .. } => {}
            PipelineNode::Merge { header, .. } => {
                for ni in &header.inputs {
                    if let Some(&from) = name_to_idx.get(node_input_name(&ni.value)) {
                        connections.push((from, idx));
                        predecessors[idx].push(from);
                    }
                }
            }
            PipelineNode::Combine { header, .. } => {
                for ni in header.input.values() {
                    if let Some(&from) = name_to_idx.get(node_input_name(&ni.value)) {
                        connections.push((from, idx));
                        predecessors[idx].push(from);
                    }
                }
            }
            PipelineNode::Transform { header, .. }
            | PipelineNode::Aggregate { header, .. }
            | PipelineNode::Route { header, .. }
            | PipelineNode::Output { header, .. }
            | PipelineNode::Composition { header, .. } => {
                if let Some(&from) = name_to_idx.get(node_input_name(&header.input.value)) {
                    connections.push((from, idx));
                    predecessors[idx].push(from);
                }
            }
        }
    }

    // Place nodes: barycenter ordering within each column + even centered
    // spacing. `positions` is parallel to `nodes`.
    let positions = layout_positions(&cols, &predecessors);
    let stages: Vec<StageView> = nodes
        .iter()
        .zip(positions)
        .map(|(spanned, (x, y))| build_stage_view(&spanned.value, x, y))
        .collect();

    PipelineView {
        stages,
        connections,
    }
}

/// Derive a canvas view for a composition file (`*.comp.yaml`).
///
/// Draws the composition's `_compose` contract as boundary nodes around the
/// body DAG: each declared input port is a left-column node feeding the body
/// nodes that reference it, and each output port is a right-column node fed by
/// the body node named in its `internal_ref`. Body nodes render exactly as in a
/// pipeline (via [`build_stage_view`]); ports and body share one index space so
/// the barycenter [`layout_positions`] pass lays them out together.
pub fn derive_composition_view(comp: &CompositionFile) -> PipelineView {
    use std::collections::HashMap;

    let sig = &comp.signature;
    let body = &comp.nodes;
    let n_in = sig.inputs.len();
    let n_body = body.len();
    let n_out = sig.outputs.len();
    let total = n_in + n_body + n_out;

    // Two SEPARATE namespaces, mirroring the engine: composition input-port
    // names and body-internal node names are distinct, and a body node may
    // legally share a name with a port. `port_idx` is fixed; `body_idx` is
    // filled in declaration order. A body node's `input:` reference resolves to
    // an already-declared body node first, else an input port — so a forward or
    // self reference never resolves (and never indexes an unset column). A
    // single shared map would let a body node overwrite a same-named port and
    // index its own not-yet-finalized column → out-of-bounds panic on the
    // unvalidated YAML the canvas renders live.
    let mut port_idx: HashMap<&str, usize> = HashMap::with_capacity(n_in);
    for (i, port) in sig.inputs.keys().enumerate() {
        port_idx.insert(port.as_str(), i);
    }
    let mut body_idx: HashMap<&str, usize> = HashMap::with_capacity(n_body);

    // Resolve a body input reference to its unified index: an already-declared
    // body node first, otherwise an input port.
    let resolve = |name: &str, body_idx: &HashMap<&str, usize>| -> Option<usize> {
        body_idx
            .get(name)
            .copied()
            .or_else(|| port_idx.get(name).copied())
    };

    // Unified index space: [0, n_in) input ports (column 0), [n_in, n_in+n_body)
    // body nodes, [n_in+n_body, total) output ports. `cols`/`predecessors` are
    // sized once over that space; ports default to column 0 / no predecessors.
    let mut cols: Vec<usize> = vec![0; total];
    let mut connections: Vec<(usize, usize)> = Vec::new();
    let mut predecessors: Vec<Vec<usize>> = vec![Vec::new(); total];

    for (bi, spanned) in body.iter().enumerate() {
        let idx = n_in + bi;
        let node = &spanned.value;

        // Resolved predecessors: input ports + upstream body nodes.
        let mut preds: Vec<usize> = Vec::new();
        match node {
            PipelineNode::Source { .. } => {}
            PipelineNode::Merge { header, .. } => {
                for ni in &header.inputs {
                    if let Some(p) = resolve(node_input_name(&ni.value), &body_idx) {
                        preds.push(p);
                    }
                }
            }
            PipelineNode::Combine { header, .. } => {
                for ni in header.input.values() {
                    if let Some(p) = resolve(node_input_name(&ni.value), &body_idx) {
                        preds.push(p);
                    }
                }
            }
            PipelineNode::Transform { header, .. }
            | PipelineNode::Aggregate { header, .. }
            | PipelineNode::Route { header, .. }
            | PipelineNode::Output { header, .. }
            | PipelineNode::Composition { header, .. } => {
                if let Some(p) = resolve(node_input_name(&header.input.value), &body_idx) {
                    preds.push(p);
                }
            }
        }

        // Column = 1 + max predecessor column; a Source anchors at 0, an
        // unresolved reference at 1. Every `p` is a port or an earlier body
        // node, so `cols[p]` is always already set.
        cols[idx] = if matches!(node, PipelineNode::Source { .. }) {
            0
        } else {
            preds.iter().map(|&p| cols[p] + 1).max().unwrap_or(1)
        };
        for &p in &preds {
            connections.push((p, idx));
            predecessors[idx].push(p);
        }

        // Insert AFTER the column is computed so a self reference can't resolve
        // to this node's own (not-yet-finalized) index.
        body_idx.insert(node.name(), idx);
    }

    // Output ports share one column to the right of the whole body, each fed by
    // the body node named in its `internal_ref` ("node.channel" -> node part).
    // An output whose ref names no body node is still drawn, just unconnected.
    let out_col = cols[n_in..n_in + n_body].iter().copied().max().unwrap_or(0) + 1;
    for (oi, alias) in sig.outputs.values().enumerate() {
        let out_idx = n_in + n_body + oi;
        cols[out_idx] = out_col;
        let producer = alias.internal_ref.value.split('.').next().unwrap_or("");
        if let Some(&from) = body_idx.get(producer) {
            connections.push((from, out_idx));
            predecessors[out_idx].push(from);
        }
    }

    let positions = layout_positions(&cols, &predecessors);

    let mut stages: Vec<StageView> = Vec::with_capacity(total);
    for (i, (port, decl)) in sig.inputs.iter().enumerate() {
        let (x, y) = positions[i];
        stages.push(input_port_stage(port, decl, x, y));
    }
    for (bi, spanned) in body.iter().enumerate() {
        let (x, y) = positions[n_in + bi];
        stages.push(build_stage_view(&spanned.value, x, y));
    }
    for (oi, (port, alias)) in sig.outputs.iter().enumerate() {
        let (x, y) = positions[n_in + n_body + oi];
        stages.push(output_port_stage(port, alias, x, y));
    }

    PipelineView {
        stages,
        connections,
    }
}

/// Synthetic boundary node for a composition input port.
fn input_port_stage(name: &str, decl: &PortDecl, x: f32, y: f32) -> StageView {
    let subtitle = match &decl.schema {
        Some(schema) => {
            let n = schema.columns.len();
            format!("{n} field{}", if n == 1 { "" } else { "s" })
        }
        None => "any shape".to_string(),
    };
    StageView {
        // `port:` prefix with a colon — invalid in body-node identifiers — so a
        // port id can never collide with a body node's id (its raw name), which
        // is the RSX key and selection identity.
        id: format!("port:in:{name}"),
        label: name.to_string(),
        kind: StageKind::InputPort,
        subtitle,
        canvas_x: x,
        canvas_y: y,
        cxl_source: None,
        description: decl.description.clone(),
        error_message: None,
    }
}

/// Synthetic boundary node for a composition output port.
fn output_port_stage(name: &str, alias: &OutputAlias, x: f32, y: f32) -> StageView {
    StageView {
        // See `input_port_stage`: colon-prefixed id avoids body-node id clashes.
        id: format!("port:out:{name}"),
        label: name.to_string(),
        kind: StageKind::OutputPort,
        subtitle: format!("\u{2190} {}", alias.internal_ref.value),
        canvas_x: x,
        canvas_y: y,
        cxl_source: None,
        description: alias.description.clone(),
        error_message: None,
    }
}

/// Variant-dispatched [`StageView`] constructor. Every arm is exhaustive;
/// adding a new `PipelineNode` variant breaks the build here.
fn build_stage_view(node: &PipelineNode, x: f32, y: f32) -> StageView {
    let kind = stage_kind_for_node(node);
    match node {
        PipelineNode::Source {
            header,
            config: body,
        } => StageView {
            id: header.name.clone(),
            label: header.name.clone(),
            kind,
            subtitle: body.source.display_target(),
            canvas_x: x,
            canvas_y: y,
            cxl_source: None,
            description: header.description.clone(),
            error_message: None,
        },
        PipelineNode::Transform {
            header,
            config: body,
        } => {
            let cxl_src: &str = body.cxl.as_ref();
            StageView {
                id: header.name.clone(),
                label: header.name.clone(),
                kind,
                subtitle: cxl_subtitle(cxl_src),
                canvas_x: x,
                canvas_y: y,
                cxl_source: Some(cxl_src.to_string()),
                description: header.description.clone(),
                error_message: None,
            }
        }
        PipelineNode::Aggregate {
            header,
            config: body,
        } => {
            let cxl_src: &str = body.cxl.as_ref();
            let subtitle = if body.group_by.is_empty() {
                cxl_subtitle(cxl_src)
            } else {
                format!("by {}", body.group_by.join(", "))
            };
            StageView {
                id: header.name.clone(),
                label: header.name.clone(),
                kind,
                subtitle,
                canvas_x: x,
                canvas_y: y,
                cxl_source: Some(cxl_src.to_string()),
                description: header.description.clone(),
                error_message: None,
            }
        }
        PipelineNode::Route {
            header,
            config: body,
        } => {
            let subtitle = format!(
                "{} branch{} → {}",
                body.conditions.len(),
                if body.conditions.len() == 1 { "" } else { "es" },
                body.default
            );
            StageView {
                id: header.name.clone(),
                label: header.name.clone(),
                kind,
                subtitle,
                canvas_x: x,
                canvas_y: y,
                cxl_source: None,
                description: header.description.clone(),
                error_message: None,
            }
        }
        PipelineNode::Merge { header, .. } => StageView {
            id: header.name.clone(),
            label: header.name.clone(),
            kind,
            subtitle: format!("{} inputs", header.inputs.len()),
            canvas_x: x,
            canvas_y: y,
            cxl_source: None,
            description: header.description.clone(),
            error_message: None,
        },
        PipelineNode::Combine {
            header,
            config: body,
        } => {
            let cxl_src: &str = body.cxl.as_ref();
            StageView {
                id: header.name.clone(),
                label: header.name.clone(),
                kind,
                subtitle: format!("{} inputs", header.input.len()),
                canvas_x: x,
                canvas_y: y,
                cxl_source: Some(cxl_src.to_string()),
                description: header.description.clone(),
                error_message: None,
            }
        }
        PipelineNode::Output {
            header,
            config: body,
        } => StageView {
            id: header.name.clone(),
            label: header.name.clone(),
            kind,
            subtitle: body.output.path.clone(),
            canvas_x: x,
            canvas_y: y,
            cxl_source: None,
            description: header.description.clone(),
            error_message: None,
        },
        PipelineNode::Composition { header, r#use, .. } => StageView {
            id: header.name.clone(),
            label: header.name.clone(),
            kind,
            subtitle: format!("use: {}", r#use.display()),
            canvas_x: x,
            canvas_y: y,
            cxl_source: None,
            description: header.description.clone(),
            error_message: None,
        },
    }
}

fn stack_y_positions(count: usize, center_y: f32) -> Vec<f32> {
    if count == 0 {
        return Vec::new();
    }
    let total_h = count as f32 * NODE_HEIGHT + (count as f32 - 1.0) * STACK_GAP;
    let top = center_y - total_h / 2.0;
    (0..count)
        .map(|i| top + i as f32 * (NODE_HEIGHT + STACK_GAP))
        .collect()
}

fn build_connections(
    input_count: usize,
    input_targets: &[usize],
    transform_count: usize,
    output_count: usize,
) -> Vec<(usize, usize)> {
    let mut conns = Vec::new();
    let t_base = input_count;

    if transform_count > 0 {
        for (i, &target) in input_targets.iter().enumerate() {
            let target_stage = t_base + target.min(transform_count - 1);
            conns.push((i, target_stage));
        }
        for i in 0..transform_count - 1 {
            conns.push((t_base + i, t_base + i + 1));
        }
        let last_t = t_base + transform_count - 1;
        let o_base = t_base + transform_count;
        for j in 0..output_count {
            conns.push((last_t, o_base + j));
        }
    } else {
        let o_base = input_count;
        for i in 0..input_count {
            for j in 0..output_count {
                conns.push((i, o_base + j));
            }
        }
    }

    conns
}

/// Derive canvas nodes from a `PartialPipelineConfig` (graceful degradation).
pub fn derive_partial_pipeline_view(
    partial: &clinker_core::partial::PartialPipelineConfig,
) -> PipelineView {
    use clinker_core::partial::PartialItem;

    let input_count = partial.inputs.len();
    let transform_count = partial.transformations.len();
    let output_count = partial.outputs.len();

    let y_for = |i: usize| BASE_Y + if i.is_multiple_of(2) { 0.0 } else { STAGGER_Y };

    let transform_x_start = if input_count == 0 {
        LEFT_MARGIN
    } else {
        LEFT_MARGIN + NODE_WIDTH + NODE_GAP
    };
    let mut x = transform_x_start;
    let mut transform_stages = Vec::new();

    for (idx, item) in partial.transformations.iter().enumerate() {
        let y = y_for(idx);
        match item {
            PartialItem::Ok(t) => {
                transform_stages.push(StageView {
                    id: t.name.clone(),
                    label: t.name.clone(),
                    kind: StageKind::Transform,
                    subtitle: cxl_subtitle(t.cxl_source()),
                    canvas_x: x,
                    canvas_y: y,
                    cxl_source: Some(t.cxl_source().to_string()),
                    description: t.description.clone(),
                    error_message: None,
                });
            }
            PartialItem::Err { index, message } => {
                transform_stages.push(StageView {
                    id: format!("_err_transform_{index}"),
                    label: format!("transform #{}", index + 1),
                    kind: StageKind::Error,
                    subtitle: truncate_error(message),
                    canvas_x: x,
                    canvas_y: y,
                    cxl_source: None,
                    description: None,
                    error_message: Some(message.clone()),
                });
            }
        }
        x += NODE_WIDTH + NODE_GAP;
    }

    let input_targets: Vec<usize> = vec![0; input_count];
    let mut input_stages = Vec::new();
    for (input_idx, item) in partial.inputs.iter().enumerate() {
        let (ix, iy) = if input_idx == 0 && !transform_stages.is_empty() {
            (LEFT_MARGIN, transform_stages[0].canvas_y)
        } else if !transform_stages.is_empty() {
            let target = &transform_stages[0];
            let stack_n = input_idx - 1;
            let iy = target.canvas_y
                - NODE_HEIGHT
                - INPUT_Y_OFFSET
                - (stack_n as f32) * (NODE_HEIGHT + STACK_GAP);
            (target.canvas_x - NODE_WIDTH - NODE_GAP / 2.0, iy)
        } else {
            (
                LEFT_MARGIN,
                BASE_Y + (input_idx as f32) * (NODE_HEIGHT + STACK_GAP),
            )
        };
        match item {
            PartialItem::Ok(input) => {
                input_stages.push(StageView {
                    id: input.name.clone(),
                    label: input.name.clone(),
                    kind: StageKind::Source,
                    subtitle: input.display_target(),
                    canvas_x: ix,
                    canvas_y: iy,
                    cxl_source: None,
                    description: None,
                    error_message: None,
                });
            }
            PartialItem::Err { index, message } => {
                input_stages.push(StageView {
                    id: format!("_err_input_{index}"),
                    label: format!("input #{}", index + 1),
                    kind: StageKind::Error,
                    subtitle: truncate_error(message),
                    canvas_x: ix,
                    canvas_y: iy,
                    cxl_source: None,
                    description: None,
                    error_message: Some(message.clone()),
                });
            }
        }
    }

    let mut output_stages = Vec::new();
    let output_x = if transform_stages.is_empty() {
        LEFT_MARGIN + NODE_WIDTH + NODE_GAP
    } else {
        let last = transform_stages.last().unwrap();
        last.canvas_x + NODE_WIDTH + NODE_GAP
    };
    let center_y = BASE_Y + NODE_HEIGHT / 2.0 + STAGGER_Y / 2.0;
    let output_ys = stack_y_positions(output_count, center_y);
    for (i, item) in partial.outputs.iter().enumerate() {
        match item {
            PartialItem::Ok(output) => {
                output_stages.push(StageView {
                    id: output.name.clone(),
                    label: output.name.clone(),
                    kind: StageKind::Output,
                    subtitle: output.path.clone(),
                    canvas_x: output_x,
                    canvas_y: output_ys[i],
                    cxl_source: None,
                    description: None,
                    error_message: None,
                });
            }
            PartialItem::Err { index, message } => {
                output_stages.push(StageView {
                    id: format!("_err_output_{index}"),
                    label: format!("output #{}", index + 1),
                    kind: StageKind::Error,
                    subtitle: truncate_error(message),
                    canvas_x: output_x,
                    canvas_y: output_ys[i],
                    cxl_source: None,
                    description: None,
                    error_message: Some(message.clone()),
                });
            }
        }
    }

    let connections = build_connections(input_count, &input_targets, transform_count, output_count);

    let mut stages = Vec::with_capacity(input_count + transform_count + output_count);
    stages.extend(input_stages);
    stages.extend(transform_stages);
    stages.extend(output_stages);

    PipelineView {
        stages,
        connections,
    }
}

fn truncate_error(msg: &str) -> String {
    if msg.len() <= 40 {
        msg.to_string()
    } else {
        format!("{}...", &msg[..37])
    }
}

fn cxl_subtitle(cxl: &str) -> String {
    cxl.lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim()
        .chars()
        .take(30)
        .collect()
}

/// Derive canvas nodes from a `BoundBody`'s mini-DAG.
///
/// Used when drilled into a composition: the sub-canvas renders the
/// composition's internal nodes. Layout walks `body.topo_order` and
/// places each node at column = 1 + max(predecessor columns), then defers
/// row ordering and spacing to the shared [`layout_positions`] barycenter
/// pass — the same placement `derive_pipeline_view` uses. Edges come from
/// `body.graph.edge_references()` so route, merge, and combine branches all
/// render as the real DAG instead of a synthetic chain.
pub fn derive_body_view(body: &clinker_core::plan::composition_body::BoundBody) -> PipelineView {
    use clinker_core::plan::execution::PlanNode;
    use petgraph::Direction;
    use petgraph::graph::NodeIndex;
    use petgraph::visit::EdgeRef;
    use std::collections::HashMap;

    // Column = 1 + max column of incoming neighbors; nodes with no
    // incoming edges (sources, port-seed nodes) sit at column 0.
    let mut cols: HashMap<NodeIndex, usize> = HashMap::with_capacity(body.topo_order.len());
    for &idx in &body.topo_order {
        let col = body
            .graph
            .neighbors_directed(idx, Direction::Incoming)
            .filter_map(|p| cols.get(&p).copied())
            .map(|c| c + 1)
            .max()
            .unwrap_or(0);
        cols.insert(idx, col);
    }

    // Build stages in topo order so each node's slot is its push index, then
    // hand the column assignment and predecessor relation to the shared
    // barycenter layout pass — the same placement the top-level canvas uses,
    // so a drilled-in body feels visually consistent with its parent.
    let mut idx_to_slot: HashMap<NodeIndex, usize> = HashMap::with_capacity(body.topo_order.len());
    let mut slot_cols: Vec<usize> = Vec::with_capacity(body.topo_order.len());
    let mut stages: Vec<StageView> = Vec::with_capacity(body.topo_order.len());

    for &node_idx in &body.topo_order {
        let plan_node = &body.graph[node_idx];
        let col = cols.get(&node_idx).copied().unwrap_or(0);

        let (id, kind, subtitle) = match plan_node {
            PlanNode::Source { name, .. } => (name.clone(), StageKind::Source, String::new()),
            PlanNode::Transform { name, .. } => (name.clone(), StageKind::Transform, String::new()),
            PlanNode::Route { name, mode, .. } => {
                (name.clone(), StageKind::Route, format!("{mode:?}"))
            }
            PlanNode::Merge { name, .. } => (name.clone(), StageKind::Merge, String::new()),
            PlanNode::Output { name, .. } => (name.clone(), StageKind::Output, String::new()),
            PlanNode::Sort { name, .. } => (name.clone(), StageKind::Transform, "sort".into()),
            PlanNode::Aggregation { name, strategy, .. } => {
                (name.clone(), StageKind::Aggregate, format!("{strategy:?}"))
            }
            PlanNode::Composition { name, .. } => {
                (name.clone(), StageKind::Composition, String::new())
            }
            PlanNode::Combine { name, strategy, .. } => {
                (name.clone(), StageKind::Combine, format!("{strategy:?}"))
            }
            PlanNode::CorrelationCommit { name, .. } => (
                name.clone(),
                StageKind::Transform,
                "correlation_commit".into(),
            ),
        };

        let slot = stages.len();
        idx_to_slot.insert(node_idx, slot);
        slot_cols.push(col);
        stages.push(StageView {
            id,
            label: plan_node.name().to_string(),
            kind,
            subtitle,
            // Overwritten below once every slot is known.
            canvas_x: 0.0,
            canvas_y: 0.0,
            cxl_source: None,
            description: None,
            error_message: None,
        });
    }

    // Walk every edge in the mini-DAG and translate it to a slot pair. Stages
    // were pushed in topo order, so every edge's source and target are already
    // in `idx_to_slot`. The same pairs feed both the connector overlay and the
    // layout pass's predecessor relation.
    let mut connections: Vec<(usize, usize)> = Vec::new();
    let mut predecessors: Vec<Vec<usize>> = vec![Vec::new(); stages.len()];
    for e in body.graph.edge_references() {
        if let (Some(from), Some(to)) = (
            idx_to_slot.get(&e.source()).copied(),
            idx_to_slot.get(&e.target()).copied(),
        ) {
            connections.push((from, to));
            predecessors[to].push(from);
        }
    }

    let positions = layout_positions(&slot_cols, &predecessors);
    for (stage, (x, y)) in stages.iter_mut().zip(positions) {
        stage.canvas_x = x;
        stage.canvas_y = y;
    }

    PipelineView {
        stages,
        connections,
    }
}

#[cfg(test)]
mod migrated_fixture_tests {
    use super::*;
    use clinker_core::config::parse_config;

    /// Compile-time exhaustiveness of the variant dispatch: this function
    /// returns a distinct `StageKind` for every `PipelineNode` variant;
    /// adding a new variant without updating [`stage_kind_for_node`] is a
    /// build error.
    #[test]
    fn test_canvas_node_dispatches_on_variant() {
        // Use a minimal unified-shape YAML exercising every variant so the
        // match in `stage_kind_for_node` is hit for each at runtime too.
        let yaml = r#"
pipeline:
  name: variant_dispatch_smoke
nodes:
  - type: source
    name: src
    config:
      name: src
      type: csv
      path: ./in.csv
      schema:
        - { name: agg, type: string }

  - type: transform
    name: clean
    input: src
    config:
      cxl: |
        emit x = 1
  - type: aggregate
    name: agg
    input: clean
    config:
      group_by: [x]
      cxl: |
        emit n = 1
  - type: route
    name: split
    input: clean
    config:
      conditions:
        hi: "x > 0"
      default: lo
  - type: merge
    name: joined
    inputs: [split.hi, split.lo]
  - type: output
    name: out
    input: joined
    config:
      name: out
      type: csv
      path: ./out.csv
"#;
        let config = parse_config(yaml).expect("parse unified-shape pipeline");
        let view = derive_pipeline_view(&config);

        // Each declared node produces exactly one stage.
        assert_eq!(view.stages.len(), 6);

        // Every variant kind is represented.
        let has = |k: &StageKind| view.stages.iter().any(|s| &s.kind == k);
        assert!(has(&StageKind::Source));
        assert!(has(&StageKind::Transform));
        assert!(has(&StageKind::Aggregate));
        assert!(has(&StageKind::Route));
        assert!(has(&StageKind::Merge));
        assert!(has(&StageKind::Output));
    }

    /// A legacy-shape fixture lifted into the unified `nodes:` topology
    /// still renders via the variant-dispatch code path and produces the
    /// expected stage count.
    #[test]
    fn test_loads_migrated_fixture() {
        let yaml = r#"
pipeline:
  name: loaded_fixture_smoke
nodes:
- type: source
  name: raw
  config:
    name: raw
    type: csv
    path: ./raw.csv
    options:
      has_header: true
    schema:
      - { name: agg, type: string }

- type: transform
  name: clean
  input: raw
  config:
    cxl: 'emit a = 1

      '
- type: transform
  name: finalize
  input: clean
  config:
    cxl: 'emit b = 2

      '
- type: output
  name: results
  input: finalize
  config:
    name: results
    type: csv
    path: ./results.csv
"#;
        let config = parse_config(yaml).expect("legacy fixture lifts to nodes");
        let view = derive_pipeline_view(&config);

        // 1 source + 2 transforms + 1 output = 4 stages.
        assert_eq!(view.stages.len(), 4);
        assert_eq!(
            view.stages
                .iter()
                .filter(|s| s.kind == StageKind::Transform)
                .count(),
            2
        );
        assert_eq!(
            view.stages
                .iter()
                .filter(|s| s.kind == StageKind::Source)
                .count(),
            1
        );
        assert_eq!(
            view.stages
                .iter()
                .filter(|s| s.kind == StageKind::Output)
                .count(),
            1
        );

        // Connections are derived and non-empty for a chain of length > 1.
        assert!(!view.connections.is_empty());
    }

    /// A pipeline containing a `PipelineNode::Composition` stub renders it
    /// as a placeholder stage (badge subtitle) without panic.
    #[test]
    fn test_composition_placeholder_renders() {
        let yaml = r#"
pipeline:
  name: composition_placeholder_smoke
nodes:
  - type: source
    name: src
    config:
      name: src
      type: csv
      path: ./comp_in.csv
      schema:
        - { name: agg, type: string }

  - type: composition
    name: sub
    input: src
    use: compositions/clean_names.comp.yaml
"#;
        let config = parse_config(yaml).expect("composition stub parses");
        let view = derive_pipeline_view(&config);

        let comp = view
            .stages
            .iter()
            .find(|s| s.kind == StageKind::Composition)
            .expect("composition stage present");
        assert_eq!(comp.label, "sub");
        assert!(
            comp.subtitle.starts_with("use: "),
            "composition subtitle should show the `use:` path, got: {}",
            comp.subtitle
        );
        // Badge text is the authoritative kind label.
        assert_eq!(StageKind::Composition.badge_label(), "COMPOSITION");
    }

    fn parse_comp(yaml: &str) -> CompositionFile {
        use clinker_core::span::FileId;
        use std::num::NonZeroU32;
        CompositionFile::parse(
            yaml,
            FileId::new(NonZeroU32::new(1).expect("nonzero")),
            std::path::PathBuf::new(),
        )
        .expect("composition parses")
    }

    #[test]
    fn composition_view_draws_contract_ports() {
        let yaml = r#"_compose:
  name: t
  inputs:
    orders:
      schema:
        - { name: amount, type: float }
  outputs:
    result: b
  config_schema: {}

nodes:
  - type: transform
    name: a
    input: orders
    config:
      cxl: |
        emit doubled = amount * 2.0
  - type: transform
    name: b
    input: a
    config:
      cxl: |
        emit plus = doubled + 1.0
"#;
        let view = derive_composition_view(&parse_comp(yaml));
        // 1 input port + 2 body + 1 output port.
        assert_eq!(view.stages.len(), 4);

        let input = view
            .stages
            .iter()
            .find(|s| s.kind == StageKind::InputPort)
            .expect("input port present");
        assert_eq!(input.label, "orders");
        assert_eq!(
            input.canvas_x, LEFT_MARGIN,
            "input port in the leftmost column"
        );

        let output = view
            .stages
            .iter()
            .find(|s| s.kind == StageKind::OutputPort)
            .expect("output port present");
        assert_eq!(output.label, "result");
        // Output port is to the right of every body node.
        let max_body_x = view
            .stages
            .iter()
            .filter(|s| matches!(s.kind, StageKind::Transform))
            .map(|s| s.canvas_x)
            .fold(f32::MIN, f32::max);
        assert!(
            output.canvas_x > max_body_x,
            "output port in the rightmost column"
        );

        // Edges: input->a, a->b, b->output all present (by stage index).
        let idx = |label: &str| view.stages.iter().position(|s| s.label == label).unwrap();
        let (i_in, i_a, i_b, i_out) = (idx("orders"), idx("a"), idx("b"), idx("result"));
        assert!(view.connections.contains(&(i_in, i_a)));
        assert!(view.connections.contains(&(i_a, i_b)));
        assert!(view.connections.contains(&(i_b, i_out)));
    }

    #[test]
    fn composition_view_port_name_collision_does_not_panic() {
        // Regression: a body node sharing a name with an input port previously
        // panicked (a single shared name map let the body node overwrite the
        // port entry and index its own not-yet-set column). Ports and body
        // names are separate namespaces; the view must render without panicking.
        let yaml = r#"_compose:
  name: t
  inputs:
    dup:
      schema:
        - { name: amount, type: float }
  outputs: {}
  config_schema: {}

nodes:
  - type: transform
    name: dup
    input: dup
    config:
      cxl: |
        emit doubled = amount * 2.0
  - type: transform
    name: consumer
    input: dup
    config:
      cxl: |
        emit plus = doubled + 1.0
"#;
        let view = derive_composition_view(&parse_comp(yaml));
        // 1 input port + 2 body nodes, no outputs — and no panic.
        assert_eq!(view.stages.len(), 3);
        // The port and the same-named body node are distinct stages with
        // distinct ids (the port id is colon-prefixed, the body keeps its name).
        let ids: Vec<&str> = view.stages.iter().map(|s| s.id.as_str()).collect();
        assert!(ids.contains(&"port:in:dup"));
        assert_eq!(
            ids.iter().filter(|id| **id == "dup").count(),
            1,
            "the body node keeps its raw-name id"
        );
        // The body `dup` resolves `input: dup` to the input port (the only
        // `dup` declared when its column is computed), so the port feeds it.
        let port = view
            .stages
            .iter()
            .position(|s| s.id == "port:in:dup")
            .unwrap();
        let body_dup = view.stages.iter().position(|s| s.id == "dup").unwrap();
        assert!(view.connections.contains(&(port, body_dup)));
    }

    #[test]
    fn composition_view_dangling_output_is_drawn_unconnected() {
        // An output whose `internal_ref` names no body node is still drawn as a
        // boundary card — just with no incoming edge (graceful, not dropped).
        let yaml = r#"_compose:
  name: t
  inputs:
    src:
      schema:
        - { name: v, type: float }
  outputs:
    missing: nonexistent_node
  config_schema: {}

nodes:
  - type: transform
    name: a
    input: src
    config:
      cxl: |
        emit w = v + 1.0
"#;
        let view = derive_composition_view(&parse_comp(yaml));
        // 1 input + 1 body + 1 output port, all drawn.
        assert_eq!(view.stages.len(), 3);
        let out_pos = view
            .stages
            .iter()
            .position(|s| s.kind == StageKind::OutputPort)
            .expect("output port drawn");
        assert!(
            !view.connections.iter().any(|&(_, to)| to == out_pos),
            "output whose ref names no body node is drawn but unconnected"
        );
    }
}

#[cfg(test)]
mod layout_tests {
    use super::*;
    use clinker_core::config::parse_config;

    /// X coordinate for a given column, matching `layout_positions`.
    fn col_x(col: usize) -> f32 {
        LEFT_MARGIN + (col as f32) * (NODE_WIDTH + NODE_GAP)
    }

    /// `layout_positions` assigns each node its column's X and stacks rows
    /// within a column at the fixed `NODE_HEIGHT + STACK_GAP` pitch.
    #[test]
    fn positions_follow_columns_and_even_pitch() {
        // Three nodes: one in column 0, two in column 1 (a fan-out).
        let cols = vec![0, 1, 1];
        let preds = vec![vec![], vec![0], vec![0]];
        let pos = layout_positions(&cols, &preds);

        assert_eq!(pos[0].0, col_x(0));
        assert_eq!(pos[1].0, col_x(1));
        assert_eq!(pos[2].0, col_x(1));

        // The two column-1 nodes are stacked one pitch apart.
        let pitch = (pos[2].1 - pos[1].1).abs();
        assert!(
            (pitch - (NODE_HEIGHT + STACK_GAP)).abs() < 0.01,
            "expected even pitch {}, got {pitch}",
            NODE_HEIGHT + STACK_GAP
        );
    }

    /// A single node's column is centered on `COLUMN_CENTER_Y` (top-left at
    /// center minus half the card height).
    #[test]
    fn single_node_centered_on_midline() {
        let pos = layout_positions(&[0], &[vec![]]);
        assert_eq!(pos.len(), 1);
        assert!((pos[0].1 - (COLUMN_CENTER_Y - NODE_HEIGHT / 2.0)).abs() < 0.01);
    }

    /// Barycenter ordering: when a downstream column's nodes connect to
    /// different upstream rows, they are ordered to match their predecessors'
    /// vertical order, reducing edge crossings.
    ///
    /// Layout: column 0 has [a, b] (rows 0, 1). Column 1 has two nodes whose
    /// declaration order is [feeds_b, feeds_a] — i.e. reversed relative to
    /// their inputs. The barycenter pass must reorder column 1 so the node
    /// fed by `a` (row 0) sits above the node fed by `b` (row 1).
    #[test]
    fn barycenter_reorders_to_match_predecessors() {
        // Indices: 0=a, 1=b (column 0); 2=feeds_b, 3=feeds_a (column 1).
        let cols = vec![0, 0, 1, 1];
        let preds = vec![vec![], vec![], vec![1], vec![0]];
        let pos = layout_positions(&cols, &preds);

        // Node 3 (fed by a, the top input) should end up above node 2.
        assert!(
            pos[3].1 < pos[2].1,
            "node fed by row-0 input should sit above node fed by row-1 input: \
             node3.y={}, node2.y={}",
            pos[3].1,
            pos[2].1
        );
    }

    /// Deterministic column assignment for a fan-in / fan-out DAG built from
    /// real config: two sources fan into a merge, which fans out to two
    /// outputs. Columns must be source=0, merge=1, outputs=2.
    #[test]
    fn fan_in_fan_out_columns_are_deterministic() {
        let yaml = r#"
pipeline:
  name: fan_layout
nodes:
  - type: source
    name: src_a
    config:
      name: src_a
      type: csv
      path: ./a.csv
      schema:
        - { name: x, type: string }
  - type: source
    name: src_b
    config:
      name: src_b
      type: csv
      path: ./b.csv
      schema:
        - { name: x, type: string }
  - type: merge
    name: joined
    inputs: [src_a, src_b]
  - type: output
    name: out_a
    input: joined
    config:
      name: out_a
      type: csv
      path: ./out_a.csv
  - type: output
    name: out_b
    input: joined
    config:
      name: out_b
      type: csv
      path: ./out_b.csv
"#;
        let config = parse_config(yaml).expect("fan-in/fan-out pipeline parses");
        let view = derive_pipeline_view(&config);

        let x_of = |id: &str| view.stages.iter().find(|s| s.id == id).unwrap().canvas_x;

        // Sources in column 0, merge in column 1, outputs in column 2.
        assert_eq!(x_of("src_a"), col_x(0));
        assert_eq!(x_of("src_b"), col_x(0));
        assert_eq!(x_of("joined"), col_x(1));
        assert_eq!(x_of("out_a"), col_x(2));
        assert_eq!(x_of("out_b"), col_x(2));

        // Connections: 2 fan-in edges + 2 fan-out edges.
        assert_eq!(view.connections.len(), 4);

        // Determinism: re-deriving yields identical positions.
        let view2 = derive_pipeline_view(&config);
        let pos1: Vec<_> = view
            .stages
            .iter()
            .map(|s| (s.canvas_x, s.canvas_y))
            .collect();
        let pos2: Vec<_> = view2
            .stages
            .iter()
            .map(|s| (s.canvas_x, s.canvas_y))
            .collect();
        assert_eq!(pos1, pos2);
    }

    /// `layout_bounds` unions every card rect; empty input yields `None`.
    #[test]
    fn bounds_union_and_empty() {
        assert_eq!(layout_bounds(&[]), None);

        let stages = vec![stage_at(0.0, 0.0), stage_at(300.0, 100.0)];
        let b = layout_bounds(&stages).expect("non-empty");
        assert_eq!(b.min_x, 0.0);
        assert_eq!(b.min_y, 0.0);
        assert_eq!(b.max_x, 300.0 + NODE_WIDTH);
        assert_eq!(b.max_y, 100.0 + NODE_HEIGHT);
    }

    fn stage_at(x: f32, y: f32) -> StageView {
        StageView {
            id: format!("{x}_{y}"),
            label: String::new(),
            kind: StageKind::Source,
            subtitle: String::new(),
            canvas_x: x,
            canvas_y: y,
            cxl_source: None,
            description: None,
            error_message: None,
        }
    }

    /// `fit_transform` centers the box and never exceeds the zoom clamp. A box
    /// far smaller than the viewport caps at `zoom_max`; one far larger caps at
    /// `zoom_min`.
    #[test]
    fn fit_centers_and_clamps_zoom() {
        let b = LayoutBounds {
            min_x: 100.0,
            min_y: 100.0,
            max_x: 300.0,
            max_y: 200.0,
        };
        let (px, py, z) = fit_transform(b, 1000.0, 700.0, 60.0, 0.25, 4.0);

        // Box center maps to viewport center: pan + center*zoom == viewport/2.
        let cx = (b.min_x + b.max_x) / 2.0;
        let cy = (b.min_y + b.max_y) / 2.0;
        assert!((px + cx * z - 500.0).abs() < 0.01);
        assert!((py + cy * z - 350.0).abs() < 0.01);
        assert!((0.25..=4.0).contains(&z));

        // Huge box clamps to zoom_min.
        let huge = LayoutBounds {
            min_x: 0.0,
            min_y: 0.0,
            max_x: 100_000.0,
            max_y: 100_000.0,
        };
        let (_, _, z_min) = fit_transform(huge, 1000.0, 700.0, 60.0, 0.25, 4.0);
        assert_eq!(z_min, 0.25);

        // Tiny box clamps to zoom_max.
        let tiny = LayoutBounds {
            min_x: 0.0,
            min_y: 0.0,
            max_x: 1.0,
            max_y: 1.0,
        };
        let (_, _, z_max) = fit_transform(tiny, 1000.0, 700.0, 60.0, 0.25, 4.0);
        assert_eq!(z_max, 4.0);
    }

    /// A degenerate (zero-size or inverted) viewport must not produce NaN/inf:
    /// extents are floored so the zoom stays finite and within the clamp.
    #[test]
    fn fit_handles_degenerate_viewport() {
        let b = LayoutBounds {
            min_x: 0.0,
            min_y: 0.0,
            max_x: 160.0,
            max_y: 92.0,
        };
        let (px, py, z) = fit_transform(b, 0.0, 0.0, 60.0, 0.25, 4.0);
        assert!(px.is_finite() && py.is_finite() && z.is_finite());
        assert!((0.25..=4.0).contains(&z));
    }
}
