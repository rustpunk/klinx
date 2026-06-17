// This model is intentionally introduced ahead of the renderer migration.
#![allow(dead_code)]

use std::cmp::Ordering;
use std::collections::HashMap;

use super::{Connection, FieldEdge, NODE_WIDTH, PipelineView, StageKind, StageView};

#[derive(Clone, Debug, PartialEq)]
pub struct LayoutGraph {
    pub nodes: Vec<LayoutNode>,
    pub edges: Vec<LayoutEdge>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LayoutNode {
    pub stage_index: usize,
    pub id: String,
    pub width: f32,
    pub height: f32,
    pub rank: usize,
    pub input_ports: Vec<LayoutPort>,
    pub output_ports: Vec<LayoutPort>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayoutPort {
    pub id: String,
    pub label: String,
    pub side: LayoutPortSide,
    pub kind: LayoutPortKind,
    pub order: usize,
    pub stage_anchor: LayoutStageAnchor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LayoutPortSide {
    Input,
    Output,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutPortKind {
    Node,
    Field,
    RouteBranch,
    CullSideOutput,
}

/// A stable bridge back to today's [`StageView`] anchor helpers.
///
/// The future renderer can use `order` for the port-aware layout while still
/// resolving a port to the current node, field-row, or branch-row anchor during
/// migration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutStageAnchor {
    NodeInput,
    NodeOutput,
    FieldInput { field_index: usize },
    FieldOutput { field_index: usize },
    BranchOutput { branch_index: usize },
}

#[derive(Clone, Debug, PartialEq)]
pub struct LayoutEdge {
    pub from_node: usize,
    pub from_port: String,
    pub to_node: usize,
    pub to_port: String,
    pub kind: LayoutEdgeKind,
    pub path: ConnectorPath,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutEdgeKind {
    Node,
    Field,
    RouteBranch,
    CullSideOutput,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ConnectorPath {
    pub points: Vec<LayoutPoint>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LayoutPoint {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayoutLayer {
    pub rank: usize,
    pub nodes: Vec<usize>,
}

type PortKey = (usize, LayoutPortSide, String);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PortScore {
    total: usize,
    count: usize,
}

impl PortScore {
    fn single(value: usize) -> Self {
        Self {
            total: value,
            count: 1,
        }
    }

    fn add(&mut self, value: usize) {
        self.total += value;
        self.count += 1;
    }
}

impl LayoutGraph {
    pub fn from_pipeline_view(view: &PipelineView) -> Self {
        let nodes = view
            .stages
            .iter()
            .enumerate()
            .map(|(stage_index, stage)| LayoutNode::from_stage(stage_index, stage))
            .collect();
        let mut edges = Vec::new();
        edges.extend(
            view.connections
                .iter()
                .map(|connection| connection_edge(connection, &view.stages)),
        );
        edges.extend(view.field_edges.iter().map(field_edge));

        let mut graph = Self { nodes, edges };
        graph.assign_longest_path_ranks();
        graph.order_ports_for_crossing_reduction();
        graph.route_placeholder_paths();
        graph
    }

    pub fn layers(&self) -> Vec<LayoutLayer> {
        let mut ranks: Vec<usize> = self.nodes.iter().map(|node| node.rank).collect();
        ranks.sort_unstable();
        ranks.dedup();
        ranks
            .into_iter()
            .map(|rank| LayoutLayer {
                rank,
                nodes: self
                    .nodes
                    .iter()
                    .filter(|node| node.rank == rank)
                    .map(|node| node.stage_index)
                    .collect(),
            })
            .collect()
    }

    fn assign_longest_path_ranks(&mut self) {
        for node in &mut self.nodes {
            node.rank = 0;
        }
        for _ in 0..self.nodes.len() {
            let mut changed = false;
            for edge in &self.edges {
                let next = self.nodes[edge.from_node].rank + 1;
                if self.nodes[edge.to_node].rank < next {
                    self.nodes[edge.to_node].rank = next;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
    }

    /// Run a bounded, deterministic port-ordering pass.
    ///
    /// Only field-row ports are crossing-reordered. Route branch ports keep
    /// condition declaration order with the default branch last, and Cull side
    /// outputs keep their authored side-output identity. This preserves the
    /// semantic row order the current canvas exposes while still letting field
    /// lineage ports move toward their incident edges before renderer migration.
    fn order_ports_for_crossing_reduction(&mut self) {
        let base_orders = self.port_order_map();
        let node_layer_orders = self.node_layer_orders();
        let scores = self.port_scores(&base_orders, &node_layer_orders);

        for (node_index, node) in self.nodes.iter_mut().enumerate() {
            order_port_side(
                node_index,
                LayoutPortSide::Output,
                &mut node.output_ports,
                &scores,
                &base_orders,
            );
        }

        let output_ordered = self.port_order_map();
        let input_scores = self.port_scores(&output_ordered, &node_layer_orders);
        for (node_index, node) in self.nodes.iter_mut().enumerate() {
            order_port_side(
                node_index,
                LayoutPortSide::Input,
                &mut node.input_ports,
                &input_scores,
                &output_ordered,
            );
        }
    }

    fn port_order_map(&self) -> HashMap<PortKey, usize> {
        let mut orders = HashMap::new();
        for (node_index, node) in self.nodes.iter().enumerate() {
            for port in &node.input_ports {
                orders.insert(
                    (node_index, LayoutPortSide::Input, port.id.clone()),
                    port.order,
                );
            }
            for port in &node.output_ports {
                orders.insert(
                    (node_index, LayoutPortSide::Output, port.id.clone()),
                    port.order,
                );
            }
        }
        orders
    }

    fn node_layer_orders(&self) -> Vec<usize> {
        let mut by_rank: HashMap<usize, Vec<usize>> = HashMap::new();
        for (node_index, node) in self.nodes.iter().enumerate() {
            by_rank.entry(node.rank).or_default().push(node_index);
        }

        let mut orders = vec![0; self.nodes.len()];
        for nodes in by_rank.values_mut() {
            nodes.sort_unstable_by_key(|&node_index| self.nodes[node_index].stage_index);
            for (order, &node_index) in nodes.iter().enumerate() {
                orders[node_index] = order;
            }
        }
        orders
    }

    fn port_scores(
        &self,
        base_orders: &HashMap<PortKey, usize>,
        node_layer_orders: &[usize],
    ) -> HashMap<PortKey, PortScore> {
        let mut scores = HashMap::new();
        for edge in &self.edges {
            let target_score = port_position_score(
                edge.to_node,
                LayoutPortSide::Input,
                &edge.to_port,
                base_orders,
                node_layer_orders,
            );
            scores
                .entry((
                    edge.from_node,
                    LayoutPortSide::Output,
                    edge.from_port.clone(),
                ))
                .or_insert_with(PortScore::default)
                .add(target_score);

            let source_score = port_position_score(
                edge.from_node,
                LayoutPortSide::Output,
                &edge.from_port,
                base_orders,
                node_layer_orders,
            );
            scores
                .entry((edge.to_node, LayoutPortSide::Input, edge.to_port.clone()))
                .or_insert_with(PortScore::default)
                .add(source_score);
        }
        scores
    }

    fn route_placeholder_paths(&mut self) {
        for edge in &mut self.edges {
            let from = &self.nodes[edge.from_node];
            let to = &self.nodes[edge.to_node];
            let start = LayoutPoint {
                x: from.rank as f32,
                y: port_order(from, &edge.from_port, LayoutPortSide::Output) as f32,
            };
            let end = LayoutPoint {
                x: to.rank as f32,
                y: port_order(to, &edge.to_port, LayoutPortSide::Input) as f32,
            };
            let mid_x = (start.x + end.x) / 2.0;
            edge.path.points = vec![
                start,
                LayoutPoint {
                    x: mid_x,
                    y: start.y,
                },
                LayoutPoint { x: mid_x, y: end.y },
                end,
            ];
        }
    }
}

impl LayoutNode {
    fn from_stage(stage_index: usize, stage: &StageView) -> Self {
        let mut input_ports = vec![LayoutPort::node("node:in", LayoutPortSide::Input, 0)];
        input_ports.extend(stage.fields.iter().enumerate().map(|(idx, field)| {
            LayoutPort::field(
                &format!("field:in:{}", field.name),
                &field.name,
                LayoutPortSide::Input,
                idx,
                idx + 1,
            )
        }));

        let mut output_ports = Vec::new();
        let mut order = 0;
        if stage.keeps_node_output_port() {
            output_ports.push(LayoutPort::node("node:out", LayoutPortSide::Output, order));
            order += 1;
        }
        output_ports.extend(stage.fields.iter().enumerate().map(|(idx, field)| {
            LayoutPort::field(
                &format!("field:out:{}", field.name),
                &field.name,
                LayoutPortSide::Output,
                idx,
                order + idx,
            )
        }));
        order += stage.fields.len();
        output_ports.extend(stage.branches.iter().enumerate().map(|(idx, branch)| {
            let kind = if matches!(stage.kind, StageKind::Cull) {
                LayoutPortKind::CullSideOutput
            } else {
                LayoutPortKind::RouteBranch
            };
            LayoutPort::branch(
                &format!("branch:out:{}", branch.name),
                &branch.name,
                kind,
                idx,
                order + idx,
            )
        }));

        Self {
            stage_index,
            id: stage.id.clone(),
            width: NODE_WIDTH,
            height: stage.card_height(),
            rank: 0,
            input_ports,
            output_ports,
        }
    }
}

impl LayoutPort {
    fn node(id: &str, side: LayoutPortSide, order: usize) -> Self {
        Self {
            id: id.to_string(),
            label: String::new(),
            side,
            kind: LayoutPortKind::Node,
            order,
            stage_anchor: match side {
                LayoutPortSide::Input => LayoutStageAnchor::NodeInput,
                LayoutPortSide::Output => LayoutStageAnchor::NodeOutput,
            },
        }
    }

    fn field(
        id: &str,
        label: &str,
        side: LayoutPortSide,
        field_index: usize,
        order: usize,
    ) -> Self {
        Self {
            id: id.to_string(),
            label: label.to_string(),
            side,
            kind: LayoutPortKind::Field,
            order,
            stage_anchor: match side {
                LayoutPortSide::Input => LayoutStageAnchor::FieldInput { field_index },
                LayoutPortSide::Output => LayoutStageAnchor::FieldOutput { field_index },
            },
        }
    }

    fn branch(
        id: &str,
        label: &str,
        kind: LayoutPortKind,
        branch_index: usize,
        order: usize,
    ) -> Self {
        Self {
            id: id.to_string(),
            label: label.to_string(),
            side: LayoutPortSide::Output,
            kind,
            order,
            stage_anchor: LayoutStageAnchor::BranchOutput { branch_index },
        }
    }
}

fn connection_edge(connection: &Connection, stages: &[StageView]) -> LayoutEdge {
    let (from_port, kind) = match connection.from_branch {
        Some(branch) => (
            stages
                .get(connection.from)
                .and_then(|stage| stage.branches.get(branch))
                .map(|branch| format!("branch:out:{}", branch.name))
                .unwrap_or_else(|| format!("branch:missing:{branch}")),
            if matches!(
                stages.get(connection.from).map(|stage| &stage.kind),
                Some(StageKind::Cull)
            ) {
                LayoutEdgeKind::CullSideOutput
            } else {
                LayoutEdgeKind::RouteBranch
            },
        ),
        None => ("node:out".to_string(), LayoutEdgeKind::Node),
    };
    LayoutEdge {
        from_node: connection.from,
        from_port,
        to_node: connection.to,
        to_port: "node:in".to_string(),
        kind,
        path: ConnectorPath::default(),
    }
}

fn field_edge(edge: &FieldEdge) -> LayoutEdge {
    LayoutEdge {
        from_node: edge.from_node,
        from_port: format!("field:out:{}", edge.from_field),
        to_node: edge.to_node,
        to_port: format!("field:in:{}", edge.to_field),
        kind: LayoutEdgeKind::Field,
        path: ConnectorPath::default(),
    }
}

fn order_port_side(
    node_index: usize,
    side: LayoutPortSide,
    ports: &mut [LayoutPort],
    scores: &HashMap<PortKey, PortScore>,
    base_orders: &HashMap<PortKey, usize>,
) {
    ports.sort_by(|a, b| {
        semantic_port_group(a)
            .cmp(&semantic_port_group(b))
            .then_with(|| {
                if a.kind == LayoutPortKind::Field && b.kind == LayoutPortKind::Field {
                    compare_port_scores(node_index, side, a, b, scores, base_orders)
                } else {
                    base_port_order(node_index, side, a, base_orders).cmp(&base_port_order(
                        node_index,
                        side,
                        b,
                        base_orders,
                    ))
                }
            })
    });

    for (order, port) in ports.iter_mut().enumerate() {
        port.order = order;
    }
}

fn semantic_port_group(port: &LayoutPort) -> usize {
    match port.kind {
        LayoutPortKind::Node => 0,
        LayoutPortKind::Field => 1,
        LayoutPortKind::RouteBranch | LayoutPortKind::CullSideOutput => 2,
    }
}

fn compare_port_scores(
    node_index: usize,
    side: LayoutPortSide,
    a: &LayoutPort,
    b: &LayoutPort,
    scores: &HashMap<PortKey, PortScore>,
    base_orders: &HashMap<PortKey, usize>,
) -> Ordering {
    let a_order = base_port_order(node_index, side, a, base_orders);
    let b_order = base_port_order(node_index, side, b, base_orders);
    let a_score = scores
        .get(&(node_index, side, a.id.clone()))
        .copied()
        .unwrap_or_else(|| PortScore::single(a_order));
    let b_score = scores
        .get(&(node_index, side, b.id.clone()))
        .copied()
        .unwrap_or_else(|| PortScore::single(b_order));

    compare_scores(a_score, b_score).then_with(|| a_order.cmp(&b_order))
}

fn compare_scores(a: PortScore, b: PortScore) -> Ordering {
    match (a.count, b.count) {
        (0, 0) => Ordering::Equal,
        (0, _) => Ordering::Greater,
        (_, 0) => Ordering::Less,
        _ => (a.total as u128 * b.count as u128).cmp(&(b.total as u128 * a.count as u128)),
    }
}

fn base_port_order(
    node_index: usize,
    side: LayoutPortSide,
    port: &LayoutPort,
    base_orders: &HashMap<PortKey, usize>,
) -> usize {
    base_orders
        .get(&(node_index, side, port.id.clone()))
        .copied()
        .unwrap_or(port.order)
}

fn port_position_score(
    node_index: usize,
    side: LayoutPortSide,
    port_id: &str,
    base_orders: &HashMap<PortKey, usize>,
    node_layer_orders: &[usize],
) -> usize {
    const PORT_SCORE_STRIDE: usize = 10_000;

    node_layer_orders.get(node_index).copied().unwrap_or(0) * PORT_SCORE_STRIDE
        + base_orders
            .get(&(node_index, side, port_id.to_string()))
            .copied()
            .unwrap_or(0)
}

fn port_order(node: &LayoutNode, port_id: &str, side: LayoutPortSide) -> usize {
    let ports = match side {
        LayoutPortSide::Input => &node.input_ports,
        LayoutPortSide::Output => &node.output_ports,
    };
    ports
        .iter()
        .find(|port| port.id == port_id)
        .map(|port| port.order)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline_view::{
        Connection, FieldEdge, FieldEdgeKind, FieldKind, FieldRow, PipelineView, RouteBranch,
        StageKind, StageView,
    };

    fn stage(id: &str) -> StageView {
        StageView {
            id: id.to_string(),
            label: id.to_string(),
            kind: StageKind::Transform,
            subtitle: String::new(),
            canvas_x: 0.0,
            canvas_y: 0.0,
            cxl_source: None,
            description: None,
            error_message: None,
            fields: Vec::new(),
            branches: Vec::new(),
        }
    }

    fn field(name: &str) -> FieldRow {
        FieldRow {
            name: name.to_string(),
            kind: FieldKind::Declared,
            ty: None,
            is_correlation_key: false,
        }
    }

    fn output_port_ids(node: &LayoutNode) -> Vec<&str> {
        let mut ports = node.output_ports.iter().collect::<Vec<_>>();
        ports.sort_unstable_by_key(|port| port.order);
        ports.into_iter().map(|port| port.id.as_str()).collect()
    }

    fn input_port_ids(node: &LayoutNode) -> Vec<&str> {
        let mut ports = node.input_ports.iter().collect::<Vec<_>>();
        ports.sort_unstable_by_key(|port| port.order);
        ports.into_iter().map(|port| port.id.as_str()).collect()
    }

    #[test]
    fn simple_chain_assigns_layers_and_node_ports() {
        let view = PipelineView {
            stages: vec![stage("a"), stage("b"), stage("c")],
            connections: vec![Connection::plain(0, 1), Connection::plain(1, 2)],
            field_edges: Vec::new(),
        };
        let graph = LayoutGraph::from_pipeline_view(&view);

        assert_eq!(graph.layers()[0].nodes, vec![0]);
        assert_eq!(graph.layers()[1].nodes, vec![1]);
        assert_eq!(graph.layers()[2].nodes, vec![2]);
        assert_eq!(graph.edges[0].from_port, "node:out");
        assert_eq!(graph.edges[0].to_port, "node:in");
        assert_eq!(graph.edges[0].path.points.len(), 4);
    }

    #[test]
    fn fan_out_keeps_same_rank_consumers_in_input_order() {
        let view = PipelineView {
            stages: vec![stage("source"), stage("left"), stage("right")],
            connections: vec![Connection::plain(0, 1), Connection::plain(0, 2)],
            field_edges: Vec::new(),
        };
        let graph = LayoutGraph::from_pipeline_view(&view);

        assert_eq!(graph.layers()[0].nodes, vec![0]);
        assert_eq!(graph.layers()[1].nodes, vec![1, 2]);
    }

    #[test]
    fn fan_in_places_consumer_after_all_inputs() {
        let view = PipelineView {
            stages: vec![stage("left"), stage("right"), stage("join")],
            connections: vec![Connection::plain(0, 2), Connection::plain(1, 2)],
            field_edges: Vec::new(),
        };
        let graph = LayoutGraph::from_pipeline_view(&view);

        assert_eq!(graph.nodes[0].rank, 0);
        assert_eq!(graph.nodes[1].rank, 0);
        assert_eq!(graph.nodes[2].rank, 1);
    }

    #[test]
    fn route_branches_are_ordered_output_ports_and_edge_sources() {
        let mut route = stage("route");
        route.kind = StageKind::Route;
        route.branches = vec![
            RouteBranch {
                name: "hi".to_string(),
                predicate: Some("amount > 100".to_string()),
                is_default: false,
            },
            RouteBranch {
                name: "lo".to_string(),
                predicate: None,
                is_default: true,
            },
        ];
        let view = PipelineView {
            stages: vec![route, stage("hi_out"), stage("lo_out")],
            connections: vec![
                Connection {
                    from: 0,
                    to: 1,
                    from_branch: Some(0),
                },
                Connection {
                    from: 0,
                    to: 2,
                    from_branch: Some(1),
                },
            ],
            field_edges: Vec::new(),
        };
        let graph = LayoutGraph::from_pipeline_view(&view);

        assert_eq!(
            graph.nodes[0]
                .output_ports
                .iter()
                .map(|port| (port.id.as_str(), port.kind, port.order))
                .collect::<Vec<_>>(),
            vec![
                ("branch:out:hi", LayoutPortKind::RouteBranch, 0),
                ("branch:out:lo", LayoutPortKind::RouteBranch, 1),
            ]
        );
        assert_eq!(graph.edges[0].kind, LayoutEdgeKind::RouteBranch);
        assert_eq!(graph.edges[0].from_port, "branch:out:hi");
        assert_eq!(
            graph.nodes[0].output_ports[1].stage_anchor,
            LayoutStageAnchor::BranchOutput { branch_index: 1 }
        );
    }

    #[test]
    fn wide_schema_tall_card_keeps_ordered_field_ports() {
        let mut wide = stage("wide");
        wide.fields = (0..120)
            .map(|idx| field(&format!("field_{idx:03}")))
            .collect();
        let mut sink = stage("sink");
        sink.fields = vec![field("field_119")];
        let view = PipelineView {
            stages: vec![wide, sink],
            connections: Vec::new(),
            field_edges: vec![FieldEdge {
                from_node: 0,
                from_field: "field_119".to_string(),
                to_node: 1,
                to_field: "field_119".to_string(),
                kind: FieldEdgeKind::Passthrough,
            }],
        };
        let graph = LayoutGraph::from_pipeline_view(&view);

        assert_eq!(
            graph.nodes[0]
                .output_ports
                .iter()
                .filter(|port| port.kind == LayoutPortKind::Field)
                .count(),
            120
        );
        assert_eq!(graph.nodes[0].height, view.stages[0].card_height());
        assert_eq!(
            graph.nodes[0]
                .output_ports
                .iter()
                .find(|port| port.id == "field:out:field_119")
                .map(|port| port.stage_anchor),
            Some(LayoutStageAnchor::FieldOutput { field_index: 119 })
        );
        assert_eq!(graph.edges[0].from_port, "field:out:field_119");
    }

    #[test]
    fn branch_fan_out_preserves_route_declaration_order_and_default_identity() {
        let mut route = stage("route");
        route.kind = StageKind::Route;
        route.branches = vec![
            RouteBranch {
                name: "gold".to_string(),
                predicate: Some("tier == 'gold'".to_string()),
                is_default: false,
            },
            RouteBranch {
                name: "silver".to_string(),
                predicate: Some("tier == 'silver'".to_string()),
                is_default: false,
            },
            RouteBranch {
                name: "standard".to_string(),
                predicate: None,
                is_default: true,
            },
        ];

        let view = PipelineView {
            stages: vec![
                route,
                stage("standard_sink"),
                stage("silver_sink"),
                stage("gold_sink"),
            ],
            connections: vec![
                Connection {
                    from: 0,
                    to: 3,
                    from_branch: Some(0),
                },
                Connection {
                    from: 0,
                    to: 2,
                    from_branch: Some(1),
                },
                Connection {
                    from: 0,
                    to: 1,
                    from_branch: Some(2),
                },
            ],
            field_edges: Vec::new(),
        };

        let graph = LayoutGraph::from_pipeline_view(&view);

        assert_eq!(
            output_port_ids(&graph.nodes[0]),
            vec![
                "branch:out:gold",
                "branch:out:silver",
                "branch:out:standard"
            ]
        );
        assert_eq!(graph.nodes[0].output_ports[2].label, "standard");
        assert_eq!(
            graph.nodes[0].output_ports[2].stage_anchor,
            LayoutStageAnchor::BranchOutput { branch_index: 2 }
        );
    }

    #[test]
    fn field_to_field_lineage_orders_ports_by_opposite_row_order() {
        let mut source = stage("source");
        source.fields = vec![field("late"), field("early")];
        let mut sink = stage("sink");
        sink.fields = vec![field("alpha"), field("omega")];
        let view = PipelineView {
            stages: vec![source, sink],
            connections: Vec::new(),
            field_edges: vec![
                FieldEdge {
                    from_node: 0,
                    from_field: "late".to_string(),
                    to_node: 1,
                    to_field: "omega".to_string(),
                    kind: FieldEdgeKind::Passthrough,
                },
                FieldEdge {
                    from_node: 0,
                    from_field: "early".to_string(),
                    to_node: 1,
                    to_field: "alpha".to_string(),
                    kind: FieldEdgeKind::Passthrough,
                },
            ],
        };

        let graph = LayoutGraph::from_pipeline_view(&view);

        assert_eq!(
            output_port_ids(&graph.nodes[0]),
            vec!["node:out", "field:out:early", "field:out:late"]
        );
        assert_eq!(
            input_port_ids(&graph.nodes[1]),
            vec!["node:in", "field:in:alpha", "field:in:omega"]
        );
        assert_eq!(
            graph.nodes[0].output_ports[1].stage_anchor,
            LayoutStageAnchor::FieldOutput { field_index: 1 }
        );
        assert_eq!(graph.edges[1].path.points[0].y, 1.0);
        assert_eq!(graph.edges[1].path.points[3].y, 1.0);
    }

    #[test]
    fn mixed_field_and_branch_ports_reorder_fields_without_moving_branches() {
        let mut route = stage("route");
        route.kind = StageKind::Route;
        route.fields = vec![field("zeta"), field("alpha")];
        route.branches = vec![
            RouteBranch {
                name: "first".to_string(),
                predicate: Some("ok".to_string()),
                is_default: false,
            },
            RouteBranch {
                name: "fallback".to_string(),
                predicate: None,
                is_default: true,
            },
        ];
        let mut sink = stage("sink");
        sink.fields = vec![field("alpha"), field("zeta")];
        let view = PipelineView {
            stages: vec![route, sink, stage("first_sink"), stage("fallback_sink")],
            connections: vec![
                Connection {
                    from: 0,
                    to: 2,
                    from_branch: Some(0),
                },
                Connection {
                    from: 0,
                    to: 3,
                    from_branch: Some(1),
                },
            ],
            field_edges: vec![
                FieldEdge {
                    from_node: 0,
                    from_field: "zeta".to_string(),
                    to_node: 1,
                    to_field: "zeta".to_string(),
                    kind: FieldEdgeKind::Passthrough,
                },
                FieldEdge {
                    from_node: 0,
                    from_field: "alpha".to_string(),
                    to_node: 1,
                    to_field: "alpha".to_string(),
                    kind: FieldEdgeKind::Passthrough,
                },
            ],
        };

        let graph = LayoutGraph::from_pipeline_view(&view);

        assert_eq!(
            output_port_ids(&graph.nodes[0]),
            vec![
                "field:out:alpha",
                "field:out:zeta",
                "branch:out:first",
                "branch:out:fallback"
            ]
        );
        assert_eq!(
            graph.nodes[0].output_ports[2].kind,
            LayoutPortKind::RouteBranch
        );
        assert_eq!(graph.nodes[0].output_ports[3].label, "fallback");
        assert_eq!(
            graph.nodes[0].output_ports[3].stage_anchor,
            LayoutStageAnchor::BranchOutput { branch_index: 1 }
        );
    }

    #[test]
    fn cull_side_output_has_distinct_port_and_edge_identity() {
        let mut cull = stage("cull");
        cull.kind = StageKind::Cull;
        cull.branches = vec![RouteBranch {
            name: "dropped".to_string(),
            predicate: None,
            is_default: false,
        }];
        let view = PipelineView {
            stages: vec![cull, stage("kept"), stage("dropped")],
            connections: vec![
                Connection::plain(0, 1),
                Connection {
                    from: 0,
                    to: 2,
                    from_branch: Some(0),
                },
            ],
            field_edges: Vec::new(),
        };

        let graph = LayoutGraph::from_pipeline_view(&view);

        assert_eq!(
            graph.nodes[0]
                .output_ports
                .iter()
                .map(|port| (port.id.as_str(), port.kind, port.order))
                .collect::<Vec<_>>(),
            vec![
                ("node:out", LayoutPortKind::Node, 0),
                ("branch:out:dropped", LayoutPortKind::CullSideOutput, 1),
            ]
        );
        assert_eq!(graph.edges[1].kind, LayoutEdgeKind::CullSideOutput);
    }
}
