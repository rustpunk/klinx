/// Derives canvas-renderable stage data from a `PipelineConfig`.
///
/// The canvas dispatches on `PipelineNode` variants directly. Every variant —
/// Source, Transform, Aggregate, Route, Merge, Output, Composition — maps 1:1
/// to a [`StageKind`] via an exhaustive `match` in [`stage_kind_for_node`], so
/// adding a new variant to `PipelineNode` is a compile-time break. Composition
/// currently renders as a placeholder badge pending full sub-canvas rendering.
use clinker_plan::config::composition::{CompositionFile, OutputAlias, PortDecl};
use clinker_plan::config::node_header::NodeInput;
use clinker_plan::config::{PipelineConfig, PipelineNode};
use clinker_plan::yaml::Spanned;

mod field_lineage;

pub use field_lineage::{FieldEdge, FieldKind, FieldRow, group_endpoints_by_node, lineage_closure};

pub const NODE_HEIGHT: f32 = 92.0;
pub const NODE_WIDTH: f32 = 160.0;

/// Vertical offset (px from the card's border-box top) of the node-level
/// input/output ports, placed on the **node-name label's** mid-line rather than
/// the header's geometric center — so the port squares read as centered on the
/// node name. Derived from the header CSS box model: 3px card border + 10px
/// header padding-top + ~12px badge row + 7px badge margin + half the ~14px
/// label line ≈ 39px. Both the cable anchors ([`StageView::port_in`]/`port_out`)
/// and the rendered port squares use this, so they coincide; keep it in step
/// with the `.klinx-node-header` / `.klinx-node-label` CSS.
pub const HEADER_PORT_Y: f32 = 39.0;

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
    /// Per-correlation-group record synthesis (clinker `Reshape`). Single
    /// output whose schema differs from the input (synthesized rows + audit
    /// columns), so it is NOT a passthrough.
    Reshape,
    /// Per-correlation-group record removal (clinker `Cull`). Schema-preserving
    /// on both ports; carries a `removed_to` side-output branch port in
    /// addition to its main output.
    Cull,
    /// Body-record framing into documents (clinker `Envelope`). Single output
    /// whose framed-document shape differs from the input.
    Envelope,
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
            StageKind::Reshape => "reshape",
            StageKind::Cull => "cull",
            StageKind::Envelope => "envelope",
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
            StageKind::Reshape => "RESHAPE",
            StageKind::Cull => "CULL",
            StageKind::Envelope => "ENVELOPE",
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
        // The three per-group operators each get their own first-class kind so
        // the canvas labels, accents, and ports them distinctly (#80). No `_`
        // arm: a future `PipelineNode` variant still forces a decision here.
        PipelineNode::Reshape { .. } => StageKind::Reshape,
        PipelineNode::Cull { .. } => StageKind::Cull,
        PipelineNode::Envelope { .. } => StageKind::Envelope,
    }
}

fn node_input_name(ni: &NodeInput) -> &str {
    match ni {
        NodeInput::Single(s) => s.as_str(),
        NodeInput::Port { node, .. } => node.as_str(),
    }
}

/// The branch (port) a `node.port` reference selects, or `None` for a bare
/// `node` reference. For a reference to a Route node this is the branch name.
fn node_input_port(ni: &NodeInput) -> Option<&str> {
    match ni {
        NodeInput::Single(_) => None,
        NodeInput::Port { port, .. } => Some(port.as_str()),
    }
}

/// A node's extra named output ports, rendered below the field rows.
///
/// - **Route**: every condition branch in declaration order, then the
///   always-present `default` fallback. A Route node's outputs ARE these ports
///   (it has no separate node-level output port).
/// - **Cull**: exactly one entry — the `removed_to` side-output port carrying
///   the removed groups' records. Cull ALSO keeps its node-level main output, so
///   this is in addition to it (see [`StageView::keeps_node_output_port`]).
///
/// Empty for every other node.
fn output_branches(node: &PipelineNode) -> Vec<RouteBranch> {
    match node {
        PipelineNode::Route { config: body, .. } => {
            let mut branches: Vec<RouteBranch> = body
                .conditions
                .iter()
                .map(|(name, predicate)| RouteBranch {
                    name: name.clone(),
                    predicate: Some(predicate.as_ref().to_string()),
                    is_default: false,
                })
                .collect();
            // The default/fallback is a first-class output port, distinct from
            // the predicate branches and always present.
            branches.push(RouteBranch {
                name: body.default.clone(),
                predicate: None,
                is_default: true,
            });
            branches
        }
        // Cull's `removed_to` is a producer-side side-output port: downstream
        // nodes consume the removed groups as `<cull>.<removed_to>`, so it must
        // be an individually-addressable port (the same seam Route branches use).
        // It is NOT a default fallback — the main output is the node-level port.
        PipelineNode::Cull { config: body, .. } => vec![RouteBranch {
            name: body.removed_to.clone(),
            predicate: None,
            is_default: false,
        }],
        _ => Vec::new(),
    }
}

/// Index of the branch a `node.port` reference selects in the source's branch
/// list, or `None` for a bare reference or a source with no branches.
fn resolve_from_branch(
    ni: &NodeInput,
    from: usize,
    node_branches: &[Vec<RouteBranch>],
) -> Option<usize> {
    let port = node_input_port(ni)?;
    node_branches[from].iter().position(|b| b.name == port)
}

/// Append one resolved input edge: looks up the source by name, records the
/// source branch (if it leaves a Route via `route.branch`), and extends the
/// predecessor relation. A reference that names no declared node is skipped.
fn push_input_edge(
    ni: &NodeInput,
    to: usize,
    name_to_idx: &std::collections::HashMap<String, usize>,
    node_branches: &[Vec<RouteBranch>],
    connections: &mut Vec<Connection>,
    predecessors: &mut [Vec<usize>],
) {
    if let Some(&from) = name_to_idx.get(node_input_name(ni)) {
        let from_branch = resolve_from_branch(ni, from, node_branches);
        connections.push(Connection {
            from,
            to,
            from_branch,
        });
        predecessors[to].push(from);
    }
}

/// Card height for a stack of `rows` field+branch rows below the header — the
/// free-function form of [`StageView::card_height`], used during layout before a
/// `StageView` exists. `0` rows keeps the classic [`NODE_HEIGHT`].
fn row_stack_height(rows: usize) -> f32 {
    if rows == 0 {
        NODE_HEIGHT
    } else {
        FIELD_HEADER_HEIGHT + rows as f32 * FIELD_ROW_HEIGHT
    }
}

/// One output branch of a [`StageKind::Route`] node: a named output port the
/// route forwards matching records to. `predicate` is the branch's CXL condition
/// (`None` for the always-present `default`/fallback branch). Downstream nodes
/// consume a specific branch via `route_name.branch` — so each branch is an
/// individually-addressable port, not "just another rule" (#77).
#[derive(Clone, Debug, PartialEq)]
pub struct RouteBranch {
    pub name: String,
    /// The branch's CXL boolean predicate; `None` for the default/fallback.
    pub predicate: Option<String>,
    /// The default/fallback branch — always present, rendered distinctly.
    pub is_default: bool,
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
    /// Extra named output ports rendered below the field rows. For a Route node
    /// these are its condition branches (declaration order) then the `default`
    /// branch; for a Cull node it is the single `removed_to` side-output. Empty
    /// for every other node. Each downstream edge that consumes `producer.port`
    /// anchors at the matching port (see [`StageView::branch_anchor_out`]).
    ///
    /// Whether the node ALSO keeps its node-level output port depends on the
    /// kind: a Route's outputs ARE these ports, but a Cull keeps its main output
    /// — see [`StageView::keeps_node_output_port`].
    pub branches: Vec<RouteBranch>,
}

impl StageView {
    /// Node-level OUTPUT port — the default cable origin. Anchored on the
    /// node-name label's mid-line ([`HEADER_PORT_Y`] from the card top), inline
    /// with the name, so node→node cables connect at the header regardless of how
    /// many field rows a card carries. Per-column field cables use the per-row
    /// anchors ([`StageView::field_anchor_out`]); a Route node's OUTPUTS are its
    /// branch ports ([`StageView::branch_anchor_out`]), not this port.
    pub fn port_out(&self) -> (f32, f32) {
        (self.canvas_x + NODE_WIDTH, self.canvas_y + HEADER_PORT_Y)
    }

    /// Node-level INPUT port — the header-level entry point, inline with the node
    /// name; see [`StageView::port_out`].
    pub fn port_in(&self) -> (f32, f32) {
        (self.canvas_x, self.canvas_y + HEADER_PORT_Y)
    }

    /// Whether this node renders a node-level OUTPUT port in addition to any
    /// branch ports it carries.
    ///
    /// A Route node's outputs ARE its branch ports, so it has no separate
    /// node-level output port. Every other node — including a Cull, whose
    /// `removed_to` is a *side*-output alongside its main output — keeps the
    /// node-level port. A node with no branches trivially keeps it.
    pub fn keeps_node_output_port(&self) -> bool {
        !matches!(self.kind, StageKind::Route)
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

    /// World-space vertical center of branch-port row `i`. Branch ports stack
    /// BELOW the field rows at the same [`FIELD_ROW_HEIGHT`] pitch, so branch row
    /// `i` occupies field-row slot `fields.len() + i`.
    pub fn branch_row_y(&self, i: usize) -> f32 {
        self.canvas_y
            + FIELD_HEADER_HEIGHT
            + (self.fields.len() + i) as f32 * FIELD_ROW_HEIGHT
            + FIELD_ROW_HEIGHT / 2.0
    }

    /// World-space coordinate of branch-port row `i`'s RIGHT (output) anchor —
    /// the point a downstream edge that consumes that branch attaches to.
    pub fn branch_anchor_out(&self, i: usize) -> (f32, f32) {
        (self.canvas_x + NODE_WIDTH, self.branch_row_y(i))
    }

    /// Full world-space card height. Field rows and branch ports both stack
    /// below the header at the [`FIELD_ROW_HEIGHT`] pitch, so the card grows with
    /// their combined count. A card with neither is exactly [`NODE_HEIGHT`] (the
    /// classic look), so this is safe to use uniformly in bounds/layout math.
    pub fn card_height(&self) -> f32 {
        let rows = self.fields.len() + self.branches.len();
        if rows == 0 {
            NODE_HEIGHT
        } else {
            FIELD_HEADER_HEIGHT + rows as f32 * FIELD_ROW_HEIGHT
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

/// A node-level connection between two stages, optionally leaving a specific
/// Route branch.
#[derive(Clone, Debug, PartialEq)]
pub struct Connection {
    pub from: usize,
    pub to: usize,
    /// When the edge leaves a Route node via a `route.branch` reference, the
    /// index of that branch in the source stage's [`StageView::branches`]. `None`
    /// for ordinary edges; lets the canvas anchor the cable at the branch port
    /// rather than the shared node-level port.
    pub from_branch: Option<usize>,
}

impl Connection {
    /// An ordinary (non-branch) connection between two stage indices.
    fn plain(from: usize, to: usize) -> Self {
        Self {
            from,
            to,
            from_branch: None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PipelineView {
    pub stages: Vec<StageView>,
    /// Explicit connections between stages. A connection that leaves a Route
    /// node records the source branch it originates from (`from_branch`).
    pub connections: Vec<Connection>,
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
            PipelineNode::Reshape { header, .. }
            | PipelineNode::Cull { header, .. }
            | PipelineNode::Transform { header, .. }
            | PipelineNode::Aggregate { header, .. }
            | PipelineNode::Route { header, .. }
            | PipelineNode::Output { header, .. }
            | PipelineNode::Composition { header, .. } => name_to_idx
                .get(node_input_name(&header.input.value))
                .copied()
                .map(|i| cols[i] + 1)
                .unwrap_or(1),
            // Envelope's primary input is `body`; its header/trailer ports are
            // accepted in config but not wired this engine release.
            PipelineNode::Envelope { header, .. } => name_to_idx
                .get(node_input_name(&header.body.value))
                .copied()
                .map(|i| cols[i] + 1)
                .unwrap_or(1),
        };
        cols.push(col);
    }

    // Per-node extra output ports: a Route's condition+default branches, or a
    // Cull's `removed_to` side-output; empty for every other node. Computed
    // before connections so an edge that consumes `producer.port` can resolve its
    // source-port index, and before layout so the ports' height is reserved
    // alongside the field rows.
    let node_branches: Vec<Vec<RouteBranch>> =
        nodes.iter().map(|s| output_branches(&s.value)).collect();

    // Connections: resolve each consumer's input header reference. These also
    // form the predecessor relation the barycenter layout pass consumes. An edge
    // that leaves a Route via `route.branch` records that branch (`from_branch`).
    let mut connections: Vec<Connection> = Vec::new();
    let mut predecessors: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
    for (idx, spanned) in nodes.iter().enumerate() {
        match &spanned.value {
            PipelineNode::Source { .. } => {}
            PipelineNode::Merge { header, .. } => {
                for ni in &header.inputs {
                    push_input_edge(
                        &ni.value,
                        idx,
                        &name_to_idx,
                        &node_branches,
                        &mut connections,
                        &mut predecessors,
                    );
                }
            }
            PipelineNode::Combine { header, .. } => {
                for ni in header.input.values() {
                    push_input_edge(
                        &ni.value,
                        idx,
                        &name_to_idx,
                        &node_branches,
                        &mut connections,
                        &mut predecessors,
                    );
                }
            }
            PipelineNode::Reshape { header, .. }
            | PipelineNode::Cull { header, .. }
            | PipelineNode::Transform { header, .. }
            | PipelineNode::Aggregate { header, .. }
            | PipelineNode::Route { header, .. }
            | PipelineNode::Output { header, .. }
            | PipelineNode::Composition { header, .. } => {
                push_input_edge(
                    &header.input.value,
                    idx,
                    &name_to_idx,
                    &node_branches,
                    &mut connections,
                    &mut predecessors,
                );
            }
            // Envelope frames its `body` input; header/trailer ports unwired.
            PipelineNode::Envelope { header, .. } => {
                push_input_edge(
                    &header.body.value,
                    idx,
                    &name_to_idx,
                    &node_branches,
                    &mut connections,
                    &mut predecessors,
                );
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
        .zip(&node_branches)
        .map(|(rows, branches)| row_stack_height(rows.len() + branches.len()))
        .collect();
    let positions = layout_positions(&cols, &predecessors, &heights);
    let stages: Vec<StageView> = nodes
        .iter()
        .zip(positions)
        .enumerate()
        .map(|(i, (spanned, (x, y)))| {
            let mut stage = build_stage_view(&spanned.value, x, y);
            stage.fields = out_fields[i].clone();
            stage.branches = node_branches[i].clone();
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
    let mut connections: Vec<Connection> = Vec::new();
    let mut predecessors: Vec<Vec<usize>> = vec![Vec::new(); total];

    // Extra output ports per UNIFIED index: body slots carry their Route
    // branches (condition + default) or Cull `removed_to` side-output;
    // input/output ports carry none. Sized over the whole index space so a
    // `producer.port` edge can resolve its source-port index, and so layout
    // reserves the ports' height.
    let mut node_branches: Vec<Vec<RouteBranch>> = vec![Vec::new(); total];
    for (bi, spanned) in body.iter().enumerate() {
        node_branches[n_in + bi] = output_branches(&spanned.value);
    }

    // Resolve a body input reference to `(unified_index, source_branch_index)`;
    // the branch index is set only when the reference is `producer.branch` and
    // the producer is a Route.
    let resolve_edge =
        |ni: &NodeInput, body_idx: &HashMap<&str, usize>| -> Option<(usize, Option<usize>)> {
            let p = resolve(node_input_name(ni), body_idx)?;
            let branch = node_input_port(ni)
                .and_then(|port| node_branches[p].iter().position(|b| b.name == port));
            Some((p, branch))
        };

    for (bi, spanned) in body.iter().enumerate() {
        let idx = n_in + bi;
        let node = &spanned.value;

        // Resolved predecessors with the source branch (if any): input ports +
        // upstream body nodes.
        let mut preds: Vec<(usize, Option<usize>)> = Vec::new();
        match node {
            PipelineNode::Source { .. } => {}
            PipelineNode::Merge { header, .. } => {
                for ni in &header.inputs {
                    if let Some(e) = resolve_edge(&ni.value, &body_idx) {
                        preds.push(e);
                    }
                }
            }
            PipelineNode::Combine { header, .. } => {
                for ni in header.input.values() {
                    if let Some(e) = resolve_edge(&ni.value, &body_idx) {
                        preds.push(e);
                    }
                }
            }
            PipelineNode::Reshape { header, .. }
            | PipelineNode::Cull { header, .. }
            | PipelineNode::Transform { header, .. }
            | PipelineNode::Aggregate { header, .. }
            | PipelineNode::Route { header, .. }
            | PipelineNode::Output { header, .. }
            | PipelineNode::Composition { header, .. } => {
                if let Some(e) = resolve_edge(&header.input.value, &body_idx) {
                    preds.push(e);
                }
            }
            PipelineNode::Envelope { header, .. } => {
                if let Some(e) = resolve_edge(&header.body.value, &body_idx) {
                    preds.push(e);
                }
            }
        }

        // Column = 1 + max predecessor column; a Source anchors at 0, an
        // unresolved reference at 1. Every `p` is a port or an earlier body
        // node, so `cols[p]` is always already set.
        cols[idx] = if matches!(node, PipelineNode::Source { .. }) {
            0
        } else {
            preds.iter().map(|&(p, _)| cols[p] + 1).max().unwrap_or(1)
        };
        for &(p, from_branch) in &preds {
            connections.push(Connection {
                from: p,
                to: idx,
                from_branch,
            });
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
            // The producer→output-port edge surfaces a body node's records; it
            // is not a Route branch selection (the `internal_ref` channel
            // semantics differ), so it carries no `from_branch`.
            connections.push(Connection::plain(from, out_idx));
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
        .zip(&node_branches)
        .map(|(rows, branches)| row_stack_height(rows.len() + branches.len()))
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
        stage.branches = node_branches[n_in + bi].clone();
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

/// Whether a CXL-less node forwards its input columns unchanged, so drawing
/// passthrough field rows from its predecessors is faithful.
///
/// This gates the field-lineage pass's no-CXL branch: a node with no `cxl:`
/// would otherwise be drawn as a pure passthrough of its input columns. That is
/// honest for schema-preserving operators (Route, Output, and crucially Cull —
/// which removes whole groups but never alters columns), but MISLEADING for
/// operators whose output shape differs from the input:
///
/// - **Reshape** synthesizes new rows and appends `$meta.*` audit columns, and
///   may mutate trigger rows — its output schema is not the input schema.
/// - **Envelope** frames body records into documents — its framed-document
///   output shape differs from the body input.
///
/// For those two we have no resolvable output schema at this layer, so we emit
/// NO field rows rather than the wrong ones (an empty card, classic
/// [`NODE_HEIGHT`], no field cables). Every other CXL-less node (Route, Output,
/// Cull, Composition) keeps its existing passthrough-row treatment — schema-
/// preserving for the first three, and unchanged for Composition.
fn node_preserves_input_schema(node: &PipelineNode) -> bool {
    !matches!(
        node,
        PipelineNode::Reshape { .. } | PipelineNode::Envelope { .. }
    )
}

/// Declared origin field rows from a source/port schema's columns.
///
/// Shared by both lineage entry points: a composition input port and a pipeline
/// Source node both declare their shape as `[{name, type}]` ([`SchemaDecl`]), and
/// both seed the lineage graph with [`FieldKind::Declared`] origin rows. Each
/// row carries its declared datatype ([`ColumnDecl::ty`]) as a compact label
/// (#73); these origin types then propagate to downstream passthrough rows.
///
/// `correlation_key` is the slot's optional [`CorrelationKey`](clinker_plan::config::CorrelationKey)
/// (#88); each declared column whose name is one of the key's driver fields
/// ([`CorrelationKey::fields`] — one for `Single`, N for `Compound`) is flagged
/// [`FieldRow::is_correlation_key`]. Pass `None` for slots with no correlation
/// key (composition input ports never declare one). Matching on the
/// user-declared `schema:` column name is what restricts the marker to driver
/// columns — the engine's internal `$ck.<field>` shadow columns are never part
/// of `schema.columns`, so they cannot be marked here.
fn declared_rows(
    columns: &[clinker_plan::config::pipeline_node::ColumnDecl],
    correlation_key: Option<&clinker_plan::config::CorrelationKey>,
) -> Vec<FieldRow> {
    let ck_fields: std::collections::HashSet<&str> = correlation_key
        .map(|ck| ck.fields().into_iter().collect())
        .unwrap_or_default();
    columns
        .iter()
        .map(|c| FieldRow {
            name: c.name.clone(),
            kind: FieldKind::Declared,
            ty: Some(compact_type(&c.ty)),
            is_correlation_key: ck_fields.contains(c.name.as_str()),
        })
        .collect()
}

/// A short, lowercase datatype label for inline display on a field row, e.g.
/// `float`, `string`, `datetime`, and `int?` for `Nullable(Int)`. The engine's
/// `Display`/`display_name` are unsuitable: `display_name` drops the inner type
/// of `Nullable`, and `Display` renders the verbose `Nullable(Int)` form.
fn compact_type(ty: &cxl::typecheck::Type) -> String {
    use cxl::typecheck::Type;
    match ty {
        Type::Nullable(inner) => format!("{}?", compact_type(inner)),
        Type::Null => "null".to_string(),
        Type::Bool => "bool".to_string(),
        Type::Int => "int".to_string(),
        Type::Float => "float".to_string(),
        Type::String => "string".to_string(),
        Type::Date => "date".to_string(),
        Type::DateTime => "datetime".to_string(),
        Type::Array => "array".to_string(),
        Type::Map => "map".to_string(),
        Type::Numeric => "numeric".to_string(),
        Type::Any => "any".to_string(),
    }
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

/// Carried per-column metadata folded across all producers of an input column
/// while [`compute_field_lineage`] analyzes one node, then stamped onto the
/// node's carried [`FieldKind::PassThrough`] rows.
///
/// The two facets combine DIFFERENTLY across a multi-input fan-in, which is why
/// they share one record built in a single pass rather than two parallel maps:
/// - `ty` (#73) is **first-producer-wins** — a shared column draws one row, so
///   it shows the first input's type label (edges still fan to every producer,
///   so a type disagreement never drops the column).
/// - `is_ck` (#88) is **OR across producers** — a correlation-key role is a
///   boolean identity, so a column carried from a fan-in is a CK driver if ANY
///   contributing input declared it one.
#[derive(Default)]
struct ColMeta {
    /// Compact datatype label carried from the first producer (#73).
    ty: Option<String>,
    /// Whether any producer marked this column a correlation-key driver (#88).
    is_ck: bool,
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
///
/// Multi-input fan-in (Merge/Combine): a column present in more than one
/// predecessor (a join key carried from both joined inputs) emits a carry edge
/// to EACH contributing input, so hovering it lights up every source rather than
/// only the first-seen one (#67).
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
        // EVERY producer index recorded per column so a column present in more
        // than one input (a Merge/Combine fan-in, e.g. a join key like
        // `product_code` that lives in both joined inputs) emits a carry edge to
        // EACH contributing input rather than dimming all but the first on hover
        // (#67). `input_cols` keeps each name once, in first-seen order, so the
        // node still draws one row per column.
        let mut input_cols: Vec<String> = Vec::new();
        let mut producers_of: std::collections::HashMap<String, Vec<usize>> =
            std::collections::HashMap::new();
        // Per-column carried metadata, folded across all producers of the column
        // in a single pass (#73 datatype + #88 correlation-key role). The two
        // fields combine DIFFERENTLY across a fan-in (see [`ColMeta`]): `ty` is
        // first-producer-wins while `is_ck` ORs across every producer.
        let mut col_meta: std::collections::HashMap<String, ColMeta> =
            std::collections::HashMap::new();
        for &p in &predecessors[idx] {
            for row in &out_fields[p] {
                let producers = producers_of.entry(row.name.clone()).or_default();
                let meta = col_meta.entry(row.name.clone()).or_default();
                if producers.is_empty() {
                    // First sighting fixes the column's draw order and type label
                    // (#73: a shared column shows the first input's type; edges
                    // still fan to both, so a type clash never drops the column).
                    meta.ty = row.ty.clone();
                    input_cols.push(row.name.clone());
                }
                // The correlation-key role is a boolean identity: a column shared
                // by a fan-in IS a CK driver if ANY producer marks it one, so OR
                // across every producer rather than trusting the first (#88, #2).
                meta.is_ck |= row.is_correlation_key;
                // Guard against a single malformed input listing the column twice
                // so we never emit two carries to the same producer.
                if !producers.contains(&p) {
                    producers.push(p);
                }
            }
        }

        // Schema-changing CXL-less operators (Reshape, Envelope) get NO field
        // rows: their output shape differs from the input, so the passthrough
        // rows the no-CXL branch below would draw are MISLEADING (they would show
        // the input columns verbatim as if carried through). With no resolvable
        // output schema at this layer we leave the card empty (classic
        // [`NODE_HEIGHT`], no field cables) rather than draw a wrong shape. A
        // schema-changing node that DOES carry CXL (none today, but future-proof)
        // still falls through to the precise emit analysis.
        if node_cxl(node).is_none() && !node_preserves_input_schema(node) {
            continue;
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
                // Emits that merely re-emit an input column unchanged
                // (`emit c = c` / `emit c = src.c`) are passthroughs, not derives,
                // so their edge reads as an identity carry.
                let copies = field_lineage::emit_copy_targets(&program, &input_cols);

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
                    // An identity-copy emit carries its column unchanged, so its
                    // edge is a passthrough; a computed emit derives its target.
                    let carried = copies.contains(target.as_str());
                    for col in support {
                        if let Some(producers) = producers_of.get(col) {
                            // A support column present in several inputs derives
                            // its target from each of them (#67).
                            for &p in producers {
                                field_edges.push(FieldEdge {
                                    from_node: p,
                                    from_field: col.clone(),
                                    to_node: idx,
                                    to_field: target.clone(),
                                    passthrough: carried,
                                });
                            }
                        } else if emitted_so_far.contains(col.as_str()) {
                            field_edges.push(FieldEdge {
                                from_node: idx,
                                from_field: col.clone(),
                                to_node: idx,
                                to_field: target.clone(),
                                passthrough: carried,
                            });
                        }
                    }
                    emitted_so_far.insert(target.as_str());
                }

                // Identity edges: each input column carried through unchanged
                // (not shadowed by an emit of the same name). A column carried in
                // from several inputs (a fan-in join key) carries from each (#67).
                for col in &input_cols {
                    if !emitted.contains(col.as_str())
                        && let Some(producers) = producers_of.get(col)
                    {
                        for &p in producers {
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
                    // A CXL-less fan-in (Merge) carries a shared column from every
                    // input that produced it (#67).
                    if let Some(producers) = producers_of.get(col) {
                        for &p in producers {
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

        // Propagate carried-column datatypes AND the correlation-key marker onto
        // this node's passthrough rows from their producers (#73, #88), one
        // `col_meta` lookup per row. Only PassThrough rows carry: a Declared row
        // already holds its own metadata, and an Emitted row is a NEW identity —
        // its value was (re)computed here, so even when it shadows a CK source
        // column it is no longer that column's correlation-key driver and stays
        // unmarked/untyped (typing an emit needs the engine typechecker, Phase
        // 2b / #68).
        for row in out_fields[idx].iter_mut() {
            if matches!(row.kind, FieldKind::PassThrough)
                && let Some(meta) = col_meta.get(&row.name)
            {
                row.ty = meta.ty.clone();
                row.is_correlation_key = meta.is_ck;
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
    sig: &clinker_plan::config::composition::CompositionSignature,
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
        // Composition input ports declare a shape but never a correlation key
        // (that is a Source-only concept), so no driver columns are marked.
        let rows = decl
            .schema
            .as_ref()
            .map(|s| declared_rows(&s.columns, None));
        slots.push(LineageSlot::Origin(rows.unwrap_or_default()));
    }
    // Body nodes analyzed as transforms. No CK marking happens here (#88):
    // `correlation_key` is a Source-only concept and a composition is fed by its
    // `signature.inputs` (input ports), never by a body `source` node — the
    // engine's composition body model has no ingest path (its Source handling
    // lives only in the top-level executor), and no composition fixture declares
    // a body source. So a body Source carrying a CK is not reachable; the marker
    // is seeded exclusively at the pipeline Source path in `pipeline_field_lineage`.
    for spanned in body {
        slots.push(LineageSlot::Node(&spanned.value));
    }
    // Output ports: one Declared row named for the port, so the card draws a
    // label + anchor the producer edge lands on (rather than a blank boundary).
    for port in sig.outputs.keys() {
        slots.push(LineageSlot::Origin(vec![FieldRow {
            name: port.to_string(),
            kind: FieldKind::Declared,
            // The output-port placeholder row has no declared schema type.
            ty: None,
            // An output port is a synthetic boundary label, not a source
            // column, so it never drives a correlation key.
            is_correlation_key: false,
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
                // Mark the source columns named in `correlation_key` as CK
                // drivers (#88). The flag then propagates onto downstream
                // carried passthrough rows in `compute_field_lineage`.
                LineageSlot::Origin(declared_rows(
                    &body.schema.columns,
                    body.correlation_key.as_ref(),
                ))
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
        branches: Vec::new(),
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
        branches: Vec::new(),
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
            branches: Vec::new(),
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
                branches: Vec::new(),
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
                branches: Vec::new(),
            }
        }
        PipelineNode::Route {
            header,
            config: body,
        } => {
            // Total output branches = every condition branch PLUS the always-present
            // default/fallback branch. The default is a first-class branch (a real
            // output port downstream nodes consume), not "just another rule", so it
            // counts toward the branch total — otherwise a route with one condition
            // and a default reads as "1 branch" when it actually fans out to two.
            let branch_count = body.conditions.len() + 1;
            let subtitle = format!(
                "{} branch{} → {}",
                branch_count,
                if branch_count == 1 { "" } else { "es" },
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
                branches: Vec::new(),
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
            branches: Vec::new(),
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
                branches: Vec::new(),
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
            branches: Vec::new(),
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
            branches: Vec::new(),
        },
        // Reshape: per-group synthesis. Subtitle names the partition key (the
        // grouping that every rule observes) and the rule count, the two facts
        // that read at a glance. Output schema differs from input, so its field
        // rows are suppressed by the lineage pass (see
        // [`node_preserves_input_schema`]); `branches` stays empty (single output).
        PipelineNode::Reshape {
            header,
            config: body,
        } => StageView {
            id: header.name.clone(),
            label: header.name.clone(),
            kind,
            subtitle: group_rules_subtitle(&body.partition_by, body.rules.len()),
            canvas_x: x,
            canvas_y: y,
            cxl_source: None,
            description: header.description.clone(),
            error_message: None,
            fields: Vec::new(),
            branches: Vec::new(),
        },
        // Cull: per-group removal. Subtitle mirrors Reshape (partition key + rule
        // count). Schema-preserving, so its passthrough field rows are honest and
        // come from the lineage pass; the `removed_to` side-output is surfaced as
        // a branch port assigned by `output_branches` in the view builders.
        PipelineNode::Cull {
            header,
            config: body,
        } => StageView {
            id: header.name.clone(),
            label: header.name.clone(),
            kind,
            subtitle: group_rules_subtitle(&body.partition_by, body.rules.len()),
            canvas_x: x,
            canvas_y: y,
            cxl_source: None,
            description: header.description.clone(),
            error_message: None,
            fields: Vec::new(),
            branches: Vec::new(),
        },
        // Envelope: frames body records into documents. Subtitle names the
        // framing strategy. Output shape differs from input, so its field rows
        // are suppressed by the lineage pass; single output (no branches).
        PipelineNode::Envelope {
            header,
            config: body,
        } => StageView {
            id: header.name.clone(),
            label: header.name.clone(),
            kind,
            subtitle: format!("frame: {}", envelope_strategy_name(&body.strategy)),
            canvas_x: x,
            canvas_y: y,
            cxl_source: None,
            description: header.description.clone(),
            error_message: None,
            fields: Vec::new(),
            branches: Vec::new(),
        },
    }
}

/// Card subtitle shared by Reshape and Cull: the partition key plus the rule
/// count — the two facts that read at a glance for a per-correlation-group
/// operator. With no `partition_by` fields (structurally valid but unusual) it
/// degrades to just the rule count.
fn group_rules_subtitle(partition_by: &[String], rule_count: usize) -> String {
    let rules = format!(
        "{rule_count} rule{}",
        if rule_count == 1 { "" } else { "s" }
    );
    if partition_by.is_empty() {
        rules
    } else {
        format!("by {} · {rules}", partition_by.join(", "))
    }
}

/// Stable lowercase name of an [`EnvelopeStrategy`] for display.
fn envelope_strategy_name(
    strategy: &clinker_plan::config::pipeline_node::EnvelopeStrategy,
) -> &'static str {
    use clinker_plan::config::pipeline_node::EnvelopeStrategy;
    match strategy {
        EnvelopeStrategy::Preserve => "preserve",
        EnvelopeStrategy::Concat => "concat",
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
) -> Vec<Connection> {
    let mut conns = Vec::new();
    let t_base = input_count;

    if transform_count > 0 {
        for (i, &target) in input_targets.iter().enumerate() {
            let target_stage = t_base + target.min(transform_count - 1);
            conns.push(Connection::plain(i, target_stage));
        }
        for i in 0..transform_count - 1 {
            conns.push(Connection::plain(t_base + i, t_base + i + 1));
        }
        let last_t = t_base + transform_count - 1;
        let o_base = t_base + transform_count;
        for j in 0..output_count {
            conns.push(Connection::plain(last_t, o_base + j));
        }
    } else {
        let o_base = input_count;
        for i in 0..input_count {
            for j in 0..output_count {
                conns.push(Connection::plain(i, o_base + j));
            }
        }
    }

    conns
}

/// Derive canvas nodes from a `PartialPipelineConfig` (graceful degradation).
pub fn derive_partial_pipeline_view(
    partial: &clinker_exec::partial::PartialPipelineConfig,
) -> PipelineView {
    use clinker_exec::partial::PartialItem;

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
                    branches: Vec::new(),
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
                    branches: Vec::new(),
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
                    branches: Vec::new(),
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
                    branches: Vec::new(),
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
                    branches: Vec::new(),
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
                    branches: Vec::new(),
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
pub fn derive_body_view(body: &clinker_plan::plan::composition_body::BoundBody) -> PipelineView {
    use clinker_plan::plan::execution::PlanNode;
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
            // Per-group operators inside a drilled-in composition body. Each gets
            // its own stage kind so the body view labels/accents them as the
            // top-level canvas does. The compiled `PlanNode` DOES carry the
            // config (`PlanNode::Reshape/Cull { config }`,
            // `PlanNode::Envelope { strategy }`), but this secondary canvas keeps
            // a minimal operator-name subtitle by design — the rich per-config
            // subtitle is a top-level-canvas affordance, mirroring the other plan
            // arms here (Route/Aggregate/Combine) that likewise summarize tersely.
            PlanNode::Reshape { name, .. } => (name.clone(), StageKind::Reshape, "reshape".into()),
            PlanNode::Cull { name, .. } => (name.clone(), StageKind::Cull, "cull".into()),
            PlanNode::Envelope { name, .. } => {
                (name.clone(), StageKind::Envelope, "envelope".into())
            }
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
            branches: Vec::new(),
        });
    }

    // Walk every edge in the mini-DAG and translate it to a slot pair. Stages
    // were pushed in topo order, so every edge's source and target are already
    // in `idx_to_slot`. The same pairs feed both the connector overlay and the
    // layout pass's predecessor relation.
    let mut connections: Vec<Connection> = Vec::new();
    let mut predecessors: Vec<Vec<usize>> = vec![Vec::new(); stages.len()];
    for e in body.graph.edge_references() {
        if let (Some(from), Some(to)) = (
            idx_to_slot.get(&e.source()).copied(),
            idx_to_slot.get(&e.target()).copied(),
        ) {
            // The drilled-in body view is built from the compiled plan's graph,
            // which does not carry per-branch port identity here; route branch
            // tagging in this view is deferred (#77 follow-up).
            connections.push(Connection::plain(from, to));
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
    use clinker_plan::config::parse_config;

    /// Variant dispatch is guarded at compile time: [`stage_kind_for_node`]
    /// has no `_` arm, so adding a `PipelineNode` variant without mapping it is
    /// a build error. This test exercises the dispatch at RUNTIME over a fixture
    /// that now INCLUDES the #80 per-group / framing operators
    /// (Reshape/Cull/Envelope), asserting each derives to its OWN distinct
    /// `StageKind` (not the generic `Transform` fallback #78 left behind).
    #[test]
    fn test_canvas_node_dispatches_on_variant() {
        // A minimal unified-shape YAML exercising the single-shape variants
        // (Source/Transform/Aggregate/Route/Merge/Output) plus the three new
        // first-class kinds. Config bodies use the minimal valid shapes from
        // clinker-plan's `config::pipeline_node` (`ReshapeBody`/`CullBody`/
        // `EnvelopeBody`): Reshape needs `partition_by` + `rules`; Cull adds the
        // required `removed_to`; Envelope needs its `body` input + a (possibly
        // empty) `config`.
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
  - type: reshape
    name: synth
    input: joined
    config:
      partition_by: [x]
      rules:
        - name: fill
          when: "x > 0"
          synthesize:
            copy_from: trigger
  - type: cull
    name: prune
    input: synth
    config:
      partition_by: [x]
      removed_to: dropped
      rules:
        - name: drop_small
          drop_group_when: "count(*) < 2"
  - type: envelope
    name: frame
    body: prune
    config:
      strategy: preserve
  - type: output
    name: out
    input: frame
    config:
      name: out
      type: csv
      path: ./out.csv
"#;
        let config = parse_config(yaml).expect("parse unified-shape pipeline");
        let view = derive_pipeline_view(&config);

        // Each declared node produces exactly one stage.
        assert_eq!(view.stages.len(), 9);

        // Every variant kind is represented — including the three new ones, each
        // mapped to its OWN kind (the #78 fallback would have folded all three
        // into `Transform`).
        let kind_of = |id: &str| {
            view.stages
                .iter()
                .find(|s| s.id == id)
                .map(|s| s.kind.clone())
                .unwrap_or_else(|| panic!("stage {id} present"))
        };
        assert_eq!(kind_of("src"), StageKind::Source);
        assert_eq!(kind_of("clean"), StageKind::Transform);
        assert_eq!(kind_of("agg"), StageKind::Aggregate);
        assert_eq!(kind_of("split"), StageKind::Route);
        assert_eq!(kind_of("joined"), StageKind::Merge);
        assert_eq!(kind_of("synth"), StageKind::Reshape);
        assert_eq!(kind_of("prune"), StageKind::Cull);
        assert_eq!(kind_of("frame"), StageKind::Envelope);
        assert_eq!(kind_of("out"), StageKind::Output);
    }

    /// A pipeline with a Cull node surfaces its `removed_to` as a first-class
    /// side-output branch port, keeps its node-level MAIN output port, and routes
    /// a downstream consumer of `<cull>.<removed_to>` to that branch (so the cable
    /// anchors at the side-output, not the main output). Cull is schema-
    /// preserving, so its field rows are honest passthroughs of the input columns.
    #[test]
    fn cull_surfaces_removed_to_side_output() {
        let yaml = r#"
pipeline:
  name: cull_side_output
nodes:
  - type: source
    name: src
    config:
      name: src
      type: csv
      path: ./in.csv
      schema:
        - { name: gid, type: string }
        - { name: amount, type: int }
  - type: cull
    name: prune
    input: src
    config:
      partition_by: [gid]
      removed_to: dropped
      rules:
        - name: drop_small
          drop_group_when: "count(*) < 2"
  - type: output
    name: kept
    input: prune
    config:
      name: kept
      type: csv
      path: ./kept.csv
  - type: output
    name: removed
    input: prune.dropped
    config:
      name: removed
      type: csv
      path: ./removed.csv
"#;
        let config = parse_config(yaml).expect("cull pipeline parses");
        let view = derive_pipeline_view(&config);

        let cull_idx = stage_idx(&view, "prune");
        let cull = &view.stages[cull_idx];
        assert_eq!(cull.kind, StageKind::Cull);

        // Exactly one branch port — the `removed_to` side-output, not a default.
        assert_eq!(cull.branches.len(), 1);
        assert_eq!(cull.branches[0].name, "dropped");
        assert!(!cull.branches[0].is_default);
        assert!(cull.branches[0].predicate.is_none());

        // Cull keeps its node-level main output (unlike a Route, whose outputs
        // ARE its branch ports).
        assert!(
            cull.keeps_node_output_port(),
            "Cull must keep its node-level main output port"
        );

        // Schema-preserving: the input columns ride through as passthrough rows.
        let names: Vec<&str> = cull.fields.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, vec!["gid", "amount"]);
        assert!(
            cull.fields.iter().all(|f| f.kind == FieldKind::PassThrough),
            "Cull rows must be passthrough (schema-preserving): {:?}",
            cull.fields
        );

        // The `kept` output consumes the MAIN output (bare `prune`) → no branch;
        // the `removed` output consumes `prune.dropped` → branch index 0.
        let kept_idx = stage_idx(&view, "kept");
        let removed_idx = stage_idx(&view, "removed");
        let edge_to = |to: usize| {
            view.connections
                .iter()
                .find(|c| c.from == cull_idx && c.to == to)
                .unwrap_or_else(|| panic!("edge prune→{to} present"))
        };
        assert_eq!(
            edge_to(kept_idx).from_branch,
            None,
            "the main output edge leaves the node-level port"
        );
        assert_eq!(
            edge_to(removed_idx).from_branch,
            Some(0),
            "the removed-groups edge leaves the `removed_to` side-output port"
        );
    }

    /// Reshape and Envelope change their output shape, so the field-lineage pass
    /// must NOT draw misleading passthrough rows for them: their cards carry NO
    /// field rows and contribute NO field edges. A Cull in the same chain is
    /// schema-preserving and DOES carry passthrough rows — the contrast guards
    /// against an over-broad suppression.
    #[test]
    fn reshape_and_envelope_suppress_misleading_field_rows() {
        let yaml = r#"
pipeline:
  name: shape_changing_no_passthrough
nodes:
  - type: source
    name: src
    config:
      name: src
      type: csv
      path: ./in.csv
      schema:
        - { name: gid, type: string }
        - { name: v, type: int }
  - type: reshape
    name: synth
    input: src
    config:
      partition_by: [gid]
      rules:
        - name: fill
          when: "v > 0"
          synthesize:
            copy_from: trigger
  - type: cull
    name: prune
    input: src
    config:
      partition_by: [gid]
      removed_to: dropped
      rules:
        - name: drop_small
          drop_group_when: "count(*) < 2"
  - type: envelope
    name: frame
    body: src
    config:
      strategy: preserve
"#;
        let config = parse_config(yaml).expect("shape-changing pipeline parses");
        let view = derive_pipeline_view(&config);

        let synth = &view.stages[stage_idx(&view, "synth")];
        let frame = &view.stages[stage_idx(&view, "frame")];
        let prune = &view.stages[stage_idx(&view, "prune")];

        // Reshape and Envelope: no field rows (output shape differs from input).
        assert!(
            synth.fields.is_empty(),
            "Reshape must not draw passthrough rows (output shape differs): {:?}",
            synth.fields
        );
        assert!(
            frame.fields.is_empty(),
            "Envelope must not draw passthrough rows (output shape differs): {:?}",
            frame.fields
        );

        // Cull: schema-preserving, so it DOES carry the input columns through.
        assert_eq!(
            prune
                .fields
                .iter()
                .map(|f| f.name.as_str())
                .collect::<Vec<_>>(),
            vec!["gid", "v"],
        );
        assert!(
            prune
                .fields
                .iter()
                .all(|f| f.kind == FieldKind::PassThrough),
            "Cull rows must be passthrough: {:?}",
            prune.fields
        );

        // No field edge terminates at the schema-changing nodes (a suppressed
        // card has no row anchors for a cable to land on).
        let synth_idx = stage_idx(&view, "synth");
        let frame_idx = stage_idx(&view, "frame");
        assert!(
            view.field_edges
                .iter()
                .all(|e| e.to_node != synth_idx && e.to_node != frame_idx),
            "no field edge may terminate at a shape-changing node: {:?}",
            view.field_edges
        );
    }

    /// The per-group subtitle names the partition key and rule count, pluralizing
    /// "rule" and degrading gracefully when ungrouped; the envelope strategy name
    /// is the engine's lowercase tag.
    #[test]
    fn operator_subtitle_helpers() {
        use clinker_plan::config::pipeline_node::EnvelopeStrategy;

        assert_eq!(
            group_rules_subtitle(&["gid".to_string()], 1),
            "by gid · 1 rule"
        );
        assert_eq!(
            group_rules_subtitle(&["a".to_string(), "b".to_string()], 3),
            "by a, b · 3 rules"
        );
        // No partition fields → just the rule count (no leading "by").
        assert_eq!(group_rules_subtitle(&[], 2), "2 rules");

        assert_eq!(
            envelope_strategy_name(&EnvelopeStrategy::Preserve),
            "preserve"
        );
        assert_eq!(envelope_strategy_name(&EnvelopeStrategy::Concat), "concat");
    }

    /// Every new kind exposes a distinct `kind_attr` / `badge_label` — the CSS
    /// selector key (drives the accent that keeps cables visible) and the canvas
    /// badge text. Guards against a copy-paste collision with an existing kind.
    #[test]
    fn new_stage_kinds_have_distinct_attrs_and_badges() {
        for (kind, attr, badge) in [
            (StageKind::Reshape, "reshape", "RESHAPE"),
            (StageKind::Cull, "cull", "CULL"),
            (StageKind::Envelope, "envelope", "ENVELOPE"),
        ] {
            assert_eq!(kind.kind_attr(), attr);
            assert_eq!(kind.badge_label(), badge);
        }
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
        // Two output branches — the `priority_report` condition + the
        // `fulfilled_orders` default (the default counts as a branch).
        assert_eq!(route.subtitle, "2 branches \u{2192} fulfilled_orders");

        // No synthetic `_route` field leaks onto the card (Route has no cxl).
        assert!(route.fields.iter().all(|f| f.name != "_route"));

        // Two branch ports: the `priority_report` condition (carries a predicate)
        // then the `fulfilled_orders` default (no predicate, flagged default).
        assert_eq!(route.branches.len(), 2);
        assert_eq!(route.branches[0].name, "priority_report");
        assert!(!route.branches[0].is_default);
        assert!(route.branches[0].predicate.is_some());
        assert_eq!(route.branches[1].name, "fulfilled_orders");
        assert!(route.branches[1].is_default);
        assert!(route.branches[1].predicate.is_none());

        // Both `output` nodes consume the route node, each bound to its specific
        // branch (so the cable anchors at that branch's port, not the node port).
        let branch_targets: std::collections::HashMap<&str, Option<usize>> = view
            .connections
            .iter()
            .filter(|c| c.from == route_idx)
            .map(|c| (view.stages[c.to].id.as_str(), c.from_branch))
            .collect();
        assert_eq!(branch_targets.len(), 2, "both outputs connect to the route");
        assert_eq!(branch_targets["priority_report"], Some(0));
        assert_eq!(branch_targets["fulfilled_orders"], Some(1));
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
        use clinker_core_types::span::FileId;
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
        let has_conn = |from: usize, to: usize| {
            view.connections
                .iter()
                .any(|c| c.from == from && c.to == to)
        };
        assert!(has_conn(i_in, i_a));
        assert!(has_conn(i_a, i_b));
        assert!(has_conn(i_b, i_out));
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
        assert!(
            view.connections
                .iter()
                .any(|c| c.from == port && c.to == body_dup)
        );
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
            !view.connections.iter().any(|c| c.to == out_pos),
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
                    kind: FieldKind::Declared,
                    ty: Some("int".to_string()),
                    ..Default::default()
                },
                FieldRow {
                    name: "b".to_string(),
                    kind: FieldKind::Declared,
                    ty: Some("string".to_string()),
                    ..Default::default()
                },
            ]
        );

        // Transform: `a` shadowed? No — `c` is emitted, so a & b ride through as
        // passthrough (input order, carrying their source types), then emitted
        // `c` (no type — typing an emit is Phase 2b).
        assert_eq!(
            view.stages[i_t].fields,
            vec![
                FieldRow {
                    name: "a".to_string(),
                    kind: FieldKind::PassThrough,
                    ty: Some("int".to_string()),
                    ..Default::default()
                },
                FieldRow {
                    name: "b".to_string(),
                    kind: FieldKind::PassThrough,
                    ty: Some("string".to_string()),
                    ..Default::default()
                },
                FieldRow {
                    name: "c".to_string(),
                    kind: FieldKind::Emitted,
                    ty: None,
                    ..Default::default()
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

    /// Combine multi-input provenance (#67): a join key present in BOTH joined
    /// inputs carries from EACH of them, so hovering it lights up every source
    /// rather than only the first-seen input. A column unique to one input — and
    /// a computed emit whose support lives in one input — still resolves to that
    /// single producer; multi-producer fan-out never invents an edge from an
    /// input that did not produce the column.
    #[test]
    fn combine_multi_input_join_key_carries_from_every_input() {
        let yaml = r#"
pipeline:
  name: combine_multi_input
nodes:
  - type: source
    name: products
    config:
      name: products
      type: csv
      path: ./products.csv
      schema:
        - { name: product_code, type: string }
        - { name: product_name, type: string }
  - type: source
    name: inventory
    config:
      name: inventory
      type: csv
      path: ./inventory.csv
      schema:
        - { name: product_code, type: string }
        - { name: on_hand, type: int }
  - type: combine
    name: joined
    input:
      products: products
      inventory: inventory
    config:
      where: "products.product_code == inventory.product_code"
      match: first
      on_miss: skip
      cxl: |
        emit available = on_hand
      propagate_ck: driver
"#;
        let config = parse_config(yaml).expect("combine multi-input pipeline parses");
        let view = derive_pipeline_view(&config);

        let i_products = stage_idx(&view, "products");
        let i_inventory = stage_idx(&view, "inventory");
        let i_joined = stage_idx(&view, "joined");

        // The join key `product_code` lives in BOTH inputs, so it carries from
        // each — two passthrough edges, not one (the #67 fix).
        let carry_from_products = FieldEdge {
            from_node: i_products,
            from_field: "product_code".to_string(),
            to_node: i_joined,
            to_field: "product_code".to_string(),
            passthrough: true,
        };
        let carry_from_inventory = FieldEdge {
            from_node: i_inventory,
            from_field: "product_code".to_string(),
            to_node: i_joined,
            to_field: "product_code".to_string(),
            passthrough: true,
        };
        assert!(
            view.field_edges.contains(&carry_from_products),
            "join key must carry from `products`, got {:?}",
            view.field_edges
        );
        assert!(
            view.field_edges.contains(&carry_from_inventory),
            "join key must carry from `inventory`, got {:?}",
            view.field_edges
        );

        // A column unique to one input carries from exactly that input — the
        // fan-out is keyed on real producers, not on every predecessor.
        assert_eq!(
            view.field_edges
                .iter()
                .filter(|e| e.to_node == i_joined && e.to_field == "on_hand")
                .count(),
            1,
            "a column unique to one input carries from exactly that input"
        );

        // The computed emit `available = on_hand` derives from the single input
        // that produced `on_hand`; it is NOT fanned out to `products`.
        let derive_available = FieldEdge {
            from_node: i_inventory,
            from_field: "on_hand".to_string(),
            to_node: i_joined,
            to_field: "available".to_string(),
            passthrough: false,
        };
        assert!(
            view.field_edges.contains(&derive_available),
            "available derives from inventory.on_hand, got {:?}",
            view.field_edges
        );
        assert_eq!(
            view.field_edges
                .iter()
                .filter(|e| e.to_node == i_joined && e.to_field == "available")
                .count(),
            1,
            "available has exactly one producer (on_hand is unique to inventory)"
        );

        // Hovering the join key on the combine lights up BOTH incoming carries —
        // the legibility outcome #67 targets.
        let closure = lineage_closure(&view.field_edges, i_joined, "product_code");
        let idx_of = |e: &FieldEdge| {
            view.field_edges
                .iter()
                .position(|x| x == e)
                .expect("edge present")
        };
        assert!(
            closure.contains(&idx_of(&carry_from_products))
                && closure.contains(&idx_of(&carry_from_inventory)),
            "hovering the join key reveals carries from every input, got {closure:?}"
        );
    }

    /// CXL-less fan-in (Merge, the no-CXL lineage branch / site 3) across THREE
    /// inputs: a column in all three carries from all three; a column in exactly
    /// two carries from those two and NOT the third (the tightest "don't fan out
    /// to a non-producing input" guard); a column unique to one carries once (#67).
    #[test]
    fn merge_multi_input_shared_column_carries_from_every_producer() {
        let yaml = r#"
pipeline:
  name: merge_multi_input
nodes:
  - type: source
    name: left
    config:
      name: left
      type: csv
      path: ./left.csv
      schema:
        - { name: id, type: string }
        - { name: shared, type: int }
  - type: source
    name: mid
    config:
      name: mid
      type: csv
      path: ./mid.csv
      schema:
        - { name: id, type: string }
        - { name: shared, type: int }
  - type: source
    name: right
    config:
      name: right
      type: csv
      path: ./right.csv
      schema:
        - { name: id, type: string }
        - { name: only_right, type: int }
  - type: merge
    name: merged
    inputs: [left, mid, right]
"#;
        let config = parse_config(yaml).expect("merge multi-input pipeline parses");
        let view = derive_pipeline_view(&config);

        let i_left = stage_idx(&view, "left");
        let i_mid = stage_idx(&view, "mid");
        let i_right = stage_idx(&view, "right");
        let i_merged = stage_idx(&view, "merged");

        let carry = |from: usize, field: &str| FieldEdge {
            from_node: from,
            from_field: field.to_string(),
            to_node: i_merged,
            to_field: field.to_string(),
            passthrough: true,
        };
        let carries_to = |field: &str| {
            view.field_edges
                .iter()
                .filter(|e| e.to_node == i_merged && e.to_field == field)
                .count()
        };

        // `id` is in all three inputs → carries from each.
        assert!(view.field_edges.contains(&carry(i_left, "id")));
        assert!(view.field_edges.contains(&carry(i_mid, "id")));
        assert!(view.field_edges.contains(&carry(i_right, "id")));
        assert_eq!(carries_to("id"), 3, "got {:?}", view.field_edges);

        // `shared` is in left + mid ONLY → carries from those two, never `right`.
        assert!(view.field_edges.contains(&carry(i_left, "shared")));
        assert!(view.field_edges.contains(&carry(i_mid, "shared")));
        assert!(
            !view.field_edges.contains(&carry(i_right, "shared")),
            "must not fan out to the input that never produced the column"
        );
        assert_eq!(carries_to("shared"), 2, "got {:?}", view.field_edges);

        // `only_right` is unique to one input → exactly one carry.
        assert!(view.field_edges.contains(&carry(i_right, "only_right")));
        assert_eq!(carries_to("only_right"), 1, "got {:?}", view.field_edges);
    }

    /// Find a stage's output field row by name (test helper for the
    /// correlation-key marker assertions).
    fn field_by_name<'a>(view: &'a PipelineView, stage: usize, name: &str) -> &'a FieldRow {
        view.stages[stage]
            .fields
            .iter()
            .find(|r| r.name == name)
            .unwrap_or_else(|| panic!("field {name} present on stage {stage}"))
    }

    /// #88 (a): a Source whose `correlation_key` is a single field marks exactly
    /// that declared column as a CK driver; every other declared column is
    /// unmarked. The engine's internal `$ck.<field>` shadow column is never part
    /// of the declared `schema:` columns, so it cannot leak into the marked set.
    #[test]
    fn single_correlation_key_marks_only_the_named_source_column() {
        let yaml = r#"
pipeline:
  name: ck_single
nodes:
  - type: source
    name: orders
    config:
      name: orders
      type: csv
      path: ./orders.csv
      correlation_key: order_id
      schema:
        - { name: order_id, type: string }
        - { name: amount, type: int }
"#;
        let config = parse_config(yaml).expect("single-CK pipeline parses");
        let view = derive_pipeline_view(&config);
        let i_src = stage_idx(&view, "orders");

        let order_id = field_by_name(&view, i_src, "order_id");
        assert_eq!(order_id.kind, FieldKind::Declared);
        assert!(
            order_id.is_correlation_key,
            "the single CK driver column `order_id` is marked"
        );

        let amount = field_by_name(&view, i_src, "amount");
        assert!(
            !amount.is_correlation_key,
            "a non-driver column is not marked"
        );

        // The source declares exactly its two schema columns — no `$ck.*` shadow
        // column surfaces, so it cannot be (mis)marked.
        let names: Vec<&str> = view.stages[i_src]
            .fields
            .iter()
            .map(|r| r.name.as_str())
            .collect();
        assert_eq!(names, vec!["order_id", "amount"]);
    }

    /// #88 (b): a `Compound` correlation key (`[a, b]`) marks BOTH listed source
    /// columns and leaves the rest unmarked.
    #[test]
    fn compound_correlation_key_marks_every_listed_column() {
        let yaml = r#"
pipeline:
  name: ck_compound
nodes:
  - type: source
    name: events
    config:
      name: events
      type: csv
      path: ./events.csv
      correlation_key: [order_id, customer_id]
      schema:
        - { name: order_id, type: string }
        - { name: customer_id, type: string }
        - { name: amount, type: int }
"#;
        let config = parse_config(yaml).expect("compound-CK pipeline parses");
        let view = derive_pipeline_view(&config);
        let i_src = stage_idx(&view, "events");

        assert!(
            field_by_name(&view, i_src, "order_id").is_correlation_key,
            "first compound-key field is marked"
        );
        assert!(
            field_by_name(&view, i_src, "customer_id").is_correlation_key,
            "second compound-key field is marked"
        );
        assert!(
            !field_by_name(&view, i_src, "amount").is_correlation_key,
            "a non-key column is not marked"
        );
    }

    /// #88 (c): the correlation-key marker propagates onto a downstream
    /// transform's carried `PassThrough` row — the marker follows a CK column
    /// through a transform that does not shadow it. An emitted (new-identity)
    /// column is never marked, even when same-named state flows nearby.
    #[test]
    fn correlation_key_marker_propagates_onto_carried_passthrough() {
        let yaml = r#"
pipeline:
  name: ck_propagation
nodes:
  - type: source
    name: orders
    config:
      name: orders
      type: csv
      path: ./orders.csv
      correlation_key: order_id
      schema:
        - { name: order_id, type: string }
        - { name: amount, type: int }
  - type: transform
    name: enrich
    input: orders
    config:
      cxl: |
        emit total = amount + 1
"#;
        let config = parse_config(yaml).expect("CK-propagation pipeline parses");
        let view = derive_pipeline_view(&config);
        let i_t = stage_idx(&view, "enrich");

        // `order_id` is not shadowed by the emit, so it rides through as a
        // PassThrough row — and carries its CK marker downstream.
        let carried = field_by_name(&view, i_t, "order_id");
        assert_eq!(carried.kind, FieldKind::PassThrough);
        assert!(
            carried.is_correlation_key,
            "the CK marker follows a carried column through the transform"
        );

        // The carried non-key column stays unmarked.
        assert!(
            !field_by_name(&view, i_t, "amount").is_correlation_key,
            "a carried non-key column is not marked"
        );

        // The emitted `total` column is a new identity, never a CK driver.
        let emitted = field_by_name(&view, i_t, "total");
        assert_eq!(emitted.kind, FieldKind::Emitted);
        assert!(
            !emitted.is_correlation_key,
            "an emitted column is never marked as a correlation-key driver"
        );
    }

    /// #88 / #2 (multi-input OR-semantics): a column shared by two Merge inputs
    /// where ONLY ONE input declares it a correlation key is marked CK on the
    /// merged row — the role is a boolean identity ORed across producers, not
    /// first-producer-wins. The non-CK source is declared FIRST, so a
    /// first-producer-wins bug would (wrongly) leave `id` unmarked; the OR makes
    /// it marked regardless of order.
    #[test]
    fn correlation_key_marker_ors_across_merge_inputs() {
        let yaml = r#"
pipeline:
  name: ck_merge_or
nodes:
  - type: source
    name: plain
    config:
      name: plain
      type: csv
      path: ./plain.csv
      schema:
        - { name: id, type: string }
        - { name: a, type: int }
  - type: source
    name: keyed
    config:
      name: keyed
      type: csv
      path: ./keyed.csv
      correlation_key: id
      schema:
        - { name: id, type: string }
        - { name: b, type: int }
  - type: merge
    name: merged
    inputs: [plain, keyed]
"#;
        let config = parse_config(yaml).expect("CK-merge-OR pipeline parses");
        let view = derive_pipeline_view(&config);
        let i_merged = stage_idx(&view, "merged");

        // `id` comes from both inputs; only `keyed` declares it a CK driver. The
        // merged carry row is marked because ANY producer marks it.
        let merged_id = field_by_name(&view, i_merged, "id");
        assert_eq!(merged_id.kind, FieldKind::PassThrough);
        assert!(
            merged_id.is_correlation_key,
            "a column is a CK driver on the merge if ANY input declares it one"
        );

        // Columns unique to one input keep their own (un)marked state.
        assert!(!field_by_name(&view, i_merged, "a").is_correlation_key);
        assert!(!field_by_name(&view, i_merged, "b").is_correlation_key);
    }

    /// #88 / #4 (emit-shadow boundary): a CK source column re-emitted by a
    /// value-CHANGING expression becomes an `Emitted` (new identity) row and
    /// LOSES the marker, while an identity-copy re-emit stays `PassThrough` and
    /// KEEPS it. Two transforms over the same CK source pin both halves.
    #[test]
    fn correlation_key_marker_lost_on_value_changing_reemit_kept_on_identity_copy() {
        let yaml = r#"
pipeline:
  name: ck_emit_shadow
nodes:
  - type: source
    name: orders
    config:
      name: orders
      type: csv
      path: ./orders.csv
      correlation_key: order_id
      schema:
        - { name: order_id, type: int }
        - { name: amount, type: int }
  - type: transform
    name: bumped
    input: orders
    config:
      cxl: |
        emit order_id = order_id + 1
  - type: transform
    name: copied
    input: orders
    config:
      cxl: |
        emit order_id = order_id
"#;
        let config = parse_config(yaml).expect("CK-emit-shadow pipeline parses");
        let view = derive_pipeline_view(&config);

        // Value-changing re-emit: `order_id` is recomputed → Emitted, unmarked.
        let bumped = field_by_name(&view, stage_idx(&view, "bumped"), "order_id");
        assert_eq!(
            bumped.kind,
            FieldKind::Emitted,
            "a value-changing re-emit produces a new identity"
        );
        assert!(
            !bumped.is_correlation_key,
            "a recomputed value is no longer the CK driver, so it loses the marker"
        );

        // Identity-copy re-emit: `emit order_id = order_id` is a passthrough copy
        // → PassThrough, marker kept.
        let copied = field_by_name(&view, stage_idx(&view, "copied"), "order_id");
        assert_eq!(
            copied.kind,
            FieldKind::PassThrough,
            "an identity-copy re-emit carries the column unchanged"
        );
        assert!(
            copied.is_correlation_key,
            "an identity-copy re-emit keeps the CK marker"
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
                kind: FieldKind::Declared,
                ty: Some("float".to_string()),
                ..Default::default()
            }]
        );

        // t1: passthrough `a` (not shadowed, carries `float`) then emitted `b`
        // (no type — emit typing is Phase 2b).
        assert_eq!(
            view.stages[i_t1].fields,
            vec![
                FieldRow {
                    name: "a".to_string(),
                    kind: FieldKind::PassThrough,
                    ty: Some("float".to_string()),
                    ..Default::default()
                },
                FieldRow {
                    name: "b".to_string(),
                    kind: FieldKind::Emitted,
                    ty: None,
                    ..Default::default()
                },
            ]
        );

        // t2: passthrough `a` (still `float`), `b` (untyped — it was emitted
        // upstream), then emitted `c`.
        assert_eq!(
            view.stages[i_t2].fields,
            vec![
                FieldRow {
                    name: "a".to_string(),
                    kind: FieldKind::PassThrough,
                    ty: Some("float".to_string()),
                    ..Default::default()
                },
                FieldRow {
                    name: "b".to_string(),
                    kind: FieldKind::PassThrough,
                    ty: None,
                    ..Default::default()
                },
                FieldRow {
                    name: "c".to_string(),
                    kind: FieldKind::Emitted,
                    ty: None,
                    ..Default::default()
                },
            ]
        );

        // Output port now carries one Declared row named for the port (FIX E),
        // so the card draws a label + anchor instead of a blank boundary.
        assert_eq!(
            view.stages[i_out].fields,
            vec![FieldRow {
                name: "result".to_string(),
                kind: FieldKind::Declared,
                ty: None,
                ..Default::default()
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
                kind: FieldKind::PassThrough,
                // The carried column keeps its source type even though the node's
                // CXL failed to parse (the type comes from the producer, not the
                // emit analysis).
                ty: Some("float".to_string()),
                ..Default::default()
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
    use clinker_plan::config::parse_config;

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
        use clinker_core_types::span::FileId;
        use clinker_plan::config::composition::CompositionFile;
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
            branches: Vec::new(),
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
