// This model is intentionally introduced ahead of the renderer migration.
#![allow(dead_code)]

use super::{Connection, FieldEdge, NODE_WIDTH, PipelineView, StageView};

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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutPortSide {
    Input,
    Output,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutPortKind {
    Node,
    Field,
    Branch,
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
    Branch,
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
            LayoutPort::field(&format!("field:in:{}", field.name), &field.name, idx + 1)
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
                order + idx,
            )
        }));
        order += stage.fields.len();
        output_ports.extend(stage.branches.iter().enumerate().map(|(idx, branch)| {
            LayoutPort::branch(
                &format!("branch:out:{}", branch.name),
                &branch.name,
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
        }
    }

    fn field(id: &str, label: &str, order: usize) -> Self {
        Self {
            id: id.to_string(),
            label: label.to_string(),
            side: if id.starts_with("field:in:") {
                LayoutPortSide::Input
            } else {
                LayoutPortSide::Output
            },
            kind: LayoutPortKind::Field,
            order,
        }
    }

    fn branch(id: &str, label: &str, order: usize) -> Self {
        Self {
            id: id.to_string(),
            label: label.to_string(),
            side: LayoutPortSide::Output,
            kind: LayoutPortKind::Branch,
            order,
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
            LayoutEdgeKind::Branch,
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
                ("branch:out:hi", LayoutPortKind::Branch, 0),
                ("branch:out:lo", LayoutPortKind::Branch, 1),
            ]
        );
        assert_eq!(graph.edges[0].kind, LayoutEdgeKind::Branch);
        assert_eq!(graph.edges[0].from_port, "branch:out:hi");
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
            graph.nodes[0].output_ports.last().map(|port| port.order),
            Some(120)
        );
        assert_eq!(graph.edges[0].from_port, "field:out:field_119");
    }
}
