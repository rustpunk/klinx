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

mod field_lineage;

pub use field_lineage::{FieldEdge, FieldKind, FieldRow, lineage_closure};

pub const NODE_HEIGHT: f32 = 92.0;
pub const NODE_WIDTH: f32 = 160.0;

/// Pixel height of a node card's header (badge + label + rust-line + subtitle)
/// above the first field row, measured from the card's border-box top.
///
/// WHY this exact value: the SVG field-lineage overlay draws cables to
/// world-space anchors at `field_anchor_*(i)`, while the DOM card is a flow box;
/// the two only coincide if the header occupies a *fixed* height. The matching
/// CSS rule is `.klinx-node-header { height: calc(FIELD_HEADER_HEIGHT - 3px) }`
/// (the 3px is `.klinx-node`'s `border-top-width`, which sits above the header
/// content inside the same border box), plus `overflow:hidden` so a long
/// subtitle clips instead of growing the header. A card with no fields is just
/// this header, so keeping it equal to [`NODE_HEIGHT`] also keeps the node-level
/// port geometry (`port_in`/`port_out` at `NODE_HEIGHT/2`) exact.
pub const FIELD_HEADER_HEIGHT: f32 = NODE_HEIGHT;

/// Pixel pitch of one field row in an expanded node card. The row's SVG anchor
/// sits at its vertical center, so anchor `y = canvas_y + header + i*ROW + ROW/2`.
///
/// WHY this exact value: it must equal `.klinx-node-field`'s CSS
/// `height: 22px` (with `box-sizing:border-box; overflow:hidden`) so each row's
/// rendered mid-line lands on `field_row_y(i)`. Changing one without the other
/// silently de-syncs the cables from the dots.
pub const FIELD_ROW_HEIGHT: f32 = 22.0;

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
    /// Per-field output rows for field-level lineage. Empty for nodes the
    /// field-lineage pass cannot give rows (an accept-any source/port, a node
    /// whose predecessors declare no schema, or the error/partial-degradation
    /// paths) — an empty `fields` keeps the card at its classic [`NODE_HEIGHT`]
    /// and leaves the node-level connector geometry (`port_in`/`port_out`)
    /// unchanged, so existing rendering is untouched. Both the composition
    /// canvas (#66) and the pipeline canvas (#68) populate this.
    pub fields: Vec<FieldRow>,
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

    /// World-space vertical center of field row `i` inside this card.
    ///
    /// Rows stack below the [`FIELD_HEADER_HEIGHT`] header at the fixed
    /// [`FIELD_ROW_HEIGHT`] pitch; the anchor sits at the row's mid-line.
    pub fn field_row_y(&self, i: usize) -> f32 {
        self.canvas_y + FIELD_HEADER_HEIGHT + i as f32 * FIELD_ROW_HEIGHT + FIELD_ROW_HEIGHT / 2.0
    }

    /// World-space coordinate of field row `i`'s LEFT (input) anchor dot.
    pub fn field_anchor_in(&self, i: usize) -> (f32, f32) {
        (self.canvas_x, self.field_row_y(i))
    }

    /// World-space coordinate of field row `i`'s RIGHT (output) anchor dot.
    pub fn field_anchor_out(&self, i: usize) -> (f32, f32) {
        (self.canvas_x + NODE_WIDTH, self.field_row_y(i))
    }

    /// Index of the field named `name` in this stage's output rows, if present.
    /// Used to resolve a `FieldEdge`'s endpoint to a row anchor.
    pub fn field_index(&self, name: &str) -> Option<usize> {
        self.fields.iter().position(|f| f.name == name)
    }

    /// Full world-space card height, growing with the field-row list. A card
    /// with no fields is exactly [`NODE_HEIGHT`] (the classic look), so this is
    /// safe to use uniformly in bounds/layout math.
    pub fn card_height(&self) -> f32 {
        if self.fields.is_empty() {
            NODE_HEIGHT
        } else {
            FIELD_HEADER_HEIGHT + self.fields.len() as f32 * FIELD_ROW_HEIGHT
        }
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

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PipelineView {
    pub stages: Vec<StageView>,
    /// Explicit connections between stages: `(from_idx, to_idx)`.
    pub connections: Vec<(usize, usize)>,
    /// Field-level lineage edges between stage rows. Populated by both the
    /// composition pass (#66) and the pipeline pass (#68); empty for views
    /// without resolvable field schemas (e.g. partial/degraded views), where an
    /// empty `field_edges` means the canvas draws node-level connectors only.
    pub field_edges: Vec<FieldEdge>,
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
/// are no stages. Each card spans [`NODE_WIDTH`] × [`StageView::card_height`]
/// from its top-left `(canvas_x, canvas_y)`; the box is the union of all card
/// rects. A card with field rows is taller than [`NODE_HEIGHT`], so the box must
/// read each card's own height for fit-to-view to frame the whole graph.
pub fn layout_bounds(stages: &[StageView]) -> Option<LayoutBounds> {
    let first = stages.first()?;
    let mut b = LayoutBounds {
        min_x: first.canvas_x,
        min_y: first.canvas_y,
        max_x: first.canvas_x + NODE_WIDTH,
        max_y: first.canvas_y + first.card_height(),
    };
    for s in &stages[1..] {
        b.min_x = b.min_x.min(s.canvas_x);
        b.min_y = b.min_y.min(s.canvas_y);
        b.max_x = b.max_x.max(s.canvas_x + NODE_WIDTH);
        b.max_y = b.max_y.max(s.canvas_y + s.card_height());
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
/// Spacing: within each column the ordered nodes are stacked by their own card
/// heights (so a tall field-bearing card never overlaps the next one) and the
/// whole column is centered on [`COLUMN_CENTER_Y`], so columns with different
/// node counts share a midline. The returned `Vec` is indexed by node index
/// (parallel to `cols`).
///
/// `heights[i]` is node `i`'s rendered card height — [`StageView::card_height`]
/// for field-bearing cards, [`NODE_HEIGHT`] for everything else. A node with no
/// field rows (an accept-any source, the drilled-in body path) passes
/// [`NODE_HEIGHT`], so a schema-less graph lays out byte-for-byte as
/// fixed-`NODE_HEIGHT` stacking did before field rows existed.
fn layout_positions(
    cols: &[usize],
    predecessors: &[Vec<usize>],
    heights: &[f32],
) -> Vec<(f32, f32)> {
    use std::collections::BTreeMap;

    let n = cols.len();
    debug_assert_eq!(n, predecessors.len());
    debug_assert_eq!(n, heights.len());
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

        // Stack each card below the previous one by that card's own height plus
        // the gap, so two field-bearing cards in one column have disjoint
        // y-extents. The column's total height is the sum of card heights plus
        // the gaps between them; centering keeps it on the shared midline.
        let count = ordered.len();
        let sum_h: f32 = ordered.iter().map(|&idx| heights[idx]).sum();
        let total_h = sum_h + (count.saturating_sub(1)) as f32 * STACK_GAP;
        let top = COLUMN_CENTER_Y - total_h / 2.0;
        let x = LEFT_MARGIN + (col as f32) * (NODE_WIDTH + NODE_GAP);
        let mut y = top;
        for (row, &idx) in ordered.iter().enumerate() {
            row_of[idx] = row;
            positions[idx] = (x, y);
            y += heights[idx] + STACK_GAP;
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
///
/// Field-level lineage (#68): Source nodes seed Declared origin fields from their
/// declared schema and every other node is analyzed as a transform, via the
/// shared [`compute_field_lineage`] core (the same core the composition path
/// uses; only the origin-field source differs).
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

    // Field-level lineage pass (#68): Source nodes seed Declared origin fields
    // from their declared `schema.columns`; every other node is analyzed as a
    // transform against its predecessors' output columns. Declaration order is
    // topological for the column DAG built above (a node references only
    // earlier-declared nodes), so the shared core reads final producer rows in
    // one sweep. `out_fields[i]` is parallel to `nodes`.
    let (out_fields, field_edges) = pipeline_field_lineage(nodes, &predecessors);

    // Per-node card heights drive the column stacking so a tall field-bearing
    // card never overlaps the next one. A node with field rows is `header +
    // n*row` tall (matching [`StageView::card_height`]); a row-less node keeps
    // the classic [`NODE_HEIGHT`], so a pipeline with no schemas lays out exactly
    // as fixed-height stacking did before this change.
    let heights: Vec<f32> = out_fields
        .iter()
        .map(|rows| {
            if rows.is_empty() {
                NODE_HEIGHT
            } else {
                FIELD_HEADER_HEIGHT + rows.len() as f32 * FIELD_ROW_HEIGHT
            }
        })
        .collect();
    let positions = layout_positions(&cols, &predecessors, &heights);
    let stages: Vec<StageView> = nodes
        .iter()
        .zip(positions)
        .enumerate()
        .map(|(i, (spanned, (x, y)))| {
            let mut stage = build_stage_view(&spanned.value, x, y);
            stage.fields = out_fields[i].clone();
            stage
        })
        .collect();

    PipelineView {
        stages,
        connections,
        field_edges,
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

    // ── Field-level lineage pass (#66) ───────────────────────────────────────
    // Predecessors are now fully known, so we compute each node's output field
    // rows and the per-field edges between them. `out_fields[u]` is the ordered
    // output record of unified index `u`. Body nodes are visited in declaration
    // order, which is a topological order over `predecessors` (a body node's
    // preds are only input ports or earlier body nodes), so every predecessor's
    // field set is final before its consumer reads it.
    let (out_fields, field_edges) =
        composition_field_lineage(sig, body, n_in, n_body, &predecessors);

    // Per-node card heights drive the stacking so tall field cards in one column
    // never overlap. A node with field rows is `header + n*row` tall (matching
    // [`StageView::card_height`]); a row-less node is the classic [`NODE_HEIGHT`].
    let heights: Vec<f32> = out_fields
        .iter()
        .map(|rows| {
            if rows.is_empty() {
                NODE_HEIGHT
            } else {
                FIELD_HEADER_HEIGHT + rows.len() as f32 * FIELD_ROW_HEIGHT
            }
        })
        .collect();
    let positions = layout_positions(&cols, &predecessors, &heights);

    let mut stages: Vec<StageView> = Vec::with_capacity(total);
    for (i, (port, decl)) in sig.inputs.iter().enumerate() {
        let (x, y) = positions[i];
        let mut stage = input_port_stage(port, decl, x, y);
        stage.fields = out_fields[i].clone();
        stages.push(stage);
    }
    for (bi, spanned) in body.iter().enumerate() {
        let (x, y) = positions[n_in + bi];
        let mut stage = build_stage_view(&spanned.value, x, y);
        stage.fields = out_fields[n_in + bi].clone();
        stages.push(stage);
    }
    for (oi, (port, alias)) in sig.outputs.iter().enumerate() {
        let (x, y) = positions[n_in + n_body + oi];
        let mut stage = output_port_stage(port, alias, x, y);
        stage.fields = out_fields[n_in + n_body + oi].clone();
        stages.push(stage);
    }

    PipelineView {
        stages,
        connections,
        field_edges,
    }
}

/// Source of the CXL program for a body node, if it carries one.
///
/// Only Transform / Aggregate / Combine bodies hold a `cxl:` block; every other
/// variant returns `None` and is treated as passthrough-only in Phase 1.
fn node_cxl(node: &PipelineNode) -> Option<&str> {
    match node {
        PipelineNode::Transform { config, .. } => Some(config.cxl.as_ref()),
        PipelineNode::Aggregate { config, .. } => Some(config.cxl.as_ref()),
        PipelineNode::Combine { config, .. } => Some(config.cxl.as_ref()),
        _ => None,
    }
}

/// Declared origin field rows from a source/port schema's columns.
///
/// Shared by both lineage entry points: a composition input port and a pipeline
/// Source node both declare their shape as `[{name, type}]` ([`SchemaDecl`]), and
/// both seed the lineage graph with [`FieldKind::Declared`] origin rows.
fn declared_rows(columns: &[clinker_core::config::pipeline_node::ColumnDecl]) -> Vec<FieldRow> {
    columns
        .iter()
        .map(|c| FieldRow {
            name: c.name.clone(),
            kind: FieldKind::Declared,
        })
        .collect()
}

/// Per-index classification fed to the shared lineage core: each slot is either
/// a fixed-shape origin (its output rows are pre-seeded and it runs no transform
/// logic) or a node to analyze as a transform.
enum LineageSlot<'a> {
    /// A boundary/origin slot with a pre-seeded output record:
    /// - composition **input port** → declared columns (empty when accept-any),
    /// - composition **output port** → one [`FieldKind::Declared`] row named for
    ///   the port (so the card draws a label + anchor; the producer edge is
    ///   appended by [`composition_field_lineage`] after the core runs),
    /// - pipeline **Source node** → declared `schema.columns`.
    ///
    /// Origins have no predecessor-derived columns and emit no edges of their
    /// own; downstream consumers read these rows as input columns.
    Origin(Vec<FieldRow>),
    /// A transform-like node analyzed against its predecessors' output columns:
    /// passthrough + emitted rows, with derive/identity edges.
    Node(&'a PipelineNode),
}

/// Compute per-index output field rows and the field-level lineage edges over a
/// classified slot list. **The shared lineage core** for both the composition
/// canvas ([`derive_composition_view`]) and the pipeline canvas
/// ([`derive_view_from_nodes`]).
///
/// `slots[u]` classifies index `u`; `predecessors[u]` is its predecessor index
/// list. The ONLY difference between the two callers is how origin fields are
/// sourced — composition input ports vs. pipeline Source-node schemas — which is
/// captured entirely by the [`LineageSlot::Origin`] rows the caller supplies.
/// Transform analysis (passthrough/emit rows, let-resolved derive edges,
/// intra-node chained-emit edges, identity carries, parse-error degradation) is
/// identical for both and lives here.
///
/// Returns `(out_fields, field_edges)` where `out_fields[u]` is the ordered
/// output record of index `u`. Slots MUST be ordered so every node appears after
/// its predecessors (topological): both callers pass declaration order, which is
/// topological for the DAGs they build.
///
/// Per-node rules (Phase 1, transforms-precise):
/// - **Origin slot**: its pre-seeded rows verbatim, no edges.
/// - **Node with parseable CXL**: passthrough rows for input columns not
///   shadowed by an emit, then emitted rows — see
///   [`field_lineage::transform_output_fields`]. Edges: each emit's let-resolved
///   support ∩ input columns yields derive edges; each surviving passthrough
///   column yields an identity edge.
/// - **Node without CXL / on parse error**: passthrough of its input columns; no
///   edges on parse error, identity carries when there is simply no CXL block.
fn compute_field_lineage(
    slots: &[LineageSlot<'_>],
    predecessors: &[Vec<usize>],
) -> (Vec<Vec<FieldRow>>, Vec<FieldEdge>) {
    let total = slots.len();
    debug_assert_eq!(total, predecessors.len());
    let mut out_fields: Vec<Vec<FieldRow>> = vec![Vec::new(); total];
    let mut field_edges: Vec<FieldEdge> = Vec::new();

    // First pass: seed every origin slot's fixed output record. Origins have no
    // predecessor-derived columns, so seeding them all up front (regardless of
    // index) lets the transform pass below read final producer rows in one
    // topological sweep.
    for (idx, slot) in slots.iter().enumerate() {
        if let LineageSlot::Origin(rows) = slot {
            out_fields[idx] = rows.clone();
        }
    }

    // Second pass: analyze transform nodes in slot order (topological).
    for (idx, slot) in slots.iter().enumerate() {
        let LineageSlot::Node(node) = slot else {
            continue;
        };

        // Ordered, de-duplicated union of predecessor output column names, with
        // the producer's index recorded per column so edges can name a concrete
        // source. First producer wins for a duplicated column name.
        let mut input_cols: Vec<String> = Vec::new();
        let mut producer_of: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        for &p in &predecessors[idx] {
            for row in &out_fields[p] {
                if !producer_of.contains_key(&row.name) {
                    producer_of.insert(row.name.clone(), p);
                    input_cols.push(row.name.clone());
                }
            }
        }

        // Three cases, deliberately distinct:
        //  - no CXL at all (Source/Route/Output/… in P1): passthrough rows +
        //    safe identity carries — the node forwards its input columns.
        //  - CXL present but unparseable: passthrough rows, NO edges — we will
        //    not infer lineage from a partial/garbled AST (per the spec's
        //    "no edges for it" rule on parse error).
        //  - CXL parses: full transform-precise rows + derive/identity edges.
        let cxl = node_cxl(node);
        match cxl.map(field_lineage::parse_clean) {
            Some(Some(program)) => {
                // Output rows: passthrough (unshadowed inputs) then emitted.
                out_fields[idx] = field_lineage::transform_output_fields(&input_cols, &program);

                // Per-emit let-resolved support, in emit order. The order is
                // load-bearing for intra-node chained emits below.
                let supports = field_lineage::emit_supports(&program);
                let emitted: std::collections::HashSet<&str> =
                    supports.iter().map(|(name, _)| name.as_str()).collect();

                // Derive edges: each emit's let-resolved support resolved to its
                // producer. A support column is one of:
                //  - a predecessor output column → INTER-node derive edge, OR
                //  - a field this same node emitted in an EARLIER statement →
                //    INTRA-node derive edge (`emit c = b + 1.0` after `emit b`),
                //    so chained emits form a same-card cable the hover closure
                //    walks transitively. `emitted_so_far` tracks the prior emits
                //    in declaration order; we record the current target only
                //    after its own edges, so an emit never reads itself.
                // A support column that is neither resolves to nothing (it names
                // no producible field — e.g. an accept-any input).
                let mut emitted_so_far: std::collections::HashSet<&str> =
                    std::collections::HashSet::new();
                for (target, support) in &supports {
                    for col in support {
                        if let Some(&p) = producer_of.get(col) {
                            field_edges.push(FieldEdge {
                                from_node: p,
                                from_field: col.clone(),
                                to_node: idx,
                                to_field: target.clone(),
                                passthrough: false,
                            });
                        } else if emitted_so_far.contains(col.as_str()) {
                            field_edges.push(FieldEdge {
                                from_node: idx,
                                from_field: col.clone(),
                                to_node: idx,
                                to_field: target.clone(),
                                passthrough: false,
                            });
                        }
                    }
                    emitted_so_far.insert(target.as_str());
                }

                // Identity edges: each input column carried through unchanged
                // (not shadowed by an emit of the same name).
                for col in &input_cols {
                    if !emitted.contains(col.as_str())
                        && let Some(&p) = producer_of.get(col)
                    {
                        field_edges.push(FieldEdge {
                            from_node: p,
                            from_field: col.clone(),
                            to_node: idx,
                            to_field: col.clone(),
                            passthrough: true,
                        });
                    }
                }
            }
            Some(None) => {
                // CXL present but it failed to parse: render passthrough rows so
                // the card still shows its shape, but emit NO lineage edges — a
                // garbled AST can't be trusted to compute lineage.
                out_fields[idx] = field_lineage::passthrough_output_fields(&input_cols);
            }
            None => {
                // No CXL block at all: the node forwards its input columns. Both
                // the rows and the identity carries are safe to draw.
                out_fields[idx] = field_lineage::passthrough_output_fields(&input_cols);
                for col in &input_cols {
                    if let Some(&p) = producer_of.get(col) {
                        field_edges.push(FieldEdge {
                            from_node: p,
                            from_field: col.clone(),
                            to_node: idx,
                            to_field: col.clone(),
                            passthrough: true,
                        });
                    }
                }
            }
        }
    }

    (out_fields, field_edges)
}

/// Build the composition's classified slot list and run the shared lineage core.
///
/// Slot layout mirrors the unified index space: `[0, n_in)` input ports,
/// `[n_in, n_in+n_body)` body nodes, `[n_in+n_body, total)` output ports. Input
/// ports become [`LineageSlot::Origin`] from their declared schema (empty when
/// accept-any); body nodes become [`LineageSlot::Node`]; output ports become
/// single-row origins (one [`FieldKind::Declared`] row = the port name) so each
/// renders a labelled, anchored field instead of a blank card. `predecessors` is
/// the unified-index relation the caller already built.
///
/// After the shared core runs, one field edge per output port is appended from
/// its producer's representative field to the port's field. The producer is the
/// body node the caller already resolved as the port's predecessor (from the
/// output's `internal_ref`); a port with no resolved producer (a dangling
/// `internal_ref`) is skipped, never panicked on.
fn composition_field_lineage(
    sig: &clinker_core::config::composition::CompositionSignature,
    body: &[Spanned<PipelineNode>],
    n_in: usize,
    n_body: usize,
    predecessors: &[Vec<usize>],
) -> (Vec<Vec<FieldRow>>, Vec<FieldEdge>) {
    let total = n_in + n_body + sig.outputs.len();
    let mut slots: Vec<LineageSlot<'_>> = Vec::with_capacity(total);

    // Input ports: declared columns are origin rows. Accept-any (`schema: None`)
    // ports are empty origins — their downstream consumers simply have no
    // resolvable producer columns from them.
    for decl in sig.inputs.values() {
        let rows = decl.schema.as_ref().map(|s| declared_rows(&s.columns));
        slots.push(LineageSlot::Origin(rows.unwrap_or_default()));
    }
    // Body nodes analyzed as transforms.
    for spanned in body {
        slots.push(LineageSlot::Node(&spanned.value));
    }
    // Output ports: one Declared row named for the port, so the card draws a
    // label + anchor the producer edge lands on (rather than a blank boundary).
    for port in sig.outputs.keys() {
        slots.push(LineageSlot::Origin(vec![FieldRow {
            name: port.to_string(),
            kind: FieldKind::Declared,
        }]));
    }

    let (out_fields, mut field_edges) = compute_field_lineage(&slots, predecessors);

    // Producer → output-port field edge, one per output port. The output port
    // surfaces its producer body node's records; on the canvas we draw a single
    // cable from the producer's representative field to the port's named field.
    // The representative field is the producer's same-named column if it has one
    // (a clean 1:1 carry → passthrough), else the producer's LAST output field (a
    // rename → derive). A port with no resolved producer, or a producer with no
    // fields, contributes no edge (graceful, never a panic).
    for (oi, port) in sig.outputs.keys().enumerate() {
        let out_idx = n_in + n_body + oi;
        let Some(&producer) = predecessors[out_idx].first() else {
            continue;
        };
        let producer_field = out_fields[producer]
            .iter()
            .find(|r| r.name == *port)
            .or_else(|| out_fields[producer].last())
            .map(|r| r.name.clone());
        let Some(producer_field) = producer_field else {
            continue;
        };
        let passthrough = producer_field == *port;
        field_edges.push(FieldEdge {
            from_node: producer,
            from_field: producer_field,
            to_node: out_idx,
            to_field: port.to_string(),
            passthrough,
        });
    }

    (out_fields, field_edges)
}

/// Build a pipeline's classified slot list and run the shared lineage core.
///
/// Slots are parallel to `nodes` (declaration order). A [`PipelineNode::Source`]
/// becomes a [`LineageSlot::Origin`] seeded from its declared `schema.columns`
/// (the pipeline analogue of a composition input port); every other node becomes
/// a [`LineageSlot::Node`] analyzed as a transform. `predecessors` is the
/// node-index relation the caller already built.
fn pipeline_field_lineage(
    nodes: &[Spanned<PipelineNode>],
    predecessors: &[Vec<usize>],
) -> (Vec<Vec<FieldRow>>, Vec<FieldEdge>) {
    let slots: Vec<LineageSlot<'_>> = nodes
        .iter()
        .map(|spanned| match &spanned.value {
            PipelineNode::Source { config: body, .. } => {
                LineageSlot::Origin(declared_rows(&body.schema.columns))
            }
            node => LineageSlot::Node(node),
        })
        .collect();

    compute_field_lineage(&slots, predecessors)
}

/// Synthetic boundary node for a composition input port.
fn input_port_stage(name: &str, decl: &PortDecl, x: f32, y: f32) -> StageView {
    StageView {
        // `port:` prefix with a colon — invalid in body-node identifiers — so a
        // port id can never collide with a body node's id (its raw name), which
        // is the RSX key and selection identity.
        id: format!("port:in:{name}"),
        label: name.to_string(),
        kind: StageKind::InputPort,
        // No subtitle: the declared columns now render as field ROWS on the card,
        // so a "N fields" count line is redundant chrome. `decl` stays in the
        // signature for the description and parity with `output_port_stage`.
        subtitle: String::new(),
        canvas_x: x,
        canvas_y: y,
        cxl_source: None,
        description: decl.description.clone(),
        error_message: None,
        fields: Vec::new(),
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
        fields: Vec::new(),
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
            fields: Vec::new(),
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
                fields: Vec::new(),
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
                fields: Vec::new(),
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
                fields: Vec::new(),
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
            fields: Vec::new(),
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
                fields: Vec::new(),
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
            fields: Vec::new(),
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
            fields: Vec::new(),
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
                    fields: Vec::new(),
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
                    fields: Vec::new(),
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
                    fields: Vec::new(),
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
                    fields: Vec::new(),
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
                    fields: Vec::new(),
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
                    fields: Vec::new(),
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
        field_edges: Vec::new(),
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
            fields: Vec::new(),
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

    // Drilled-in body nodes carry no field rows, so every card is [`NODE_HEIGHT`].
    let heights = vec![NODE_HEIGHT; stages.len()];
    let positions = layout_positions(&slot_cols, &predecessors, &heights);
    for (stage, (x, y)) in stages.iter_mut().zip(positions) {
        stage.canvas_x = x;
        stage.canvas_y = y;
    }

    PipelineView {
        stages,
        connections,
        field_edges: Vec::new(),
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

    /// The shipped `order_fulfillment.yaml` example parses and models routing
    /// with clinker's dedicated `route` node — not a transform emitting a
    /// synthetic `_route` column (#74). Guards the example against rot and
    /// verifies the migration: `route_priority` derives to a Route stage whose
    /// subtitle counts the one condition branch plus the default, and both
    /// outputs connect to it.
    #[test]
    fn order_fulfillment_example_uses_route_node() {
        let yaml = include_str!("../../../examples/pipelines/order_fulfillment.yaml");
        let config = parse_config(yaml).expect("order_fulfillment.yaml parses");
        let view = derive_pipeline_view(&config);

        let route_idx = view
            .stages
            .iter()
            .position(|s| s.id == "route_priority")
            .expect("route_priority stage exists");
        let route = &view.stages[route_idx];
        assert_eq!(route.kind, StageKind::Route);
        // One condition branch (`priority_report`) + the `default` branch.
        assert_eq!(route.subtitle, "1 branch \u{2192} fulfilled_orders");

        // No synthetic `_route` field leaks onto the card (Route has no cxl).
        assert!(route.fields.iter().all(|f| f.name != "_route"));

        // Both `output` nodes consume the route node's branches.
        let from_route = view
            .connections
            .iter()
            .filter(|(from, _)| *from == route_idx)
            .count();
        assert_eq!(from_route, 2, "both outputs connect to the route node");
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

    /// Index of a stage by label (panics if absent — tests assert presence).
    fn stage_idx(view: &PipelineView, label: &str) -> usize {
        view.stages
            .iter()
            .position(|s| s.label == label)
            .unwrap_or_else(|| panic!("stage {label} present"))
    }

    /// FIX 3 (#68/#70): the PIPELINE canvas now carries field rows + lineage.
    /// A `source` with declared schema [a, b] feeding a transform `emit c = a+1`
    /// must produce: source fields [a, b] Declared; transform fields
    /// [a, b passthrough, c emitted]; and a derive edge source.a → transform.c.
    /// (Pipeline Source schema is the analogue of a composition input port.)
    #[test]
    fn pipeline_fields_and_lineage_from_source_schema() {
        let yaml = r#"
pipeline:
  name: src_schema_lineage
nodes:
  - type: source
    name: src
    config:
      name: src
      type: csv
      path: ./in.csv
      schema:
        - { name: a, type: int }
        - { name: b, type: string }
  - type: transform
    name: t
    input: src
    config:
      cxl: |
        emit c = a + 1
"#;
        let config = parse_config(yaml).expect("source-schema pipeline parses");
        let view = derive_pipeline_view(&config);

        let i_src = stage_idx(&view, "src");
        let i_t = stage_idx(&view, "t");

        // Source: its two declared columns as Declared origin rows (in order).
        assert_eq!(
            view.stages[i_src].fields,
            vec![
                FieldRow {
                    name: "a".to_string(),
                    kind: FieldKind::Declared
                },
                FieldRow {
                    name: "b".to_string(),
                    kind: FieldKind::Declared
                },
            ]
        );

        // Transform: `a` shadowed? No — `c` is emitted, so a & b ride through as
        // passthrough (input order), then emitted `c`.
        assert_eq!(
            view.stages[i_t].fields,
            vec![
                FieldRow {
                    name: "a".to_string(),
                    kind: FieldKind::PassThrough
                },
                FieldRow {
                    name: "b".to_string(),
                    kind: FieldKind::PassThrough
                },
                FieldRow {
                    name: "c".to_string(),
                    kind: FieldKind::Emitted
                },
            ]
        );

        // Derive edge: c is computed from the source column a.
        let derive_a_c = FieldEdge {
            from_node: i_src,
            from_field: "a".to_string(),
            to_node: i_t,
            to_field: "c".to_string(),
            passthrough: false,
        };
        assert!(
            view.field_edges.contains(&derive_a_c),
            "expected derive edge src.a → t.c, got {:?}",
            view.field_edges
        );

        // The pipeline path now populates field lineage (regression guard for
        // the pre-#68 behavior where these were always empty).
        assert!(
            view.stages.iter().any(|s| !s.fields.is_empty()),
            "pipeline view must now carry field rows"
        );
        assert!(
            !view.field_edges.is_empty(),
            "pipeline view must now carry field edges"
        );

        // Hovering the computed column `c` reveals only its derive lineage
        // (src.a → t.c), never the passthrough carries of a or b — FIX 1.
        let closure = lineage_closure(&view.field_edges, i_t, "c");
        let derive_idx = view
            .field_edges
            .iter()
            .position(|e| *e == derive_a_c)
            .expect("derive edge present");
        assert_eq!(
            closure,
            std::collections::HashSet::from([derive_idx]),
            "c's closure is exactly the a→c derive edge, no carries"
        );
    }

    /// Full field-rows + lineage-edges pass over the canonical chain:
    /// input port {a:float} → t1 `emit b = a*2` → t2 `emit c = b + a` → output.
    /// Asserts the EXACT field set per node and the EXACT edge set.
    #[test]
    fn composition_fields_and_lineage() {
        let yaml = r#"_compose:
  name: chain
  inputs:
    src:
      schema:
        - { name: a, type: float }
  outputs:
    result: t2
  config_schema: {}

nodes:
  - type: transform
    name: t1
    input: src
    config:
      cxl: |
        emit b = a * 2.0
  - type: transform
    name: t2
    input: t1
    config:
      cxl: |
        emit c = b + a
"#;
        let view = derive_composition_view(&parse_comp(yaml));

        let i_src = stage_idx(&view, "src");
        let i_t1 = stage_idx(&view, "t1");
        let i_t2 = stage_idx(&view, "t2");
        let i_out = stage_idx(&view, "result");

        // Input port: the one declared column, kind Declared.
        assert_eq!(
            view.stages[i_src].fields,
            vec![FieldRow {
                name: "a".to_string(),
                kind: FieldKind::Declared
            }]
        );

        // t1: passthrough `a` (not shadowed) then emitted `b`.
        assert_eq!(
            view.stages[i_t1].fields,
            vec![
                FieldRow {
                    name: "a".to_string(),
                    kind: FieldKind::PassThrough
                },
                FieldRow {
                    name: "b".to_string(),
                    kind: FieldKind::Emitted
                },
            ]
        );

        // t2: passthrough `a`, `b` (input order) then emitted `c`.
        assert_eq!(
            view.stages[i_t2].fields,
            vec![
                FieldRow {
                    name: "a".to_string(),
                    kind: FieldKind::PassThrough
                },
                FieldRow {
                    name: "b".to_string(),
                    kind: FieldKind::PassThrough
                },
                FieldRow {
                    name: "c".to_string(),
                    kind: FieldKind::Emitted
                },
            ]
        );

        // Output port now carries one Declared row named for the port (FIX E),
        // so the card draws a label + anchor instead of a blank boundary.
        assert_eq!(
            view.stages[i_out].fields,
            vec![FieldRow {
                name: "result".to_string(),
                kind: FieldKind::Declared
            }]
        );

        // Exact edge set. Build the expected set and compare as multisets.
        let derive = |fn_: usize, ff: &str, tn: usize, tf: &str| FieldEdge {
            from_node: fn_,
            from_field: ff.to_string(),
            to_node: tn,
            to_field: tf.to_string(),
            passthrough: false,
        };
        let pass = |fn_: usize, ff: &str, tn: usize, tf: &str| FieldEdge {
            from_node: fn_,
            from_field: ff.to_string(),
            to_node: tn,
            to_field: tf.to_string(),
            passthrough: true,
        };
        let expected = [
            // t1: b derives from a; a carries through.
            derive(i_src, "a", i_t1, "b"),
            pass(i_src, "a", i_t1, "a"),
            // t2: c derives from b and a; a, b carry through.
            derive(i_t1, "b", i_t2, "c"),
            derive(i_t1, "a", i_t2, "c"),
            pass(i_t1, "a", i_t2, "a"),
            pass(i_t1, "b", i_t2, "b"),
            // Producer → output port (FIX E): the port name `result` matches no
            // t2 field, so its representative field is t2's last field `c` — a
            // rename, hence a derive edge t2.c → result.
            derive(i_t2, "c", i_out, "result"),
        ];
        assert_eq!(
            view.field_edges.len(),
            expected.len(),
            "exact edge count; got {:?}",
            view.field_edges
        );
        for e in &expected {
            assert!(
                view.field_edges.contains(e),
                "missing expected field edge {e:?} in {:?}",
                view.field_edges
            );
        }
    }

    /// Intra-node chained emits: a later emit reading an EARLIER emit's output
    /// of the SAME node gets a same-node derive edge. Here `emit b = a*2` then
    /// `emit c = b + 1` produces both the inter-node edge a→b and the intra-node
    /// edge b→c (both on node `t`). Hovering `c` reveals its DIRECT (1-hop)
    /// neighbourhood — the incoming b→c derive and the outgoing edge to the
    /// output port — but NOT the 2-hop a→b derive (FIX C: direct, not transitive).
    #[test]
    fn intra_node_chained_emit_lineage() {
        let yaml = r#"_compose:
  name: chained
  inputs:
    src:
      schema:
        - { name: a, type: float }
  outputs:
    result: t
  config_schema: {}

nodes:
  - type: transform
    name: t
    input: src
    config:
      cxl: |
        emit b = a * 2.0
        emit c = b + 1.0
"#;
        let view = derive_composition_view(&parse_comp(yaml));
        let i_src = stage_idx(&view, "src");
        let i_t = stage_idx(&view, "t");

        // Inter-node derive: `b` reads input column `a` from the source.
        let a_to_b = FieldEdge {
            from_node: i_src,
            from_field: "a".to_string(),
            to_node: i_t,
            to_field: "b".to_string(),
            passthrough: false,
        };
        // Intra-node derive: `c` reads `b`, an EARLIER emit of the SAME node `t`.
        let b_to_c = FieldEdge {
            from_node: i_t,
            from_field: "b".to_string(),
            to_node: i_t,
            to_field: "c".to_string(),
            passthrough: false,
        };
        assert!(
            view.field_edges.contains(&a_to_b),
            "inter-node a→b derive edge missing: {:?}",
            view.field_edges
        );
        assert!(
            view.field_edges.contains(&b_to_c),
            "intra-node b→c derive edge missing: {:?}",
            view.field_edges
        );
        // `c`'s support is the column `b`, not the let-free input `a` directly —
        // so there must be NO direct a→c edge (the chain runs a→b→c).
        let a_to_c = FieldEdge {
            from_node: i_src,
            from_field: "a".to_string(),
            to_node: i_t,
            to_field: "c".to_string(),
            passthrough: false,
        };
        assert!(
            !view.field_edges.contains(&a_to_c),
            "c must chain through b, not derive from a directly: {:?}",
            view.field_edges
        );

        // Hover closure on `c` is its DIRECT (1-hop) neighbourhood: the incoming
        // intra-node derive b→c, plus the outgoing producer→output edge c→result
        // (FIX E). The 2-hop a→b derive is NOT pulled in — the closure is direct,
        // not transitive (FIX C).
        let i_out = stage_idx(&view, "result");
        let c_to_result = FieldEdge {
            from_node: i_t,
            from_field: "c".to_string(),
            to_node: i_out,
            to_field: "result".to_string(),
            passthrough: false,
        };
        let closure = lineage_closure(&view.field_edges, i_t, "c");
        let idx_of = |e: &FieldEdge| view.field_edges.iter().position(|x| x == e).unwrap();
        assert!(
            closure.contains(&idx_of(&b_to_c)),
            "closure of c must contain its incoming b→c derive: {closure:?}"
        );
        assert!(
            closure.contains(&idx_of(&c_to_result)),
            "closure of c must contain its outgoing c→result edge: {closure:?}"
        );
        assert!(
            !closure.contains(&idx_of(&a_to_b)),
            "closure of c must NOT contain the 2-hop a→b derive (1-hop only): {closure:?}"
        );
    }

    /// `let` chains resolve to base input columns: `let w = a + 1.0; emit y = w
    /// * 2.0` makes `y` derive from `a`, NOT from `w`. The composition view must
    /// therefore draw a derive edge from the input column `a`, never from `w`.
    #[test]
    fn let_chain_resolution() {
        let yaml = r#"_compose:
  name: lets
  inputs:
    src:
      schema:
        - { name: a, type: float }
  outputs:
    result: t
  config_schema: {}

nodes:
  - type: transform
    name: t
    input: src
    config:
      cxl: |
        let w = a + 1.0
        emit y = w * 2.0
"#;
        let view = derive_composition_view(&parse_comp(yaml));
        let i_src = stage_idx(&view, "src");
        let i_t = stage_idx(&view, "t");

        // y's lineage points at the input column `a` (let `w` is resolved away).
        let derive_a_y = FieldEdge {
            from_node: i_src,
            from_field: "a".to_string(),
            to_node: i_t,
            to_field: "y".to_string(),
            passthrough: false,
        };
        assert!(
            view.field_edges.contains(&derive_a_y),
            "y must derive from input column a, got {:?}",
            view.field_edges
        );
        // No edge may name `w` (a let, not a column) at either endpoint.
        assert!(
            !view
                .field_edges
                .iter()
                .any(|e| e.from_field == "w" || e.to_field == "w"),
            "no field edge may reference the let name `w`: {:?}",
            view.field_edges
        );
    }

    /// A body node with invalid CXL still renders its fields (passthrough of its
    /// input columns) and produces NO lineage edges for that node — never a
    /// panic.
    #[test]
    fn lineage_skipped_on_parse_error() {
        let yaml = r#"_compose:
  name: broken
  inputs:
    src:
      schema:
        - { name: a, type: float }
  outputs: {}
  config_schema: {}

nodes:
  - type: transform
    name: bad
    input: src
    config:
      cxl: |
        emit y = a +
"#;
        let view = derive_composition_view(&parse_comp(yaml));
        let _i_src = stage_idx(&view, "src");
        let i_bad = stage_idx(&view, "bad");

        // Fields still render: the input column carried through as passthrough.
        assert_eq!(
            view.stages[i_bad].fields,
            vec![FieldRow {
                name: "a".to_string(),
                kind: FieldKind::PassThrough
            }]
        );
        // No field edge — derive OR carry — terminates at the parse-error node:
        // a garbled CXL AST is never trusted to compute any lineage.
        assert!(
            view.field_edges.iter().all(|e| e.to_node != i_bad),
            "a parse-error node yields no field edges at all: {:?}",
            view.field_edges
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
        let heights = vec![NODE_HEIGHT; 3];
        let pos = layout_positions(&cols, &preds, &heights);

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
        let pos = layout_positions(&[0], &[vec![]], &[NODE_HEIGHT]);
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
        let heights = vec![NODE_HEIGHT; 4];
        let pos = layout_positions(&cols, &preds, &heights);

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

    /// Two field-bearing cards in the SAME column must not overlap: their
    /// world-space y-extents `[canvas_y, canvas_y + card_height)` are disjoint.
    /// Stacking by each card's own `card_height` (not a fixed `NODE_HEIGHT`) is
    /// what guarantees this once a card grows with its field-row list.
    #[test]
    fn field_cards_in_one_column_do_not_overlap() {
        use clinker_core::config::composition::CompositionFile;
        use clinker_core::span::FileId;
        use std::num::NonZeroU32;

        // One input port fans out to two independent transforms, so both land
        // in column 1. `wide` emits three fields (tall card); `narrow` emits
        // one (short card) — distinct heights exercise per-card stacking.
        let yaml = r#"_compose:
  name: fanout
  inputs:
    src:
      schema:
        - { name: a, type: float }
  outputs: {}
  config_schema: {}

nodes:
  - type: transform
    name: wide
    input: src
    config:
      cxl: |
        emit p = a + 1.0
        emit q = a + 2.0
        emit r = a + 3.0
  - type: transform
    name: narrow
    input: src
    config:
      cxl: |
        emit s = a * 2.0
"#;
        let comp = CompositionFile::parse(
            yaml,
            FileId::new(NonZeroU32::new(1).expect("nonzero")),
            std::path::PathBuf::new(),
        )
        .expect("composition parses");
        let view = derive_composition_view(&comp);

        let stage = |label: &str| {
            view.stages
                .iter()
                .find(|s| s.label == label)
                .unwrap_or_else(|| panic!("stage {label} present"))
        };
        let wide = stage("wide");
        let narrow = stage("narrow");

        // Both fanned-out transforms share column 1 (same x).
        assert_eq!(wide.canvas_x, narrow.canvas_x, "both cards in one column");

        // The taller card carries more rows, so the heights genuinely differ —
        // a fixed-NODE_HEIGHT stack would have collided the tall card into the
        // next one. (wide: a passthrough + p,q,r = 4 rows; narrow: a + s = 2.)
        assert!(wide.card_height() > narrow.card_height());

        // Disjoint y-extents: the upper card's bottom is at or above the lower
        // card's top. Order is unknown (barycenter ties), so test both ways.
        let (top, bottom) = if wide.canvas_y <= narrow.canvas_y {
            (wide, narrow)
        } else {
            (narrow, wide)
        };
        assert!(
            top.canvas_y + top.card_height() <= bottom.canvas_y,
            "cards overlap: top [{}, {}) intersects bottom [{}, {})",
            top.canvas_y,
            top.canvas_y + top.card_height(),
            bottom.canvas_y,
            bottom.canvas_y + bottom.card_height(),
        );
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
            fields: Vec::new(),
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
