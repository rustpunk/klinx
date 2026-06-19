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
pub mod layout_model;

pub use field_lineage::{
    EdgeNature, FieldEdge, FieldEdgeKind, FieldKind, FieldRow, Precision, compact_type,
    field_lineage_full, field_lineage_full_capped, group_endpoints_by_node, lineage_closure,
    lineage_keep_nodes,
};
/// Composition input-boundary marker grammar (#155), re-exported crate-internally so
/// the Inspector's scope-aware trace builds/parses the marker through the SAME
/// single-source-of-truth helpers `field_lineage` uses to mint it (#154).
pub(crate) use field_lineage::{
    composition_in_boundary_field, parse_composition_in_boundary_field,
};
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

/// One point in a routed canvas connector path.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CanvasPoint {
    pub x: f32,
    pub y: f32,
}

/// Optional renderer-ready connector route.
///
/// Empty views use endpoint-derived connector geometry. Port-aware layout
/// migration fills these paths so the SVG renderer can draw reserved lanes
/// without re-running layout work in the component layer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CanvasConnectorPath {
    pub points: Vec<CanvasPoint>,
}

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

pub fn aggregate_group_key_port_id(key: &str) -> String {
    format!("group_by:{key}")
}

fn role_ports(node: &PipelineNode) -> Vec<StagePortRow> {
    match node {
        PipelineNode::Aggregate { config, .. } => aggregate_group_key_role_ports(&config.group_by),
        _ => Vec::new(),
    }
}

fn aggregate_group_key_role_ports(group_by: &[String]) -> Vec<StagePortRow> {
    let mut ports = Vec::with_capacity(group_by.len());
    let mut seen = std::collections::HashSet::new();
    for key in group_by {
        if seen.insert(key.as_str()) {
            ports.push(StagePortRow {
                id: aggregate_group_key_port_id(key),
                label: key.clone(),
                role: "group_by".to_string(),
                kind: StagePortKind::AggregateGroupKey,
                side: StagePortSide::Input,
            });
        }
    }
    ports
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

/// Which side of a stage card a semantic role port attaches to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StagePortSide {
    Input,
    Output,
}

/// Semantic, non-column row category rendered inside a stage card.
///
/// These are distinct from output-record fields: a role port explains how an
/// input value is used by the operator (for example an Aggregate grouping key),
/// while [`FieldRow`] explains fields present on the node's output record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StagePortKind {
    AggregateGroupKey,
}

/// A named semantic role port rendered as a fixed-height row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StagePortRow {
    pub id: String,
    pub label: String,
    pub role: String,
    pub kind: StagePortKind,
    pub side: StagePortSide,
}

/// Lineage from an output field row into a semantic role input port.
#[derive(Clone, Debug, PartialEq)]
pub struct RoleEdge {
    pub from_node: usize,
    pub from_field: String,
    pub to_node: usize,
    pub to_port: String,
    pub kind: FieldEdgeKind,
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
    /// Semantic role rows such as Aggregate `group_by` inputs. These are not
    /// output fields and are anchored independently, so one producer field can
    /// connect both to a role input and to a grouped output field without
    /// drawing duplicate cables into the same row.
    pub role_ports: Vec<StagePortRow>,
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
    /// Input role rows, if any, occupy the first row slots below the
    /// [`FIELD_HEADER_HEIGHT`] header, optionally under a role-section header.
    /// Field rows start after them at the fixed [`FIELD_ROW_HEIGHT`] pitch; the
    /// anchor sits at the row's mid-line.
    pub fn field_row_y(&self, i: usize) -> f32 {
        self.canvas_y
            + FIELD_HEADER_HEIGHT
            + (self.input_role_header_count() + self.input_role_count() + i) as f32
                * FIELD_ROW_HEIGHT
            + FIELD_ROW_HEIGHT / 2.0
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

    pub fn role_port_index(&self, side: StagePortSide, id: &str) -> Option<usize> {
        self.role_ports
            .iter()
            .filter(|port| port.side == side)
            .position(|port| port.id == id)
    }

    pub fn role_port_anchor_in(&self, i: usize) -> (f32, f32) {
        (self.canvas_x, self.role_port_row_y(StagePortSide::Input, i))
    }

    #[allow(dead_code)]
    pub fn role_port_anchor_out(&self, i: usize) -> (f32, f32) {
        (
            self.canvas_x + NODE_WIDTH,
            self.role_port_row_y(StagePortSide::Output, i),
        )
    }

    pub fn role_ports_on(&self, side: StagePortSide) -> impl Iterator<Item = &StagePortRow> {
        self.role_ports.iter().filter(move |port| port.side == side)
    }

    pub fn input_role_header_count(&self) -> usize {
        role_port_header_rows(&self.role_ports, StagePortSide::Input)
    }

    fn output_role_header_count(&self) -> usize {
        role_port_header_rows(&self.role_ports, StagePortSide::Output)
    }

    fn input_role_count(&self) -> usize {
        self.role_ports_on(StagePortSide::Input).count()
    }

    fn output_role_count(&self) -> usize {
        self.role_ports_on(StagePortSide::Output).count()
    }

    fn role_port_row_y(&self, side: StagePortSide, i: usize) -> f32 {
        let row_slot = match side {
            StagePortSide::Input => self.input_role_header_count() + i,
            StagePortSide::Output => {
                self.input_role_header_count()
                    + self.input_role_count()
                    + self.fields.len()
                    + self.output_role_header_count()
                    + i
            }
        };
        self.canvas_y
            + FIELD_HEADER_HEIGHT
            + row_slot as f32 * FIELD_ROW_HEIGHT
            + FIELD_ROW_HEIGHT / 2.0
    }

    /// World-space vertical center of branch-port row `i`. Branch ports stack
    /// BELOW role and field rows at the same [`FIELD_ROW_HEIGHT`] pitch.
    pub fn branch_row_y(&self, i: usize) -> f32 {
        self.canvas_y
            + FIELD_HEADER_HEIGHT
            + (self.input_role_header_count()
                + self.input_role_count()
                + self.fields.len()
                + self.output_role_header_count()
                + self.output_role_count()
                + i) as f32
                * FIELD_ROW_HEIGHT
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
        let rows = role_port_header_rows_total(&self.role_ports)
            + self.role_ports.len()
            + self.fields.len()
            + self.branches.len();
        if rows == 0 {
            NODE_HEIGHT
        } else {
            FIELD_HEADER_HEIGHT + rows as f32 * FIELD_ROW_HEIGHT
        }
    }
}

fn role_port_header_rows(ports: &[StagePortRow], side: StagePortSide) -> usize {
    let mut ports = ports.iter().filter(|port| port.side == side);
    let Some(first) = ports.next() else {
        return 0;
    };
    if matches!(first.kind, StagePortKind::AggregateGroupKey)
        && ports.all(|port| matches!(port.kind, StagePortKind::AggregateGroupKey))
    {
        1
    } else {
        0
    }
}

fn role_port_header_rows_total(ports: &[StagePortRow]) -> usize {
    role_port_header_rows(ports, StagePortSide::Input)
        + role_port_header_rows(ports, StagePortSide::Output)
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
    /// Optional routed paths parallel to [`PipelineView::connections`].
    ///
    /// Empty in the current default barycenter layout. When populated by the
    /// port-aware layout migration, the canvas renderer uses these point lists
    /// instead of deriving every connector from only its endpoints.
    pub connection_paths: Vec<CanvasConnectorPath>,
    /// Field-level lineage edges between stage rows. Populated by both the
    /// composition pass (#66) and the pipeline pass (#68); empty for views
    /// without resolvable field schemas (e.g. partial/degraded views), where an
    /// empty `field_edges` means the canvas draws node-level connectors only.
    pub field_edges: Vec<FieldEdge>,
    /// Optional routed paths parallel to [`PipelineView::field_edges`].
    pub field_edge_paths: Vec<CanvasConnectorPath>,
    /// Field-to-role lineage edges, currently used for Aggregate `group_by`
    /// input role ports.
    pub role_edges: Vec<RoleEdge>,
    /// Optional routed paths parallel to [`PipelineView::role_edges`].
    pub role_edge_paths: Vec<CanvasConnectorPath>,
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

/// Minimal pan (no zoom change) that brings `node`'s screen rect fully inside a
/// `viewport_w` × `viewport_h` pixel viewport, keeping `margin` px of breathing
/// room on each edge. Returns the new `(pan_x, pan_y)`, or `None` when the node
/// is already comfortably within view so the caller can skip the pan entirely.
///
/// Uses the same screen mapping as the canvas transform — a world point `p` maps
/// to screen `pan + p * zoom` (`transform-origin: 0 0`). Each axis is adjusted
/// independently and only as far as needed: an already-visible axis is left
/// untouched. When the node is larger than the available window on an axis, its
/// near (left/top) edge is pinned to the margin so the node's start is visible
/// rather than centering it with both ends clipped.
///
/// This is the "reveal" companion to [`fit_transform`]: `fit_transform` reframes
/// the whole graph (changing zoom), whereas this nudges an off-screen node into
/// view without disturbing the user's current zoom.
pub fn pan_to_reveal(
    node: LayoutBounds,
    pan_x: f32,
    pan_y: f32,
    zoom: f32,
    viewport_w: f32,
    viewport_h: f32,
    margin: f32,
) -> Option<(f32, f32)> {
    let dx = axis_reveal_delta(
        pan_x + node.min_x * zoom,
        pan_x + node.max_x * zoom,
        viewport_w,
        margin,
    );
    let dy = axis_reveal_delta(
        pan_y + node.min_y * zoom,
        pan_y + node.max_y * zoom,
        viewport_h,
        margin,
    );
    if dx == 0.0 && dy == 0.0 {
        None
    } else {
        Some((pan_x + dx, pan_y + dy))
    }
}

/// Pan delta (screen px) that brings the screen interval `[lo, hi]` within
/// `[margin, extent - margin]`, or `0.0` when it already fits. A span wider than
/// the available window pins its low edge to the near margin. Degenerate
/// viewports (`extent <= 2 * margin`) yield `0.0` so the caller never pans on a
/// not-yet-measured pane.
fn axis_reveal_delta(lo: f32, hi: f32, extent: f32, margin: f32) -> f32 {
    let near = margin;
    let far = extent - margin;
    if far <= near {
        return 0.0;
    }
    if lo >= near && hi <= far {
        return 0.0;
    }
    if hi - lo > far - near {
        // Larger than the window: show the start, accept the far edge clipping.
        near - lo
    } else if lo < near {
        // Off the near edge: shift content toward the far edge (positive delta).
        near - lo
    } else {
        // Off the far edge: shift content toward the near edge (negative delta).
        far - hi
    }
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

/// Derive a top-level canvas view from a compiled plan.
///
/// Resolved mode uses the engine's typed output rows as the authoritative field
/// row/type source while keeping the same node/connection geometry as the raw
/// pipeline view. Missing typed rows degrade to empty field rows for that node
/// instead of falling back to confident approximation.
pub fn derive_resolved_pipeline_view(plan: &clinker_plan::plan::CompiledPlan) -> PipelineView {
    derive_view_from_nodes_inner(&plan.config().nodes, Some(plan))
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
    derive_view_from_nodes_inner(nodes, None)
}

fn derive_view_from_nodes_inner(
    nodes: &[Spanned<PipelineNode>],
    resolved_plan: Option<&clinker_plan::plan::CompiledPlan>,
) -> PipelineView {
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
    let node_role_ports: Vec<Vec<StagePortRow>> =
        nodes.iter().map(|s| role_ports(&s.value)).collect();

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
    let (out_fields, field_edges, role_edges) = match resolved_plan {
        Some(plan) => resolved_pipeline_field_lineage(nodes, plan, &predecessors),
        None => pipeline_field_lineage(nodes, &predecessors),
    };

    // Per-node card heights drive the column stacking so a tall field-bearing
    // card never overlaps the next one. A node with field rows is `header +
    // n*row` tall (matching [`StageView::card_height`]); a row-less node keeps
    // the classic [`NODE_HEIGHT`], so a pipeline with no schemas lays out exactly
    // as fixed-height stacking did before this change.
    let heights: Vec<f32> = out_fields
        .iter()
        .zip(&node_branches)
        .zip(&node_role_ports)
        .map(|((rows, branches), role_ports)| {
            row_stack_height(
                rows.len()
                    + branches.len()
                    + role_ports.len()
                    + role_port_header_rows_total(role_ports),
            )
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
            stage.branches = node_branches[i].clone();
            stage.role_ports = node_role_ports[i].clone();
            stage
        })
        .collect();

    PipelineView {
        stages,
        connections,
        connection_paths: Vec::new(),
        field_edges,
        field_edge_paths: Vec::new(),
        role_edges,
        role_edge_paths: Vec::new(),
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
    let mut node_role_ports: Vec<Vec<StagePortRow>> = vec![Vec::new(); total];
    for (bi, spanned) in body.iter().enumerate() {
        node_branches[n_in + bi] = output_branches(&spanned.value);
        node_role_ports[n_in + bi] = role_ports(&spanned.value);
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
    let (out_fields, field_edges, role_edges) =
        composition_field_lineage(sig, body, n_in, n_body, &predecessors);

    // Per-node card heights drive the stacking so tall field cards in one column
    // never overlap. A node with field rows is `header + n*row` tall (matching
    // [`StageView::card_height`]); a row-less node is the classic [`NODE_HEIGHT`].
    let heights: Vec<f32> = out_fields
        .iter()
        .zip(&node_branches)
        .zip(&node_role_ports)
        .map(|((rows, branches), role_ports)| {
            row_stack_height(
                rows.len()
                    + branches.len()
                    + role_ports.len()
                    + role_port_header_rows_total(role_ports),
            )
        })
        .collect();
    let positions = layout_positions(&cols, &predecessors, &heights);

    let mut stages: Vec<StageView> = Vec::with_capacity(total);
    for (i, (port, decl)) in sig.inputs.iter().enumerate() {
        let (x, y) = positions[i];
        let mut stage = input_port_stage(port, decl, x, y);
        stage.fields = out_fields[i].clone();
        stage.role_ports = node_role_ports[i].clone();
        stages.push(stage);
    }
    for (bi, spanned) in body.iter().enumerate() {
        let (x, y) = positions[n_in + bi];
        let mut stage = build_stage_view(&spanned.value, x, y);
        stage.fields = out_fields[n_in + bi].clone();
        stage.branches = node_branches[n_in + bi].clone();
        stage.role_ports = node_role_ports[n_in + bi].clone();
        stages.push(stage);
    }
    for (oi, (port, alias)) in sig.outputs.iter().enumerate() {
        let (x, y) = positions[n_in + n_body + oi];
        let mut stage = output_port_stage(port, alias, x, y);
        stage.fields = out_fields[n_in + n_body + oi].clone();
        stage.role_ports = node_role_ports[n_in + n_body + oi].clone();
        stages.push(stage);
    }

    PipelineView {
        stages,
        connections,
        connection_paths: Vec::new(),
        field_edges,
        field_edge_paths: Vec::new(),
        role_edges,
        role_edge_paths: Vec::new(),
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
            ..Default::default()
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

/// The bare column name an emit-support member denotes — the final dotted
/// segment of an alias/source-qualified ref (`orders.order_id` → `order_id`),
/// or the member itself when unqualified.
///
/// CXL emits in a Combine body name columns through an input-port alias, and
/// `Expr::support_into` reports such a ref as the joined dotted string. The
/// canvas's per-column maps (`producers_of`, `used_cols`) are keyed by the BARE
/// column name, so support members are normalised through here before lookup.
/// Mirrors [`field_lineage::bare_field_ref`]'s last-segment rule for copies, so
/// edge resolution and copy classification agree.
fn bare_column(member: &str) -> &str {
    member.rsplit_once('.').map_or(member, |(_, last)| last)
}

/// A node's input-port aliases mapped to the slot index each names — the
/// qualifier resolver for alias-qualified CXL refs in a Combine body.
///
/// Only [`PipelineNode::Combine`] declares input-port aliases (its
/// `input:` map keys, e.g. `orders` / `products`); every other node returns an
/// empty map, so its CXL refs (bare, or a single-source `src.col`) fall through
/// to bare-column resolution unchanged. `name_to_idx` maps a producer's name to
/// its slot index in the caller's index space (node indices for a pipeline,
/// unified port/body indices for a composition), so the same helper serves both
/// callers.
fn node_input_aliases(
    node: &PipelineNode,
    name_to_idx: &std::collections::HashMap<String, usize>,
) -> std::collections::HashMap<String, usize> {
    let mut aliases = std::collections::HashMap::new();
    if let PipelineNode::Combine { header, .. } = node {
        for (alias, ni) in &header.input {
            if let Some(&p) = name_to_idx.get(node_input_name(&ni.value)) {
                aliases.insert(alias.clone(), p);
            }
        }
    }
    aliases
}

/// Resolve one CXL emit-support member to its `(producer_index, bare_column)`
/// edge anchors, bridging the alias-qualified support vocabulary to the
/// bare-keyed producer map.
///
/// Three cases, in precedence order:
///   - `alias.column` whose `alias` is one of this node's input ports
///     (`input_aliases`) → the bare `column` on EXACTLY that port's predecessor,
///     or nothing when that predecessor does not produce the column. This is the
///     precise case: a join key copied from one specific side
///     (`emit product_code = orders.product_code`) connects only to that side,
///     even when the column also exists in the other input.
///   - any other dotted ref (a single-input `src.col`, or an unknown qualifier)
///     → bare-column fallback: every producer of the final dotted segment.
///   - a bare member → every producer of that column.
///
/// Returns an empty vec when no producer matches — a member that names no
/// predecessor column (an intra-node `let`/earlier-emit the caller resolves via
/// `emitted_so_far`, or a typo) — never panicking.
fn resolve_support_anchors(
    member: &str,
    producers_of: &std::collections::HashMap<String, Vec<usize>>,
    input_aliases: &std::collections::HashMap<String, usize>,
) -> Vec<(usize, String)> {
    if let Some((alias, rest)) = member.split_once('.')
        && let Some(&p) = input_aliases.get(alias)
    {
        // Alias-qualified by a declared input port → that port's predecessor
        // only. The producer column is the FIRST segment after the alias, so a
        // deeper nested access (`alias.record.subfield`) still pins the `record`
        // column it reads rather than missing on the bare-keyed map's absent
        // `record.subfield` key. A column the port does not produce yields no
        // edge (the alias pins the producer; we never fan to another input to
        // "find" it).
        let column = rest.split_once('.').map_or(rest, |(col, _)| col);
        return match producers_of.get(column) {
            Some(producers) if producers.contains(&p) => vec![(p, column.to_string())],
            _ => Vec::new(),
        };
    }
    // Bare, or dotted-but-not-a-known-port: resolve the bare column across all
    // its producers (a shared fan-in column carries from each, per #67).
    let column = bare_column(member);
    producers_of
        .get(column)
        .map(|producers| producers.iter().map(|&p| (p, column.to_string())).collect())
        .unwrap_or_default()
}

/// The full identity of a [`FieldEdge`] for dedup: `(from_node, from_field,
/// to_node, to_field, kind)` — two edges are duplicates iff these are equal, so a
/// value carry and an influence edge between the same endpoints (differing only
/// in `kind`) are kept distinct.
///
/// `EdgeKeyRef` is the borrowed view used for the membership PROBE (the two field
/// names stay `&str`, so probing allocates nothing); [`EdgeKey`] is the owned form
/// stored on the insert-success path. They are compared field-by-field via
/// [`EdgeKey::matches`] and hashed by the SAME [`hash_edge_key_ref`] over the
/// borrowed view, so an owned key and a borrowed key with equal contents hash and
/// compare identically (a `String` hashes exactly as the `str` it derefs to).
type EdgeKeyRef<'a> = (usize, &'a str, usize, &'a str, FieldEdgeKind);

struct EdgeKey {
    from_node: usize,
    from_field: String,
    to_node: usize,
    to_field: String,
    kind: FieldEdgeKind,
}

/// Hash an [`EdgeKeyRef`] field-by-field. The owned [`EdgeKey`] is hashed by
/// projecting it to the same borrowed view (see [`EdgeAccumulator::push_deduped`]),
/// so the two are hash-coherent.
fn hash_edge_key_ref<H: std::hash::Hasher>(key: &EdgeKeyRef<'_>, state: &mut H) {
    use std::hash::Hash;
    key.0.hash(state);
    key.1.hash(state);
    key.2.hash(state);
    key.3.hash(state);
    key.4.hash(state);
}

impl EdgeKey {
    /// Field-by-field equality against a borrowed key — no allocation.
    fn matches(&self, other: &EdgeKeyRef<'_>) -> bool {
        self.from_node == other.0
            && self.from_field == other.1
            && self.to_node == other.2
            && self.to_field == other.3
            && self.kind == other.4
    }

    /// The owned key with equal contents — built ONLY on the insert path.
    fn from_ref(key: &EdgeKeyRef<'_>) -> EdgeKey {
        EdgeKey {
            from_node: key.0,
            from_field: key.1.to_string(),
            to_node: key.2,
            to_field: key.3.to_string(),
            kind: key.4,
        }
    }
}

/// A growing edge list paired with an O(1)-membership dedup index over the edges
/// already pushed through [`EdgeAccumulator::push_deduped`] (#147).
///
/// The `seen` map keys edges by their full identity hash; it is maintained
/// ALONGSIDE `edges` so a deduped push is O(1) rather than the O(n) a
/// `Vec::contains` rescan would cost inside the predicate fan's support ×
/// producer × row triple loop. Bundling the two together (rather than threading
/// both as separate `&mut` parameters) keeps the influence-edge helpers'
/// signatures small. Insertion order in `edges` is unchanged — `seen` only gates
/// whether a candidate is appended. DIRECT derive/carry edges that cannot collide
/// by construction go through [`EdgeAccumulator::push_direct`] instead, which
/// bypasses `seen` so the can't-collide contract is expressed by the type.
struct EdgeAccumulator {
    edges: Vec<FieldEdge>,
    /// Dedup index keyed by the full-identity hash → the owned keys that hash
    /// there (a bucket list to resolve the rare hash collision exactly). A probe
    /// hashes the BORROWED key and compares against only its bucket, so a
    /// duplicate allocates no `String`; the owned key is built ONLY when the edge
    /// is genuinely new.
    seen: std::collections::HashMap<u64, Vec<EdgeKey>>,
}

impl EdgeAccumulator {
    fn new() -> Self {
        Self {
            edges: Vec::new(),
            seen: std::collections::HashMap::new(),
        }
    }

    /// Push a [`FieldEdge`] only if no exact `(from_node, from_field, to_node,
    /// to_field, kind)` tuple has been pushed before.
    ///
    /// INDIRECT influence edges fan a single predicate-support producer out to
    /// every surviving output row, and a Cull may OR several rules over the same
    /// column, so the same edge can be produced more than once. De-duplicating on
    /// the full tuple keeps the edge list — and therefore the lineage closure walk
    /// — bounded, while still permitting a column to carry BOTH a DIRECT and an
    /// INDIRECT edge to the same endpoint (the `kind` is part of the identity, so
    /// a value carry and an influence edge coexist).
    ///
    /// The membership probe hashes a BORROWED key ([`EdgeKeyRef`]) and compares
    /// against its hash bucket, so a duplicate — the common case in the predicate
    /// fan — allocates nothing; the two field names are cloned into an owned
    /// [`EdgeKey`] ONLY when the edge is genuinely new and is about to be stored.
    fn push_deduped(&mut self, edge: FieldEdge) {
        let key_ref: EdgeKeyRef<'_> = (
            edge.from_node,
            edge.from_field.as_str(),
            edge.to_node,
            edge.to_field.as_str(),
            edge.kind,
        );
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        hash_edge_key_ref(&key_ref, &mut hasher);
        let hash = std::hash::Hasher::finish(&hasher);

        let bucket = self.seen.entry(hash).or_default();
        if bucket.iter().any(|k| k.matches(&key_ref)) {
            return;
        }
        bucket.push(EdgeKey::from_ref(&key_ref));
        self.edges.push(edge);
    }

    /// Push a DIRECT derive/carry edge that CANNOT collide by construction,
    /// bypassing the dedup index.
    ///
    /// The transform pass emits each derive/identity-carry edge from a distinct
    /// `(producer, target)` pairing within a single node sweep, so two such pushes
    /// can never share the full identity tuple — re-probing `seen` would be wasted
    /// work. Exposing the bypass as a method (rather than poking a public `edges`
    /// field directly) makes the "can't-collide" contract part of the type's
    /// surface: a caller asserts collision-freedom by *choosing* `push_direct`, so
    /// the dedup invariant is expressed by the type, not by doc-comment convention.
    /// The pushed edge is NOT recorded in `seen`; a later `push_deduped` of the
    /// same identity would not observe it, which is correct precisely because the
    /// construction guarantees that never happens.
    fn push_direct(&mut self, edge: FieldEdge) {
        self.edges.push(edge);
    }
}

/// Emit INDIRECT *predicate* influence edges for one control-flow node (#147):
/// a `Cull` removal predicate (`Filter`) or a `Route` branch condition
/// (`Conditional`).
///
/// **Edge target-set policy (decided once here):** a filter / conditional
/// predicate influences *which rows survive*, not any single output value, so
/// each column the predicate reads is connected to **every surviving output row**
/// of the node — `surviving_rows` is the node's own resolved output record
/// (`out_fields[idx]`), not its role ports. This is the honest "this column
/// decided whether these rows exist" relation; routing it to every output row
/// (rather than a single representative) keeps the influence visible from any
/// downstream column, and the de-dup keeps the closure bounded.
///
/// `predicate` is the raw CXL predicate string; it is run through
/// [`field_lineage::predicate_support`], which returns `None` on a parse failure
/// — in which case NO edges are emitted (never infer from unparseable CXL). Each
/// resolved support column is mapped to its producer anchors via
/// [`resolve_support_anchors`] (so a fan-in column connects to each producing
/// input). Edges are de-duplicated on the full tuple.
fn emit_predicate_influence_edges(
    acc: &mut EdgeAccumulator,
    idx: usize,
    predicate: &str,
    kind: FieldEdgeKind,
    surviving_rows: &[FieldRow],
    producers_of: &std::collections::HashMap<String, Vec<usize>>,
    input_aliases: &std::collections::HashMap<String, usize>,
) {
    let Some(support) = field_lineage::predicate_support(predicate) else {
        return;
    };
    for member in &support {
        for (p, from_field) in resolve_support_anchors(member, producers_of, input_aliases) {
            for row in surviving_rows {
                // INDIRECT influence edges are always Approximate (#148): the
                // predicate column states which rows survive, not a value.
                acc.push_deduped(FieldEdge::influence(
                    p,
                    from_field.clone(),
                    idx,
                    row.name.clone(),
                    kind,
                ));
            }
        }
    }
}

/// The control-flow predicates a node imposes, each paired with the INDIRECT
/// edge kind it produces (#147) — the single place the clinker config shapes for
/// influence predicates are read.
///
/// - **Route** (`RouteBody.conditions: IndexMap<String, CxlSource>`): each branch
///   condition is a `Conditional` predicate. The always-present default/fallback
///   branch has no predicate, so contributes nothing.
/// - **Cull** (`CullBody.rules: Vec<CullRule>`, each
///   `CullRule.drop_group_when: CxlSource`): every removal rule is a `Filter`
///   predicate (a group is removed when ANY rule holds, so each rule's read
///   columns influence which rows survive).
/// - **Combine** (`CombineBody.where_expr: CxlSource`): the join predicate is the
///   real join key. It is a `JoinKey` influence predicate, resolved through the
///   SAME `predicate_support` path as Cull/Route — its read columns (including
///   qualified `products.product_code` refs, resolved via `input_aliases`)
///   influence which records are matched. A `Merge` is a streamwise row UNION
///   (`MergeMode::Concat`/`Interleave`), never a join, so it imposes no predicate
///   and yields no `JoinKey` edge.
///
/// Returns the borrowed predicate source strings; every other node yields none.
fn node_influence_predicates(node: &PipelineNode) -> Vec<(&str, FieldEdgeKind)> {
    match node {
        PipelineNode::Route { config, .. } => config
            .conditions
            .values()
            .map(|predicate| (predicate.as_ref(), FieldEdgeKind::Conditional))
            .collect(),
        PipelineNode::Cull { config, .. } => config
            .rules
            .iter()
            .map(|rule| (rule.drop_group_when.as_ref(), FieldEdgeKind::Filter))
            .collect(),
        PipelineNode::Combine { config, .. } => {
            vec![(config.where_expr.as_ref(), FieldEdgeKind::JoinKey)]
        }
        _ => Vec::new(),
    }
}

/// Emit every INDIRECT influence edge a node contributes (#147): Route's
/// `Conditional` branch conditions, Cull's `Filter` removal predicates, and
/// Combine's `JoinKey` `where_expr` join predicate. Shared by both lineage paths
/// so they stay identical.
///
/// Every influence edge now flows from one place — [`node_influence_predicates`]
/// → [`emit_predicate_influence_edges`] — so the same `predicate_support`
/// resolution backs all influence kinds. A `Merge` is a streamwise row UNION, not
/// a join, so it imposes no predicate and contributes no `JoinKey` edge.
///
/// Called AFTER the node's normal DIRECT rows/carries are emitted, reading the
/// node's already-populated output record (`surviving_rows`) so predicate edges
/// land on the real surviving rows. `Aggregate` `group_by` `GroupBy` edges are
/// NOT emitted here — they are produced inline alongside the group-key rows in
/// each path (where the group-key resolution already happens).
fn emit_indirect_influence_edges(
    acc: &mut EdgeAccumulator,
    idx: usize,
    node: &PipelineNode,
    surviving_rows: &[FieldRow],
    producers_of: &std::collections::HashMap<String, Vec<usize>>,
    input_aliases: &std::collections::HashMap<String, usize>,
) {
    for (predicate, kind) in node_influence_predicates(node) {
        emit_predicate_influence_edges(
            acc,
            idx,
            predicate,
            kind,
            surviving_rows,
            producers_of,
            input_aliases,
        );
    }
}

/// Emit the INDIRECT `GroupBy` field edge AND its semantic-role-port edge for one
/// Aggregate group key (#147), resolving the key to each producing input.
///
/// Both lineage paths ([`compute_field_lineage`] and
/// [`resolved_pipeline_field_lineage`]) need the identical group-key emission;
/// extracting it here (PR1's deferred cleanup, folded into #148) keeps the two
/// from drifting — a group key defines the grouped-record grain, the single honest
/// representation of the post-Aggregate grain that retired the former
/// `is_aggregate_grain` row flag. The field edge dedups (a key resolved from two
/// inputs, or repeated across paths, must not double); the matching role edge is
/// appended verbatim because role edges carry no dedup index.
fn emit_group_by_edges(
    acc: &mut EdgeAccumulator,
    role_edges: &mut Vec<RoleEdge>,
    idx: usize,
    group_key: &str,
    producers_of: &std::collections::HashMap<String, Vec<usize>>,
    input_aliases: &std::collections::HashMap<String, usize>,
) {
    for (p, from_field) in resolve_support_anchors(group_key, producers_of, input_aliases) {
        acc.push_deduped(FieldEdge::influence(
            p,
            from_field.clone(),
            idx,
            group_key.to_string(),
            FieldEdgeKind::GroupBy,
        ));
        role_edges.push(RoleEdge {
            from_node: p,
            from_field,
            to_node: idx,
            to_port: aggregate_group_key_port_id(group_key),
            kind: FieldEdgeKind::GroupBy,
        });
    }
}

/// Stamp carried metadata onto pass-through rows.
///
/// Normal pass-through rows use exact field names. Aggregate group keys may be
/// authored as qualified refs (`source.field`) while the producer row is bare
/// (`field`), so aggregate callers can also fall back to the final segment.
fn stamp_passthrough_metadata(
    rows: &mut [FieldRow],
    col_meta: &std::collections::HashMap<String, ColMeta>,
    allow_bare_fallback: bool,
) {
    for row in rows {
        if !matches!(row.kind, FieldKind::PassThrough) {
            continue;
        }
        let meta = col_meta.get(&row.name).or_else(|| {
            allow_bare_fallback
                .then(|| col_meta.get(bare_column(&row.name)))
                .flatten()
        });
        if let Some(meta) = meta {
            row.ty = meta.ty.clone();
            row.is_correlation_key = meta.is_ck;
        }
    }
}

/// Mark every row of a node `Unknown` precision because the node's CXL failed
/// [`field_lineage::parse_clean`] (#148).
///
/// A parse failure suppresses the node's lineage edges entirely (a garbled AST
/// yields wrong edges, worse than none), so there is no edge to carry the
/// degradation — it lives on the rows. The shared reason makes the Inspector badge
/// explain *why* provenance is missing rather than silently showing an empty
/// trace. Applied to BOTH lineage paths' parse-fail arms so the two stay aligned.
fn mark_rows_unknown(rows: &mut [FieldRow]) {
    for row in rows {
        row.lineage_precision = Precision::Unknown;
        row.precision_reason = "CXL did not parse; lineage edges suppressed";
    }
}

/// Fold each output row's lineage precision from the edges that PRODUCE it (#148).
///
/// Run as a post-pass once every node's edges exist. A row already marked
/// [`Precision::Unknown`] by [`mark_rows_unknown`] (its node's CXL failed to parse)
/// keeps that verdict — there is no incident edge to reconsider. Every other row's
/// precision is the WORST tier among the edges arriving INTO it (`(to_node,
/// to_field)`), defaulting to [`Precision::Exact`] when no degraded edge feeds it.
///
/// Only the CONSUMER (`to`) side is folded, deliberately: a row's precision
/// describes how faithfully *its own* provenance is known, which is decided by the
/// edges that produce/influence it. A clean source column that merely FEEDS a
/// downstream filter is itself exact — the approximation is in how it is used
/// downstream, and that degradation correctly lands on the downstream consumer
/// row (which is the `to` endpoint there), not back on the pristine producer.
fn derive_row_precision(out_fields: &mut [Vec<FieldRow>], edges: &[FieldEdge]) {
    // Per (node, field) worst producing-edge precision, folded in one pass over
    // edges so the cost is O(edges) rather than O(rows × edges). The `&'static str`
    // reason is copied from the edge whose precision currently wins, so the worst
    // tier's explanation is kept; no allocation, since the reason is static.
    let mut worst: std::collections::HashMap<(usize, &str), (Precision, &'static str)> =
        std::collections::HashMap::new();
    for edge in edges {
        let entry = worst
            .entry((edge.to_node, edge.to_field.as_str()))
            .or_insert((Precision::Exact, edge.precision_reason));
        // `worst` keeps the less-precise tier; update the stored reason only when
        // this edge is strictly worse so the winning tier's explanation survives.
        if edge.precision.worst(entry.0) != entry.0 {
            *entry = (edge.precision, edge.precision_reason);
        }
    }
    for (node, rows) in out_fields.iter_mut().enumerate() {
        for row in rows.iter_mut() {
            // An `Unknown` row (its node's CXL failed to parse) keeps that verdict:
            // there is no incident edge to reconsider.
            if row.lineage_precision == Precision::Unknown {
                continue;
            }
            if let Some((precision, reason)) = worst.get(&(node, row.name.as_str())) {
                row.lineage_precision = *precision;
                // `Exact` carries no degraded reason, so clear any stale one; a
                // degraded tier keeps its producing edge's explanation.
                row.precision_reason = if *precision == Precision::Exact {
                    ""
                } else {
                    *reason
                };
            }
        }
    }
}

/// The output-name gate a stage applies to its emit targets and carried columns
/// (#180).
///
/// - `Resolved(set)` — the engine-resolved (or body) row set is authoritative, so
///   an emit/carry is drawn ONLY into a column the engine actually produced
///   (`output_names.contains(col)`), and the intra-node chained-emit fallback is
///   likewise gated. A target/column the engine dropped draws no edge.
/// - `Unfiltered` — the raw path builds its rows from
///   [`field_lineage::transform_output_fields`], so every emit target and every
///   unshadowed input column is an output column by construction; no gate is
///   needed and none is applied (matching the raw path's original behavior, which
///   had no `output_names` check at all).
enum OutputGate<'a> {
    Resolved(&'a std::collections::HashSet<String>),
    Unfiltered,
}

impl OutputGate<'_> {
    /// Whether `col` is an output column this stage may draw an edge into. The
    /// `Unfiltered` raw path admits every column (its rows are emit-derived by
    /// construction); the resolved/body path admits only engine-produced columns.
    fn admits(&self, col: &str) -> bool {
        match self {
            OutputGate::Resolved(names) => names.contains(col),
            OutputGate::Unfiltered => true,
        }
    }
}

/// What drives a stage's emit/anchor analysis in [`field_edges_for_stage`] (#180).
///
/// The top-level paths feed a `cxl::ast::Program` directly; the composition body's
/// Aggregate has no plan-node `Program` — its emit RHS was rewritten into a
/// [`CompiledAggregate`] residual at compile time (every `AggCall` → `Expr::AggSlot`,
/// every group-by `FieldRef` → `Expr::GroupKey`), which `emit_supports` cannot
/// recover. So an Aggregate supplies its per-emit support set PRECOMPUTED (via
/// [`aggregate_emit_supports`]) and the analyzer runs the SAME derive/carry loop on
/// it — keeping the analyzer single-sourced rather than forking a second emit loop.
///
/// - `Cxl(p)` — run the full `emit_supports` / `emit_copy_targets` /
///   `emit_each_fanned_targets` analysis on the program (Transform).
/// - `Aggregate(supports)` — use the precomputed `(target, support)` pairs.
///   An aggregate emit is always a value DERIVE (it folds the group), never an
///   identity copy and never `emit each`-fanned, so its copy/fanned sets are empty.
/// - `None` — no program: only the same-name carry fallback runs (a CXL-less /
///   unresolved node forwarding columns).
enum StageProgram<'a> {
    Cxl(&'a cxl::ast::Program),
    Aggregate(Vec<(String, std::collections::HashSet<String>)>),
    None,
}

/// The single emit/anchor classification core for ONE consumer stage, shared by
/// the raw ([`compute_field_lineage`]), engine-resolved
/// ([`resolved_pipeline_field_lineage`]), and composition-body ([`body_field_edges`])
/// lineage paths (#180).
///
/// This is the behavior-preserving extraction of the field-edge skeleton that was
/// triplicated across those three paths: `emit_supports` → `emit_copy_targets` /
/// `used_cols` / `carry_kind` → derive/carry (`mk` closure + `emitted_so_far`
/// intra-node chained-emit fallback) → identity-carry loop, plus the no-program
/// same-name carry fallback. The caller keeps its OWN predecessor fold, row
/// production, Composition special-casing, `col_meta` stamping, parse-error
/// (`mark_rows_unknown`) arm, and post-row influence emission; only the
/// emit/anchor core moves here.
///
/// Edges are pushed onto `acc` (DIRECT derive/carry edges via `push_direct`, which
/// the construction guarantees cannot collide; group-by influence edges via the
/// shared [`emit_group_by_edges`], which dedups). Emission order is fixed
/// group-by → emit-supports → identity-carry so the resulting edge list is
/// byte-identical to each path's prior inline order.
///
/// Parameters that capture every known difference between the three paths:
/// - `to` — the consumer stage/node/slot index in the caller's index space.
/// - `program` — [`StageProgram::Cxl`] runs the full emit/anchor analysis;
///   [`StageProgram::Aggregate`] runs that same analysis over precomputed aggregate
///   supports; [`StageProgram::None`] runs only the same-name carry fallback (a
///   no-CXL / unresolved node forwarding columns).
/// - `input_cols` / `producers_of` — the caller's predecessor fold (ordered column
///   union and per-column producer indices).
/// - `output_gate` — `Resolved` gates emits/carries on the engine row set;
///   `Unfiltered` admits everything (the raw path's emit-derived rows).
/// - `aliases` — input-port alias → producer-index map for alias-qualified Combine
///   refs (empty for every non-Combine node and for the body path).
/// - `group_keys` — Aggregate group-key columns: a `GroupBy` influence edge (plus
///   its role edge) is emitted for each, and they are skipped by the emit/carry
///   loops (a group key defines grain, it is not a value derive). The top-level and
///   body Aggregate paths populate it; empty for every non-Aggregate node.
///
/// A no-program multi-producer (Merge/Combine join-key) fan-in carry is always
/// graded conservatively ([`Precision::Approximate`] via
/// [`FieldEdge::conservative_fan_in`]) on every path — top-level raw/resolved AND the
/// composition body (#180 GAP 3) — while a single-producer carry stays an exact
/// `Passthrough`.
#[allow(clippy::too_many_arguments)]
fn field_edges_for_stage(
    acc: &mut EdgeAccumulator,
    role_edges: &mut Vec<RoleEdge>,
    to: usize,
    program: StageProgram<'_>,
    input_cols: &[String],
    producers_of: &std::collections::HashMap<String, Vec<usize>>,
    output_gate: &OutputGate<'_>,
    aliases: &std::collections::HashMap<String, usize>,
    group_keys: &std::collections::HashSet<String>,
) {
    // Resolve the variant to its `(supports, copies, fanned)` triple. A `Cxl`
    // program runs the full emit analysis; an `Aggregate` supplies its supports
    // precomputed (its emits are pure derives, so copies/fanned are empty); a
    // `None` program runs only the same-name carry fallback below. The triple's
    // type is fixed by the `Cxl` arm (`Vec<(String, HashSet<String>)>` + two
    // `HashSet<String>`s); the `Aggregate` arm's empty sets unify against it.
    let (supports, copies, fanned) = match program {
        StageProgram::None => {
            // No (resolved) CXL program: the node forwards its input columns. Each
            // surfaced column carries from every producer of it (#67 fan-in). A
            // multi-producer carry rides in from a Merge/Combine fan-in with no CXL
            // to confirm it passes through unchanged, so it is graded `Approximate`;
            // a single-producer carry is an exact `Passthrough`.
            for col in input_cols {
                if !output_gate.admits(col) {
                    continue;
                }
                if let Some(producers) = producers_of.get(col) {
                    let conservative = producers.len() > 1;
                    for &p in producers {
                        let edge = if conservative {
                            FieldEdge::conservative_fan_in(p, col.clone(), to, col.clone())
                        } else {
                            FieldEdge::carry(
                                p,
                                col.clone(),
                                to,
                                col.clone(),
                                FieldEdgeKind::Passthrough,
                            )
                        };
                        acc.push_direct(edge);
                    }
                }
            }
            return;
        }
        StageProgram::Cxl(program) => (
            field_lineage::emit_supports(program),
            field_lineage::emit_copy_targets(program, input_cols),
            field_lineage::emit_each_fanned_targets(program),
        ),
        // An aggregate emit folds the group, so it is always a value DERIVE — never
        // an identity copy and never `emit each`-fanned (the extractor rejects
        // `emit each` inside an aggregate body). Empty copy/fanned sets reproduce
        // that classification through the shared loop.
        StageProgram::Aggregate(supports) => (
            supports,
            std::collections::HashSet::new(),
            std::collections::HashSet::new(),
        ),
    };

    // Group keys define the grouped-record grain — an INDIRECT GROUP_BY influence,
    // not a value derive (#147), always Approximate (#148). Emitted first (matching
    // each path's prior order) so the field + role edges land before the emit
    // analysis. Empty for every non-Aggregate node, so this is a no-op there.
    for group_key in group_keys {
        if !output_gate.admits(group_key) {
            continue;
        }
        emit_group_by_edges(acc, role_edges, to, group_key, producers_of, aliases);
    }

    let emitted: std::collections::HashSet<&str> =
        supports.iter().map(|(name, _)| name.as_str()).collect();
    // Columns read by a COMPUTED or renamed emit (a pure copy excluded): a same-name
    // carry of one is an `Access` carry rather than a pure `Passthrough` (#72). Keyed
    // by bare column so a qualified `orders.line_total` read and a bare `line_total`
    // carry speak the same vocabulary.
    let used_cols: std::collections::HashSet<&str> = supports
        .iter()
        .filter(|(target, _)| !copies.contains(target.as_str()))
        .flat_map(|(_, support)| support.iter().map(|m| bare_column(m)))
        .collect();
    let carry_kind = |col: &str| {
        if used_cols.contains(col) {
            FieldEdgeKind::Access
        } else {
            FieldEdgeKind::Passthrough
        }
    };
    let mut emitted_so_far: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for (target, support) in &supports {
        // Skip a target the engine did not produce or a group key (its grain edge is
        // the GroupBy influence above, not a value derive); still record it as
        // emitted so a later chained emit reading it resolves intra-node.
        if !output_gate.admits(target) || group_keys.contains(target.as_str()) {
            emitted_so_far.insert(target.as_str());
            continue;
        }
        // An identity-copy emit carries its column unchanged (carry); a computed or
        // renamed emit derives its target.
        let carried = copies.contains(target.as_str());
        let target_fanned = fanned.contains(target.as_str());
        for member in support {
            // Resolve the (possibly alias-qualified) support member to its bare column
            // on the right predecessor(s); `col` is the bare name so the carry/Access
            // split and the edge's `from_field` both read the column, not the dotted ref.
            let col = bare_column(member);
            // A carry edge is Exact (identity reproduces the value); a derive edge is
            // Exact unless fanned by `emit each` (#148).
            let mk = |p: usize, from_field: String, consumer: usize| {
                if carried {
                    FieldEdge::carry(p, from_field, consumer, target.clone(), carry_kind(col))
                } else {
                    FieldEdge::derive(p, from_field, consumer, target.clone(), target_fanned)
                }
            };
            let anchors = resolve_support_anchors(member, producers_of, aliases);
            if !anchors.is_empty() {
                // A support column present in several inputs derives its target from
                // each of them (#67); an alias-qualified ref resolves to its single
                // declared port.
                for (p, from_field) in anchors {
                    acc.push_direct(mk(p, from_field, to));
                }
            } else if emitted_so_far.contains(col) && output_gate.admits(col) {
                // An intra-node chained emit (`emit d = c + 1` after the node emitted
                // `c`) derives from the earlier emit on the same card.
                acc.push_direct(mk(to, col.to_string(), to));
            }
        }
        emitted_so_far.insert(target.as_str());
    }

    // Identity edges: each input column carried through unchanged (surfaced on the
    // consumer, not shadowed by an emit, not a group key). A column carried in from
    // several inputs (a fan-in join key) carries from each (#67). A clean-CXL identity
    // carry is Exact.
    for col in input_cols {
        if output_gate.admits(col)
            && !emitted.contains(col.as_str())
            && !group_keys.contains(col.as_str())
            && let Some(producers) = producers_of.get(col)
        {
            let kind = carry_kind(col);
            for &p in producers {
                acc.push_direct(FieldEdge::carry(p, col.clone(), to, col.clone(), kind));
            }
        }
    }
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
/// output record of index `u`; `role_edges` names dependencies into semantic
/// role input ports. Slots MUST be ordered so every node appears after its
/// predecessors (topological): both callers pass declaration order, which is
/// topological for the DAGs they build.
///
/// Per-node rules (Phase 1, transforms-precise):
/// - **Origin slot**: its pre-seeded rows verbatim, no edges.
/// - **Aggregate node**: group-key rows, then aggregate emit rows. Group-key and
///   aggregate expression supports produce derive edges; unrelated input columns
///   do not pass through.
/// - **Other node with parseable CXL**: passthrough rows for input columns not
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
///
/// Alias-qualified Combine refs: a Combine body names columns through its
/// input-port aliases (`emit name = orders.order_id`), which `support_into`
/// reports as the dotted string `orders.order_id`. `input_aliases[u]` maps index
/// `u`'s port aliases to predecessor indices so each qualified ref resolves to
/// the column on EXACTLY that port (the bare-keyed `producers_of` alone cannot
/// disambiguate a column shared by two inputs). It is empty for every non-Combine
/// node, whose bare / single-source refs resolve by bare-column fallback.
fn compute_field_lineage(
    slots: &[LineageSlot<'_>],
    predecessors: &[Vec<usize>],
    input_aliases: &[std::collections::HashMap<String, usize>],
) -> (Vec<Vec<FieldRow>>, Vec<FieldEdge>, Vec<RoleEdge>) {
    debug_assert_eq!(slots.len(), input_aliases.len());
    let total = slots.len();
    debug_assert_eq!(total, predecessors.len());
    let mut out_fields: Vec<Vec<FieldRow>> = vec![Vec::new(); total];
    // Edge list + its O(1) dedup index (see [`EdgeAccumulator`]). The predicate
    // fan pushes through `push_deduped`; DIRECT derive/carry edges that cannot
    // collide by construction push through `push_direct` (bypassing dedup).
    let mut acc = EdgeAccumulator::new();
    let mut role_edges: Vec<RoleEdge> = Vec::new();

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

        // The raw Aggregate arm keeps its own emit loop rather than routing through
        // the shared `field_edges_for_stage` analyzer (#180): it emits a `derive`
        // for EVERY aggregate emit (it never computes `emit_copy_targets`), whereas
        // the shared analyzer — and the engine-resolved path — would reclassify an
        // identity-copy emit as a `carry`. Folding this arm in would therefore change
        // raw-mode edges for an aggregate that copies an input column, so it stays
        // inline to keep Phase A behavior-preserving. Reconciling the raw/resolved
        // aggregate emit classification is the Aggregation adapter (GAP 1, #180 PR2).
        // Its group-key grain edges DO use the shared `emit_group_by_edges` helper.
        if let PipelineNode::Aggregate { config, .. } = node {
            match node_cxl(node).map(field_lineage::parse_clean) {
                Some(Some(program)) => {
                    out_fields[idx] =
                        field_lineage::aggregate_output_fields(&config.group_by, &program);
                    let output_names: std::collections::HashSet<&str> = out_fields[idx]
                        .iter()
                        .map(|row| row.name.as_str())
                        .collect();
                    let group_key_names: std::collections::HashSet<&str> = out_fields[idx]
                        .iter()
                        .filter(|row| matches!(row.kind, FieldKind::PassThrough))
                        .map(|row| row.name.as_str())
                        .collect();

                    // Group keys are output fields on the grouped record. They
                    // derive from the matching input field(s), but the rest of
                    // the input row does not pass through the aggregate.
                    let aliases = &input_aliases[idx];
                    let group_keys: Vec<String> = out_fields[idx]
                        .iter()
                        .filter(|row| matches!(row.kind, FieldKind::PassThrough))
                        .map(|row| row.name.clone())
                        .collect();
                    // Group keys define the grouped-record grain — an INDIRECT
                    // GROUP_BY influence, not a value derive (#147), always
                    // Approximate (#148). This is the single honest representation
                    // of the post-Aggregate grain (it retired the duplicate
                    // `is_aggregate_grain` row flag). Emitted via the shared helper
                    // so both lineage paths cannot drift.
                    for group_key in group_keys {
                        emit_group_by_edges(
                            &mut acc,
                            &mut role_edges,
                            idx,
                            &group_key,
                            &producers_of,
                            aliases,
                        );
                    }

                    let supports = field_lineage::emit_supports(&program);
                    // An aggregate emit fanned out by `emit each` loses per-element
                    // provenance, so its derive edges are Approximate (#148).
                    let fanned = field_lineage::emit_each_fanned_targets(&program);
                    let mut emitted_so_far: std::collections::HashSet<&str> =
                        std::collections::HashSet::new();
                    for (target, support) in &supports {
                        if !output_names.contains(target.as_str())
                            || group_key_names.contains(target.as_str())
                        {
                            continue;
                        }
                        let target_fanned = fanned.contains(target.as_str());
                        for member in support {
                            let col = bare_column(member);
                            let anchors = resolve_support_anchors(member, &producers_of, aliases);
                            if !anchors.is_empty() {
                                for (p, from_field) in anchors {
                                    acc.push_direct(FieldEdge::derive(
                                        p,
                                        from_field,
                                        idx,
                                        target.clone(),
                                        target_fanned,
                                    ));
                                }
                            } else if emitted_so_far.contains(col) && output_names.contains(col) {
                                acc.push_direct(FieldEdge::derive(
                                    idx,
                                    col.to_string(),
                                    idx,
                                    target.clone(),
                                    target_fanned,
                                ));
                            }
                        }
                        emitted_so_far.insert(target.as_str());
                    }
                }
                Some(None) => {
                    // CXL present but it failed to parse: group keys are
                    // config-derived and safe to show, but edges are suppressed —
                    // a garbled AST can't be trusted to infer lineage. With no edge
                    // to annotate, the degradation lives on the rows as `Unknown`
                    // precision (#148).
                    out_fields[idx] =
                        field_lineage::aggregate_group_key_output_fields(&config.group_by);
                    mark_rows_unknown(&mut out_fields[idx]);
                }
                None => {
                    // No CXL block at all (not a parse failure): show the
                    // config-derived group keys; edges are skipped because emit
                    // analysis is unavailable, but precision stays at the default
                    // Exact — there is no degraded inference, just absent CXL.
                    out_fields[idx] =
                        field_lineage::aggregate_group_key_output_fields(&config.group_by);
                }
            }

            stamp_passthrough_metadata(&mut out_fields[idx], &col_meta, true);
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
                // Output rows: passthrough (unshadowed inputs) then emitted. The
                // emit/anchor edge core is the shared stage analyzer (#180): the raw
                // path's rows are emit-derived by construction, so it gates nothing
                // (`OutputGate::Unfiltered`) and carries nothing through a group key
                // (this is the non-Aggregate arm).
                out_fields[idx] = field_lineage::transform_output_fields(&input_cols, &program);
                field_edges_for_stage(
                    &mut acc,
                    &mut role_edges,
                    idx,
                    StageProgram::Cxl(&program),
                    &input_cols,
                    &producers_of,
                    &OutputGate::Unfiltered,
                    &input_aliases[idx],
                    &std::collections::HashSet::new(),
                );
            }
            Some(None) => {
                // CXL present but it failed to parse: render passthrough rows so
                // the card still shows its shape, but emit NO lineage edges — a
                // garbled AST can't be trusted to compute lineage. With no edge to
                // carry the degradation, the rows are marked `Unknown` (#148).
                out_fields[idx] = field_lineage::passthrough_output_fields(&input_cols);
                mark_rows_unknown(&mut out_fields[idx]);
            }
            None => {
                // No CXL block at all: the node forwards its input columns. The
                // shared analyzer's no-program fallback draws the same-name carries,
                // grading a multi-producer (Merge/Combine) fan-in conservatively
                // (#67/#148). The raw path applies no output gate.
                out_fields[idx] = field_lineage::passthrough_output_fields(&input_cols);
                field_edges_for_stage(
                    &mut acc,
                    &mut role_edges,
                    idx,
                    StageProgram::None,
                    &input_cols,
                    &producers_of,
                    &OutputGate::Unfiltered,
                    &input_aliases[idx],
                    &std::collections::HashSet::new(),
                );
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
        stamp_passthrough_metadata(&mut out_fields[idx], &col_meta, false);

        // INDIRECT influence edges (#147): Route's `Conditional` and Cull's
        // `Filter` predicate edges and Combine's `JoinKey` `where_expr` edges,
        // added AFTER the node's DIRECT rows so they land on the real surviving
        // output rows. Aggregate `group_by` GroupBy edges are emitted inline in
        // the Aggregate arm above (which `continue`s before reaching here). A
        // Merge is a row UNION and contributes none.
        emit_indirect_influence_edges(
            &mut acc,
            idx,
            node,
            &out_fields[idx],
            &producers_of,
            &input_aliases[idx],
        );
    }

    // Fold each row's precision from the edges now incident to it (#148), once all
    // edges exist. Rows already marked `Unknown` (parse-fail nodes) are preserved.
    derive_row_precision(&mut out_fields, &acc.edges);

    (out_fields, acc.edges, role_edges)
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
) -> (Vec<Vec<FieldRow>>, Vec<FieldEdge>, Vec<RoleEdge>) {
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
            ..Default::default()
        }]));
    }

    // Per-slot input-port alias → unified-index map for body Combine nodes
    // (#67 qualified refs). The producer name space is the same one the caller
    // wired predecessors over: input-port names occupy `[0, n_in)`, body node
    // names `[n_in, n_in+n_body)`. Ports and output slots carry no aliases.
    let mut name_to_idx: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for (i, port) in sig.inputs.keys().enumerate() {
        name_to_idx.insert(port.to_string(), i);
    }
    for (j, spanned) in body.iter().enumerate() {
        name_to_idx.insert(spanned.value.name().to_string(), n_in + j);
    }
    // Only body slots carry aliases; ports and output slots stay empty. Mirrors
    // the `node_branches` build above (vec sized over `total`, body slots filled
    // by an indexed loop) rather than a per-index range guard.
    let mut input_aliases: Vec<std::collections::HashMap<String, usize>> =
        vec![std::collections::HashMap::new(); total];
    for (bi, spanned) in body.iter().enumerate() {
        input_aliases[n_in + bi] = node_input_aliases(&spanned.value, &name_to_idx);
    }

    let (mut out_fields, mut field_edges, role_edges) =
        compute_field_lineage(&slots, predecessors, &input_aliases);

    // Producer → output-port field edge, one per output port. The output port
    // surfaces its producer body node's records; on the canvas we draw a single
    // cable from the producer's representative field to the port's named field.
    // The representative field is the producer's same-named column if it has one
    // (a clean 1:1 carry → passthrough), else the producer's LAST output field (a
    // rename → derive). A port with no resolved producer, or a producer with no
    // fields, contributes no edge (graceful, never a panic).
    //
    // These edges are appended AFTER `compute_field_lineage` already ran its own
    // `derive_row_precision`, so the output-port rows have not yet folded in any
    // degradation arriving along these edges (#154). The producer's representative
    // ROW already carries its folded precision (`compute_field_lineage` ran the
    // fold), so when that row is degraded the producer→port edge carries the same
    // degradation, and the port row re-folds to reflect it via the `derive_row_precision`
    // re-run below.
    for (oi, port) in sig.outputs.keys().enumerate() {
        let out_idx = n_in + n_body + oi;
        let Some(&producer) = predecessors[out_idx].first() else {
            continue;
        };
        let producer_row = out_fields[producer]
            .iter()
            .find(|r| r.name == *port)
            .or_else(|| out_fields[producer].last());
        let Some(producer_row) = producer_row else {
            continue;
        };
        let producer_field = producer_row.name.clone();
        // A degraded producer column (Approximate/Unknown) makes its port crossing
        // approximate; a clean producer column keeps the port edge Exact.
        let producer_degraded = producer_row.lineage_precision != Precision::Exact;
        // A same-named port surfaces its producer column unchanged (a pure
        // pass-through — a port reads nothing, so never an Access carry); a
        // differently-named port is a clean rename → derive.
        let mut edge = if producer_field == *port {
            FieldEdge::carry(
                producer,
                producer_field,
                out_idx,
                port.to_string(),
                FieldEdgeKind::Passthrough,
            )
        } else {
            FieldEdge::derive(producer, producer_field, out_idx, port.to_string(), false)
        };
        // Propagate the producer column's degradation onto the port crossing
        // (precision is orthogonal to edge identity, so this never splits an edge).
        if producer_degraded {
            edge.precision = producer_row.lineage_precision;
            edge.precision_reason = "composition output port surfaces a degraded producer column";
        }
        field_edges.push(edge);
    }

    // Re-fold output-port row precision over the appended producer→port edges so a
    // degraded producer column degrades the port row it feeds — the fold that ran
    // inside `compute_field_lineage` could not see these later-appended edges
    // (#154). Re-running over the full edge set is idempotent for rows whose
    // incident edges are unchanged, so only the output-port rows are updated.
    derive_row_precision(&mut out_fields, &field_edges);

    (out_fields, field_edges, role_edges)
}

/// Build a pipeline's classified slot list and run the shared lineage core.
///
/// Slots are parallel to `nodes` (declaration order). A [`PipelineNode::Source`]
/// becomes a [`LineageSlot::Origin`] seeded from its declared `schema.columns`
/// (the pipeline analogue of a composition input port); every other node becomes
/// a [`LineageSlot::Node`] analyzed as a transform. `predecessors` is the
/// node-index relation the caller already built.
///
/// SCOPE: this RAW (CXL-only) path materializes NO composition boundary edges
/// (#154). Boundary crossings + synthesized composition output rows require the
/// compiled `BoundBody` (output-port rows, body assignments), which exists only in
/// the resolved path ([`resolved_pipeline_field_lineage`]); raw mode has no compiled
/// plan. A Composition here is analyzed as an ordinary transform with no CXL — that
/// is intended, not an omission.
fn pipeline_field_lineage(
    nodes: &[Spanned<PipelineNode>],
    predecessors: &[Vec<usize>],
) -> (Vec<Vec<FieldRow>>, Vec<FieldEdge>, Vec<RoleEdge>) {
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

    // Per-node input-port alias → node-index map, so a Combine body's
    // alias-qualified CXL refs (`orders.order_id`) resolve to the right
    // predecessor. Rebuilds the same name→index relation `derive_view_from_nodes`
    // used to wire predecessors; empty for every non-Combine node.
    let name_to_idx: std::collections::HashMap<String, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, spanned)| (spanned.value.name().to_string(), i))
        .collect();
    let input_aliases: Vec<std::collections::HashMap<String, usize>> = nodes
        .iter()
        .map(|spanned| node_input_aliases(&spanned.value, &name_to_idx))
        .collect();

    compute_field_lineage(&slots, predecessors, &input_aliases)
}

/// Engine-resolved top-level field rows plus conservative lineage edges.
///
/// The compiled plan's [`cxl::typecheck::Row`] is the source of truth for the
/// rows and compact datatype labels shown in Resolved mode. The existing CXL
/// support analysis still supplies edge intent, but every edge is gated on the
/// resolved endpoint fields so a stale approximation cannot draw cables to rows
/// the engine did not produce.
fn resolved_pipeline_field_lineage(
    nodes: &[Spanned<PipelineNode>],
    plan: &clinker_plan::plan::CompiledPlan,
    predecessors: &[Vec<usize>],
) -> (Vec<Vec<FieldRow>>, Vec<FieldEdge>, Vec<RoleEdge>) {
    let total = nodes.len();
    let name_to_idx: std::collections::HashMap<String, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, spanned)| (spanned.value.name().to_string(), i))
        .collect();
    let input_aliases: Vec<std::collections::HashMap<String, usize>> = nodes
        .iter()
        .map(|spanned| node_input_aliases(&spanned.value, &name_to_idx))
        .collect();

    let mut out_fields: Vec<Vec<FieldRow>> = vec![Vec::new(); total];
    // Edge list + its O(1) dedup index (see [`EdgeAccumulator`]). Mirrors the raw
    // builder (`compute_field_lineage`) so both paths dedup identically.
    let mut acc = EdgeAccumulator::new();
    let mut role_edges: Vec<RoleEdge> = Vec::new();

    for (idx, spanned) in nodes.iter().enumerate() {
        let node = &spanned.value;

        let mut input_cols: Vec<String> = Vec::new();
        let mut producers_of: std::collections::HashMap<String, Vec<usize>> =
            std::collections::HashMap::new();
        let mut col_meta: std::collections::HashMap<String, ColMeta> =
            std::collections::HashMap::new();
        for &p in &predecessors[idx] {
            for row in &out_fields[p] {
                let producers = producers_of.entry(row.name.clone()).or_default();
                let meta = col_meta.entry(row.name.clone()).or_default();
                if producers.is_empty() {
                    meta.ty = row.ty.clone();
                    input_cols.push(row.name.clone());
                }
                meta.is_ck |= row.is_correlation_key;
                if !producers.contains(&p) {
                    producers.push(p);
                }
            }
        }

        let parsed = node_cxl(node).and_then(field_lineage::parse_clean);
        let supports = parsed
            .as_ref()
            .map(field_lineage::emit_supports)
            .unwrap_or_default();
        let emitted: std::collections::HashSet<String> =
            supports.iter().map(|(name, _)| name.clone()).collect();
        let copies = parsed
            .as_ref()
            .map(|program| field_lineage::emit_copy_targets(program, &input_cols))
            .unwrap_or_default();

        // Composition boundary handling (#154): a Composition node has no CXL of
        // its own, so the engine puts no entry in `artifacts.typed` and
        // `typed_output_row` returns None — its output columns are otherwise lost
        // in this view and the value chain would skip straight from the upstream
        // producer to the downstream consumer, never passing through the
        // composition (which would block #155's descent). So we:
        //   1. SYNTHESIZE the composition's output rows from the body's declared
        //      output-port columns, set into `out_fields[idx]` here (before the
        //      typed-row `continue`) so the DOWNSTREAM node — processed later in
        //      topological order — sees them as real producer columns and draws
        //      normal `comp.col → consumer.col` carry/derive edges automatically.
        //      Those `comp → consumer` edges are re-tagged `Boundary` in a
        //      post-loop pass (they ARE the OUTPUT boundary crossings).
        //   2. Emit the INPUT boundary edges (outer producer column → synthetic
        //      `\u{2190}in:port:col` marker on `comp`), reading the predecessors'
        //      already-computed output rows (topological order guarantees they
        //      exist) and the bound body.
        if let PipelineNode::Composition { inputs, .. } = node {
            // Resolve the bound body ONCE (the resolved view is rebuilt on every
            // render, so the duplicate `composition_body_assignments` lookups across
            // synthesize + emit-input were pure waste). A missing assignment or a
            // `body_of` miss degrades gracefully: the composition still renders, just
            // with no synthesized rows and no boundary edges.
            if let Some(body) = plan
                .artifacts()
                .composition_body_assignments
                .get(node.name())
                .and_then(|&body_id| plan.body_of(body_id))
            {
                out_fields[idx] = synthesize_composition_output_rows(body);
                emit_composition_input_boundary_edges(
                    &mut acc,
                    idx,
                    body,
                    inputs,
                    &name_to_idx,
                    &out_fields,
                );
            }
            // A Composition has no typed_output_row of its own; its rows are the
            // synthesized ones above and it runs no transform/emit analysis. Skip
            // the rest of the per-node body (it is keyed on `typed_output_row`).
            continue;
        }

        let Some(row) = plan.typed_output_row(node.name()) else {
            continue;
        };
        out_fields[idx] =
            resolved_row_fields(row, node, &producers_of, &col_meta, &emitted, &copies);
        let output_names: std::collections::HashSet<String> =
            out_fields[idx].iter().map(|row| row.name.clone()).collect();
        let aggregate_group_keys: std::collections::HashSet<String> = match node {
            PipelineNode::Aggregate { config, .. } => config.group_by.iter().cloned().collect(),
            _ => std::collections::HashSet::new(),
        };

        if let Some(program) = &parsed {
            // The shared stage analyzer (#180): the engine row set is authoritative,
            // so emits/carries are gated on `output_names` (`OutputGate::Resolved`)
            // and the aggregate's group keys (skipped by the emit/carry loops, drawn
            // as `GroupBy` influence + role edges). The fan-in policy is unused on the
            // `Some(program)` arm. Aliases resolve a Combine body's qualified refs.
            field_edges_for_stage(
                &mut acc,
                &mut role_edges,
                idx,
                StageProgram::Cxl(program),
                &input_cols,
                &producers_of,
                &OutputGate::Resolved(&output_names),
                &input_aliases[idx],
                &aggregate_group_keys,
            );
        } else if node_cxl(node).is_none() && node_preserves_input_schema(node) {
            // No CXL: the shared analyzer's no-program fallback carries each
            // engine-produced input column, grading a multi-producer fan-in
            // conservatively (#67/#148).
            field_edges_for_stage(
                &mut acc,
                &mut role_edges,
                idx,
                StageProgram::None,
                &input_cols,
                &producers_of,
                &OutputGate::Resolved(&output_names),
                &input_aliases[idx],
                &aggregate_group_keys,
            );
        } else if parsed.is_none() && node_cxl(node).is_some() {
            // CXL present but it failed to parse: edges were suppressed (no derive
            // analysis), so the degradation lives on the rows as `Unknown` (#148).
            mark_rows_unknown(&mut out_fields[idx]);
        }

        // INDIRECT influence edges (#147): Route's `Conditional` and Cull's
        // `Filter` predicate edges and Combine's `JoinKey` `where_expr` edges,
        // added AFTER this node's DIRECT carries so they land on the real
        // surviving output rows. A Merge is a row UNION and contributes none.
        emit_indirect_influence_edges(
            &mut acc,
            idx,
            node,
            &out_fields[idx],
            &producers_of,
            &input_aliases[idx],
        );
    }

    // OUTPUT boundary crossings (#154): every edge LEAVING a Composition node is a
    // value crossing OUT of the composition wall, so re-tag it `Boundary`. The
    // downstream consumer drew these as ordinary carry/derive edges (it saw the
    // composition's synthesized output rows as producer columns); re-tagging marks
    // them as the "exit composition / descend here" crossings #155 consumes,
    // distinguished from intra-scope carries. Approximate-ness is preserved from the
    // original edge (a degraded producer column stays degraded across the wall).
    let composition_nodes: std::collections::HashSet<usize> = nodes
        .iter()
        .enumerate()
        .filter(|(_, s)| matches!(s.value, PipelineNode::Composition { .. }))
        .map(|(i, _)| i)
        .collect();
    for edge in &mut acc.edges {
        // A `\u{2190}in:` input-marker edge ENTERS a composition (its `to_node` is
        // the comp); it is already `Boundary` and must keep its kind. Only re-tag
        // edges whose SOURCE is a composition (a true exit crossing).
        if composition_nodes.contains(&edge.from_node) && edge.kind != FieldEdgeKind::Boundary {
            let approximate = edge.precision != Precision::Exact;
            *edge = FieldEdge::boundary(
                edge.from_node,
                edge.from_field.clone(),
                edge.to_node,
                edge.to_field.clone(),
                approximate,
            );
        }
    }

    // Fold each row's precision from its incident edges (#148); `Unknown` rows
    // (parse-fail nodes) are preserved.
    derive_row_precision(&mut out_fields, &acc.edges);

    (out_fields, acc.edges, role_edges)
}

/// Synthesize a Composition node's output [`FieldRow`]s from its bound body's
/// declared output ports (#154).
///
/// The engine gives a Composition no `typed_output_row` (it has no CXL of its own),
/// so in the resolved top-level view its output columns would otherwise be lost and
/// the value chain would bypass the composition entirely. We rebuild them from the
/// body's `output_port_rows`: the user-facing columns each port surfaces back to the
/// parent scope (engine-internal `$`-columns excluded via [`is_engine_internal_column`]),
/// unioned across ports in port-declaration order. Each row carries the engine's
/// compact type. The rows are classified [`FieldKind::PassThrough`] — a composition
/// surfaces its body's records to the parent unchanged at the boundary; it computes
/// nothing at this scope.
///
/// **Same-named columns across two output ports (FIDELITY #8):** dedup is by name,
/// first-seen winning. `output_port_rows` is an `IndexMap` in port-declaration
/// order, so "first-seen" is the FIRST declared port deterministically. Two ports
/// surfacing a same-named column therefore collapse to one row — a deliberate
/// limitation: a `(node, field)` endpoint must be unique for the lineage graph
/// (two rows with the same name would make the trace ambiguous), and multi-port
/// same-name fan-out is rare. #155's descent disambiguates which port a column
/// belongs to from the body itself.
///
/// **Row precision (FIDELITY #7):** synthesized rows are pinned [`Precision::Exact`].
/// The composition surfaces its body's records unchanged at THIS scope — the
/// crossing itself is exact. If a column is derived APPROXIMATELY inside the body
/// (e.g. an `emit each` fan-out), that body-internal degradation is a property of
/// the body's own lineage, which is surfaced by #155's descent INTO the body, not on
/// the comp's own row. Computing it here would require running full body lineage
/// (duplicative/expensive), so it is deliberately deferred to #155. (The comp→consumer
/// crossing's approximate flag is likewise taken from the consumer-side edge, which
/// has no body signal in this scope.)
///
/// #155 contract: given an output row `comp.col`, the body node + output port it
/// descends into is recoverable from `body.output_port_rows` (which port declares
/// `col`) and `body.output_port_to_node_idx` (that port's terminal body NodeIndex).
fn synthesize_composition_output_rows(
    body: &clinker_plan::plan::composition_body::BoundBody,
) -> Vec<FieldRow> {
    let mut rows: Vec<FieldRow> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for row in body.output_port_rows.values() {
        for (field, ty) in row.fields() {
            let col = field.name.as_ref();
            if is_engine_internal_column(col) || !seen.insert(col.to_string()) {
                continue;
            }
            rows.push(FieldRow {
                name: col.to_string(),
                kind: FieldKind::PassThrough,
                ty: Some(compact_type(ty)),
                ..Default::default()
            });
        }
    }
    rows
}

/// Materialize composition INPUT boundary edges (#154) for one Composition node.
///
/// In the resolved top-level view the engine gives a Composition no incoming field
/// edges and no input-port surface. This fills the INCOMING side with
/// [`FieldEdgeKind::Boundary`] crossings — an outer producer column entering an
/// input port — so the Inspector can mark "↳ enters composition X" and #155 can
/// recover the (port, column) the column flows into. The OUTGOING side is handled
/// separately: the composition's synthesized output rows let the downstream
/// consumer draw ordinary edges, which the caller re-tags `Boundary` post-loop.
///
/// It adds NO visible [`FieldRow`] to the composition for the input crossings —
/// each lands on a synthetic `\u{2190}in:port:col` marker field (see
/// [`field_lineage::composition_in_boundary_field`]) that can never collide with a
/// real output column.
///
/// **Port → outer-node binding (correctness):** each input port is wired to a
/// SPECIFIC outer node by the call-site `inputs:` map (`port name → outer node
/// name`, resolved to an index via `name_to_idx`). A port's columns bind to THAT
/// node's output rows only — not "any predecessor carrying the column name" — so two
/// input ports that share a column name never mis-bind to each other's producer.
///
/// Two behaviours:
/// - **Input crossing** (per input port × user column the bound producer surfaces):
///   a synthetic `\u{2190}in:port:col` marker edge from the bound producer (Exact).
///   Engine-internal `$`-prefixed columns are excluded via [`is_engine_internal_column`].
/// - **Degrade**: a port that binds no user column (its `inputs:` mapping is missing
///   / unresolvable, the bound producer lacks every declared column, or the port is
///   accept-any with no user columns) contributes to a single NODE-LEVEL boundary
///   connector (empty endpoint fields, [`Precision::Approximate`]). That connector is
///   emitted AT MOST ONCE per composition even when several ports degrade — duplicate
///   byte-identical node-level edges would otherwise accumulate (`push_direct` bypasses
///   dedup).
fn emit_composition_input_boundary_edges(
    acc: &mut EdgeAccumulator,
    comp_idx: usize,
    body: &clinker_plan::plan::composition_body::BoundBody,
    inputs: &indexmap::IndexMap<String, String>,
    name_to_idx: &std::collections::HashMap<String, usize>,
    out_fields: &[Vec<FieldRow>],
) {
    // A port degrades when its bound outer node can't be resolved or surfaces none
    // of its declared user columns. All degrading ports share ONE node-level
    // connector; record whether any degraded and the first available producer index
    // to anchor it on, then emit it once after the per-port loop.
    let mut any_port_degraded = false;
    let mut degrade_anchor: Option<usize> = None;

    // INPUT crossings: one synthetic marker per (port, user column the bound
    // producer surfaces). Engine-internal `$`-prefixed bookkeeping columns
    // ($widened, $source.*, $ck.*) are NOT part of the user-facing port crossing
    // contract #155 descends along, so they are excluded (broader than the
    // row-level `$ck.`-only filter).
    for (port, row) in &body.input_port_rows {
        // Resolve the outer node this port is wired to via the call-site `inputs:`
        // map; an unmapped / unresolvable port degrades.
        let producer = inputs
            .get(port)
            .and_then(|outer| name_to_idx.get(outer).copied());
        let Some(producer) = producer else {
            any_port_degraded = true;
            continue;
        };
        if degrade_anchor.is_none() {
            degrade_anchor = Some(producer);
        }

        // The bound producer's output columns, indexed once for O(1) membership
        // (replacing the former O(predecessors × rows) per-column scan).
        let producer_cols: std::collections::HashSet<&str> = out_fields
            .get(producer)
            .map(|rows| rows.iter().map(|r| r.name.as_str()).collect())
            .unwrap_or_default();

        let mut any_column_bound = false;
        for field in row.field_names() {
            let col = field.name.as_ref();
            if is_engine_internal_column(col) || !producer_cols.contains(col) {
                continue;
            }
            any_column_bound = true;
            acc.push_direct(FieldEdge::boundary(
                producer,
                col.to_string(),
                comp_idx,
                field_lineage::composition_in_boundary_field(port, col),
                false,
            ));
        }
        // A port whose bound producer surfaces none of its declared columns (or an
        // accept-any port with no user columns) records a degraded crossing.
        if !any_column_bound {
            any_port_degraded = true;
        }
    }

    // Emit the shared node-level degrade connector at most ONCE per composition.
    if any_port_degraded {
        let from = degrade_anchor.unwrap_or(comp_idx);
        acc.push_direct(FieldEdge::boundary(
            from,
            String::new(),
            comp_idx,
            String::new(),
            true,
        ));
    }
}

fn resolved_row_fields(
    row: &cxl::typecheck::Row,
    node: &PipelineNode,
    producers_of: &std::collections::HashMap<String, Vec<usize>>,
    col_meta: &std::collections::HashMap<String, ColMeta>,
    emitted: &std::collections::HashSet<String>,
    copies: &std::collections::HashSet<String>,
) -> Vec<FieldRow> {
    let ck_fields: std::collections::HashSet<&str> = match node {
        PipelineNode::Source { config: body, .. } => body
            .correlation_key
            .as_ref()
            .map(|ck| ck.fields().into_iter().collect())
            .unwrap_or_default(),
        _ => std::collections::HashSet::new(),
    };
    row.fields()
        .filter_map(|(field, ty)| {
            let name = field.name.as_ref();
            if is_internal_field_name(name) {
                return None;
            }
            let kind = if matches!(node, PipelineNode::Source { .. }) {
                FieldKind::Declared
            } else if copies.contains(name)
                || (!emitted.contains(name) && producers_of.contains_key(name))
            {
                FieldKind::PassThrough
            } else {
                FieldKind::Emitted
            };
            let is_correlation_key = match kind {
                FieldKind::Declared => ck_fields.contains(name),
                FieldKind::PassThrough => col_meta.get(name).is_some_and(|meta| meta.is_ck),
                FieldKind::Emitted => false,
            };
            Some(FieldRow {
                name: name.to_string(),
                kind,
                ty: Some(compact_type(ty)),
                is_correlation_key,
                ..Default::default()
            })
        })
        .collect()
}

fn is_internal_field_name(name: &str) -> bool {
    name.starts_with("$ck.")
}

/// Whether a column name is an engine-internal bookkeeping column rather than a
/// user-facing data column — any `$`-prefixed name (`$ck.<field>` correlation-key
/// shadows, `$widened`, `$source.*`, etc.). Used to keep composition boundary
/// crossings (#154) to the user-meaningful port contract #155 will descend along;
/// broader than [`is_internal_field_name`], which suppresses only `$ck.` rows.
fn is_engine_internal_column(name: &str) -> bool {
    name.starts_with('$')
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
        role_ports: Vec::new(),
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
        role_ports: Vec::new(),
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
            role_ports: Vec::new(),
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
                role_ports: Vec::new(),
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
                role_ports: Vec::new(),
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
                role_ports: Vec::new(),
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
            role_ports: Vec::new(),
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
                role_ports: Vec::new(),
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
            role_ports: Vec::new(),
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
            role_ports: Vec::new(),
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
            role_ports: Vec::new(),
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
            role_ports: Vec::new(),
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
            role_ports: Vec::new(),
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
                    role_ports: Vec::new(),
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
                    role_ports: Vec::new(),
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
                    role_ports: Vec::new(),
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
                    role_ports: Vec::new(),
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
                    role_ports: Vec::new(),
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
                    role_ports: Vec::new(),
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
        connection_paths: Vec::new(),
        field_edges: Vec::new(),
        field_edge_paths: Vec::new(),
        role_edges: Vec::new(),
        role_edge_paths: Vec::new(),
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
///
/// Field cables come from [`body_field_edges`], which is derive-aware (#174) and, after
/// #180 PR2, near top-level parity: a body Transform OR Aggregation computed column
/// draws a `Derive` cable to its producer column; a body Route/Cull draws
/// `Conditional`/`Filter` influence cables; an Aggregate group-by draws `GroupBy`
/// cables; and a multi-producer Merge fan-in grades the shared carry `Approximate`.
/// **Combine is not at parity** (clinker#621): a body Combine's computed columns and
/// join key are not reachable from a `BoundBody`, so they fall to the same-name carry
/// fallback.
pub fn derive_body_view(body: &clinker_plan::plan::composition_body::BoundBody) -> PipelineView {
    // The public drill-in canvas needs laid-out coordinates, so run the barycenter
    // pass. The signature and behavior are unchanged; the implementation now delegates
    // to a shared core that the lineage-only [`derive_body_scope`] path reuses WITHOUT
    // layout (#155 review item 4).
    let (view, _idx_to_slot) = build_body_view(body, true);
    view
}

/// Shared core for [`derive_body_view`] and [`derive_body_scope`]: build a composition
/// body's mini-DAG view (#155 review). Returns the view plus the NodeIndex→slot map so
/// the scope path can project the body's port→NodeIndex tables into slot space without
/// recomputing it.
///
/// `with_layout` gates the barycenter [`layout_positions`] pass. The drill-in canvas
/// needs the x/y coordinates (`true`); the Inspector's lineage trace reads only
/// `field_edges`, the rows, and the port→slot maps, so it skips the pass (`false`) —
/// `build_selected_inspector` is unmemoized and runs every inspector render, and the
/// layout cost is wasted there.
fn build_body_view(
    body: &clinker_plan::plan::composition_body::BoundBody,
    with_layout: bool,
) -> (
    PipelineView,
    std::collections::HashMap<petgraph::graph::NodeIndex, usize>,
) {
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
            fields: body
                .body_rows
                .get(plan_node.name())
                .map(body_row_fields)
                .unwrap_or_default(),
            branches: Vec::new(),
            role_ports: Vec::new(),
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

    let (field_edges, role_edges) = body_field_edges(body, &stages, &predecessors);

    if with_layout {
        // Body rows come from the engine's body-scoped output rows. Missing row data
        // keeps the classic [`NODE_HEIGHT`] and contributes no field edges. The
        // lineage-only path (`with_layout == false`) leaves every `canvas_x/y` at 0.0,
        // which the trace never reads.
        let heights: Vec<f32> = stages
            .iter()
            .map(|stage| row_stack_height(stage.fields.len()))
            .collect();
        let positions = layout_positions(&slot_cols, &predecessors, &heights);
        for (stage, (x, y)) in stages.iter_mut().zip(positions) {
            stage.canvas_x = x;
            stage.canvas_y = y;
        }
    }

    let view = PipelineView {
        stages,
        connections,
        connection_paths: Vec::new(),
        field_edges,
        field_edge_paths: Vec::new(),
        // Body role edges (Aggregate group-by role-port edges) are now surfaced on
        // the view rather than discarded (#180 GAP 2). They feed the Inspector's
        // role-usage list the same way the top-level paths' role edges do.
        role_edges,
        role_edge_paths: Vec::new(),
    };
    (view, idx_to_slot)
}

/// A composition body's own lineage view plus the port↔slot maps the Inspector's
/// scope-aware lineage trace (#155) needs to descend into and resurface from the
/// body.
///
/// The `view` is exactly what [`derive_body_view`] produces — the body's mini-DAG
/// in the body's own index space, where a stage's SLOT is its position in
/// `body.topo_order` (the loop in `derive_body_view` pushes stages in that order,
/// so slot == topo position). The two maps translate the body's declared ports to
/// that slot space:
///
/// - `input_port_to_slot`: input-port name → the body slot of the node that
///   consumes the port (from [`BoundBody::port_name_to_node_idx`]). Tracing
///   UPSTREAM, when the in-body BFS reaches this slot it has hit the body boundary
///   and resurfaces to the parent scope; tracing DOWNSTREAM it is the descent
///   entry point.
/// - `output_port_to_slot`: output-port name → the body slot of the terminal node
///   the port surfaces (from [`BoundBody::output_port_to_node_idx`]). The mirror
///   of the above: the UPSTREAM descent entry / DOWNSTREAM resurface boundary.
///
/// A port whose NodeIndex is not present in `body.topo_order` (a malformed body)
/// is omitted from the map rather than panicking, so the trace degrades to a leaf
/// at that crossing instead of crashing the Inspector.
pub struct BodyScope {
    pub view: PipelineView,
    pub input_port_to_slot: std::collections::HashMap<String, usize>,
    pub output_port_to_slot: std::collections::HashMap<String, usize>,
}

/// Derive a composition body's [`BodyScope`] — its lineage view plus the port→slot
/// maps #155's scope-aware trace needs.
///
/// Reuses the shared [`build_body_view`] core WITHOUT the barycenter layout pass: the
/// trace reads only `field_edges`, the rows, and the port→slot maps, never the canvas
/// coordinates, so the layout cost is skipped on this hot (per-render, unmemoized)
/// path. The returned NodeIndex→slot map projects the body's port→NodeIndex tables
/// into slot space; it comes straight from the builder (slot == position in
/// `body.topo_order`), so the scope path never recomputes it.
pub fn derive_body_scope(body: &clinker_plan::plan::composition_body::BoundBody) -> BodyScope {
    use petgraph::graph::NodeIndex;
    use std::collections::HashMap;

    let (view, idx_to_slot) = build_body_view(body, false);

    let project = |ports: &mut HashMap<String, usize>, port: &str, idx: NodeIndex| {
        if let Some(&slot) = idx_to_slot.get(&idx) {
            ports.insert(port.to_string(), slot);
        }
    };

    let mut input_port_to_slot = HashMap::new();
    for (port, &idx) in &body.port_name_to_node_idx {
        project(&mut input_port_to_slot, port, idx);
    }
    let mut output_port_to_slot = HashMap::new();
    for (port, &idx) in &body.output_port_to_node_idx {
        project(&mut output_port_to_slot, port, idx);
    }

    BodyScope {
        view,
        input_port_to_slot,
        output_port_to_slot,
    }
}

fn body_row_fields(row: &cxl::typecheck::Row) -> Vec<FieldRow> {
    row.fields()
        .map(|(field, ty)| FieldRow {
            name: field.to_string(),
            kind: FieldKind::Declared,
            ty: Some(compact_type(ty)),
            is_correlation_key: false,
            ..Default::default()
        })
        .collect()
}

/// Collect the input-column support of one [`CompiledAggregate`] emit residual,
/// resolving the extractor-produced `AggSlot`/`GroupKey` leaves back to the
/// aggregate's input columns (#180 GAP 1).
///
/// `Expr::support_into` treats [`Expr::AggSlot`] and [`Expr::GroupKey`] as terminal
/// (they are leaves with no field name), so it CANNOT recover the columns an
/// aggregate emit reads — they were rewritten away at `extract_aggregates` time.
/// This custom walk descends every `AggSlot`/`GroupKey` in the residual and resolves
/// it through `compiled`, then runs `support_into` once more to catch any bare
/// `FieldRef` the extractor left in the residual (a literal/passthrough term).
fn aggregate_residual_support(
    residual: &cxl::ast::Expr,
    compiled: &cxl::plan::CompiledAggregate,
    agg_input_schema: &[String],
    out: &mut std::collections::HashSet<String>,
) {
    use cxl::ast::Expr;
    match residual {
        Expr::AggSlot { slot, .. } => {
            if let Some(binding) = compiled.bindings.get(*slot as usize) {
                binding_arg_support(&binding.arg, agg_input_schema, out);
            }
        }
        Expr::GroupKey { slot, .. } => {
            if let Some(field) = compiled.group_by_fields.get(*slot as usize) {
                out.insert(field.clone());
            }
        }
        Expr::Binary { lhs, rhs, .. } | Expr::Coalesce { lhs, rhs, .. } => {
            aggregate_residual_support(lhs, compiled, agg_input_schema, out);
            aggregate_residual_support(rhs, compiled, agg_input_schema, out);
        }
        Expr::Unary { operand, .. } => {
            aggregate_residual_support(operand, compiled, agg_input_schema, out)
        }
        Expr::IfThenElse {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            aggregate_residual_support(condition, compiled, agg_input_schema, out);
            aggregate_residual_support(then_branch, compiled, agg_input_schema, out);
            if let Some(e) = else_branch {
                aggregate_residual_support(e, compiled, agg_input_schema, out);
            }
        }
        Expr::Match { subject, arms, .. } => {
            if let Some(s) = subject {
                aggregate_residual_support(s, compiled, agg_input_schema, out);
            }
            for arm in arms {
                aggregate_residual_support(&arm.pattern, compiled, agg_input_schema, out);
                aggregate_residual_support(&arm.body, compiled, agg_input_schema, out);
            }
        }
        Expr::MethodCall { receiver, args, .. } => {
            aggregate_residual_support(receiver, compiled, agg_input_schema, out);
            for a in args {
                aggregate_residual_support(a, compiled, agg_input_schema, out);
            }
        }
        Expr::WindowCall { args, .. } | Expr::AggCall { args, .. } => {
            for a in args {
                aggregate_residual_support(a, compiled, agg_input_schema, out);
            }
        }
        Expr::IndexAccess {
            receiver, index, ..
        } => {
            aggregate_residual_support(receiver, compiled, agg_input_schema, out);
            aggregate_residual_support(index, compiled, agg_input_schema, out);
        }
        Expr::Closure { body, .. } => {
            aggregate_residual_support(body, compiled, agg_input_schema, out)
        }
        // A bare `FieldRef`/`QualifiedFieldRef` (or any non-recursive leaf) the
        // extractor left in place is recovered by `support_into`, which the caller
        // runs over the whole residual — so the recursive arms above only need to
        // reach the `AggSlot`/`GroupKey` leaves `support_into` ignores.
        _ => {}
    }
}

/// Resolve one [`BindingArg`] to the input columns it reads (#180 GAP 1).
///
/// **SHARP EDGE:** `BindingArg::Field(idx)` indexes the aggregate's input schema in
/// the order `extract_aggregates` was fed — the aggregate's bound input row =
/// `typed.field_types.keys()`, which is its predecessor's body Row in PORT order —
/// NOT klinx's deduped `input_cols`. The caller passes that as `agg_input_schema`,
/// reconstructed from the predecessor stage's `fields` (the predecessor's body row,
/// port order; see [`body_field_edges`]). Only the DECLARED prefix is ever indexed:
/// an aggregate cannot read an undeclared (open-tail) column — CXL rejects it at
/// compile time with `E200`
/// (see `body_aggregate_referencing_open_tail_column_does_not_compile`) — so every
/// `Field(idx)` the adapter can see falls inside the declared columns, which the body
/// Row reproduces in exactly the order `Field(idx)` expects. The `.get(idx)` guard
/// against `None` is a belt-and-braces degrade-to-no-edge for an out-of-range index
/// that today is unreachable.
fn binding_arg_support(
    arg: &cxl::plan::BindingArg,
    agg_input_schema: &[String],
    out: &mut std::collections::HashSet<String>,
) {
    use cxl::plan::BindingArg;
    match arg {
        BindingArg::Field(idx) => {
            if let Some(name) = agg_input_schema.get(*idx as usize) {
                out.insert(name.clone());
            }
        }
        BindingArg::Expr(e) => e.support_into(out),
        BindingArg::Wildcard => {}
        BindingArg::Pair(a, b) => {
            binding_arg_support(a, agg_input_schema, out);
            binding_arg_support(b, agg_input_schema, out);
        }
    }
}

/// Per-emit support for a body [`CompiledAggregate`], same shape
/// [`field_lineage::emit_supports`] returns (#180 GAP 1).
///
/// One `(output_name, support)` pair per [`CompiledEmit`], in emit order. `support`
/// is the set of aggregate-input columns the emit reads, recovered from the residual
/// via [`aggregate_residual_support`] (the `AggSlot`/`GroupKey` leaves) plus
/// `support_into` (any bare `FieldRef` left in the residual). The shared
/// [`field_edges_for_stage`] turns each into a `Derive` edge.
fn aggregate_emit_supports(
    compiled: &cxl::plan::CompiledAggregate,
    agg_input_schema: &[String],
) -> Vec<(String, std::collections::HashSet<String>)> {
    compiled
        .emits
        .iter()
        .map(|emit| {
            let mut support = std::collections::HashSet::new();
            aggregate_residual_support(&emit.residual, compiled, agg_input_schema, &mut support);
            // Bare `FieldRef`s the extractor left untouched (e.g. a group-by column
            // referenced raw, or a passthrough term) are not `AggSlot`/`GroupKey`
            // leaves, so `support_into` is the one walk that recovers them.
            emit.residual.support_into(&mut support);
            (emit.output_name.to_string(), support)
        })
        .collect()
}

/// What drives a body node's emit/anchor analysis (#174, #180 GAP 1).
///
/// Reads the in-process-compiled artifact off the already-resolved body plan node:
///
/// - [`PlanNode::Transform`](clinker_plan::plan::execution::PlanNode::Transform)
///   with a resolved payload → [`StageProgram::Cxl`] over its
///   [`PlanTransformPayload`](clinker_plan::plan::execution::PlanTransformPayload)'s
///   `typed.program`, the same `cxl::ast::Program` the top-level resolved path
///   consumes, reused verbatim with no `.comp.yaml` re-parse.
/// - [`PlanNode::Aggregation`](clinker_plan::plan::execution::PlanNode::Aggregation)
///   with a populated `compiled` → [`StageProgram::Aggregate`] over
///   [`aggregate_emit_supports`], so a computed aggregate column (`emit total =
///   sum(x)`) draws a `Derive` cable to the input column it folds (#180 GAP 1).
///   `agg_input_schema` is the aggregate's input schema — its sole predecessor's body
///   row in PORT order (the caller reconstructs it from the predecessor stage's
///   `fields`; see [`body_field_edges`]), which is the index space `BindingArg::Field(idx)`
///   was assigned against. It is computed LAZILY — only this arm forces the closure.
/// - Every other node — and a `Transform` whose `resolved` is `None` or an
///   `Aggregation` whose `compiled` is empty (a deserialized plan never populates
///   the `#[serde(skip)]` fields) — → [`StageProgram::None`], the same-name carry
///   fallback. `Combine` stays on this fallback (its computed columns are
///   engine-blocked, clinker#621), as does a nested `Composition` (its output
///   columns surface from its body rows).
fn body_node_stage_program<'a>(
    node: &'a clinker_plan::plan::execution::PlanNode,
    agg_input_schema: impl FnOnce() -> Vec<String>,
) -> StageProgram<'a> {
    use clinker_plan::plan::execution::PlanNode;

    match node {
        PlanNode::Transform {
            resolved: Some(payload),
            ..
        } => StageProgram::Cxl(&payload.typed.program),
        PlanNode::Aggregation { compiled, .. } if !compiled.emits.is_empty() => {
            StageProgram::Aggregate(aggregate_emit_supports(compiled, &agg_input_schema()))
        }
        _ => StageProgram::None,
    }
}

/// Body-internal field lineage edges + role edges in the body's slot space (#174,
/// #180 PR2).
///
/// Mirrors the top-level resolved path ([`resolved_pipeline_field_lineage`]) against
/// a [`BoundBody`], reusing the shared [`field_edges_for_stage`] analyzer so the body
/// matches the top-level path on every closeable gap:
/// - A body **Transform** runs the full emit/anchor analysis ([`StageProgram::Cxl`]),
///   so a COMPUTED body column (`emit c = a + 1`) draws a `Derive` cable to its
///   producer column — the cable the Inspector's in-body BFS follows and the drill-in
///   canvas renders.
/// - A body **Aggregate** runs that same analysis over precomputed supports
///   ([`StageProgram::Aggregate`] via [`aggregate_emit_supports`]), so a computed
///   aggregate column (`emit total = sum(x)`) derives to the input column it folds,
///   and its `group_by` columns draw `GroupBy` influence + role edges (#180 GAP 1/2).
/// - A body **Route**/**Cull** emits its `Conditional`/`Filter` INDIRECT influence
///   edges via [`body_node_influence_predicates`] (#180 GAP 2). The body resolves
///   predicate columns WITHOUT port-alias disambiguation (it passes an empty alias
///   map), consistent with the body's empty-alias model for derive edges — a
///   precision divergence from the top-level path, acceptable and pre-existing for the
///   body (a body Merge/Combine join key is not alias-resolved here).
/// - Multi-producer fan-in carries are graded conservatively, so a body Merge
///   join-key fan-in grades `Approximate` like the top level (#180 GAP 3).
///
/// A body node with NO resolved program (every CXL-less node — Source/Route/Merge/
/// Output/Cull/Reshape/Envelope/nested `Composition`, and `Combine`) keeps the
/// same-name passthrough carry fallback. **Combine is NOT at parity** (clinker#621):
/// its computed columns and its `JoinKey` influence are not reachable from a
/// `BoundBody`, so they degrade to the carry fallback. A body **Aggregate** that folds
/// an open-tail pass-through column the input port did not declare is a non-issue: CXL
/// rejects reading an undeclared column at compile time (E200), so no such aggregate
/// exists to trace (see `body_aggregate_referencing_open_tail_column_does_not_compile`).
fn body_field_edges(
    body: &clinker_plan::plan::composition_body::BoundBody,
    stages: &[StageView],
    graph_predecessors: &[Vec<usize>],
) -> (Vec<FieldEdge>, Vec<RoleEdge>) {
    let name_to_slot: std::collections::HashMap<&str, usize> = stages
        .iter()
        .enumerate()
        .map(|(idx, stage)| (stage.id.as_str(), idx))
        .collect();

    let mut acc = EdgeAccumulator::new();
    // Role edges (Aggregate group-by role-port edges) are now LIVE for the body
    // (#180 GAP 2), so they accumulate across the whole body rather than being
    // discarded per-stage. The Combine alias map stays empty — a body Combine has
    // no resolved program / no alias-qualified support to resolve (clinker#621).
    let mut role_edges: Vec<RoleEdge> = Vec::new();
    let no_aliases: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (to, stage) in stages.iter().enumerate() {
        let ordered_predecessors: Vec<usize> = body
            .node_input_refs
            .get(&stage.id)
            .map(|refs| {
                refs.iter()
                    .filter_map(|raw| {
                        let producer = raw.split_once('.').map_or(raw.as_str(), |(node, _)| node);
                        name_to_slot.get(producer).copied()
                    })
                    .collect()
            })
            .filter(|preds: &Vec<usize>| !preds.is_empty())
            .unwrap_or_else(|| graph_predecessors.get(to).cloned().unwrap_or_default());

        if stage.fields.is_empty() {
            continue;
        }

        // Ordered, de-duplicated union of predecessor output column names, with
        // every producer slot recorded per column (#67 fan-in), mirroring the
        // top-level `compute_field_lineage` producer fold but in slot space.
        let mut input_cols: Vec<String> = Vec::new();
        let mut producers_of: std::collections::HashMap<String, Vec<usize>> =
            std::collections::HashMap::new();
        for &from in &ordered_predecessors {
            let Some(source) = stages.get(from) else {
                continue;
            };
            for field in &source.fields {
                let producers = producers_of.entry(field.name.clone()).or_default();
                if producers.is_empty() {
                    input_cols.push(field.name.clone());
                }
                if !producers.contains(&from) {
                    producers.push(from);
                }
            }
        }
        if producers_of.is_empty() {
            continue;
        }

        // The consumer stage's real output columns; an edge is only emitted into one
        // of these (a support member naming a non-output column draws nothing). The
        // body row set is authoritative, so the shared analyzer gates on it
        // (`OutputGate::Resolved`).
        let output_names: std::collections::HashSet<String> =
            stage.fields.iter().map(|f| f.name.clone()).collect();

        // Resolve the body plan node ONCE; the stage-program, group-key, and influence
        // helpers all read from it (rather than each re-resolving via `name_to_idx` +
        // `node_weight`). A stage whose id is not in the body graph — only a malformed
        // body, since stages are built FROM `body.topo_order` — keeps the no-program
        // same-name carry fallback and emits no group-by / influence edges.
        let resolved = body
            .name_to_idx
            .get(&stage.id)
            .and_then(|&idx| body.graph.node_weight(idx).map(|node| (idx, node)));

        // Aggregate group keys define the grouped-record grain — a `GroupBy`
        // influence edge (#147) — and are skipped by the emit/carry loops. The body
        // path reads them off the plan node's `config.group_by` (#180 GAP 2), the
        // mirror of the top-level path's `config.group_by`.
        let group_keys = resolved
            .map(|(_, plan_node)| body_node_group_keys(plan_node))
            .unwrap_or_default();

        // GAP 1 + GAP 3: a body Transform/Aggregate runs the shared analyzer (the
        // Aggregate via precomputed supports keyed on its input schema, reconstructed
        // lazily — only the Aggregate arm needs it); fan-in grading is now Conservative,
        // so a body Merge fanning a column in from two distinct producers grades that
        // carry Approximate, matching the top-level paths. A CXL-less / unresolved
        // node (Route/Merge/Output/Cull/nested Composition, and Combine — clinker#621)
        // falls to the same-name carry fallback.
        //
        // The Aggregate's input schema is its sole predecessor stage's `fields` — the
        // predecessor body row, in PORT order. That is exactly the index space
        // `BindingArg::Field(idx)` was assigned against (the engine indexes it into the
        // aggregate's bound input row = the predecessor's body Row, port order), and it
        // is populated for EVERY predecessor kind (a Route's body row carries its
        // preserved input schema), so it covers cases the engine `stored_output_schema`
        // does not (a Route predecessor has no stored schema). See the closure below.
        let stage_program = match resolved {
            Some((_node_idx, plan_node)) => body_node_stage_program(plan_node, || {
                ordered_predecessors
                    .first()
                    .and_then(|&p| stages.get(p))
                    .map(|source| source.fields.iter().map(|f| f.name.clone()).collect())
                    .unwrap_or_default()
            }),
            None => StageProgram::None,
        };
        field_edges_for_stage(
            &mut acc,
            &mut role_edges,
            to,
            stage_program,
            &input_cols,
            &producers_of,
            &OutputGate::Resolved(&output_names),
            &no_aliases,
            &group_keys,
        );

        // INDIRECT influence edges (#147, #180 GAP 2): a body Route's branch
        // `Conditional` conditions and a body Cull's `Filter` removal predicates,
        // added AFTER the node's DIRECT carries so they land on the real surviving
        // output rows. A Merge is a row UNION and contributes none; Combine's
        // `JoinKey` is engine-blocked (clinker#621). The body resolves predicate
        // columns WITHOUT port-alias disambiguation (`no_aliases`) — consistent with
        // the body's empty-alias model for derive edges; see `body_field_edges` doc.
        if let Some((_, plan_node)) = resolved {
            for (predicate, kind) in body_node_influence_predicates(plan_node, &body.route_bodies) {
                emit_predicate_influence_edges(
                    &mut acc,
                    to,
                    &predicate,
                    kind,
                    &stage.fields,
                    &producers_of,
                    &no_aliases,
                );
            }
        }
    }
    (acc.edges, role_edges)
}

/// A body Aggregate's group-by columns in the body's own scope (#180 GAP 2).
///
/// Read off the already-resolved plan node's `config.group_by` — the mirror of the
/// top-level resolved path's `PipelineNode::Aggregate { config }.group_by`. Every
/// other body node (and a `Transform`) groups nothing, so this returns an empty set.
fn body_node_group_keys(
    node: &clinker_plan::plan::execution::PlanNode,
) -> std::collections::HashSet<String> {
    use clinker_plan::plan::execution::PlanNode;

    match node {
        PlanNode::Aggregation { config, .. } => config.group_by.iter().cloned().collect(),
        _ => std::collections::HashSet::new(),
    }
}

/// The control-flow predicates a BODY node imposes, each paired with the INDIRECT
/// edge kind it produces (#180 GAP 2) — the body-scope analogue of
/// [`node_influence_predicates`], reading the already-resolved engine `PlanNode`
/// (plus the body's `route_bodies` table) instead of the top-level config.
///
/// - **Route** — body Route conditions live in
///   [`BoundBody::route_bodies`]`[name].conditions` (the top-level Route's
///   conditions would mis-route a body Route, so the body keeps its own). Each
///   branch condition is a `Conditional` predicate; the default branch has none.
/// - **Cull** — [`PlanNode::Cull`](clinker_plan::plan::execution::PlanNode::Cull)'s
///   `config.rules[].drop_group_when` is a `Filter` predicate per removal rule.
/// - **Combine** `JoinKey` is OMITTED — a body Combine's typed `where` predicate is
///   not on the plan node (it lives in `CompileArtifacts`, not reachable from a
///   `BoundBody`), so the body cannot emit the join-key influence (clinker#621).
///
/// Returns owned predicate strings (the `RouteBody` conditions are borrowed from a
/// map the caller does not keep alive across the influence emission).
fn body_node_influence_predicates(
    node: &clinker_plan::plan::execution::PlanNode,
    route_bodies: &std::collections::HashMap<
        String,
        clinker_plan::config::pipeline_node::RouteBody,
    >,
) -> Vec<(String, FieldEdgeKind)> {
    use clinker_plan::plan::execution::PlanNode;

    // A body Route's conditions are keyed by node name in `route_bodies` (the plan
    // node itself carries only branch wiring, not the compiled conditions).
    if let PlanNode::Route { name, .. } = node
        && let Some(route) = route_bodies.get(name)
    {
        return route
            .conditions
            .values()
            .map(|predicate| (predicate.as_ref().to_string(), FieldEdgeKind::Conditional))
            .collect();
    }
    match node {
        PlanNode::Cull { config, .. } => config
            .rules
            .iter()
            .map(|rule| {
                (
                    rule.drop_group_when.as_ref().to_string(),
                    FieldEdgeKind::Filter,
                )
            })
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod migrated_fixture_tests {
    use super::*;
    use clinker_plan::config::{CompileContext, parse_config};

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

    /// Compile the shared composition fixture pipeline (source `src`(a:int,b:string)
    /// → composition `comp` running body `body_lineage` → output `out`) and return
    /// the full [`CompiledPlan`]. The temporary workspace is removed before
    /// returning; the plan owns everything the view derivation needs. Both
    /// [`compiled_body_fixture`] (which extracts the bound body) and the #154
    /// boundary-edge tests (which build the resolved top-level view) consume this.
    fn compiled_plan_fixture() -> clinker_plan::plan::CompiledPlan {
        let unique = format!(
            "klinx-body-view-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock after epoch")
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&root).expect("create temporary composition workspace");
        std::fs::write(
            root.join("body_lineage.comp.yaml"),
            r#"_compose:
  name: body_lineage
  inputs:
    src:
      schema:
        - { name: a, type: int }
        - { name: b, type: string }
  outputs:
    result: second
  config_schema: {}

nodes:
  - type: transform
    name: first
    input: src
    config:
      cxl: |
        emit c = a + 1
  - type: transform
    name: second
    input: first
    config:
      cxl: |
        emit d = c + 1
"#,
        )
        .expect("write composition fixture");

        let pipeline = r#"
pipeline:
  name: body_drill
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
  - type: composition
    name: comp
    input: src
    use: ./body_lineage.comp.yaml
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

    fn compiled_body_fixture() -> clinker_plan::plan::composition_body::BoundBody {
        let plan = compiled_plan_fixture();
        let body_id = plan
            .dag()
            .graph
            .node_weights()
            .find_map(|node| match node {
                clinker_plan::plan::execution::PlanNode::Composition { name, body, .. }
                    if name == "comp" =>
                {
                    Some(*body)
                }
                _ => None,
            })
            .expect("compiled composition body id");
        plan.body_of(body_id).expect("compiled body exists").clone()
    }

    /// Compile a composition whose body contains a Route + Merge (no computing
    /// Transform) so the body has CXL-less nodes (`split` Route, `joined` Merge)
    /// whose field cables exercise the #174 no-program carry fallback. The body
    /// `src(a:int,b:string) → split (route on a>0) → joined (merge of both branches)`,
    /// output port `result: joined`. Returns the extracted [`BoundBody`].
    fn compiled_routing_body_fixture() -> clinker_plan::plan::composition_body::BoundBody {
        let unique = format!(
            "klinx-routing-body-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock after epoch")
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&root).expect("create temporary composition workspace");
        std::fs::write(
            root.join("routing.comp.yaml"),
            r#"_compose:
  name: routing
  inputs:
    src:
      schema:
        - { name: a, type: int }
        - { name: b, type: string }
  outputs:
    result: joined
  config_schema: {}

nodes:
  - type: route
    name: split
    input: src
    config:
      conditions:
        hi: "a > 0"
      default: lo
  - type: merge
    name: joined
    inputs: [split.hi, split.lo]
"#,
        )
        .expect("write routing composition fixture");

        let pipeline = r#"
pipeline:
  name: routing_drill
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
  - type: composition
    name: comp
    input: src
    use: ./routing.comp.yaml
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
        let config = parse_config(pipeline).expect("routing pipeline parses");
        let plan = config
            .compile(&CompileContext::new(root.clone()))
            .expect("routing pipeline compiles");
        let _ = std::fs::remove_dir_all(root);
        let body_id = plan
            .dag()
            .graph
            .node_weights()
            .find_map(|node| match node {
                clinker_plan::plan::execution::PlanNode::Composition { name, body, .. }
                    if name == "comp" =>
                {
                    Some(*body)
                }
                _ => None,
            })
            .expect("compiled routing composition body id");
        plan.body_of(body_id).expect("compiled body exists").clone()
    }

    /// The composition index in the shared fixture's resolved top-level view.
    fn comp_idx(view: &PipelineView) -> usize {
        stage_idx(view, "comp")
    }

    /// Compile a composition `body.comp.yaml` (given verbatim) wired into a minimal
    /// `src → comp → out` pipeline, returning the compile `Result` so callers can
    /// assert SUCCESS (extract the body) or FAILURE (a characterization test). The
    /// temporary workspace is removed before returning.
    fn try_compile_body_fixture(
        label: &str,
        src_schema: &str,
        comp_yaml: &str,
    ) -> Result<clinker_plan::plan::CompiledPlan, Vec<clinker_core_types::Diagnostic>> {
        let unique = format!(
            "klinx-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock after epoch")
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&root).expect("create temporary composition workspace");
        std::fs::write(root.join("body.comp.yaml"), comp_yaml).expect("write composition fixture");

        let pipeline = format!(
            r#"
pipeline:
  name: {label}_drill
nodes:
  - type: source
    name: src
    config:
      name: src
      type: csv
      path: ./in.csv
      schema:
{src_schema}
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
"#
        );
        let config = parse_config(&pipeline).expect("pipeline fixture parses");
        let result = config.compile(&CompileContext::new(root.clone()));
        let _ = std::fs::remove_dir_all(root);
        result
    }

    /// Compile a body fixture expecting SUCCESS, extract the bound body of `comp`, and
    /// return it. The common path for #180 PR2 GAP fixtures.
    fn compile_body_fixture(label: &str, src_schema: &str, comp_yaml: &str) -> BoundBody {
        let plan = try_compile_body_fixture(label, src_schema, comp_yaml)
            .expect("pipeline fixture compiles");
        let body_id = plan
            .dag()
            .graph
            .node_weights()
            .find_map(|node| match node {
                clinker_plan::plan::execution::PlanNode::Composition { name, body, .. }
                    if name == "comp" =>
                {
                    Some(*body)
                }
                _ => None,
            })
            .expect("compiled composition body id");
        plan.body_of(body_id).expect("compiled body exists").clone()
    }

    use clinker_plan::plan::composition_body::BoundBody;

    /// A composition body that AGGREGATES: `src(x:int, k:string) → agg(group_by:[k],
    /// emit total = sum(x))`, output port `result: agg`. Exercises #180 GAP 1 (the
    /// computed aggregate column traces to its folded input column) and GAP 2 (the
    /// `k` group-by draws a `GroupBy` edge).
    fn compiled_aggregating_body_fixture() -> BoundBody {
        compile_body_fixture(
            "agg-body",
            "        - { name: x, type: int }\n        - { name: k, type: string }",
            r#"_compose:
  name: aggregating
  inputs:
    src:
      schema:
        - { name: x, type: int }
        - { name: k, type: string }
  outputs:
    result: agg
  config_schema: {}

nodes:
  - type: aggregate
    name: agg
    input: src
    config:
      group_by: [k]
      cxl: |
        emit total = sum(x)
"#,
        )
    }

    /// Attempt to compile a composition body whose input port declares FEWER columns
    /// than the caller supplies (port declares only `k`; the caller's source supplies
    /// `x, k`) and whose aggregate folds an OPEN-TAIL pass-through column the port did
    /// not declare (`emit total = sum(x)`). Returns the compile `Result` — this body
    /// is EXPECTED to fail to compile (see
    /// `body_aggregate_referencing_open_tail_column_does_not_compile`).
    fn try_compile_open_tail_aggregate_reference()
    -> Result<clinker_plan::plan::CompiledPlan, Vec<clinker_core_types::Diagnostic>> {
        try_compile_body_fixture(
            "open-tail-agg-body",
            "        - { name: x, type: int }\n        - { name: k, type: string }",
            r#"_compose:
  name: open_tail_agg
  inputs:
    src:
      schema:
        - { name: k, type: string }
  outputs:
    result: agg
  config_schema: {}

nodes:
  - type: aggregate
    name: agg
    input: src
    config:
      group_by: [k]
      cxl: |
        emit total = sum(x)
"#,
        )
    }

    /// A composition body where a Merge genuinely fans in TWO DISTINCT producer slots
    /// carrying the same column: `src → split(route) → t1/t2(transform) →
    /// joined(merge of t1,t2)`. `t1` and `t2` each pass `k` through unchanged, so the
    /// Merge sees `k` from two separate producers (unlike `compiled_routing_body_fixture`,
    /// whose `split.hi`/`split.lo` dedup to one Route slot). Exercises #180 GAP 3:
    /// the shared `k` carry into the Merge grades `Approximate`, while a column from a
    /// single producer stays `Exact`.
    fn compiled_multi_producer_merge_body_fixture() -> BoundBody {
        compile_body_fixture(
            "multi-merge-body",
            "        - { name: k, type: int }\n        - { name: v, type: string }",
            r#"_compose:
  name: multi_merge
  inputs:
    src:
      schema:
        - { name: k, type: int }
        - { name: v, type: string }
  outputs:
    result: joined
  config_schema: {}

nodes:
  - type: route
    name: split
    input: src
    config:
      conditions:
        hi: "k > 0"
      default: lo
  - type: transform
    name: t1
    input: split.hi
    config:
      cxl: |
        emit tag = 1
  - type: transform
    name: t2
    input: split.lo
    config:
      cxl: |
        emit tag = 2
  - type: merge
    name: joined
    inputs: [t1, t2]
"#,
        )
    }

    /// A composition body where an Aggregate sits DOWNSTREAM of a Route:
    /// `src(x:int, k:string) → split(route on k) → agg(group_by:[k],
    /// emit total = sum(x))`, output port `result: agg`. The aggregate's body-graph
    /// predecessor is the Route `split`, which carries NO engine `stored_output_schema`
    /// (Route/Output/Sort/CorrelationCommit return `None`). The aggregate's input
    /// schema must therefore come from the Route's body row (`body_rows["split"]`,
    /// which IS populated — a Route preserves its input schema), not the engine
    /// accessor. Guards #180 PR2 Finding 1: a Route-fed aggregate keeps its derive edges.
    fn compiled_route_then_aggregate_body_fixture() -> BoundBody {
        compile_body_fixture(
            "route-agg-body",
            "        - { name: x, type: int }\n        - { name: k, type: string }",
            r#"_compose:
  name: route_then_agg
  inputs:
    src:
      schema:
        - { name: x, type: int }
        - { name: k, type: string }
  outputs:
    result: agg
  config_schema: {}

nodes:
  - type: route
    name: split
    input: src
    config:
      conditions:
        hi: "k != ''"
      default: lo
  - type: aggregate
    name: agg
    input: split.hi
    config:
      group_by: [k]
      cxl: |
        emit total = sum(x)
"#,
        )
    }

    /// A composition body whose input port declares its columns in a DIFFERENT order
    /// than the caller supplies: the caller's source is `[x, k]` (parent order); the
    /// port declares `[k, x]` (port order). Both columns are present, so it compiles.
    /// The aggregate `emit total = sum(x)` binds `Field(idx)` in PORT order — so the
    /// adapter MUST resolve `idx` against the port row (`body_rows["src"] = [k, x, …]`),
    /// not the engine input-port Source's `stored_output_schema`, which carries the
    /// PARENT order `[x, k, …]` and would mis-resolve to `k`. Guards #180 PR2 Finding 2.
    fn compiled_port_reordered_aggregate_body_fixture() -> BoundBody {
        compile_body_fixture(
            "reorder-agg-body",
            "        - { name: x, type: int }\n        - { name: k, type: string }",
            r#"_compose:
  name: reorder_agg
  inputs:
    src:
      schema:
        - { name: k, type: string }
        - { name: x, type: int }
  outputs:
    result: agg
  config_schema: {}

nodes:
  - type: aggregate
    name: agg
    input: src
    config:
      group_by: [k]
      cxl: |
        emit total = sum(x)
"#,
        )
    }

    /// A composition body that CULLS: `src(gid:string, status:string) →
    /// prune(cull, drop_group_when reads status)`, output port `result: prune`.
    /// Exercises #180 GAP 2: the body Cull's removal predicate draws a `Filter`
    /// influence edge from its read column.
    fn compiled_culling_body_fixture() -> BoundBody {
        compile_body_fixture(
            "cull-body",
            "        - { name: gid, type: string }\n        - { name: status, type: string }",
            r#"_compose:
  name: culling
  inputs:
    src:
      schema:
        - { name: gid, type: string }
        - { name: status, type: string }
  outputs:
    result: prune
  config_schema: {}

nodes:
  - type: cull
    name: prune
    input: src
    config:
      partition_by: [gid]
      removed_to: dropped
      rules:
        - name: drop_errored
          drop_group_when: "sum(if status == 'error' then 1 else 0) > 0"
"#,
        )
    }

    /// Compile a composition whose input port declares NO schema (accept-any), so
    /// its body input-port row carries no user columns — the #154 degrade path's
    /// trigger. The body emits one computed output port. Returns the compiled plan.
    fn compiled_accept_any_plan_fixture() -> clinker_plan::plan::CompiledPlan {
        let unique = format!(
            "klinx-accept-any-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock after epoch")
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&root).expect("create temporary composition workspace");
        std::fs::write(
            root.join("accept_any.comp.yaml"),
            r#"_compose:
  name: accept_any
  inputs:
    anyin: {}
  outputs:
    result: only
  config_schema: {}

nodes:
  - type: transform
    name: only
    input: anyin
    config:
      cxl: |
        emit tag = 1
"#,
        )
        .expect("write accept-any composition fixture");

        let pipeline = r#"
pipeline:
  name: accept_any_drill
nodes:
  - type: source
    name: src
    config:
      name: src
      type: csv
      path: ./in.csv
      schema:
        - { name: a, type: int }
  - type: composition
    name: comp
    input: src
    use: ./accept_any.comp.yaml
    inputs:
      anyin: src
  - type: output
    name: out
    input: comp
    config:
      name: out
      type: csv
      path: ./out.csv
"#;
        let config = parse_config(pipeline).expect("accept-any pipeline parses");
        let plan = config
            .compile(&CompileContext::new(root.clone()))
            .expect("accept-any pipeline compiles");
        let _ = std::fs::remove_dir_all(root);
        plan
    }

    /// Compile a composition with TWO accept-any input ports (`anyin`, `anyin2`),
    /// each wired to its own outer source, so BOTH ports degrade (neither declares a
    /// user column). Returns the compiled plan; used to prove the node-level degrade
    /// connector is emitted at most once per composition.
    fn compiled_two_accept_any_plan_fixture() -> clinker_plan::plan::CompiledPlan {
        let unique = format!(
            "klinx-two-any-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock after epoch")
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&root).expect("create temporary composition workspace");
        std::fs::write(
            root.join("two_any.comp.yaml"),
            r#"_compose:
  name: two_any
  inputs:
    anyin: {}
    anyin2: {}
  outputs:
    result: only
  config_schema: {}

nodes:
  - type: transform
    name: only
    input: anyin
    config:
      cxl: |
        emit tag = 1
"#,
        )
        .expect("write two-accept-any composition fixture");

        let pipeline = r#"
pipeline:
  name: two_any_drill
nodes:
  - type: source
    name: srcA
    config:
      name: srcA
      type: csv
      path: ./a.csv
      schema:
        - { name: a, type: int }
  - type: source
    name: srcB
    config:
      name: srcB
      type: csv
      path: ./b.csv
      schema:
        - { name: b, type: int }
  - type: composition
    name: comp
    input: srcA
    use: ./two_any.comp.yaml
    inputs:
      anyin: srcA
      anyin2: srcB
  - type: output
    name: out
    input: comp
    config:
      name: out
      type: csv
      path: ./out.csv
"#;
        let config = parse_config(pipeline).expect("two-accept-any pipeline parses");
        let plan = config
            .compile(&CompileContext::new(root.clone()))
            .expect("two-accept-any pipeline compiles");
        let _ = std::fs::remove_dir_all(root);
        plan
    }

    /// Compile a composition with TWO input ports (`left`, `right`) that SHARE a
    /// column name `k`, each wired to a DISTINCT outer source. A body Combine
    /// consumes both. Used to prove each port's columns bind to the right producer
    /// (the call-site `inputs:` mapping), not "any predecessor carrying the name".
    fn compiled_two_port_shared_column_plan_fixture() -> clinker_plan::plan::CompiledPlan {
        let unique = format!(
            "klinx-two-port-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock after epoch")
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&root).expect("create temporary composition workspace");
        std::fs::write(
            root.join("two_port.comp.yaml"),
            r#"_compose:
  name: two_port
  inputs:
    left:
      schema:
        - { name: k, type: int }
        - { name: lv, type: int }
    right:
      schema:
        - { name: k, type: int }
        - { name: rv, type: int }
  outputs:
    result: joined
  config_schema: {}

nodes:
  - type: combine
    name: joined
    input:
      l: left
      r: right
    config:
      where: "l.k == r.k"
      match: first
      on_miss: skip
      propagate_ck: all
      cxl: |
        emit k = l.k
        emit lv = l.lv
        emit rv = r.rv
"#,
        )
        .expect("write two-port composition fixture");

        let pipeline = r#"
pipeline:
  name: two_port_drill
nodes:
  - type: source
    name: srcL
    config:
      name: srcL
      type: csv
      path: ./l.csv
      schema:
        - { name: k, type: int }
        - { name: lv, type: int }
  - type: source
    name: srcR
    config:
      name: srcR
      type: csv
      path: ./r.csv
      schema:
        - { name: k, type: int }
        - { name: rv, type: int }
  - type: composition
    name: comp
    input: srcL
    inputs:
      left: srcL
      right: srcR
    use: ./two_port.comp.yaml
  - type: output
    name: out
    input: comp
    config:
      name: out
      type: csv
      path: ./out.csv
"#;
        let config = parse_config(pipeline).expect("two-port pipeline parses");
        let plan = config
            .compile(&CompileContext::new(root.clone()))
            .expect("two-port pipeline compiles");
        let _ = std::fs::remove_dir_all(root);
        plan
    }

    /// #154 (MUST-FIX 1): when 2+ input ports degrade, exactly ONE node-level
    /// Approximate boundary connector is emitted — not one per degrading port.
    /// `push_direct` bypasses dedup, so emitting per-port would accumulate
    /// byte-identical duplicate edges. Both ports here are accept-any (no user
    /// columns), so both degrade; the result must still be a single node-level edge.
    #[test]
    fn composition_multiple_degrading_ports_emit_single_node_level_edge() {
        let plan = compiled_two_accept_any_plan_fixture();
        let view = derive_resolved_pipeline_view(&plan);
        let i_comp = comp_idx(&view);

        let node_level: Vec<&FieldEdge> = view
            .field_edges
            .iter()
            .filter(|e| {
                e.kind == FieldEdgeKind::Boundary
                    && e.to_node == i_comp
                    && e.from_field.is_empty()
                    && e.to_field.is_empty()
            })
            .collect();
        assert_eq!(
            node_level.len(),
            1,
            "two degrading input ports must yield exactly ONE node-level boundary \
             edge (no byte-identical duplicates): {:?}",
            view.field_edges,
        );
        assert_eq!(node_level[0].precision, Precision::Approximate);
    }

    /// #154 (MUST-FIX 3): each input port's columns bind to the SPECIFIC outer node
    /// the call-site `inputs:` map wires it to — not "any predecessor carrying the
    /// column name". Two ports share a column name `k` but are wired to distinct
    /// producers (`left: srcL`, `right: srcR`); the shared `k` must bind once per
    /// port to the correct producer, never cross-binding to the other side.
    #[test]
    fn composition_input_ports_bind_to_their_mapped_producer() {
        let plan = compiled_two_port_shared_column_plan_fixture();
        let view = derive_resolved_pipeline_view(&plan);
        let i_srcl = stage_idx(&view, "srcL");
        let i_srcr = stage_idx(&view, "srcR");
        let i_comp = comp_idx(&view);

        // `left.k` binds to srcL; `right.k` binds to srcR — the shared name does NOT
        // cross-bind.
        let expect_in = |from: usize, col: &str, port: &str| {
            FieldEdge::boundary(
                from,
                col.to_string(),
                i_comp,
                field_lineage::composition_in_boundary_field(port, col),
                false,
            )
        };
        for (from, col, port) in [
            (i_srcl, "k", "left"),
            (i_srcl, "lv", "left"),
            (i_srcr, "k", "right"),
            (i_srcr, "rv", "right"),
        ] {
            assert!(
                view.field_edges.contains(&expect_in(from, col, port)),
                "expected {col} from node {from} to bind into port `{port}`; got {:?}",
                view.field_edges,
            );
        }

        // The shared column `k` must NOT cross-bind: srcL must not feed the `right`
        // port, and srcR must not feed the `left` port.
        assert!(
            !view.field_edges.contains(&expect_in(i_srcl, "k", "right")),
            "srcL.k must not mis-bind to the `right` port",
        );
        assert!(
            !view.field_edges.contains(&expect_in(i_srcr, "k", "left")),
            "srcR.k must not mis-bind to the `left` port",
        );
    }

    /// #154 degrade path: a composition input port with no resolvable USER columns
    /// (an accept-any `inputs: { anyin: {} }` port, whose body row carries only
    /// engine-internal `$`-columns) emits a single NODE-LEVEL boundary edge
    /// classified `Precision::Approximate` instead of per-column edges. The
    /// node-level connector has empty endpoint fields (a producer→comp crossing
    /// with no column granularity) — distinct from the Exact per-column crossings.
    #[test]
    fn composition_degrades_to_node_level_boundary_when_port_has_no_user_columns() {
        let plan = compiled_accept_any_plan_fixture();
        let view = derive_resolved_pipeline_view(&plan);
        let i_src = stage_idx(&view, "src");
        let i_comp = comp_idx(&view);

        // Exactly one node-level Approximate boundary edge: src → comp, empty fields.
        let node_level: Vec<&FieldEdge> = view
            .field_edges
            .iter()
            .filter(|e| {
                e.kind == FieldEdgeKind::Boundary
                    && e.from_node == i_src
                    && e.to_node == i_comp
                    && e.from_field.is_empty()
                    && e.to_field.is_empty()
            })
            .collect();
        assert_eq!(
            node_level.len(),
            1,
            "an accept-any input port degrades to exactly one node-level boundary edge: {:?}",
            view.field_edges,
        );
        assert_eq!(
            node_level[0].precision,
            Precision::Approximate,
            "the degraded node-level boundary edge is Approximate"
        );

        // No per-column INPUT marker was emitted for the accept-any port (it had no
        // user columns to bind), confirming the degrade replaced the per-column path.
        assert!(
            !view.field_edges.iter().any(|e| {
                e.kind == FieldEdgeKind::Boundary
                    && e.to_node == i_comp
                    && e.to_field.starts_with('\u{2190}')
            }),
            "the degraded port emits no per-column input markers: {:?}",
            view.field_edges,
        );
    }

    /// #154 precision-fold regression: in `composition_field_lineage` the
    /// producer→output-port edges are appended AFTER `compute_field_lineage` already
    /// ran `derive_row_precision`, so the output-port placeholder row never folded in
    /// any degradation arriving along that edge. Here the producer `gen` fans out
    /// `vals` via `emit each` (an Approximate derive, #148); the output port `result`
    /// ← `gen` must inherit that degradation. The fix re-folds row precision over the
    /// appended edges — without it the port row stays Exact (the regression).
    #[test]
    fn composition_output_port_row_reflects_approximate_producer() {
        let yaml = r#"_compose:
  name: fanout
  inputs:
    src:
      schema:
        - { name: items, type: array }
  outputs:
    result: gen
  config_schema: {}

nodes:
  - type: transform
    name: gen
    input: src
    config:
      cxl: |
        emit each x in items {
          emit result = x
        }
"#;
        let view = derive_composition_view(&parse_comp(yaml));

        // The output-port placeholder stage `result` carries one Declared row named
        // for the port. The producer `gen`'s representative column (`result`, fanned
        // out by `emit each`) is Approximate, so the appended producer→port edge is
        // Approximate and the re-fold degrades the port row. Without the fix the
        // port row would remain Exact.
        let result_idx = stage_idx(&view, "result");
        let port_row = view.stages[result_idx]
            .fields
            .iter()
            .find(|r| r.name == "result")
            .expect("output-port placeholder row present");
        assert_eq!(
            port_row.lineage_precision,
            Precision::Approximate,
            "the output-port row must inherit the Approximate fanned producer's \
             degradation via the post-append precision re-fold: {:?}",
            view.stages[result_idx].fields,
        );
        assert!(
            !port_row.precision_reason.is_empty(),
            "a degraded port row carries a precision reason"
        );
    }

    /// #154: the resolved top-level view materializes INPUT boundary edges binding
    /// each outer producer column to a synthetic per-port marker on the composition
    /// node. `src.a` and `src.b` cross into the `src` input port; each edge is
    /// `FieldEdgeKind::Boundary`, `Exact`, and carries the documented
    /// `composition_in_boundary_field(port, col)` mapping so #155 can recover the
    /// (port, column) pair.
    #[test]
    fn composition_input_boundary_edges_bind_outer_columns_to_ports() {
        let plan = compiled_plan_fixture();
        let view = derive_resolved_pipeline_view(&plan);
        let i_src = stage_idx(&view, "src");
        let i_comp = comp_idx(&view);

        for col in ["a", "b"] {
            let expected = FieldEdge::boundary(
                i_src,
                col.to_string(),
                i_comp,
                field_lineage::composition_in_boundary_field("src", col),
                false,
            );
            assert!(
                view.field_edges.contains(&expected),
                "expected INPUT boundary edge src.{col} → comp port `src` \
                 (to_field {:?}), got {:?}",
                field_lineage::composition_in_boundary_field("src", col),
                view.field_edges,
            );
            // The matching edge is genuinely a Boundary crossing, classified Exact.
            let edge = view
                .field_edges
                .iter()
                .find(|e| **e == expected)
                .expect("boundary edge present");
            assert_eq!(edge.kind, FieldEdgeKind::Boundary);
            assert_eq!(edge.precision, Precision::Exact);
        }

        // The composition node now shows its OUTPUT schema (synthesized from the
        // body's output ports), so the value chain passes through it and #155 can
        // descend. The body's `result` port surfaces a, b (carried) + c, d
        // (computed); engine-internal `$`-columns are excluded.
        let comp_cols: Vec<&str> = view.stages[i_comp]
            .fields
            .iter()
            .map(|f| f.name.as_str())
            .collect();
        assert_eq!(
            comp_cols,
            vec!["a", "b", "c", "d"],
            "composition node must show its body's output columns, got {comp_cols:?}",
        );
    }

    /// #154 CONNECTIVITY (the test proving #155 can descend): the composition is not
    /// disconnected — tracing UPSTREAM from the downstream consumer `out` reaches the
    /// composition `comp` via a `Boundary` edge. Specifically the computed columns
    /// `c` and `d` (which originate inside the body) each have an upstream
    /// `comp.col → out.col` edge tagged `FieldEdgeKind::Boundary`. Without the
    /// synthesized comp output rows + re-tag pass, `out` would draw its columns
    /// straight from its own typed row with ZERO upstream lineage through comp.
    #[test]
    fn composition_output_crossing_connects_comp_to_consumer() {
        let plan = compiled_plan_fixture();
        let view = derive_resolved_pipeline_view(&plan);
        let i_comp = comp_idx(&view);
        let i_out = stage_idx(&view, "out");

        for col in ["c", "d"] {
            let crossing = view.field_edges.iter().find(|e| {
                e.from_node == i_comp
                    && e.from_field == col
                    && e.to_node == i_out
                    && e.to_field == col
            });
            let crossing = crossing.unwrap_or_else(|| {
                panic!(
                    "expected an upstream edge comp.{col} → out.{col}; \
                     got {:?}",
                    view.field_edges,
                )
            });
            assert_eq!(
                crossing.kind,
                FieldEdgeKind::Boundary,
                "the comp.{col} → out.{col} crossing must be tagged Boundary so a \
                 trace upstream from `out` reaches the composition"
            );
        }

        // The carried columns a, b also cross the wall as Boundary edges.
        for col in ["a", "b"] {
            assert!(
                view.field_edges.iter().any(|e| {
                    e.from_node == i_comp
                        && e.from_field == col
                        && e.to_node == i_out
                        && e.kind == FieldEdgeKind::Boundary
                }),
                "carried column {col} must also cross comp → out as a Boundary edge",
            );
        }

        // Every edge LEAVING the composition is a Boundary crossing — no plain
        // carry/derive leaks out of the wall.
        for edge in &view.field_edges {
            if edge.from_node == i_comp {
                assert_eq!(
                    edge.kind,
                    FieldEdgeKind::Boundary,
                    "every edge leaving the composition must be a Boundary crossing: {edge:?}",
                );
            }
        }
    }

    /// #154: boundary edges are STABLE across two independent builds of the same
    /// plan — the 5-tuple identity `(from_node, from_field, to_node, to_field,
    /// kind)` set is identical, so the canvas memoization and the #155 contract see
    /// a deterministic edge set (no set-iteration order leaking into the result).
    #[test]
    fn composition_boundary_edges_are_stable_across_builds() {
        let plan = compiled_plan_fixture();
        let view_a = derive_resolved_pipeline_view(&plan);
        let view_b = derive_resolved_pipeline_view(&plan);

        let boundary_5tuples = |view: &PipelineView| {
            view.field_edges
                .iter()
                .filter(|e| e.kind == FieldEdgeKind::Boundary)
                .map(|e| {
                    (
                        e.from_node,
                        e.from_field.clone(),
                        e.to_node,
                        e.to_field.clone(),
                        e.kind,
                    )
                })
                .collect::<std::collections::HashSet<_>>()
        };

        let a = boundary_5tuples(&view_a);
        let b = boundary_5tuples(&view_b);
        assert!(
            !a.is_empty(),
            "the fixture must materialize at least one boundary edge"
        );
        assert_eq!(
            a, b,
            "boundary edge identity set must be identical across two builds"
        );
    }

    /// #154 (companion to `edge_kind_nature_partition_is_exhaustive`): a materialized
    /// boundary edge is a DIRECT crossing — the trace follows it as part of the
    /// value graph.
    #[test]
    fn composition_boundary_edges_have_direct_nature() {
        let plan = compiled_plan_fixture();
        let view = derive_resolved_pipeline_view(&plan);
        let mut saw_boundary = false;
        for edge in &view.field_edges {
            if edge.kind == FieldEdgeKind::Boundary {
                saw_boundary = true;
                assert_eq!(
                    edge.kind.nature(),
                    EdgeNature::Direct,
                    "a composition boundary crossing is a DIRECT edge"
                );
            }
        }
        assert!(saw_boundary, "the fixture materializes boundary edges");
    }

    #[test]
    fn body_view_uses_body_scoped_rows_for_fields_and_edges() {
        let body = compiled_body_fixture();
        let view = derive_body_view(&body);

        let first_idx = stage_idx(&view, "first");
        let second_idx = stage_idx(&view, "second");
        let first = &view.stages[first_idx];

        for (stage, field, ty) in [
            (first_idx, "a", "int"),
            (first_idx, "b", "string"),
            (first_idx, "c", "int"),
            (second_idx, "a", "int"),
            (second_idx, "b", "string"),
            (second_idx, "c", "int"),
            (second_idx, "d", "int"),
        ] {
            let row = field_by_name(&view, stage, field);
            assert_eq!(row.kind, FieldKind::Declared);
            assert_eq!(row.ty.as_deref(), Some(ty));
        }
        assert!(
            first
                .fields
                .iter()
                .any(|field| field.name == "$source.name"),
            "body view should render the full engine row, including engine/system fields"
        );

        // Same-name carries `first → second` survive #174's derive awareness. `a`
        // and `b` ride through `second` unread → pure `Passthrough`. `c` ALSO rides
        // through unchanged but is READ by `second`'s `emit d = c + 1`, so its carry
        // is the more precise `Access` kind (#72) — exactly the classification the
        // top-level resolved path computes for a carried-and-accessed column. (The
        // carry-only code could not distinguish these and drew every same-name carry
        // as `Passthrough`.)
        for (field, kind) in [
            ("a", FieldEdgeKind::Passthrough),
            ("b", FieldEdgeKind::Passthrough),
            ("c", FieldEdgeKind::Access),
        ] {
            assert!(
                view.field_edges.contains(&FieldEdge {
                    from_node: first_idx,
                    from_field: field.to_string(),
                    to_node: second_idx,
                    to_field: field.to_string(),
                    kind,
                    ..Default::default()
                }),
                "expected body {kind:?} carry edge for {field}, got {:?}",
                view.field_edges
            );
        }
        // #174: the body view is now derive-aware. `second: emit d = c + 1` derives
        // `d` from `first.c`, and `first: emit c = a + 1` derives `c` from the seeded
        // input column `a` — so both computed columns draw a `Derive` cable to their
        // producer column. Precision is excluded from `FieldEdge`'s PartialEq, so the
        // `..Default::default()` pattern matches the edge identity regardless of tier.
        let src_idx = stage_idx(&view, "src");
        assert!(
            view.field_edges.contains(&FieldEdge {
                from_node: first_idx,
                from_field: "c".to_string(),
                to_node: second_idx,
                to_field: "d".to_string(),
                kind: FieldEdgeKind::Derive,
                ..Default::default()
            }),
            "body view must derive second.d from first.c: {:?}",
            view.field_edges
        );
        assert!(
            view.field_edges.contains(&FieldEdge {
                from_node: src_idx,
                from_field: "a".to_string(),
                to_node: first_idx,
                to_field: "c".to_string(),
                kind: FieldEdgeKind::Derive,
                ..Default::default()
            }),
            "body view must derive first.c from src.a: {:?}",
            view.field_edges
        );
    }

    /// #180 PR2 GAP-1 schema-source characterization: this is WHY the predecessor body
    /// row's DECLARED columns suffice as the aggregate input schema.
    ///
    /// The aggregate input schema is reconstructed from the predecessor body row (port
    /// order); its declared prefix matches the engine's `field_types` order exactly. The
    /// only place the two could diverge is the inferred-trailing (open-tail) region — a
    /// column the caller supplies that the port did not declare. But an aggregate cannot
    /// reference such a column: CXL `resolve` binds a `FieldRef` only against the
    /// explicitly declared field set, so reading an undeclared (open-tail) column is a
    /// compile-time `E200` "unresolved identifier" and the body never exists. So every
    /// `BindingArg::Field(idx)` the adapter can ever see points at a DECLARED column,
    /// which the body row reproduces in exactly the order `Field(idx)` expects — the
    /// open-tail region is never indexed. If a future engine change lets an aggregate
    /// fold open-tail columns, this test flips (the body would compile) — a deliberate
    /// signal to revisit the schema source.
    #[test]
    fn body_aggregate_referencing_open_tail_column_does_not_compile() {
        let result = try_compile_open_tail_aggregate_reference();
        let diags = result.expect_err(
            "a body aggregate folding an undeclared (open-tail) column must NOT compile \
             — if it now does, the GAP-1 Field(idx) schema-source assumption needs review",
        );
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("unresolved identifier")),
            "expected an E200 unresolved-identifier diagnostic for the open-tail read, got {diags:?}",
        );
    }

    /// #180 PR2 Finding 1 regression: a body Aggregate DOWNSTREAM of a Route still
    /// traces its computed column to the folded input.
    ///
    /// The aggregate's body-graph predecessor is the Route, which carries NO engine
    /// `stored_output_schema` (`None` for Route/Output/Sort/CorrelationCommit). The
    /// input schema is therefore reconstructed from the Route's body row (populated —
    /// a Route preserves its input schema), so `emit total = sum(x)` still derives
    /// `agg.total` from `x`. Sourcing the schema from the engine accessor instead would
    /// yield an empty schema here and drop every aggregate-derive edge.
    #[test]
    fn body_view_aggregate_downstream_of_route_traces_derive_edges() {
        let body = compiled_route_then_aggregate_body_fixture();
        let view = derive_body_view(&body);
        let split_idx = stage_idx(&view, "split");
        let agg_idx = stage_idx(&view, "agg");

        // The derive edge `<route>.x → agg.total` IS drawn — the Route predecessor's
        // body row supplied the aggregate input schema.
        assert!(
            view.field_edges.contains(&FieldEdge {
                from_node: split_idx,
                from_field: "x".to_string(),
                to_node: agg_idx,
                to_field: "total".to_string(),
                kind: FieldEdgeKind::Derive,
                ..Default::default()
            }),
            "a Route-fed body Aggregate must still derive agg.total from x (Finding 1): {:?}",
            view.field_edges
        );
        // And the group-by edge for `k` is likewise present.
        assert!(
            view.field_edges.iter().any(|e| {
                e.to_node == agg_idx && e.to_field == "k" && e.kind == FieldEdgeKind::GroupBy
            }),
            "a Route-fed body Aggregate must still draw the GroupBy edge for k: {:?}",
            view.field_edges
        );
    }

    /// #180 PR2 Finding 2 regression: a body Aggregate resolves `BindingArg::Field(idx)`
    /// in PORT order, not the parent's column order.
    ///
    /// The input port declares `[k, x]` while the caller supplies `[x, k]` (parent
    /// order). The engine binds `Field(idx)` in PORT order, so `sum(x)` is `Field(1)`.
    /// The adapter must resolve it against the predecessor body row (`[k, x, …]`,
    /// port order) → `x`. The engine input-port Source's `stored_output_schema` carries
    /// PARENT order (`[x, k, …]`), so resolving `Field(1)` against IT would wrongly
    /// target `k`.
    #[test]
    fn body_view_aggregate_resolves_field_idx_in_port_order() {
        let body = compiled_port_reordered_aggregate_body_fixture();
        let view = derive_body_view(&body);
        let src_idx = stage_idx(&view, "src");
        let agg_idx = stage_idx(&view, "agg");

        // The derive edge targets `x` (the folded column), NOT `k` — proving the
        // adapter indexed `Field(idx)` against the port-order body row.
        assert!(
            view.field_edges.contains(&FieldEdge {
                from_node: src_idx,
                from_field: "x".to_string(),
                to_node: agg_idx,
                to_field: "total".to_string(),
                kind: FieldEdgeKind::Derive,
                ..Default::default()
            }),
            "sum(x) must derive agg.total from `x` under a reordered port (Finding 2): {:?}",
            view.field_edges
        );
        assert!(
            !view.field_edges.iter().any(|e| {
                e.from_node == src_idx
                    && e.from_field == "k"
                    && e.to_node == agg_idx
                    && e.to_field == "total"
                    && e.kind == FieldEdgeKind::Derive
            }),
            "sum(x) must NOT derive agg.total from `k` (the parent-order mis-resolution): {:?}",
            view.field_edges
        );

        // Single source of truth: the schema the adapter resolves against IS the
        // predecessor body row in port order, distinct from the parent order here.
        let port_order: Vec<String> = body
            .body_rows
            .get("src")
            .expect("src body row")
            .fields()
            .map(|(qf, _)| qf.name.to_string())
            .collect();
        assert_eq!(
            port_order.iter().position(|c| c == "k"),
            Some(0),
            "the port row declares `k` first (port order), {port_order:?}",
        );
        assert_eq!(
            port_order.iter().position(|c| c == "x"),
            Some(1),
            "the port row declares `x` second (port order), {port_order:?}",
        );
    }

    /// #180 PR2 GAP 1 + GAP 2: a body Aggregate's computed column traces to the input
    /// column it folds, its group-by draws a `GroupBy` edge, and the
    /// `BindingArg::Field(idx)` → column-name resolution maps to the RIGHT column.
    #[test]
    fn body_view_aggregate_traces_computed_column_to_folded_input() {
        use clinker_plan::plan::execution::PlanNode;

        let body = compiled_aggregating_body_fixture();
        let view = derive_body_view(&body);
        let src_idx = stage_idx(&view, "src");
        let agg_idx = stage_idx(&view, "agg");

        // GAP 1: `emit total = sum(x)` derives `agg.total` from the aggregate's input
        // column `x` — the cable that was a dead end before the adapter.
        assert!(
            view.field_edges.contains(&FieldEdge {
                from_node: src_idx,
                from_field: "x".to_string(),
                to_node: agg_idx,
                to_field: "total".to_string(),
                kind: FieldEdgeKind::Derive,
                ..Default::default()
            }),
            "body Aggregate must derive agg.total from src.x (GAP 1): {:?}",
            view.field_edges
        );

        // GAP 2: the `k` group-by draws a `GroupBy` influence edge to the grouped row.
        assert!(
            view.field_edges.contains(&FieldEdge {
                from_node: src_idx,
                from_field: "k".to_string(),
                to_node: agg_idx,
                to_field: "k".to_string(),
                kind: FieldEdgeKind::GroupBy,
                ..Default::default()
            }),
            "body Aggregate must draw a GroupBy edge for k (GAP 2): {:?}",
            view.field_edges
        );

        // GAP 1 SHARP EDGE: assert the `BindingArg::Field(idx)` → column resolution
        // directly. The single binding folds `sum(x)`, so its arg is `Field(idx)`
        // where the aggregate's input schema at `idx` is `x` — NOT some other column.
        // This is the one place the adapter can silently produce a wrong column.
        let agg_node = body
            .topo_order
            .iter()
            .find_map(|&i| match &body.graph[i] {
                node @ PlanNode::Aggregation { .. } => Some(node),
                _ => None,
            })
            .expect("body has an Aggregation node");
        let PlanNode::Aggregation { compiled, .. } = agg_node else {
            unreachable!("matched Aggregation above")
        };
        // Source the schema the SAME way production does — the aggregate's predecessor
        // body row (here `src`) in PORT order, which is the index space `Field(idx)`
        // was assigned against. (`stages["src"].fields` is built from this same row.)
        let agg_input_schema: Vec<String> = body
            .body_rows
            .get("src")
            .expect("src body row")
            .fields()
            .map(|(qf, _)| qf.name.to_string())
            .collect();
        let binding = compiled
            .bindings
            .first()
            .expect("sum(x) produces one binding");
        let cxl::plan::BindingArg::Field(idx) = &binding.arg else {
            panic!("sum(x) binds a bare field, got {:?}", binding.arg);
        };
        assert_eq!(
            agg_input_schema.get(*idx as usize).map(String::as_str),
            Some("x"),
            "BindingArg::Field({idx}) must resolve to `x` in the input schema \
             {agg_input_schema:?}, not another column",
        );

        // The same resolution flows through the adapter: the precomputed support for
        // the `total` emit is exactly `{x}`.
        let supports = aggregate_emit_supports(compiled, &agg_input_schema);
        let total_support = supports
            .iter()
            .find(|(name, _)| name == "total")
            .map(|(_, s)| s)
            .expect("adapter produced a support set for `total`");
        assert!(
            total_support.contains("x") && total_support.len() == 1,
            "aggregate_emit_supports must resolve total's support to {{x}}, got {total_support:?}",
        );
    }

    /// #180 PR2 GAP 3: a body Merge that genuinely fans in two DISTINCT producers of
    /// the same column grades that carry `Approximate` (`conservative_fan_in`), while a
    /// single-producer carry elsewhere in the body stays `Exact`.
    #[test]
    fn body_view_multi_producer_merge_grades_fan_in_conservatively() {
        let body = compiled_multi_producer_merge_body_fixture();
        let view = derive_body_view(&body);
        let t1_idx = stage_idx(&view, "t1");
        let t2_idx = stage_idx(&view, "t2");
        let joined_idx = stage_idx(&view, "joined");
        let split_idx = stage_idx(&view, "split");

        // The Merge sees `tag` from BOTH t1 and t2 (two distinct producer slots), so
        // each carry into `joined.tag` is graded Approximate.
        for producer in [t1_idx, t2_idx] {
            let carry = view
                .field_edges
                .iter()
                .find(|e| {
                    e.from_node == producer
                        && e.from_field == "tag"
                        && e.to_node == joined_idx
                        && e.to_field == "tag"
                })
                .unwrap_or_else(|| {
                    panic!(
                        "expected a tag carry from producer {producer} into the merge: {:?}",
                        view.field_edges
                    )
                });
            assert_eq!(
                carry.kind,
                FieldEdgeKind::Passthrough,
                "a fan-in carry is still a Passthrough kind"
            );
            assert_eq!(
                carry.precision,
                Precision::Approximate,
                "a multi-producer merge fan-in carry grades Approximate (GAP 3): {:?}",
                view.field_edges
            );
        }

        // Contrast: a single-producer carry (split → t2, one predecessor) stays Exact.
        let single = view
            .field_edges
            .iter()
            .find(|e| {
                e.from_node == split_idx
                    && e.from_field == "k"
                    && e.to_node == t2_idx
                    && e.to_field == "k"
            })
            .expect("split → t2 carries k from a single producer");
        assert_eq!(
            single.precision,
            Precision::Exact,
            "a single-producer carry stays Exact: {:?}",
            view.field_edges
        );
    }

    /// A body Route draws same-name passthrough carries AND, after #180 PR2 GAP 2, the
    /// `Conditional` influence edges its branch condition imposes.
    ///
    /// Before GAP 2 the body emitted no influence edges, so this test asserted the
    /// Route drew ONLY `Passthrough` carries. That assertion is now deliberately
    /// flipped: the Route's branch condition (`a > 0`) reads `a`, so `a` draws a
    /// `Conditional` edge to each surviving row — exactly the top-level Route's
    /// behavior. The same-name passthrough carries still ride through, and no derive
    /// edge appears (a Route computes nothing).
    #[test]
    fn body_view_route_emits_conditional_influence_and_passthrough_carries() {
        let body = compiled_routing_body_fixture();
        let view = derive_body_view(&body);

        let route_idx = stage_idx(&view, "split");

        // GAP 2: the Route's `a > 0` condition reads `a`, so `a` now draws a
        // `Conditional` influence edge to surviving rows on the Route.
        assert!(
            view.field_edges.iter().any(|e| {
                e.to_node == route_idx
                    && e.from_field == "a"
                    && e.kind == FieldEdgeKind::Conditional
            }),
            "body Route must draw a Conditional edge from its predicate column `a` (GAP 2): {:?}",
            view.field_edges
        );
        // Every edge incident to the Route is EITHER a same-name passthrough carry OR a
        // `Conditional` influence edge — never anything else, and never a derive.
        assert!(
            view.field_edges
                .iter()
                .filter(|e| e.from_node == route_idx || e.to_node == route_idx)
                .all(|e| matches!(
                    e.kind,
                    FieldEdgeKind::Passthrough | FieldEdgeKind::Conditional
                )),
            "a Route node draws only passthrough carries and Conditional influence: {:?}",
            view.field_edges
        );
        // The carry fallback is still live: at least one same-name passthrough rides
        // through the Route.
        assert!(
            view.field_edges.iter().any(|e| {
                (e.from_node == route_idx || e.to_node == route_idx)
                    && e.kind == FieldEdgeKind::Passthrough
            }),
            "the no-program fallback must still draw same-name carries: {:?}",
            view.field_edges
        );
        // No edge anywhere in this CXL-light body is a Derive (no Transform computes).
        assert!(
            view.field_edges
                .iter()
                .all(|e| e.kind != FieldEdgeKind::Derive),
            "a body with no computing Transform draws no derive edges: {:?}",
            view.field_edges
        );
    }

    /// #180 PR2 GAP 2: a body Cull's `drop_group_when` removal predicate draws a
    /// `Filter` influence edge from each column it reads to the surviving rows — the
    /// body analogue of the top-level Cull `Filter` edges.
    #[test]
    fn body_view_cull_emits_filter_influence_edges() {
        let body = compiled_culling_body_fixture();
        let view = derive_body_view(&body);
        let cull_idx = stage_idx(&view, "prune");

        // The predicate `sum(if status == 'error' ...)` reads `status`, so `status`
        // draws a `Filter` edge to surviving Cull rows.
        assert!(
            view.field_edges.iter().any(|e| {
                e.to_node == cull_idx && e.from_field == "status" && e.kind == FieldEdgeKind::Filter
            }),
            "body Cull must draw a Filter edge from its predicate column `status`: {:?}",
            view.field_edges
        );
        // Every Cull edge is EITHER a same-name passthrough carry OR a `Filter`
        // influence edge (a Cull computes nothing and groups nothing).
        assert!(
            view.field_edges
                .iter()
                .filter(|e| e.from_node == cull_idx || e.to_node == cull_idx)
                .all(|e| matches!(e.kind, FieldEdgeKind::Passthrough | FieldEdgeKind::Filter)),
            "a body Cull draws only passthrough carries and Filter influence: {:?}",
            view.field_edges
        );
    }

    #[test]
    fn body_view_missing_row_data_keeps_node_level_fallback() {
        let mut body = compiled_body_fixture();
        body.body_rows.remove("first");
        let view = derive_body_view(&body);

        let first_idx = stage_idx(&view, "first");
        let second_idx = stage_idx(&view, "second");
        assert!(
            view.connections
                .iter()
                .any(|edge| edge.from == first_idx && edge.to == second_idx),
            "node-level body connector should remain when row data is missing"
        );
        assert!(
            view.stages[first_idx].fields.is_empty(),
            "missing body row data degrades this node to node-level only"
        );
        assert!(
            view.field_edges
                .iter()
                .all(|edge| edge.from_node != first_idx && edge.to_node != first_idx),
            "missing row data should suppress field edges for that node: {:?}",
            view.field_edges
        );
    }

    /// #155 recovery contract: `derive_body_scope` projects the body's port→NodeIndex
    /// maps into the `derive_body_view` slot space (slot == topo position). The engine
    /// seeds each input port as a dedicated body Source node, so the fixture body view
    /// is `src → first → second`: `port_name_to_node_idx["src"]` is the body `src`
    /// node (slot 0) and `output_port_to_node_idx["result"]` is `second`. These are the
    /// boundary slots #155's descent enters at / resurfaces from.
    #[test]
    fn body_scope_maps_ports_to_view_slots() {
        let body = compiled_body_fixture();
        let scope = derive_body_scope(&body);

        let src_idx = stage_idx(&scope.view, "src");
        let second_idx = stage_idx(&scope.view, "second");

        assert_eq!(
            scope.input_port_to_slot.get("src").copied(),
            Some(src_idx),
            "input port `src` seeds the body `src` source node: {:?}",
            scope.input_port_to_slot,
        );
        assert_eq!(
            scope.output_port_to_slot.get("result").copied(),
            Some(second_idx),
            "output port `result` is surfaced by body node `second`: {:?}",
            scope.output_port_to_slot,
        );
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

    /// `resolve_support_anchors` bridges alias-qualified CXL refs to the
    /// bare-keyed producer map across all three branches: a port alias pins one
    /// predecessor (precise, even for a shared column), an unknown qualifier and
    /// a bare ref both fan to every producer of the bare column, and a name no
    /// input produced resolves to nothing.
    #[test]
    fn resolve_support_anchors_three_branches() {
        use std::collections::HashMap;
        // `product_code` lives in both inputs (node 0 = orders, node 1 = products);
        // `order_id` only in orders; `product_name` only in products.
        let producers_of: HashMap<String, Vec<usize>> = HashMap::from([
            ("product_code".to_string(), vec![0, 1]),
            ("order_id".to_string(), vec![0]),
            ("product_name".to_string(), vec![1]),
        ]);
        let aliases: HashMap<String, usize> =
            HashMap::from([("orders".to_string(), 0), ("products".to_string(), 1)]);

        // Alias pins the producer: a shared column copied from `orders` resolves
        // to node 0 ONLY — never fanning to the products side that also has it.
        assert_eq!(
            resolve_support_anchors("orders.product_code", &producers_of, &aliases),
            vec![(0, "product_code".to_string())]
        );
        assert_eq!(
            resolve_support_anchors("products.product_code", &producers_of, &aliases),
            vec![(1, "product_code".to_string())]
        );

        // A bare ref fans to every producer of the column (#67 shared fan-in).
        let mut bare = resolve_support_anchors("product_code", &producers_of, &aliases);
        bare.sort();
        assert_eq!(
            bare,
            vec![
                (0, "product_code".to_string()),
                (1, "product_code".to_string())
            ]
        );

        // An unknown qualifier (`src.col` style) falls back to the bare column.
        assert_eq!(
            resolve_support_anchors("src.order_id", &producers_of, &aliases),
            vec![(0, "order_id".to_string())]
        );

        // A nested access through a known alias (`alias.record.subfield`) pins
        // the FIRST post-alias segment — the real producer column — not the
        // dotted remainder the bare-keyed map has no key for.
        assert_eq!(
            resolve_support_anchors("orders.product_code.len", &producers_of, &aliases),
            vec![(0, "product_code".to_string())],
            "a nested access resolves to its first-segment column on the aliased port"
        );

        // A known alias whose column it does not produce → no edge (we never fan
        // to another input to "find" a missing column).
        assert!(
            resolve_support_anchors("orders.product_name", &producers_of, &aliases).is_empty(),
            "orders has no product_name, so an orders.product_name ref draws nothing"
        );

        // A name no input produced (intra-node let / typo) → no anchors.
        assert!(resolve_support_anchors("nonexistent", &producers_of, &aliases).is_empty());
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
        // `c` with its conservatively inferred type (#149: arithmetic → numeric).
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
                    ty: Some("numeric".to_string()),
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
            kind: FieldEdgeKind::Derive,
            ..Default::default()
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

    /// Aggregate nodes produce grouped rows, not transform-style passthrough
    /// rows. Group keys appear first and derive from the matching input fields;
    /// aggregate emits derive from the input fields their CXL expressions read.
    #[test]
    fn aggregate_fields_are_group_keys_and_emits_with_lineage() {
        let yaml = r#"
pipeline:
  name: aggregate_lineage
nodes:
  - type: source
    name: orders
    config:
      name: orders
      type: csv
      path: ./orders.csv
      schema:
        - { name: department, type: string }
        - { name: amount, type: float }
        - { name: status, type: string }
  - type: aggregate
    name: totals
    input: orders
    config:
      group_by: [department]
      cxl: |
        emit total = sum(amount)
        emit row_count = count(*)
"#;
        let config = parse_config(yaml).expect("aggregate fixture parses");
        let view = derive_pipeline_view(&config);

        let i_orders = stage_idx(&view, "orders");
        let i_totals = stage_idx(&view, "totals");

        assert_eq!(
            view.stages[i_totals].fields,
            vec![
                FieldRow {
                    name: "department".to_string(),
                    kind: FieldKind::PassThrough,
                    ty: Some("string".to_string()),
                    // The group key is incident to its INDIRECT GroupBy edge, so
                    // its row precision folds down to Approximate (#148).
                    lineage_precision: Precision::Approximate,
                    precision_reason: "INDIRECT group-by grain influence",
                    ..Default::default()
                },
                FieldRow {
                    name: "total".to_string(),
                    kind: FieldKind::Emitted,
                    ..Default::default()
                },
                FieldRow {
                    name: "row_count".to_string(),
                    kind: FieldKind::Emitted,
                    ..Default::default()
                },
            ],
            "aggregate rows should be group keys plus emits only"
        );
        assert!(
            view.stages[i_totals]
                .fields
                .iter()
                .all(|row| row.name != "amount" && row.name != "status"),
            "non-group input fields must not pass through aggregate rows"
        );

        // The group key is an INDIRECT GROUP_BY influence edge (#147), not a
        // value derive — it defines the grouped grain. It is represented exactly
        // once (no duplicate Derive edge, no `is_aggregate_grain` row flag).
        let group_key_edge = FieldEdge {
            from_node: i_orders,
            from_field: "department".to_string(),
            to_node: i_totals,
            to_field: "department".to_string(),
            kind: FieldEdgeKind::GroupBy,
            ..Default::default()
        };
        let total_edge = FieldEdge {
            from_node: i_orders,
            from_field: "amount".to_string(),
            to_node: i_totals,
            to_field: "total".to_string(),
            kind: FieldEdgeKind::Derive,
            ..Default::default()
        };
        assert!(
            view.field_edges.contains(&group_key_edge),
            "group key should be a GroupBy edge from source department, got {:?}",
            view.field_edges
        );
        // The grain is represented EXACTLY ONCE: no stale Derive duplicate of the
        // GroupBy edge survives (#147 acceptance).
        assert!(
            !view.field_edges.iter().any(|edge| {
                edge.from_node == i_orders
                    && edge.from_field == "department"
                    && edge.to_node == i_totals
                    && edge.to_field == "department"
                    && edge.kind == FieldEdgeKind::Derive
            }),
            "the group-key grain must not also appear as a Derive edge, got {:?}",
            view.field_edges
        );
        assert!(
            view.field_edges.contains(&total_edge),
            "aggregate emit should derive from source amount, got {:?}",
            view.field_edges
        );
        assert!(
            !view
                .field_edges
                .iter()
                .any(|edge| edge.to_node == i_totals && edge.to_field == "status"),
            "unrelated source fields should not create aggregate passthrough edges"
        );
    }

    /// A Merge may carry the same field from multiple upstream producers, but an
    /// Aggregate consuming that merged row still has one immediate input row.
    /// When `group_by` and `emit` both name that key, the Aggregate keeps one
    /// output row and one incoming edge to it.
    #[test]
    fn aggregate_group_key_duplicate_emit_has_one_immediate_input_edge() {
        let yaml = r#"
pipeline:
  name: merge_then_aggregate
nodes:
  - type: source
    name: src_web
    config:
      name: src_web
      type: csv
      path: ./web.csv
      schema:
        - { name: user_id, type: string }
        - { name: event_ts, type: date_time }
  - type: source
    name: src_mobile
    config:
      name: src_mobile
      type: csv
      path: ./mobile.csv
      schema:
        - { name: user_id, type: string }
        - { name: event_ts, type: date_time }
  - type: merge
    name: all_logins
    inputs: [src_web, src_mobile]
  - type: aggregate
    name: user_sessions
    input: all_logins
    config:
      group_by: [user_id]
      cxl: |
        emit user_id = user_id
        emit logins = count(*)
"#;
        let config = parse_config(yaml).expect("merge aggregate fixture parses");
        let view = derive_pipeline_view(&config);

        let i_web = stage_idx(&view, "src_web");
        let i_mobile = stage_idx(&view, "src_mobile");
        let i_merge = stage_idx(&view, "all_logins");
        let i_aggregate = stage_idx(&view, "user_sessions");

        assert_eq!(
            view.stages[i_aggregate]
                .fields
                .iter()
                .filter(|row| row.name == "user_id")
                .count(),
            1,
            "group_by user_id and emit user_id should share one aggregate row"
        );
        assert_eq!(
            view.stages[i_aggregate]
                .role_ports
                .iter()
                .map(|port| (port.id.as_str(), port.role.as_str(), port.label.as_str()))
                .collect::<Vec<_>>(),
            vec![("group_by:user_id", "group_by", "user_id")],
            "group_by user_id should render as a distinct aggregate input role port"
        );

        for source in [i_web, i_mobile] {
            assert!(
                view.field_edges.iter().any(|edge| {
                    edge.from_node == source
                        && edge.from_field == "user_id"
                        && edge.to_node == i_merge
                        && edge.to_field == "user_id"
                        && edge.kind == FieldEdgeKind::Passthrough
                }),
                "each source user_id should feed the merge row"
            );
        }

        let outgoing_user_id_edges: Vec<&FieldEdge> = view
            .field_edges
            .iter()
            .filter(|edge| {
                edge.from_node == i_merge
                    && edge.from_field == "user_id"
                    && edge.to_node == i_aggregate
                    && edge.to_field == "user_id"
            })
            .collect();
        // The group key into the Aggregate is a single GroupBy influence edge
        // (#147), not a value Derive — and exactly one (the grain is represented
        // once, not duplicated per merged input).
        assert_eq!(
            outgoing_user_id_edges,
            vec![&FieldEdge {
                from_node: i_merge,
                from_field: "user_id".to_string(),
                to_node: i_aggregate,
                to_field: "user_id".to_string(),
                kind: FieldEdgeKind::GroupBy,
                ..Default::default()
            }],
            "merged user_id should have exactly one downstream edge into the aggregate group key"
        );
        assert_eq!(
            view.role_edges,
            vec![RoleEdge {
                from_node: i_merge,
                from_field: "user_id".to_string(),
                to_node: i_aggregate,
                to_port: "group_by:user_id".to_string(),
                kind: FieldEdgeKind::GroupBy,
            }],
            "merged user_id should also feed exactly one group_by role input port"
        );
    }

    /// Multiple Aggregate group keys render as separate semantic input ports,
    /// while each grouped field remains a single normal output row.
    #[test]
    fn aggregate_multiple_group_keys_render_distinct_role_ports() {
        let yaml = r#"
pipeline:
  name: invoice_rollup
nodes:
  - type: source
    name: invoices
    config:
      name: invoices
      type: csv
      path: ./invoices.csv
      schema:
        - { name: customer_id, type: string }
        - { name: invoice_date, type: string }
        - { name: amount, type: float }
  - type: aggregate
    name: daily_totals
    input: invoices
    config:
      group_by: [customer_id, invoice_date]
      cxl: |
        emit invoice_count = count(*)
"#;
        let config = parse_config(yaml).expect("multi-key aggregate fixture parses");
        let view = derive_pipeline_view(&config);

        let i_source = stage_idx(&view, "invoices");
        let i_aggregate = stage_idx(&view, "daily_totals");

        assert_eq!(
            view.stages[i_aggregate]
                .role_ports
                .iter()
                .map(|port| (port.id.as_str(), port.role.as_str(), port.label.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("group_by:customer_id", "group_by", "customer_id"),
                ("group_by:invoice_date", "group_by", "invoice_date")
            ],
            "each group key should render as its own aggregate input role port"
        );

        for field in ["customer_id", "invoice_date"] {
            assert_eq!(
                view.stages[i_aggregate]
                    .fields
                    .iter()
                    .filter(|row| row.name == field)
                    .count(),
                1,
                "group key {field} should remain one normal aggregate output row"
            );
            // The grain is the GroupBy edge (#147), represented exactly once —
            // no separate row flag.
            assert!(
                view.field_edges.iter().any(|edge| {
                    edge.from_node == i_source
                        && edge.from_field == field
                        && edge.to_node == i_aggregate
                        && edge.to_field == field
                        && edge.kind == FieldEdgeKind::GroupBy
                }),
                "group key {field} should have a GroupBy influence edge into the output row"
            );
            assert!(
                view.role_edges.iter().any(|edge| {
                    edge.from_node == i_source
                        && edge.from_field == field
                        && edge.to_node == i_aggregate
                        && edge.to_port == aggregate_group_key_port_id(field)
                        && edge.kind == FieldEdgeKind::GroupBy
                }),
                "group key {field} should also feed the matching role input port as GroupBy"
            );
        }
    }

    /// An Aggregate changes the failure/correlation grain from the source
    /// record key to the grouped record. Source CK fields that are also group
    /// keys keep their source-CK marker; non-CK group keys still get the
    /// aggregate failure-grain marker, and that marker follows downstream
    /// passthrough rows.
    #[test]
    fn aggregate_group_keys_mark_post_aggregate_failure_grain() {
        let yaml = r#"
pipeline:
  name: invoice_daily_rollup
nodes:
  - type: source
    name: invoices
    config:
      name: invoices
      type: csv
      path: ./invoices.csv
      correlation_key: [invoice_id, customer_id]
      schema:
        - { name: invoice_id, type: string }
        - { name: customer_id, type: string }
        - { name: invoice_date, type: string }
        - { name: amount, type: float }
  - type: aggregate
    name: daily_totals
    input: invoices
    config:
      group_by: [customer_id, invoice_date]
      cxl: |
        emit total_amount = sum(amount)
        emit invoice_count = count(*)
  - type: transform
    name: annotate
    input: daily_totals
    config:
      cxl: |
        emit rollup_customer = customer_id
  - type: output
    name: daily_rollup
    input: annotate
    config:
      name: daily_rollup
      type: csv
      path: ./daily-rollup.csv
"#;

        let config = parse_config(yaml).expect("invoice rollup fixture parses");
        let raw = derive_pipeline_view(&config);
        assert_invoice_rollup_grain(&raw);

        let plan = config
            .compile(&CompileContext::default())
            .expect("invoice rollup fixture compiles");
        let resolved = derive_resolved_pipeline_view(&plan);
        assert_invoice_rollup_grain(&resolved);
    }

    /// Whether a `GroupBy` influence edge lands on `field` as the group-key
    /// output row of stage `to_node` (#147) — the single representation of the
    /// aggregate grain.
    fn has_group_by_grain(view: &PipelineView, to_node: usize, field: &str) -> bool {
        view.field_edges.iter().any(|edge| {
            edge.kind == FieldEdgeKind::GroupBy && edge.to_node == to_node && edge.to_field == field
        })
    }

    fn assert_invoice_rollup_grain(view: &PipelineView) {
        let i_source = stage_idx(view, "invoices");
        let i_aggregate = stage_idx(view, "daily_totals");
        let i_transform = stage_idx(view, "annotate");

        assert!(field_by_name(view, i_source, "invoice_id").is_correlation_key);
        assert!(
            !view.stages[i_aggregate]
                .fields
                .iter()
                .any(|row| row.name == "invoice_id"),
            "invoice_id is a source-row CK and must not appear on grouped rows"
        );

        let aggregate_customer = field_by_name(view, i_aggregate, "customer_id");
        assert!(
            aggregate_customer.is_correlation_key,
            "customer_id remains the surviving source CK component"
        );
        // The grain is now the INDIRECT GroupBy edge, not a row flag (#147).
        assert!(
            has_group_by_grain(view, i_aggregate, "customer_id"),
            "customer_id is part of the aggregate grain (a GroupBy edge lands on it)"
        );

        let aggregate_date = field_by_name(view, i_aggregate, "invoice_date");
        assert!(
            !aggregate_date.is_correlation_key,
            "invoice_date was not declared as a source CK"
        );
        assert!(
            has_group_by_grain(view, i_aggregate, "invoice_date"),
            "invoice_date is part of the aggregate grain (a GroupBy edge lands on it)"
        );

        // The grain is represented EXACTLY ONCE — at the Aggregate node, as the
        // GroupBy edge. It does NOT re-propagate as a flag onto downstream
        // carried rows; those rows trace back to the GroupBy edge instead. The
        // carried columns still pass through as ordinary rows.
        for field in ["customer_id", "invoice_date"] {
            let carried = field_by_name(view, i_transform, field);
            assert_eq!(carried.kind, FieldKind::PassThrough);
            assert!(
                !has_group_by_grain(view, i_transform, field),
                "{field}'s grain is recorded once at the Aggregate, not duplicated downstream"
            );
        }

        let rollup_customer = field_by_name(view, i_transform, "rollup_customer");
        assert_eq!(rollup_customer.kind, FieldKind::Emitted);
    }

    /// A qualified group key is displayed as the aggregate output key while its
    /// lineage resolves to the matching bare producer field. This keeps the UI
    /// answer precise for cases like `source_b.field → aggregate.group_by`.
    #[test]
    fn aggregate_qualified_group_key_resolves_to_source_field() {
        let yaml = r#"
pipeline:
  name: aggregate_qualified_group_key
nodes:
  - type: source
    name: source_b
    config:
      name: source_b
      type: csv
      path: ./source_b.csv
      schema:
        - { name: field, type: string }
        - { name: amount, type: float }
  - type: aggregate
    name: grouped
    input: source_b
    config:
      group_by: [source_b.field]
      cxl: |
        emit n = count(*)
"#;
        let config = parse_config(yaml).expect("qualified group-key fixture parses");
        let view = derive_pipeline_view(&config);

        let i_source = stage_idx(&view, "source_b");
        let i_grouped = stage_idx(&view, "grouped");

        let group_key = field_by_name(&view, i_grouped, "source_b.field");
        assert_eq!(group_key.kind, FieldKind::PassThrough);
        assert_eq!(
            group_key.ty.as_deref(),
            Some("string"),
            "qualified group key should inherit the matching producer field type"
        );

        // The grain is the GroupBy edge (#147); the qualified key still resolves
        // its lineage to the bare producer field `field`.
        let edge = FieldEdge {
            from_node: i_source,
            from_field: "field".to_string(),
            to_node: i_grouped,
            to_field: "source_b.field".to_string(),
            kind: FieldEdgeKind::GroupBy,
            ..Default::default()
        };
        assert!(
            view.field_edges.contains(&edge),
            "qualified group key should draw a GroupBy edge from source_b.field, got {:?}",
            view.field_edges
        );

        // #148 S2: the grain row name (`source_b.field`) and the GroupBy edge's
        // `to_field` (`source_b.field`) agree in the raw path — both are the
        // qualified key — so `derive_row_precision` folds the grain row to
        // Approximate. A bare/qualified mismatch would leave it Exact (the bug S2
        // guards against).
        assert_eq!(
            group_key.lineage_precision,
            Precision::Approximate,
            "the qualified grain row must fold to Approximate from its GroupBy edge"
        );
        assert_eq!(
            group_key.precision_reason,
            "INDIRECT group-by grain influence"
        );
    }

    /// Invalid CXL cannot be trusted for emit extraction. Aggregate group keys
    /// still render from normal node config, but no lineage edges are inferred.
    #[test]
    fn aggregate_invalid_cxl_keeps_group_keys_without_edges() {
        let yaml = r#"
pipeline:
  name: aggregate_invalid_cxl
nodes:
  - type: source
    name: orders
    config:
      name: orders
      type: csv
      path: ./orders.csv
      schema:
        - { name: department, type: string }
        - { name: amount, type: float }
  - type: aggregate
    name: totals
    input: orders
    config:
      group_by: [department]
      cxl: |
        emit total =
"#;
        let config = parse_config(yaml).expect("invalid-CXL aggregate YAML parses");
        let view = derive_pipeline_view(&config);

        let i_totals = stage_idx(&view, "totals");
        assert_eq!(
            view.stages[i_totals].fields,
            vec![FieldRow {
                name: "department".to_string(),
                kind: FieldKind::PassThrough,
                ty: Some("string".to_string()),
                // CXL failed to parse, so edges were suppressed and the row carries
                // the degradation as Unknown precision (#148).
                lineage_precision: Precision::Unknown,
                precision_reason: "CXL did not parse; lineage edges suppressed",
                ..Default::default()
            }],
            "invalid aggregate CXL should degrade to config-derived group keys"
        );
        assert!(
            view.field_edges
                .iter()
                .all(|edge| edge.to_node != i_totals && edge.from_node != i_totals),
            "invalid aggregate CXL should not infer edges, got {:?}",
            view.field_edges
        );
    }

    /// #147 acceptance: a Cull emits INDIRECT `Filter` edges from every column its
    /// removal predicate reads to EVERY surviving output row (the predicate
    /// decides which rows survive), AND keeps the ordinary DIRECT passthrough
    /// carries. A column the predicate does NOT read gets no Filter edge.
    #[test]
    fn cull_predicate_emits_filter_influence_edges() {
        let yaml = r#"
pipeline:
  name: cull_filter
nodes:
  - type: source
    name: src
    config:
      name: src
      type: csv
      path: ./in.csv
      schema:
        - { name: gid, type: string }
        - { name: status, type: string }
        - { name: amount, type: int }
  - type: cull
    name: prune
    input: src
    config:
      partition_by: [gid]
      removed_to: dropped
      rules:
        - name: drop_clean
          drop_group_when: "sum(if status == 'error' then 1 else 0) > 0"
"#;
        let config = parse_config(yaml).expect("cull filter fixture parses");
        let view = derive_pipeline_view(&config);
        let i_src = stage_idx(&view, "src");
        let i_cull = stage_idx(&view, "prune");

        // The predicate reads `status`; every surviving output row of the Cull is
        // a Filter target from `status` (gid, status, amount all survive).
        for row in ["gid", "status", "amount"] {
            assert!(
                view.field_edges.contains(&FieldEdge {
                    from_node: i_src,
                    from_field: "status".to_string(),
                    to_node: i_cull,
                    to_field: row.to_string(),
                    kind: FieldEdgeKind::Filter,
                    ..Default::default()
                }),
                "status should Filter-influence surviving row {row}, got {:?}",
                view.field_edges
            );
        }
        // `amount` is not read by the predicate, so it produces no Filter edge.
        assert!(
            !view.field_edges.iter().any(|e| {
                e.from_node == i_src && e.from_field == "amount" && e.kind == FieldEdgeKind::Filter
            }),
            "a column the predicate does not read must not emit a Filter edge"
        );
        // Every Filter edge is INDIRECT by nature.
        assert!(
            view.field_edges
                .iter()
                .filter(|e| e.kind == FieldEdgeKind::Filter)
                .all(|e| e.kind.nature() == EdgeNature::Indirect)
        );
        // The DIRECT passthrough carries still exist alongside the Filter edges.
        assert!(view.field_edges.contains(&FieldEdge {
            from_node: i_src,
            from_field: "status".to_string(),
            to_node: i_cull,
            to_field: "status".to_string(),
            kind: FieldEdgeKind::Passthrough,
            ..Default::default()
        }));
    }

    /// #147 acceptance: a Cull predicate that fails to parse infers NO Filter
    /// edges — never lineage from unparseable CXL (the degrade-gracefully rule).
    #[test]
    fn cull_unparseable_predicate_emits_no_filter_edges() {
        let yaml = r#"
pipeline:
  name: cull_bad_predicate
nodes:
  - type: source
    name: src
    config:
      name: src
      type: csv
      path: ./in.csv
      schema:
        - { name: gid, type: string }
        - { name: status, type: string }
  - type: cull
    name: prune
    input: src
    config:
      partition_by: [gid]
      removed_to: dropped
      rules:
        - name: broken
          drop_group_when: "status == == 'error'"
"#;
        let config = parse_config(yaml).expect("cull fixture parses even with bad predicate CXL");
        let view = derive_pipeline_view(&config);
        assert!(
            !view
                .field_edges
                .iter()
                .any(|e| e.kind == FieldEdgeKind::Filter),
            "an unparseable Cull predicate must infer no Filter edges, got {:?}",
            view.field_edges
        );
    }

    /// #147 acceptance: a Route emits INDIRECT `Conditional` edges from every
    /// column a branch condition reads to the Route's surviving output rows; the
    /// always-present default/fallback branch has no predicate, so emits none.
    #[test]
    fn route_conditions_emit_conditional_influence_edges() {
        let yaml = r#"
pipeline:
  name: route_conditional
nodes:
  - type: source
    name: src
    config:
      name: src
      type: csv
      path: ./in.csv
      schema:
        - { name: region, type: string }
        - { name: amount, type: int }
  - type: route
    name: split
    input: src
    config:
      conditions:
        eu: "region == 'EU'"
      default: rest
"#;
        let config = parse_config(yaml).expect("route conditional fixture parses");
        let view = derive_pipeline_view(&config);
        let i_src = stage_idx(&view, "src");
        let i_route = stage_idx(&view, "split");

        // The `eu` branch condition reads `region`; it conditionally influences
        // every surviving output row of the Route.
        for row in ["region", "amount"] {
            assert!(
                view.field_edges.contains(&FieldEdge {
                    from_node: i_src,
                    from_field: "region".to_string(),
                    to_node: i_route,
                    to_field: row.to_string(),
                    kind: FieldEdgeKind::Conditional,
                    ..Default::default()
                }),
                "region should Conditional-influence row {row}, got {:?}",
                view.field_edges
            );
        }
        // `amount` is not read by any condition → no Conditional edge from it.
        assert!(
            !view.field_edges.iter().any(|e| {
                e.from_node == i_src
                    && e.from_field == "amount"
                    && e.kind == FieldEdgeKind::Conditional
            }),
            "a column no branch condition reads must not emit a Conditional edge"
        );
        assert!(
            view.field_edges
                .iter()
                .any(|e| e.kind == FieldEdgeKind::Conditional),
            "the Route should emit at least one Conditional edge"
        );
    }

    /// #147 acceptance: a Merge is a streamwise row UNION
    /// (`MergeMode::Concat`/`Interleave`) — it stacks rows, it never joins — so a
    /// column shared by more than one input is a plain `Passthrough` value carry
    /// from EACH producer and carries NO `JoinKey` influence edge. (A join key is
    /// emitted only by a `Combine`, derived from its `where_expr`; see
    /// [`combine_multi_input_join_key_carries_from_every_input`].)
    #[test]
    fn merge_shared_column_is_passthrough_not_join_key() {
        let yaml = r#"
pipeline:
  name: merge_join_key
nodes:
  - type: source
    name: web
    config:
      name: web
      type: csv
      path: ./web.csv
      schema:
        - { name: user_id, type: string }
        - { name: web_only, type: int }
  - type: source
    name: mobile
    config:
      name: mobile
      type: csv
      path: ./mobile.csv
      schema:
        - { name: user_id, type: string }
        - { name: mobile_only, type: int }
  - type: merge
    name: all_events
    inputs: [web, mobile]
"#;
        let config = parse_config(yaml).expect("merge join-key fixture parses");
        let view = derive_pipeline_view(&config);
        let i_web = stage_idx(&view, "web");
        let i_mobile = stage_idx(&view, "mobile");
        let i_merge = stage_idx(&view, "all_events");

        // `user_id` is produced by both inputs → a plain Passthrough value carry
        // from EACH input (the #67 fan-in carry), and nothing more.
        for src in [i_web, i_mobile] {
            assert!(
                view.field_edges.contains(&FieldEdge {
                    from_node: src,
                    from_field: "user_id".to_string(),
                    to_node: i_merge,
                    to_field: "user_id".to_string(),
                    kind: FieldEdgeKind::Passthrough,
                    ..Default::default()
                }),
                "shared user_id should carry a Passthrough value edge from each input, got {:?}",
                view.field_edges
            );
        }
        // #148 S3(a): a CXL-less MULTI-producer fan-in carry is a conservative
        // over-approximation → Approximate. `FieldEdge` `==` ignores precision, so
        // the `.contains` checks above cannot catch a precision regression here —
        // assert each carry edge's `.precision` field directly.
        for src in [i_web, i_mobile] {
            let edge = view
                .field_edges
                .iter()
                .find(|e| {
                    e.from_node == src
                        && e.from_field == "user_id"
                        && e.to_node == i_merge
                        && e.to_field == "user_id"
                        && e.kind == FieldEdgeKind::Passthrough
                })
                .expect("the fan-in carry exists");
            assert_eq!(
                edge.precision,
                Precision::Approximate,
                "a CXL-less multi-producer fan-in carry is Approximate, got {edge:?}"
            );
        }
        // The merged row folds to Approximate from its conservative producing edges.
        let merged = view.stages[i_merge]
            .fields
            .iter()
            .find(|r| r.name == "user_id")
            .expect("merged user_id row");
        assert_eq!(
            merged.lineage_precision,
            Precision::Approximate,
            "the merged fan-in row folds to Approximate"
        );
        // A Merge performs no join, so it emits ZERO JoinKey edges — for the
        // shared column or any other.
        assert_eq!(
            view.field_edges
                .iter()
                .filter(|e| e.kind == FieldEdgeKind::JoinKey)
                .count(),
            0,
            "a Merge is a row UNION and must emit no JoinKey edges, got {:?}",
            view.field_edges
        );
    }

    /// #147 raw/resolved parity: the INDIRECT influence edges are emitted by BOTH
    /// lineage builders. For a normal typed pipeline (every node has a
    /// `typed_output_row`, so the resolved path does not `continue`), a Cull's
    /// `drop_group_when` must drive `Filter` edges in resolved mode just as it does
    /// in raw mode — locking the two paths together so an INDIRECT regression
    /// cannot hide in only one. (The resolved `continue` for a node WITHOUT a typed
    /// row is the deferred-acceptable gap; a normal pipeline never hits it.)
    #[test]
    fn resolved_mode_emits_cull_filter_edges_in_parity_with_raw() {
        let yaml = r#"
pipeline:
  name: cull_filter_parity
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
          drop_group_when: "sum(amount) < 100"
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
        let config = parse_config(yaml).expect("cull parity fixture parses");

        // The Filter edge from `amount` to the Cull's surviving rows: assert it in
        // BOTH the raw approximation and the engine-resolved view.
        let has_amount_filter = |view: &PipelineView| {
            let i_src = stage_idx(view, "src");
            let i_cull = stage_idx(view, "prune");
            view.field_edges.iter().any(|e| {
                e.from_node == i_src
                    && e.from_field == "amount"
                    && e.to_node == i_cull
                    && e.kind == FieldEdgeKind::Filter
            })
        };

        let raw = derive_pipeline_view(&config);
        assert!(
            has_amount_filter(&raw),
            "raw mode must emit the Cull Filter edge from amount, got {:?}",
            raw.field_edges
        );

        let plan = config
            .compile(&CompileContext::default())
            .expect("cull parity fixture compiles");
        let resolved = derive_resolved_pipeline_view(&plan);
        assert!(
            has_amount_filter(&resolved),
            "resolved mode must emit the Cull Filter edge from amount too (raw/resolved parity), got {:?}",
            resolved.field_edges
        );
    }

    /// #147 raw/resolved parity for COMBINE: a Combine's `where_expr`-driven
    /// JoinKey edges must be emitted by the schema-resolved builder too, not just
    /// the raw approximation — so a future resolved-path special-casing (e.g. the
    /// typed-output-row `continue`) cannot silently drop the join key. Mirrors
    /// `resolved_mode_emits_cull_filter_edges_in_parity_with_raw`.
    #[test]
    fn resolved_mode_emits_combine_join_key_edges_in_parity_with_raw() {
        let yaml = r#"
pipeline:
  name: combine_join_key_parity
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
  - type: output
    name: out
    input: joined
    config:
      name: out
      type: csv
      path: ./out.csv
"#;
        let config = parse_config(yaml).expect("combine parity fixture parses");

        // A JoinKey edge from EACH side's `product_code`, asserted in BOTH the raw
        // approximation and the engine-resolved view.
        let has_both_side_join_keys = |view: &PipelineView| {
            let i_products = stage_idx(view, "products");
            let i_inventory = stage_idx(view, "inventory");
            let i_joined = stage_idx(view, "joined");
            let from_side = |from: usize| {
                view.field_edges.iter().any(|e| {
                    e.from_node == from
                        && e.from_field == "product_code"
                        && e.to_node == i_joined
                        && e.kind == FieldEdgeKind::JoinKey
                })
            };
            from_side(i_products) && from_side(i_inventory)
        };

        let raw = derive_pipeline_view(&config);
        assert!(
            has_both_side_join_keys(&raw),
            "raw mode must emit Combine JoinKey edges from both sides, got {:?}",
            raw.field_edges
        );

        let plan = config
            .compile(&CompileContext::default())
            .expect("combine parity fixture compiles");
        let resolved = derive_resolved_pipeline_view(&plan);
        assert!(
            has_both_side_join_keys(&resolved),
            "resolved mode must emit Combine JoinKey edges from both sides too (raw/resolved parity), got {:?}",
            resolved.field_edges
        );
    }

    /// Resolved mode reads the engine's typed output rows, so emitted fields can
    /// carry compact type labels that the Raw approximation deliberately leaves
    /// unknown. The edge still resolves to the same field-row anchors.
    #[test]
    fn resolved_pipeline_fields_use_compiled_output_row_types() {
        let yaml = r#"
pipeline:
  name: resolved_types
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
  - type: output
    name: out
    input: t
    config:
      name: out
      type: csv
      path: ./out.csv
"#;
        let config = parse_config(yaml).expect("resolved fixture parses");
        let raw = derive_pipeline_view(&config);
        let plan = config
            .compile(&CompileContext::default())
            .expect("resolved fixture compiles");
        let resolved = derive_resolved_pipeline_view(&plan);

        let raw_t = stage_idx(&raw, "t");
        let raw_c = field_by_name(&raw, raw_t, "c");
        assert_eq!(raw_c.kind, FieldKind::Emitted);
        // Raw mode now infers a conservative type for the emit (#149): arithmetic
        // is the `numeric` over-approximation — a supertype of the engine's `int`.
        assert_eq!(
            raw_c.ty.as_deref(),
            Some("numeric"),
            "Raw mode infers arithmetic emits as numeric"
        );

        let resolved_t = stage_idx(&resolved, "t");
        let resolved_c = field_by_name(&resolved, resolved_t, "c");
        assert_eq!(resolved_c.kind, FieldKind::Emitted);
        assert_eq!(
            resolved_c.ty.as_deref(),
            Some("int"),
            "Resolved mode uses the engine's typed output row"
        );
        // The raw inference is conservative: it never contradicts the engine. The
        // inferred `numeric` unifies with the engine's `int` (numeric ⊇ int).
        let raw_ty = compact_type(&cxl::typecheck::Type::Numeric);
        assert!(
            cxl::typecheck::Type::Numeric
                .unify(&cxl::typecheck::Type::Int)
                .is_some(),
            "inferred {raw_ty} must be consistent with the engine's int"
        );

        let resolved_src = stage_idx(&resolved, "src");
        let derive_a_c = FieldEdge {
            from_node: resolved_src,
            from_field: "a".to_string(),
            to_node: resolved_t,
            to_field: "c".to_string(),
            kind: FieldEdgeKind::Derive,
            ..Default::default()
        };
        assert!(
            resolved.field_edges.contains(&derive_a_c),
            "resolved lineage edge should still target the typed row: {:?}",
            resolved.field_edges
        );
    }

    /// #149 validation harness: the raw type inferencer never contradicts the
    /// engine. For a fixture exercising every covered emit shape, each
    /// raw-inferred emitted type is *consistent* with the engine's compiled
    /// `typed_output_row` — identical, the `numeric` supertype of an engine
    /// int/float, or the liberal Unknown (`None`). This bounds the inferencer's
    /// error rate to zero on the covered shapes while letting it over-approximate
    /// (numeric) or abstain (None).
    #[test]
    fn raw_inferred_emit_types_are_consistent_with_compiled_truth() {
        let yaml = r#"
pipeline:
  name: inferred_types
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
        - { name: name, type: string }
  - type: transform
    name: t
    input: src
    config:
      cxl: |
        let w = a + 1
        emit lit_i = 1
        emit lit_s = "x"
        emit ari = a + 1
        emit cmp = a > 3
        emit logic = a > 1 and a < 10
        emit up = name.upper()
        emit chained = w * 2
        emit renamed = b
  - type: output
    name: out
    input: t
    config:
      name: out
      type: csv
      path: ./out.csv
"#;
        let config = parse_config(yaml).expect("inference fixture parses");
        let raw = derive_pipeline_view(&config);
        let plan = config
            .compile(&CompileContext::default())
            .expect("inference fixture compiles");
        let resolved = derive_resolved_pipeline_view(&plan);

        let raw_t = stage_idx(&raw, "t");
        let resolved_t = stage_idx(&resolved, "t");

        // Inferred-vs-engine consistency: Unknown is always safe; an exact match
        // is safe; `numeric` is the safe supertype of an engine int/float. The
        // raw inferencer does not track nullability, so an engine `T?` is
        // consistent with an inferred base `T`.
        let consistent = |raw: Option<&str>, engine: Option<&str>| {
            let engine_base = engine.map(|e| e.trim_end_matches('?'));
            match (raw, engine_base) {
                (None, _) => true,
                (Some(r), Some(e)) if r == e => true,
                (Some("numeric"), Some("int" | "float" | "numeric")) => true,
                _ => false,
            }
        };

        // Lock in the inferred raw label for each covered shape, and prove each
        // is consistent with the engine's compiled truth.
        let expected_raw = [
            ("lit_i", Some("int")),
            ("lit_s", Some("string")),
            ("ari", Some("numeric")),
            ("cmp", Some("bool")),
            ("logic", Some("bool")),
            ("up", Some("string")),
            ("chained", Some("numeric")),
            ("renamed", None), // bare input-column ref → Unknown in raw mode
        ];
        for (field, want) in expected_raw {
            let raw_ty = field_by_name(&raw, raw_t, field).ty.clone();
            assert_eq!(raw_ty.as_deref(), want, "raw inferred type for `{field}`");
            let engine_ty = field_by_name(&resolved, resolved_t, field).ty.clone();
            assert!(
                consistent(raw_ty.as_deref(), engine_ty.as_deref()),
                "raw `{field}`={raw_ty:?} contradicts engine {engine_ty:?}",
            );
        }
    }

    /// #150: a field fanned out inside `emit each` derives from the iterated
    /// source column. `emit each x in items { emit y = x.v }` must produce a
    /// derive edge `items -> y`, and no spurious carry/derive for `items` beyond
    /// it (the array column is consumed by the fan-out, not passed through).
    #[test]
    fn emit_each_source_binding_produces_derive_edge() {
        let yaml = r#"
pipeline:
  name: fan_out
nodes:
  - type: source
    name: src
    config:
      name: src
      type: csv
      path: ./in.csv
      schema:
        - { name: items, type: array }
  - type: transform
    name: t
    input: src
    config:
      cxl: |
        emit each x in items {
          emit y = x.v
        }
  - type: output
    name: out
    input: t
    config:
      name: out
      type: csv
      path: ./out.csv
"#;
        let config = parse_config(yaml).expect("fan-out fixture parses");
        let view = derive_pipeline_view(&config);
        let i_src = stage_idx(&view, "src");
        let i_t = stage_idx(&view, "t");

        let derive_items_y = FieldEdge {
            from_node: i_src,
            from_field: "items".to_string(),
            to_node: i_t,
            to_field: "y".to_string(),
            kind: FieldEdgeKind::Derive,
            ..Default::default()
        };
        assert!(
            view.field_edges.contains(&derive_items_y),
            "emit-each body field derives from the iterated source column: {:?}",
            view.field_edges,
        );

        // `items` derives exactly the one fanned-out field — no spurious derive
        // to any other column. (Its identity passthrough carry, unchanged by
        // #150, is a separate, legitimate edge.)
        let items_derives: Vec<_> = view
            .field_edges
            .iter()
            .filter(|e| {
                e.from_node == i_src && e.from_field == "items" && e.kind == FieldEdgeKind::Derive
            })
            .collect();
        assert_eq!(
            items_derives,
            vec![&derive_items_y],
            "the iterated source column derives only the fanned-out field",
        );
    }

    /// #96: top-level pipeline field lineage is not only populated from Source
    /// schemas; it also feeds the same hover and click-to-pin reveal helpers the
    /// canvas uses. Hover stays local to the adjacent field edge, while a pinned
    /// field follows the full directed lineage across the pipeline.
    #[test]
    fn pipeline_lineage_supports_hover_and_pin_reveal_sets() {
        let yaml = r#"
pipeline:
  name: pipeline_hover_pin_lineage
nodes:
  - type: source
    name: src
    config:
      name: src
      type: csv
      path: ./in.csv
      schema:
        - { name: a, type: int }
  - type: transform
    name: first
    input: src
    config:
      cxl: |
        emit b = a + 1
  - type: transform
    name: second
    input: first
    config:
      cxl: |
        emit c = b + 1
"#;
        let config = parse_config(yaml).expect("pipeline lineage fixture parses");
        let view = derive_pipeline_view(&config);

        let i_src = stage_idx(&view, "src");
        let i_first = stage_idx(&view, "first");
        let i_second = stage_idx(&view, "second");

        assert_eq!(field_by_name(&view, i_src, "a").kind, FieldKind::Declared);
        assert_eq!(field_by_name(&view, i_first, "b").kind, FieldKind::Emitted);
        assert_eq!(field_by_name(&view, i_second, "c").kind, FieldKind::Emitted);

        let a_to_b = FieldEdge {
            from_node: i_src,
            from_field: "a".to_string(),
            to_node: i_first,
            to_field: "b".to_string(),
            kind: FieldEdgeKind::Derive,
            ..Default::default()
        };
        let b_to_c = FieldEdge {
            from_node: i_first,
            from_field: "b".to_string(),
            to_node: i_second,
            to_field: "c".to_string(),
            kind: FieldEdgeKind::Derive,
            ..Default::default()
        };
        let edge_idx = |edge: &FieldEdge| {
            view.field_edges
                .iter()
                .position(|candidate| candidate == edge)
                .unwrap_or_else(|| panic!("expected edge {edge:?}, got {:?}", view.field_edges))
        };
        let a_to_b_idx = edge_idx(&a_to_b);
        let b_to_c_idx = edge_idx(&b_to_c);

        assert_eq!(
            lineage_closure(&view.field_edges, i_second, "c"),
            std::collections::HashSet::from([b_to_c_idx]),
            "hovering second.c should reveal only the adjacent producer edge"
        );
        assert_eq!(
            field_lineage_full(&view.field_edges, i_second, "c"),
            std::collections::HashSet::from([a_to_b_idx, b_to_c_idx]),
            "pinning second.c should reveal the full directed pipeline lineage"
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
        // each — two carry edges, not one (the #67 fix). It is a pure
        // Passthrough: it is not read by the node's emit (`available = on_hand`).
        let carry_from_products = FieldEdge {
            from_node: i_products,
            from_field: "product_code".to_string(),
            to_node: i_joined,
            to_field: "product_code".to_string(),
            kind: FieldEdgeKind::Passthrough,
            ..Default::default()
        };
        let carry_from_inventory = FieldEdge {
            from_node: i_inventory,
            from_field: "product_code".to_string(),
            to_node: i_joined,
            to_field: "product_code".to_string(),
            kind: FieldEdgeKind::Passthrough,
            ..Default::default()
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

        // #147: the Combine's `where_expr`
        // (`products.product_code == inventory.product_code`) drives an INDIRECT
        // `JoinKey` edge from EACH side's `product_code` to the joined output's
        // `product_code` row — derived from the predicate's read columns (the SAME
        // `predicate_support` path as Cull/Route), NOT a name-collision carry. It
        // COEXISTS with the DIRECT Passthrough value carries asserted above.
        let join_key_from_products = FieldEdge {
            from_node: i_products,
            from_field: "product_code".to_string(),
            to_node: i_joined,
            to_field: "product_code".to_string(),
            kind: FieldEdgeKind::JoinKey,
            ..Default::default()
        };
        let join_key_from_inventory = FieldEdge {
            from_node: i_inventory,
            from_field: "product_code".to_string(),
            to_node: i_joined,
            to_field: "product_code".to_string(),
            kind: FieldEdgeKind::JoinKey,
            ..Default::default()
        };
        assert!(
            view.field_edges.contains(&join_key_from_products),
            "where_expr must drive a JoinKey edge from `products.product_code`, got {:?}",
            view.field_edges
        );
        assert!(
            view.field_edges.contains(&join_key_from_inventory),
            "where_expr must drive a JoinKey edge from `inventory.product_code`, got {:?}",
            view.field_edges
        );

        // FAN-TO-EVERY-ROW POLICY (#147): a JoinKey influence edge connects each
        // join-key producer to EVERY surviving output row of the Combine — not
        // only the key row `product_code`. This is the policy the value/influence
        // axis-separation relies on (the `nature()==Direct` scoping of the
        // value-provenance counts below is only sound because JoinKey deliberately
        // lands on non-key rows too). Assert it lands on the carried non-key row
        // `on_hand` AND the computed emit row `available`, from BOTH sides — a
        // regression narrowing JoinKey to only the key row would fail HERE.
        for to_field in ["on_hand", "available"] {
            for from in [i_products, i_inventory] {
                assert!(
                    view.field_edges.contains(&FieldEdge {
                        from_node: from,
                        from_field: "product_code".to_string(),
                        to_node: i_joined,
                        to_field: to_field.to_string(),
                        kind: FieldEdgeKind::JoinKey,
                        ..Default::default()
                    }),
                    "JoinKey must fan from node {from}'s product_code to non-key row \
                     `{to_field}`, got {:?}",
                    view.field_edges
                );
            }
        }
        // Concretely: the join key influences ALL FOUR surviving rows
        // (product_code, product_name, on_hand, available) from each of the two
        // sides → eight JoinKey edges in total.
        assert_eq!(
            view.field_edges
                .iter()
                .filter(|e| e.to_node == i_joined && e.kind == FieldEdgeKind::JoinKey)
                .count(),
            8,
            "two producers × four surviving rows = eight JoinKey edges, got {:?}",
            view.field_edges
        );

        // A column unique to one input carries from exactly that input — the
        // fan-out is keyed on real producers, not on every predecessor. Scoped to
        // DIRECT value carries: the Combine's `where_expr` JoinKey influence
        // (#147) fans to EVERY surviving row (including `on_hand`), so it is
        // counted separately and does not change the value-provenance count.
        assert_eq!(
            view.field_edges
                .iter()
                .filter(|e| e.to_node == i_joined
                    && e.to_field == "on_hand"
                    && e.kind.nature() == EdgeNature::Direct)
                .count(),
            1,
            "a column unique to one input carries (as a value) from exactly that input"
        );

        // The computed emit `available = on_hand` derives from the single input
        // that produced `on_hand`; it is NOT fanned out to `products`.
        let derive_available = FieldEdge {
            from_node: i_inventory,
            from_field: "on_hand".to_string(),
            to_node: i_joined,
            to_field: "available".to_string(),
            kind: FieldEdgeKind::Derive,
            ..Default::default()
        };
        assert!(
            view.field_edges.contains(&derive_available),
            "available derives from inventory.on_hand, got {:?}",
            view.field_edges
        );
        assert_eq!(
            view.field_edges
                .iter()
                .filter(|e| e.to_node == i_joined
                    && e.to_field == "available"
                    && e.kind.nature() == EdgeNature::Direct)
                .count(),
            1,
            "available has exactly one value producer (on_hand is unique to inventory)"
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

    /// #147 (the case the retired name-collision heuristic missed): a Combine
    /// whose join columns DIFFER in name (`left.k1 == right.k2`) must still drive
    /// `JoinKey` edges, sourced from the real `where_expr` read columns — `k1` on
    /// the left side and `k2` on the right — NOT from any shared-name carry. And a
    /// non-join column that happens to share a name across BOTH inputs
    /// (`created_at`) must get NO `JoinKey` edge, proving the key comes from the
    /// predicate, not a name collision.
    #[test]
    fn combine_join_key_from_where_expr_not_name_collision() {
        let yaml = r#"
pipeline:
  name: combine_diff_keys
nodes:
  - type: source
    name: left
    config:
      name: left
      type: csv
      path: ./left.csv
      schema:
        - { name: k1, type: string }
        - { name: created_at, type: string }
        - { name: left_val, type: int }
  - type: source
    name: right
    config:
      name: right
      type: csv
      path: ./right.csv
      schema:
        - { name: k2, type: string }
        - { name: created_at, type: string }
        - { name: right_val, type: int }
  - type: combine
    name: joined
    input:
      left: left
      right: right
    config:
      where: "left.k1 == right.k2"
      match: first
      on_miss: skip
      cxl: |
        emit out_val = left_val + right_val
      propagate_ck: driver
"#;
        let config = parse_config(yaml).expect("combine diff-keys pipeline parses");
        let view = derive_pipeline_view(&config);

        let i_left = stage_idx(&view, "left");
        let i_right = stage_idx(&view, "right");
        let i_joined = stage_idx(&view, "joined");

        // The differently-named join columns each drive a JoinKey edge from their
        // own side — `left.k1` and `right.k2`. (The edge fans to every surviving
        // output row per the influence-edge target policy; here we only assert the
        // origin column is correct and the side is correct.)
        let has_join_key_from = |from: usize, field: &str| {
            view.field_edges.iter().any(|e| {
                e.from_node == from
                    && e.from_field == field
                    && e.to_node == i_joined
                    && e.kind == FieldEdgeKind::JoinKey
            })
        };
        assert!(
            has_join_key_from(i_left, "k1"),
            "where_expr must drive a JoinKey edge from left.k1, got {:?}",
            view.field_edges
        );
        assert!(
            has_join_key_from(i_right, "k2"),
            "where_expr must drive a JoinKey edge from right.k2, got {:?}",
            view.field_edges
        );

        // `created_at` is a non-join column present on BOTH inputs with the SAME
        // name. The retired heuristic would have falsely tagged it a join key; the
        // where_expr-driven path must NOT — no JoinKey edge originates from it on
        // either side.
        assert!(
            !view
                .field_edges
                .iter()
                .any(|e| e.kind == FieldEdgeKind::JoinKey && e.from_field == "created_at"),
            "a same-named non-join column must get NO JoinKey edge, got {:?}",
            view.field_edges
        );
    }

    /// A Combine body references columns through its input-port aliases
    /// (`emit name = orders.order_id`); `Expr::support_into` reports that ref as
    /// the dotted string `"orders.order_id"`, but `producers_of` is keyed by the
    /// BARE column name, so a naive lookup misses and the node draws NO
    /// input-side lineage at all (the reported `order_fulfillment` bug). Each
    /// alias-qualified emit must resolve to the column on EXACTLY that alias's
    /// predecessor — including the precise case where the column also exists in
    /// the other input (`product_code`): an explicit `orders.product_code` copy
    /// connects only to `orders`, never fanning to the `products` side.
    #[test]
    fn combine_alias_qualified_emits_resolve_to_their_port() {
        let yaml = r#"
pipeline:
  name: combine_qualified
nodes:
  - type: source
    name: orders
    config:
      name: orders
      type: csv
      path: ./orders.csv
      schema:
        - { name: order_id, type: string }
        - { name: product_code, type: string }
        - { name: line_total, type: float }
  - type: source
    name: products
    config:
      name: products
      type: csv
      path: ./products.csv
      schema:
        - { name: product_code, type: string }
        - { name: product_name, type: string }
  - type: combine
    name: enriched
    input:
      orders: orders
      products: products
    config:
      where: "orders.product_code == products.product_code"
      match: first
      on_miss: null_fields
      cxl: |
        emit order_id = orders.order_id
        emit product_code = orders.product_code
        emit product_name = products.product_name
        emit shipping_cost = orders.line_total * 0.1
      propagate_ck: driver
"#;
        let config = parse_config(yaml).expect("combine qualified pipeline parses");
        let view = derive_pipeline_view(&config);

        let i_orders = stage_idx(&view, "orders");
        let i_products = stage_idx(&view, "products");
        let i_enriched = stage_idx(&view, "enriched");

        let has_edge = |from: usize, ff: &str, tf: &str, kind: FieldEdgeKind| {
            view.field_edges.iter().any(|e| {
                e.from_node == from
                    && e.from_field == ff
                    && e.to_node == i_enriched
                    && e.to_field == tf
                    && e.kind == kind
            })
        };

        // A column unique to `orders`, copied unchanged → an identity carry from
        // orders (the alias prefix is stripped to the bare `order_id`).
        assert!(
            has_edge(i_orders, "order_id", "order_id", FieldEdgeKind::Passthrough),
            "order_id must carry from orders, got {:?}",
            view.field_edges
        );
        // A column unique to `products`, copied unchanged → an identity carry
        // from products.
        assert!(
            has_edge(
                i_products,
                "product_name",
                "product_name",
                FieldEdgeKind::Passthrough
            ),
            "product_name must carry from products, got {:?}",
            view.field_edges
        );
        // A computed emit derives from the orders column it reads.
        assert!(
            has_edge(
                i_orders,
                "line_total",
                "shipping_cost",
                FieldEdgeKind::Derive
            ),
            "shipping_cost must derive from orders.line_total, got {:?}",
            view.field_edges
        );

        // PRECISION: `emit product_code = orders.product_code` connects ONLY to
        // the orders side for its VALUE, even though `product_code` also exists in
        // products — an explicit alias pins the value producer; it does not fan to
        // every input. (The Combine's `where_expr`
        // `orders.product_code == products.product_code` separately drives an
        // INDIRECT JoinKey influence from BOTH sides per #147; this precision
        // claim is about DIRECT value provenance, so it is scoped to value
        // carries.)
        assert!(
            has_edge(
                i_orders,
                "product_code",
                "product_code",
                FieldEdgeKind::Passthrough
            ),
            "product_code must carry from the orders side it was copied from"
        );
        assert!(
            !view.field_edges.iter().any(|e| {
                e.from_node == i_products
                    && e.from_field == "product_code"
                    && e.to_node == i_enriched
                    && e.to_field == "product_code"
                    && e.kind.nature() == EdgeNature::Direct
            }),
            "an explicit orders.product_code copy must NOT fan a value edge from products"
        );

        // The node must surface SOME input-side lineage (the bug was zero edges).
        assert!(
            view.field_edges
                .iter()
                .any(|e| e.to_node == i_enriched && e.from_node != i_enriched),
            "the combine must draw at least one input-side field edge"
        );
    }

    /// #72 classifier acceptance: the canonical `value_tier` node yields one
    /// edge of EACH kind, and a self-shadowing computed emit
    /// (`emit status = status + 1`) stays a `Derive` with its row `Emitted` —
    /// never an `Access` carry.
    ///
    /// Source emits `line_total, status, shipping_method`; the node runs
    /// `emit value_tier = line_total * 2.0` and `emit status = status + 1`:
    ///   - `line_total → value_tier`         : Derive (input feeds a compute)
    ///   - `line_total → line_total`          : Access (carried AND feeds value_tier)
    ///   - `shipping_method → shipping_method`: Passthrough (carried, read by none)
    ///   - `status → status`                  : Derive (self-shadow compute, no carry)
    #[test]
    fn field_edge_kind_classifier_value_tier_fixture() {
        let yaml = r#"
pipeline:
  name: value_tier_kinds
nodes:
  - type: source
    name: orders
    config:
      name: orders
      type: csv
      path: ./orders.csv
      schema:
        - { name: line_total, type: float }
        - { name: status, type: string }
        - { name: shipping_method, type: string }
  - type: transform
    name: tier
    input: orders
    config:
      cxl: |
        emit value_tier = line_total * 2.0
        emit status = status + 1
"#;
        let config = parse_config(yaml).expect("value_tier pipeline parses");
        let view = derive_pipeline_view(&config);
        let i_src = stage_idx(&view, "orders");
        let i_t = stage_idx(&view, "tier");

        let mk = |ff: &str, tf: &str, kind: FieldEdgeKind| FieldEdge {
            from_node: i_src,
            from_field: ff.to_string(),
            to_node: i_t,
            to_field: tf.to_string(),
            kind,
            ..Default::default()
        };

        // One edge of each kind.
        assert!(
            view.field_edges
                .contains(&mk("line_total", "value_tier", FieldEdgeKind::Derive)),
            "line_total → value_tier must be Derive, got {:?}",
            view.field_edges
        );
        assert!(
            view.field_edges
                .contains(&mk("line_total", "line_total", FieldEdgeKind::Access)),
            "line_total → line_total must be Access (carried AND feeds value_tier), got {:?}",
            view.field_edges
        );
        assert!(
            view.field_edges.contains(&mk(
                "shipping_method",
                "shipping_method",
                FieldEdgeKind::Passthrough
            )),
            "shipping_method → shipping_method must be Passthrough, got {:?}",
            view.field_edges
        );

        // Self-shadow compute: `status → status` is a Derive, NOT an Access carry —
        // the value is recomputed here, so the column is not "carried".
        assert!(
            view.field_edges
                .contains(&mk("status", "status", FieldEdgeKind::Derive)),
            "status → status (self-shadow) must be Derive, got {:?}",
            view.field_edges
        );
        assert!(
            !view
                .field_edges
                .contains(&mk("status", "status", FieldEdgeKind::Access)),
            "status → status must NOT be an Access carry: {:?}",
            view.field_edges
        );

        // Row kinds: the self-shadowed and computed columns are Emitted; the
        // carried columns stay PassThrough.
        assert_eq!(
            field_by_name(&view, i_t, "status").kind,
            FieldKind::Emitted,
            "a computed self-shadow row stays Emitted, not a carry"
        );
        assert_eq!(
            field_by_name(&view, i_t, "value_tier").kind,
            FieldKind::Emitted
        );
        assert_eq!(
            field_by_name(&view, i_t, "line_total").kind,
            FieldKind::PassThrough
        );
        assert_eq!(
            field_by_name(&view, i_t, "shipping_method").kind,
            FieldKind::PassThrough
        );

        // Clean one-of-each sample: 2 derives (value_tier + the self-shadow),
        // 1 access, 1 passthrough — no other edges.
        let count = |k: FieldEdgeKind| view.field_edges.iter().filter(|e| e.kind == k).count();
        assert_eq!(count(FieldEdgeKind::Access), 1, "exactly one Access edge");
        assert_eq!(
            count(FieldEdgeKind::Passthrough),
            1,
            "exactly one Passthrough edge"
        );
        assert_eq!(count(FieldEdgeKind::Derive), 2, "exactly two Derive edges");
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
            // A CXL-less merge has no emits, so every value carry is a pure
            // Passthrough.
            kind: FieldEdgeKind::Passthrough,
            ..Default::default()
        };
        let edges_to = |field: &str, kind: FieldEdgeKind| {
            view.field_edges
                .iter()
                .filter(|e| e.to_node == i_merged && e.to_field == field && e.kind == kind)
                .count()
        };

        // `id` is in all three inputs → a value carry from each. A Merge is a row
        // UNION, never a join, so a shared column is ONLY a Passthrough carry —
        // no JoinKey edge (#147).
        assert!(view.field_edges.contains(&carry(i_left, "id")));
        assert!(view.field_edges.contains(&carry(i_mid, "id")));
        assert!(view.field_edges.contains(&carry(i_right, "id")));
        assert_eq!(
            edges_to("id", FieldEdgeKind::Passthrough),
            3,
            "got {:?}",
            view.field_edges
        );

        // `shared` is in left + mid ONLY → value carries from those two, never
        // `right`.
        assert!(view.field_edges.contains(&carry(i_left, "shared")));
        assert!(view.field_edges.contains(&carry(i_mid, "shared")));
        assert!(
            !view.field_edges.contains(&carry(i_right, "shared")),
            "must not fan out to the input that never produced the column"
        );
        assert_eq!(
            edges_to("shared", FieldEdgeKind::Passthrough),
            2,
            "got {:?}",
            view.field_edges
        );

        // `only_right` is unique to one input → exactly one value carry.
        assert!(view.field_edges.contains(&carry(i_right, "only_right")));
        assert_eq!(
            edges_to("only_right", FieldEdgeKind::Passthrough),
            1,
            "got {:?}",
            view.field_edges
        );

        // A Merge performs no join: across the WHOLE node there is not a single
        // JoinKey edge, regardless of how many inputs share a column.
        assert_eq!(
            view.field_edges
                .iter()
                .filter(|e| e.to_node == i_merged && e.kind == FieldEdgeKind::JoinKey)
                .count(),
            0,
            "a Merge is a row UNION and must emit no JoinKey edges, got {:?}",
            view.field_edges
        );
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
        // with its inferred type (#149: `a * 2.0` arithmetic → numeric).
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
                    ty: Some("numeric".to_string()),
                    ..Default::default()
                },
            ]
        );

        // t2: passthrough `a` (still `float`), `b` (carries t1's inferred
        // `numeric` through the passthrough), then emitted `c` = `b + a`. Both
        // operands are bare refs (Unknown in raw mode), and `+` is overloaded
        // (numeric add vs string concat), so inference abstains → `None` (#149).
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
                    ty: Some("numeric".to_string()),
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
            kind: FieldEdgeKind::Derive,
            ..Default::default()
        };
        // Every carried column in this chain ALSO feeds a downstream emit (`a`
        // feeds `b`, then `a`/`b` feed `c`), so each carry is an Access carry, not
        // a pure Passthrough (#72).
        let access = |fn_: usize, ff: &str, tn: usize, tf: &str| FieldEdge {
            from_node: fn_,
            from_field: ff.to_string(),
            to_node: tn,
            to_field: tf.to_string(),
            kind: FieldEdgeKind::Access,
            ..Default::default()
        };
        let expected = [
            // t1: b derives from a; a carries through AND feeds b (Access).
            derive(i_src, "a", i_t1, "b"),
            access(i_src, "a", i_t1, "a"),
            // t2: c derives from b and a; a, b carry through AND feed c (Access).
            derive(i_t1, "b", i_t2, "c"),
            derive(i_t1, "a", i_t2, "c"),
            access(i_t1, "a", i_t2, "a"),
            access(i_t1, "b", i_t2, "b"),
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
            kind: FieldEdgeKind::Derive,
            ..Default::default()
        };
        // Intra-node derive: `c` reads `b`, an EARLIER emit of the SAME node `t`.
        let b_to_c = FieldEdge {
            from_node: i_t,
            from_field: "b".to_string(),
            to_node: i_t,
            to_field: "c".to_string(),
            kind: FieldEdgeKind::Derive,
            ..Default::default()
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
            kind: FieldEdgeKind::Derive,
            ..Default::default()
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
            kind: FieldEdgeKind::Derive,
            ..Default::default()
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
            kind: FieldEdgeKind::Derive,
            ..Default::default()
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
                // Parse failure suppressed edges, so the row is Unknown (#148).
                lineage_precision: Precision::Unknown,
                precision_reason: "CXL did not parse; lineage edges suppressed",
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

    // ── #148 precision tiers (Exact / Approximate / Unknown) ─────────────────

    /// Find the precision of the edge with the given identity in a view. Panics if
    /// absent so a missing edge fails loudly rather than silently passing.
    fn edge_precision(
        view: &PipelineView,
        from_node: usize,
        from_field: &str,
        to_node: usize,
        to_field: &str,
        kind: FieldEdgeKind,
    ) -> Precision {
        view.field_edges
            .iter()
            .find(|e| {
                e.from_node == from_node
                    && e.from_field == from_field
                    && e.to_node == to_node
                    && e.to_field == to_field
                    && e.kind == kind
            })
            .unwrap_or_else(|| {
                panic!(
                    "expected edge {from_field}->{to_field} kind {kind:?}, got {:?}",
                    view.field_edges
                )
            })
            .precision
    }

    fn field_precision(view: &PipelineView, stage_label: &str, field: &str) -> Precision {
        let idx = stage_idx(view, stage_label);
        view.stages[idx]
            .fields
            .iter()
            .find(|row| row.name == field)
            .unwrap_or_else(|| panic!("field {field} on {stage_label}"))
            .lineage_precision
    }

    /// EXACT tier: a clean-CXL straight-line `Derive` edge is `Exact`, and the
    /// output row it feeds is `Exact` (#148). A genuine break would mislabel a
    /// faithful derive as degraded.
    #[test]
    fn clean_derive_edge_and_row_are_exact() {
        let yaml = r#"
pipeline:
  name: exact_derive
nodes:
  - type: source
    name: src
    config:
      name: src
      type: csv
      path: ./in.csv
      schema:
        - { name: amount, type: float }
  - type: transform
    name: scale
    input: src
    config:
      cxl: |
        emit doubled = amount * 2.0
"#;
        let config = parse_config(yaml).expect("exact derive fixture parses");
        let view = derive_pipeline_view(&config);
        let i_src = stage_idx(&view, "src");
        let i_scale = stage_idx(&view, "scale");

        assert_eq!(
            edge_precision(
                &view,
                i_src,
                "amount",
                i_scale,
                "doubled",
                FieldEdgeKind::Derive
            ),
            Precision::Exact,
            "a clean straight-line derive edge is Exact"
        );
        assert_eq!(
            field_precision(&view, "scale", "doubled"),
            Precision::Exact,
            "the row a clean derive feeds is Exact"
        );
    }

    /// APPROXIMATE tier (edge + row): a Route branch condition emits a
    /// `Conditional` INDIRECT edge classified `Approximate`, and the surviving
    /// output row it influences folds down to `Approximate` even though the row is
    /// a clean passthrough (#148).
    #[test]
    fn indirect_conditional_edge_and_row_are_approximate() {
        let yaml = r#"
pipeline:
  name: route_conditional
nodes:
  - type: source
    name: src
    config:
      name: src
      type: csv
      path: ./in.csv
      schema:
        - { name: region, type: string }
        - { name: amount, type: float }
  - type: route
    name: split
    input: src
    config:
      conditions:
        eu: "region == 'EU'"
      default: other
"#;
        let config = parse_config(yaml).expect("route fixture parses");
        let view = derive_pipeline_view(&config);
        let i_src = stage_idx(&view, "src");
        let i_split = stage_idx(&view, "split");

        // `region` drives the branch condition → a Conditional influence edge onto
        // every surviving row, classified Approximate.
        assert_eq!(
            edge_precision(
                &view,
                i_src,
                "region",
                i_split,
                "region",
                FieldEdgeKind::Conditional
            ),
            Precision::Approximate,
            "an INDIRECT Conditional edge is Approximate"
        );
        // `region` survives as a passthrough row but is incident to its own
        // Conditional influence edge, so its row precision is Approximate.
        assert_eq!(
            field_precision(&view, "split", "region"),
            Precision::Approximate,
            "a row incident to an INDIRECT edge folds to Approximate"
        );
    }

    /// APPROXIMATE tier via Aggregate GroupBy: the group-key edge is `Approximate`
    /// and the group-key output row folds to `Approximate` (#148) — the grain is an
    /// influence, not a value.
    #[test]
    fn group_by_edge_and_grain_row_are_approximate() {
        let yaml = r#"
pipeline:
  name: agg_precision
nodes:
  - type: source
    name: src
    config:
      name: src
      type: csv
      path: ./in.csv
      schema:
        - { name: dept, type: string }
        - { name: amount, type: float }
  - type: aggregate
    name: totals
    input: src
    config:
      group_by: [dept]
      cxl: |
        emit total = sum(amount)
"#;
        let config = parse_config(yaml).expect("aggregate precision fixture parses");
        let view = derive_pipeline_view(&config);
        let i_src = stage_idx(&view, "src");
        let i_totals = stage_idx(&view, "totals");

        assert_eq!(
            edge_precision(
                &view,
                i_src,
                "dept",
                i_totals,
                "dept",
                FieldEdgeKind::GroupBy
            ),
            Precision::Approximate,
            "a GroupBy grain edge is Approximate"
        );
        assert_eq!(
            field_precision(&view, "totals", "dept"),
            Precision::Approximate,
            "the group-key grain row folds to Approximate"
        );
        // The aggregate value emit stays Exact — only the grain is approximate.
        assert_eq!(
            field_precision(&view, "totals", "total"),
            Precision::Exact,
            "the aggregate value derive stays Exact"
        );
    }

    /// UNKNOWN tier: a node whose CXL fails `parse_clean` has its edges suppressed,
    /// so the degradation lives on the row as `Unknown` (#148). The row still
    /// renders (shape preserved) but its precision is Unknown with the parse-fail
    /// reason.
    #[test]
    fn parse_fail_node_rows_are_unknown_with_no_edges() {
        let yaml = r#"
pipeline:
  name: parse_fail
nodes:
  - type: source
    name: src
    config:
      name: src
      type: csv
      path: ./in.csv
      schema:
        - { name: amount, type: float }
  - type: transform
    name: broken
    input: src
    config:
      cxl: |
        emit y = amount +
"#;
        let config = parse_config(yaml).expect("parse-fail fixture YAML parses");
        let view = derive_pipeline_view(&config);
        let i_broken = stage_idx(&view, "broken");

        let row = view.stages[i_broken]
            .fields
            .iter()
            .find(|r| r.name == "amount")
            .expect("the carried column still renders");
        assert_eq!(
            row.lineage_precision,
            Precision::Unknown,
            "a parse-fail node's rows are Unknown"
        );
        assert_eq!(
            row.precision_reason,
            "CXL did not parse; lineage edges suppressed"
        );
        // Acceptance: no edge terminates at the parse-fail node (edges suppressed).
        assert!(
            view.field_edges
                .iter()
                .all(|e| e.to_node != i_broken && e.from_node != i_broken),
            "a parse-fail node infers no edges"
        );
    }

    /// Worst-of-incident-edges row aggregation: a column that is BOTH a clean
    /// passthrough (Exact carry) AND incident to an INDIRECT influence edge folds
    /// to the worse tier, Approximate (#148). This is the core aggregation rule —
    /// a single degraded incident edge dominates an otherwise-Exact row.
    #[test]
    fn row_precision_is_worst_of_incident_edges() {
        // A Cull keeps every input column as a passthrough (Exact carry) AND emits
        // a Filter influence edge from the predicate column onto each surviving
        // row, so the predicate column carries both an Exact and an Approximate
        // incident edge — its row must read Approximate.
        let yaml = r#"
pipeline:
  name: cull_worst_of
nodes:
  - type: source
    name: src
    config:
      name: src
      type: csv
      path: ./in.csv
      schema:
        - { name: gid, type: string }
        - { name: status, type: string }
  - type: cull
    name: prune
    input: src
    config:
      partition_by: [gid]
      removed_to: dropped
      rules:
        - name: drop_errored
          drop_group_when: "sum(if status == 'error' then 1 else 0) > 0"
"#;
        let config = parse_config(yaml).expect("cull worst-of fixture parses");
        let view = derive_pipeline_view(&config);
        let i_src = stage_idx(&view, "src");
        let i_prune = stage_idx(&view, "prune");

        // Both incident edges exist on `status`: the Exact passthrough carry AND
        // the Approximate Filter influence.
        assert_eq!(
            edge_precision(
                &view,
                i_src,
                "status",
                i_prune,
                "status",
                FieldEdgeKind::Passthrough
            ),
            Precision::Exact
        );
        assert_eq!(
            edge_precision(
                &view,
                i_src,
                "status",
                i_prune,
                "status",
                FieldEdgeKind::Filter
            ),
            Precision::Approximate
        );
        // The Cull row folds to the worse tier.
        assert_eq!(
            field_precision(&view, "prune", "status"),
            Precision::Approximate,
            "a row with both an Exact and an Approximate producing edge folds to Approximate"
        );
        // The UPSTREAM source `status` row has no producing influence edge (it is
        // only the `from` side of the downstream Filter), so it stays Exact: the
        // degradation lands on the consumer, not the pristine producer.
        assert_eq!(
            field_precision(&view, "src", "status"),
            Precision::Exact,
            "a clean producer row with no degraded producing edge stays Exact"
        );
    }
}

#[cfg(test)]
mod layout_tests {
    use super::*;
    use crate::pipeline_view::layout_model::{CanvasLayoutEngine, apply_canvas_layout};
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

    #[test]
    fn explicit_layout_selection_keeps_current_default_and_allows_port_aware_preview() {
        let yaml = r#"
pipeline:
  name: migration_layout
nodes:
  - type: source
    name: src
    config:
      name: src
      type: csv
      path: ./in.csv
      schema:
        - { name: x, type: string }
  - type: transform
    name: step
    input: src
    config:
      cxl: |
        emit y = x
  - type: output
    name: out
    input: step
    config:
      name: out
      type: csv
      path: ./out.csv
"#;
        let config = parse_config(yaml).expect("migration layout pipeline parses");
        let current = derive_pipeline_view(&config);

        let default_result =
            apply_canvas_layout(derive_pipeline_view(&config), CanvasLayoutEngine::default());
        assert_eq!(
            default_result.applied,
            CanvasLayoutEngine::CurrentBarycenter
        );
        assert_eq!(default_result.fallback, None);
        assert_eq!(default_result.view, current);

        let preview = apply_canvas_layout(
            derive_pipeline_view(&config),
            CanvasLayoutEngine::PortAwareSugiyama,
        );
        assert_eq!(preview.applied, CanvasLayoutEngine::PortAwareSugiyama);
        assert_eq!(preview.fallback, None);
        assert_eq!(preview.view.connections, current.connections);
        assert_eq!(preview.view.field_edges, current.field_edges);
        assert_eq!(
            preview
                .view
                .stages
                .iter()
                .map(|stage| stage.id.as_str())
                .collect::<Vec<_>>(),
            vec!["src", "step", "out"]
        );
        let preview_again = apply_canvas_layout(
            derive_pipeline_view(&config),
            CanvasLayoutEngine::PortAwareSugiyama,
        );
        assert_eq!(
            preview_again
                .view
                .stages
                .iter()
                .map(|stage| (stage.canvas_x, stage.canvas_y))
                .collect::<Vec<_>>(),
            preview
                .view
                .stages
                .iter()
                .map(|stage| (stage.canvas_x, stage.canvas_y))
                .collect::<Vec<_>>()
        );
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

    #[test]
    fn input_role_ports_shift_field_anchors_below_group_header() {
        let mut stage = stage_at(10.0, 20.0);
        stage.role_ports = vec![StagePortRow {
            id: "group_by:user_id".to_string(),
            label: "user_id".to_string(),
            role: "group_by".to_string(),
            kind: StagePortKind::AggregateGroupKey,
            side: StagePortSide::Input,
        }];
        stage.fields = vec![FieldRow {
            name: "user_id".to_string(),
            kind: FieldKind::PassThrough,
            ty: None,
            is_correlation_key: false,
            ..Default::default()
        }];

        assert_eq!(
            stage.role_port_anchor_in(0),
            (
                10.0,
                20.0 + FIELD_HEADER_HEIGHT + FIELD_ROW_HEIGHT + FIELD_ROW_HEIGHT / 2.0
            )
        );
        assert_eq!(
            stage.field_anchor_in(0),
            (
                10.0,
                20.0 + FIELD_HEADER_HEIGHT + 2.0 * FIELD_ROW_HEIGHT + FIELD_ROW_HEIGHT / 2.0
            )
        );
        assert_eq!(
            stage.card_height(),
            FIELD_HEADER_HEIGHT + 3.0 * FIELD_ROW_HEIGHT
        );
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
            role_ports: Vec::new(),
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

    /// One node card ([`NODE_WIDTH`] × [`NODE_HEIGHT`]) at world origin, used by
    /// the `pan_to_reveal` cases below.
    fn reveal_node() -> LayoutBounds {
        LayoutBounds {
            min_x: 0.0,
            min_y: 0.0,
            max_x: NODE_WIDTH,
            max_y: NODE_HEIGHT,
        }
    }

    /// A node already inside the viewport (with margin to spare) needs no pan.
    #[test]
    fn pan_to_reveal_noop_when_visible() {
        // Centered-ish in a 1000×700 viewport at zoom 1, well inside the margin.
        assert_eq!(
            pan_to_reveal(reveal_node(), 400.0, 300.0, 1.0, 1000.0, 700.0, 48.0),
            None
        );
    }

    /// A node clipped past the RIGHT edge (the #163 repro) pans left just enough
    /// to seat its right edge at `viewport_w - margin`, leaving the other axis and
    /// the zoom untouched.
    #[test]
    fn pan_to_reveal_pulls_right_clipped_node_into_view() {
        let z = 1.0;
        let vw = 400.0;
        let vh = 700.0;
        let margin = 48.0;
        // pan_x = 320 → node spans screen-x [320, 480]; right edge past 400-48=352.
        let node = reveal_node();
        let (nx, ny) = pan_to_reveal(node, 320.0, 300.0, z, vw, vh, margin)
            .expect("right-clipped node should require a pan");
        // Vertical axis was already visible → pan_y unchanged.
        assert!((ny - 300.0).abs() < 0.001);
        // Right edge now sits exactly at the far margin: pan + max_x*zoom == vw-margin.
        assert!((nx + node.max_x * z - (vw - margin)).abs() < 0.001);
        // And the whole card is now within [margin, vw-margin].
        assert!(nx + node.min_x * z >= margin - 0.001);
    }

    /// A node off the LEFT edge pans right so its left edge seats at the margin.
    #[test]
    fn pan_to_reveal_pulls_left_clipped_node_into_view() {
        let z = 1.0;
        let node = reveal_node();
        // pan_x = -40 → node spans screen-x [-40, 120]; left edge past margin 48.
        let (nx, _) = pan_to_reveal(node, -40.0, 300.0, z, 1000.0, 700.0, 48.0)
            .expect("left-clipped node should require a pan");
        assert!((nx + node.min_x * z - 48.0).abs() < 0.001);
    }

    /// When the node is wider than the available window, its near (left) edge is
    /// pinned to the margin so its start is visible even though the far edge stays
    /// clipped.
    #[test]
    fn pan_to_reveal_pins_near_edge_when_larger_than_window() {
        let z = 4.0; // 160 * 4 = 640 wide, wider than a 400px viewport.
        let node = reveal_node();
        let (nx, _) = pan_to_reveal(node, -500.0, 300.0, z, 400.0, 700.0, 48.0)
            .expect("oversized node off-screen should still pan");
        // Left edge pinned at the margin.
        assert!((nx + node.min_x * z - 48.0).abs() < 0.001);
    }

    /// A not-yet-measured / degenerate viewport (`extent <= 2*margin`) yields no
    /// pan rather than a nonsensical jump.
    #[test]
    fn pan_to_reveal_noop_on_degenerate_viewport() {
        assert_eq!(
            pan_to_reveal(reveal_node(), 0.0, 0.0, 1.0, 1.0, 1.0, 48.0),
            None
        );
    }
}
