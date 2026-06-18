// This model is intentionally introduced ahead of the renderer migration.
#![allow(dead_code)]

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};

use super::{
    COLUMN_CENTER_Y, CanvasConnectorPath, CanvasPoint, Connection, FIELD_HEADER_HEIGHT,
    FIELD_ROW_HEIGHT, FieldEdge, FieldEdgeKind, HEADER_PORT_Y, LEFT_MARGIN, NODE_GAP, NODE_WIDTH,
    PipelineView, RoleEdge, STACK_GAP, StageKind, StagePortKind,
    StagePortSide as ViewStagePortSide, StageView,
};

/// Canvas layout path requested by callers during the renderer migration.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CanvasLayoutEngine {
    /// Existing `layout_positions` geometry and curved connector renderer.
    #[default]
    CurrentBarycenter,
    /// New Rust port-aware layered model. This is opt-in until visual QA proves
    /// the orthogonal renderer path should become the visible default.
    PortAwareSugiyama,
}

/// Result of applying a requested canvas layout path.
#[derive(Clone, Debug, PartialEq)]
pub struct CanvasLayoutResult {
    pub view: PipelineView,
    pub requested: CanvasLayoutEngine,
    pub applied: CanvasLayoutEngine,
    pub fallback: Option<CanvasLayoutFallback>,
}

impl CanvasLayoutResult {
    pub fn used_fallback(&self) -> bool {
        self.fallback.is_some()
    }
}

/// Why the port-aware layout request fell back to the current layout path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CanvasLayoutFallback {
    UnsupportedInput {
        reason: &'static str,
    },
    MissingStage {
        edge_index: usize,
        stage_index: usize,
    },
    MissingBranch {
        edge_index: usize,
        stage_index: usize,
        branch_index: usize,
    },
    MissingPort {
        edge_index: usize,
        stage_index: usize,
        side: LayoutPortSide,
        port_id: String,
    },
}

pub fn apply_canvas_layout(
    view: PipelineView,
    requested: CanvasLayoutEngine,
) -> CanvasLayoutResult {
    match requested {
        CanvasLayoutEngine::CurrentBarycenter => CanvasLayoutResult {
            view,
            requested,
            applied: CanvasLayoutEngine::CurrentBarycenter,
            fallback: None,
        },
        CanvasLayoutEngine::PortAwareSugiyama => apply_port_aware_layout(view, requested),
    }
}

fn apply_port_aware_layout(
    view: PipelineView,
    requested: CanvasLayoutEngine,
) -> CanvasLayoutResult {
    if let Some(fallback) = validate_port_aware_input(&view) {
        return CanvasLayoutResult {
            view,
            requested,
            applied: CanvasLayoutEngine::CurrentBarycenter,
            fallback: Some(fallback),
        };
    }

    let graph = LayoutGraph::from_pipeline_view(&view);
    if let Some(fallback) = validate_port_aware_graph(&graph) {
        return CanvasLayoutResult {
            view,
            requested,
            applied: CanvasLayoutEngine::CurrentBarycenter,
            fallback: Some(fallback),
        };
    }

    let mut migrated = view;
    apply_graph_positions(&mut migrated, &graph);
    apply_graph_paths(&mut migrated, &graph);
    CanvasLayoutResult {
        view: migrated,
        requested,
        applied: CanvasLayoutEngine::PortAwareSugiyama,
        fallback: None,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LayoutGraph {
    pub nodes: Vec<LayoutNode>,
    pub edges: Vec<LayoutEdge>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LayoutNode {
    pub stage_index: usize,
    pub id: String,
    pub kind: StageKind,
    pub x: f32,
    pub y: f32,
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
    AggregateGroupKey,
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
    RoleInput { role_index: usize },
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
    /// An INDIRECT influence field edge (#147) — a `Cull` `Filter`, `Route`
    /// `Conditional`, or `Merge`/`Combine` `JoinKey`. It is rendered as a
    /// revealed-on-selection overlay, so it gets a routed path but **zero**
    /// rank/order weight: an influence overlay must not pull the structural
    /// layout (which the DIRECT topology already determines) toward itself.
    /// (`Aggregate` `GroupBy` stays an ordinary [`LayoutEdgeKind::Field`]: it
    /// replaced the former group-key `Derive` edge one-for-one, so it keeps that
    /// edge's prior layout weight and does not shift any tuned layout.)
    IndirectField,
    AggregateGroupKey,
    RouteBranch,
    CullSideOutput,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ConnectorPath {
    pub points: Vec<LayoutPoint>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayoutMetrics {
    pub node_count: usize,
    pub edge_count: usize,
    pub rank_count: usize,
    pub max_rank_height: usize,
    pub total_rank_span: usize,
    pub average_rank_span: f32,
    pub max_rank_span: usize,
    pub skip_rank_edge_count: usize,
    pub source_skip_rank_edge_count: usize,
    pub estimated_crossings: usize,
    pub route_length: f32,
    pub card_overlap_risk: usize,
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

#[derive(Clone, Copy, Debug, PartialEq)]
struct NodeOrderScore {
    total: f32,
    weight: f32,
}

impl NodeOrderScore {
    fn average(self) -> f32 {
        self.total / self.weight
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NodeOrderDirection {
    Predecessors,
    Successors,
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
        edges.extend(view.role_edges.iter().map(role_edge));

        let mut graph = Self { nodes, edges };
        graph.assign_weighted_layered_ranks();
        let mut node_layer_orders = graph.initial_node_layer_orders();
        graph.order_ports_for_crossing_reduction(&node_layer_orders);
        node_layer_orders = graph.minimize_node_crossings(node_layer_orders);
        graph.order_ports_for_crossing_reduction(&node_layer_orders);
        node_layer_orders = graph.minimize_node_crossings(node_layer_orders);
        graph.assign_node_positions(&node_layer_orders);
        graph.route_orthogonal_lane_paths();
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
                nodes: {
                    let mut nodes = self
                        .nodes
                        .iter()
                        .filter(|node| node.rank == rank)
                        .collect::<Vec<_>>();
                    nodes.sort_by(|a, b| {
                        a.y.partial_cmp(&b.y)
                            .unwrap_or(Ordering::Equal)
                            .then_with(|| a.stage_index.cmp(&b.stage_index))
                    });
                    nodes.into_iter().map(|node| node.stage_index).collect()
                },
            })
            .collect()
    }

    pub fn metrics(&self) -> LayoutMetrics {
        let rank_heights = self.rank_heights();
        let rank_count = rank_heights.len();
        let max_rank_height = rank_heights.values().copied().max().unwrap_or(0);
        let mut total_rank_span = 0;
        let mut max_rank_span = 0;
        let mut skip_rank_edge_count = 0;
        let mut source_skip_rank_edge_count = 0;
        for edge in &self.edges {
            let span = rank_span(edge, &self.nodes);
            total_rank_span += span;
            max_rank_span = max_rank_span.max(span);
            if span > 1 {
                skip_rank_edge_count += 1;
                if matches!(&self.nodes[edge.from_node].kind, StageKind::Source) {
                    source_skip_rank_edge_count += 1;
                }
            }
        }

        LayoutMetrics {
            node_count: self.nodes.len(),
            edge_count: self.edges.len(),
            rank_count,
            max_rank_height,
            total_rank_span,
            average_rank_span: if self.edges.is_empty() {
                0.0
            } else {
                total_rank_span as f32 / self.edges.len() as f32
            },
            max_rank_span,
            skip_rank_edge_count,
            source_skip_rank_edge_count,
            estimated_crossings: self.estimated_crossings(),
            route_length: self.total_route_length(),
            card_overlap_risk: self.card_overlap_risk(),
        }
    }

    fn assign_weighted_layered_ranks(&mut self) {
        self.assign_longest_path_ranks();
        self.rank_sources_by_first_use();
        self.repair_downstream_ranks();
        self.relax_ranks_for_short_edges();
        self.repair_downstream_ranks();
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

    fn rank_sources_by_first_use(&mut self) {
        for node_index in 0..self.nodes.len() {
            if !matches!(&self.nodes[node_index].kind, StageKind::Source) {
                continue;
            }
            let first_consumer_rank = self
                .edges
                .iter()
                .filter(|edge| edge.from_node == node_index)
                .map(|edge| self.nodes[edge.to_node].rank)
                .min();
            if let Some(rank) = first_consumer_rank {
                self.nodes[node_index].rank = rank.saturating_sub(1);
            }
        }
    }

    fn repair_downstream_ranks(&mut self) {
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

    fn relax_ranks_for_short_edges(&mut self) {
        const RANK_RELAX_PASSES: usize = 3;

        for _ in 0..RANK_RELAX_PASSES {
            let mut changed = false;
            let current_ranks = self.nodes.iter().map(|node| node.rank).collect::<Vec<_>>();
            let mut next_ranks = current_ranks.clone();

            for node_index in 0..self.nodes.len() {
                let min_rank = self
                    .edges
                    .iter()
                    .filter(|edge| edge.to_node == node_index)
                    .map(|edge| current_ranks[edge.from_node] + 1)
                    .max()
                    .unwrap_or(0);
                let max_rank = self
                    .edges
                    .iter()
                    .filter(|edge| edge.from_node == node_index)
                    .map(|edge| current_ranks[edge.to_node].saturating_sub(1))
                    .min()
                    .unwrap_or(current_ranks[node_index]);
                if min_rank > max_rank {
                    continue;
                }

                let current_cost =
                    self.incident_rank_cost(node_index, current_ranks[node_index], &current_ranks);
                let mut best_rank = current_ranks[node_index];
                let mut best_cost = current_cost;
                for candidate in min_rank..=max_rank {
                    let cost = self.incident_rank_cost(node_index, candidate, &current_ranks);
                    if cost < best_cost
                        || (cost == best_cost
                            && candidate.abs_diff(current_ranks[node_index])
                                < best_rank.abs_diff(current_ranks[node_index]))
                    {
                        best_rank = candidate;
                        best_cost = cost;
                    }
                }

                if best_rank != current_ranks[node_index] && best_cost < current_cost {
                    next_ranks[node_index] = best_rank;
                    changed = true;
                }
            }

            if !changed {
                break;
            }
            for (node, rank) in self.nodes.iter_mut().zip(next_ranks) {
                node.rank = rank;
            }
        }
    }

    fn incident_rank_cost(
        &self,
        node_index: usize,
        candidate_rank: usize,
        ranks: &[usize],
    ) -> u128 {
        let mut cost = 0_u128;
        for edge in &self.edges {
            let weight = edge_rank_weight(edge.kind) as u128;
            if edge.from_node == node_index {
                let span = ranks[edge.to_node].saturating_sub(candidate_rank);
                cost += weight * (span as u128) * (span as u128);
            } else if edge.to_node == node_index {
                let span = candidate_rank.saturating_sub(ranks[edge.from_node]);
                cost += weight * (span as u128) * (span as u128);
            }
        }
        cost
    }

    /// Run a bounded, deterministic port-ordering pass.
    ///
    /// Only field-row ports are crossing-reordered. Route branch ports keep
    /// condition declaration order with the default branch last, and Cull side
    /// outputs keep their authored side-output identity. This preserves the
    /// semantic row order the current canvas exposes while still letting field
    /// lineage ports move toward their incident edges before renderer migration.
    fn order_ports_for_crossing_reduction(&mut self, node_layer_orders: &[usize]) {
        let base_orders = self.port_order_map();
        let scores = self.port_scores(&base_orders, node_layer_orders);

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
        let input_scores = self.port_scores(&output_ordered, node_layer_orders);
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

    fn initial_node_layer_orders(&self) -> Vec<usize> {
        let mut by_rank: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
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

    fn minimize_node_crossings(&self, initial_orders: Vec<usize>) -> Vec<usize> {
        const NODE_ORDER_SWEEPS: usize = 6;

        let ranks = self.sorted_ranks();
        let mut orders = initial_orders;
        for _ in 0..NODE_ORDER_SWEEPS {
            let previous = orders.clone();

            for &rank in ranks.iter().skip(1) {
                self.reorder_rank_by_neighbor_scores(
                    rank,
                    NodeOrderDirection::Predecessors,
                    &mut orders,
                );
            }
            for &rank in ranks.iter().rev().skip(1) {
                self.reorder_rank_by_neighbor_scores(
                    rank,
                    NodeOrderDirection::Successors,
                    &mut orders,
                );
            }

            if orders == previous {
                break;
            }
        }
        orders
    }

    fn reorder_rank_by_neighbor_scores(
        &self,
        rank: usize,
        direction: NodeOrderDirection,
        orders: &mut [usize],
    ) {
        let mut node_indices = self
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(node_index, node)| (node.rank == rank).then_some(node_index))
            .collect::<Vec<_>>();
        node_indices.sort_by(|&a, &b| {
            compare_optional_node_scores(
                self.node_order_score(a, direction, orders),
                self.node_order_score(b, direction, orders),
            )
            .then_with(|| orders[a].cmp(&orders[b]))
            .then_with(|| self.nodes[a].stage_index.cmp(&self.nodes[b].stage_index))
        });

        for (order, node_index) in node_indices.into_iter().enumerate() {
            orders[node_index] = order;
        }
    }

    fn node_order_score(
        &self,
        node_index: usize,
        direction: NodeOrderDirection,
        orders: &[usize],
    ) -> Option<NodeOrderScore> {
        let mut score = NodeOrderScore {
            total: 0.0,
            weight: 0.0,
        };

        for edge in &self.edges {
            let (neighbor, port_side, port_id) = match direction {
                NodeOrderDirection::Predecessors if edge.to_node == node_index => (
                    edge.from_node,
                    LayoutPortSide::Output,
                    edge.from_port.as_str(),
                ),
                NodeOrderDirection::Successors if edge.from_node == node_index => {
                    (edge.to_node, LayoutPortSide::Input, edge.to_port.as_str())
                }
                _ => continue,
            };
            let weight = edge_order_weight(edge.kind);
            score.total +=
                endpoint_order_score(&self.nodes[neighbor], neighbor, port_side, port_id, orders)
                    * weight;
            score.weight += weight;
        }

        (score.weight > 0.0).then_some(score)
    }

    fn sorted_ranks(&self) -> Vec<usize> {
        let mut ranks: Vec<usize> = self.nodes.iter().map(|node| node.rank).collect();
        ranks.sort_unstable();
        ranks.dedup();
        ranks
    }

    fn rank_heights(&self) -> BTreeMap<usize, usize> {
        let mut heights = BTreeMap::new();
        for node in &self.nodes {
            *heights.entry(node.rank).or_insert(0) += 1;
        }
        heights
    }

    fn estimated_crossings(&self) -> usize {
        let mut edge_groups: HashMap<(usize, usize), Vec<(f32, f32)>> = HashMap::new();
        for edge in &self.edges {
            if !matches!(
                edge.kind,
                LayoutEdgeKind::Node | LayoutEdgeKind::RouteBranch | LayoutEdgeKind::CullSideOutput
            ) {
                continue;
            }
            let from = &self.nodes[edge.from_node];
            let to = &self.nodes[edge.to_node];
            if to.rank <= from.rank {
                continue;
            }
            let start = port_anchor(from, &edge.from_port, LayoutPortSide::Output);
            let end = port_anchor(to, &edge.to_port, LayoutPortSide::Input);
            edge_groups
                .entry((from.rank, to.rank))
                .or_default()
                .push((start.y, end.y));
        }

        let mut crossings = 0;
        for endpoints in edge_groups.values() {
            for (idx, &(a_start, a_end)) in endpoints.iter().enumerate() {
                for &(b_start, b_end) in &endpoints[idx + 1..] {
                    if (a_start < b_start && a_end > b_end) || (a_start > b_start && a_end < b_end)
                    {
                        crossings += 1;
                    }
                }
            }
        }
        crossings
    }

    fn total_route_length(&self) -> f32 {
        self.edges
            .iter()
            .map(|edge| {
                edge.path
                    .points
                    .windows(2)
                    .map(|points| {
                        (points[1].x - points[0].x).abs() + (points[1].y - points[0].y).abs()
                    })
                    .sum::<f32>()
            })
            .sum()
    }

    fn card_overlap_risk(&self) -> usize {
        let mut overlaps = 0;
        for (idx, a) in self.nodes.iter().enumerate() {
            for b in &self.nodes[idx + 1..] {
                if rects_overlap(a, b) {
                    overlaps += 1;
                }
            }
        }
        overlaps
    }

    fn assign_node_positions(&mut self, node_layer_orders: &[usize]) {
        let ranks = self.sorted_ranks();

        for rank in ranks {
            let mut node_indices = self
                .nodes
                .iter()
                .enumerate()
                .filter_map(|(node_index, node)| (node.rank == rank).then_some(node_index))
                .collect::<Vec<_>>();
            node_indices.sort_unstable_by_key(|&node_index| {
                (
                    node_layer_orders
                        .get(node_index)
                        .copied()
                        .unwrap_or(usize::MAX),
                    self.nodes[node_index].stage_index,
                )
            });

            let x = rank as f32 * (NODE_WIDTH + NODE_GAP);
            let mut y = 0.0;
            for node_index in node_indices {
                self.nodes[node_index].x = x;
                self.nodes[node_index].y = y;
                y += self.nodes[node_index].height + STACK_GAP;
            }
        }
    }

    fn route_orthogonal_lane_paths(&mut self) {
        let lane_assignments = self.edge_lane_assignments();
        for (edge_index, (lane_index, lane_count)) in lane_assignments
            .iter()
            .copied()
            .enumerate()
            .take(self.edges.len())
        {
            let edge = &self.edges[edge_index];
            let from = &self.nodes[edge.from_node];
            let to = &self.nodes[edge.to_node];
            let start = port_anchor(from, &edge.from_port, LayoutPortSide::Output);
            let end = port_anchor(to, &edge.to_port, LayoutPortSide::Input);
            let lane_x = connector_lane_x(from, to, lane_index, lane_count);

            self.edges[edge_index].path.points = compact_path_points([
                start,
                LayoutPoint {
                    x: lane_x,
                    y: start.y,
                },
                LayoutPoint {
                    x: lane_x,
                    y: end.y,
                },
                end,
            ]);
        }
    }

    fn edge_lane_assignments(&self) -> Vec<(usize, usize)> {
        let mut by_rank_gap: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
        for (edge_index, edge) in self.edges.iter().enumerate() {
            by_rank_gap
                .entry(edge_rank_gap(edge, &self.nodes))
                .or_default()
                .push(edge_index);
        }

        let mut assignments = vec![(0, 1); self.edges.len()];
        for edge_indices in by_rank_gap.values_mut() {
            edge_indices.sort_unstable_by_key(|&edge_index| {
                let edge = &self.edges[edge_index];
                (
                    self.nodes[edge.from_node].stage_index,
                    port_order(
                        &self.nodes[edge.from_node],
                        &edge.from_port,
                        LayoutPortSide::Output,
                    ),
                    self.nodes[edge.to_node].stage_index,
                    port_order(
                        &self.nodes[edge.to_node],
                        &edge.to_port,
                        LayoutPortSide::Input,
                    ),
                    edge_kind_order(edge.kind),
                )
            });
            let lane_count = edge_indices.len();
            for (lane_index, &edge_index) in edge_indices.iter().enumerate() {
                assignments[edge_index] = (lane_index, lane_count);
            }
        }

        assignments
    }
}

impl LayoutNode {
    fn from_stage(stage_index: usize, stage: &StageView) -> Self {
        let mut input_ports = vec![LayoutPort::node("node:in", LayoutPortSide::Input, 0)];
        let input_role_ports = stage
            .role_ports
            .iter()
            .filter(|port| port.side == ViewStagePortSide::Input)
            .collect::<Vec<_>>();
        input_ports.extend(input_role_ports.iter().enumerate().map(|(idx, port)| {
            LayoutPort::role_input(
                &format!("role:in:{}", port.id),
                &port.label,
                port.kind,
                idx,
                stage.input_role_header_count() + idx + 1,
            )
        }));
        let input_field_order = stage.input_role_header_count() + input_role_ports.len() + 1;
        input_ports.extend(stage.fields.iter().enumerate().map(|(idx, field)| {
            LayoutPort::field(
                &format!("field:in:{}", field.name),
                &field.name,
                LayoutPortSide::Input,
                idx,
                input_field_order + idx,
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
            kind: stage.kind.clone(),
            x: 0.0,
            y: 0.0,
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

    fn role_input(
        id: &str,
        label: &str,
        kind: StagePortKind,
        role_index: usize,
        order: usize,
    ) -> Self {
        let kind = match kind {
            StagePortKind::AggregateGroupKey => LayoutPortKind::AggregateGroupKey,
        };
        Self {
            id: id.to_string(),
            label: label.to_string(),
            side: LayoutPortSide::Input,
            kind,
            order,
            stage_anchor: LayoutStageAnchor::RoleInput { role_index },
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
    // An INDIRECT influence edge becomes a zero-weight overlay edge so it routes
    // a cable without distorting the structural layout; `GroupBy` (which replaced
    // the group-key `Derive`) stays an ordinary weighted field edge (#147).
    let kind = match edge.kind {
        FieldEdgeKind::Filter | FieldEdgeKind::JoinKey | FieldEdgeKind::Conditional => {
            LayoutEdgeKind::IndirectField
        }
        FieldEdgeKind::Passthrough
        | FieldEdgeKind::Access
        | FieldEdgeKind::Derive
        | FieldEdgeKind::GroupBy => LayoutEdgeKind::Field,
    };
    LayoutEdge {
        from_node: edge.from_node,
        from_port: format!("field:out:{}", edge.from_field),
        to_node: edge.to_node,
        to_port: format!("field:in:{}", edge.to_field),
        kind,
        path: ConnectorPath::default(),
    }
}

fn role_edge(edge: &RoleEdge) -> LayoutEdge {
    LayoutEdge {
        from_node: edge.from_node,
        from_port: format!("field:out:{}", edge.from_field),
        to_node: edge.to_node,
        to_port: format!("role:in:{}", edge.to_port),
        kind: LayoutEdgeKind::AggregateGroupKey,
        path: ConnectorPath::default(),
    }
}

fn compare_optional_node_scores(a: Option<NodeOrderScore>, b: Option<NodeOrderScore>) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) => a
            .average()
            .partial_cmp(&b.average())
            .unwrap_or(Ordering::Equal),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn endpoint_order_score(
    node: &LayoutNode,
    node_index: usize,
    side: LayoutPortSide,
    port_id: &str,
    node_layer_orders: &[usize],
) -> f32 {
    const NODE_SCORE_STRIDE: f32 = 10_000.0;

    let ports = match side {
        LayoutPortSide::Input => &node.input_ports,
        LayoutPortSide::Output => &node.output_ports,
    };
    let port_order = ports
        .iter()
        .find(|port| port.id == port_id)
        .map(|port| port.order)
        .unwrap_or(0);
    node_layer_orders.get(node_index).copied().unwrap_or(0) as f32 * NODE_SCORE_STRIDE
        + port_order as f32
}

fn edge_rank_weight(kind: LayoutEdgeKind) -> usize {
    match kind {
        LayoutEdgeKind::Node => 64,
        LayoutEdgeKind::RouteBranch | LayoutEdgeKind::CullSideOutput => 48,
        LayoutEdgeKind::AggregateGroupKey => 24,
        LayoutEdgeKind::Field => 16,
        // INDIRECT influence overlays carry no structural weight (#147): they are
        // revealed on selection and must not pull node ranks/ordering, which the
        // DIRECT topology already fixes.
        LayoutEdgeKind::IndirectField => 0,
    }
}

fn edge_order_weight(kind: LayoutEdgeKind) -> f32 {
    edge_rank_weight(kind) as f32
}

fn rank_span(edge: &LayoutEdge, nodes: &[LayoutNode]) -> usize {
    nodes[edge.to_node]
        .rank
        .saturating_sub(nodes[edge.from_node].rank)
}

fn rects_overlap(a: &LayoutNode, b: &LayoutNode) -> bool {
    let a_left = a.x;
    let a_right = a.x + a.width;
    let a_top = a.y;
    let a_bottom = a.y + a.height;
    let b_left = b.x;
    let b_right = b.x + b.width;
    let b_top = b.y;
    let b_bottom = b.y + b.height;

    a_left < b_right && b_left < a_right && a_top < b_bottom && b_top < a_bottom
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
        LayoutPortKind::AggregateGroupKey => 1,
        LayoutPortKind::Field => 2,
        LayoutPortKind::RouteBranch | LayoutPortKind::CullSideOutput => 3,
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

fn edge_rank_gap(edge: &LayoutEdge, nodes: &[LayoutNode]) -> (usize, usize) {
    let from_rank = nodes[edge.from_node].rank;
    let to_rank = nodes[edge.to_node].rank;
    if from_rank <= to_rank {
        (from_rank, to_rank)
    } else {
        (to_rank, from_rank)
    }
}

fn edge_kind_order(kind: LayoutEdgeKind) -> usize {
    match kind {
        LayoutEdgeKind::Node => 0,
        LayoutEdgeKind::Field => 1,
        // INDIRECT overlays route in the same lane band as ordinary field edges.
        LayoutEdgeKind::IndirectField => 1,
        LayoutEdgeKind::AggregateGroupKey => 2,
        LayoutEdgeKind::RouteBranch => 3,
        LayoutEdgeKind::CullSideOutput => 4,
    }
}

fn connector_lane_x(
    from: &LayoutNode,
    to: &LayoutNode,
    lane_index: usize,
    lane_count: usize,
) -> f32 {
    let source_right = from.x + from.width;
    let target_left = to.x;
    if target_left > source_right {
        let gap = target_left - source_right;
        source_right + gap * (lane_index as f32 + 1.0) / (lane_count as f32 + 1.0)
    } else {
        source_right.max(target_left) + NODE_GAP * (lane_index as f32 + 1.0)
    }
}

fn port_anchor(node: &LayoutNode, port_id: &str, side: LayoutPortSide) -> LayoutPoint {
    let ports = match side {
        LayoutPortSide::Input => &node.input_ports,
        LayoutPortSide::Output => &node.output_ports,
    };
    let Some(port) = ports.iter().find(|port| port.id == port_id) else {
        return LayoutPoint {
            x: port_x(node, side),
            y: node.y + HEADER_PORT_Y,
        };
    };

    let y = match port.stage_anchor {
        LayoutStageAnchor::NodeInput | LayoutStageAnchor::NodeOutput => node.y + HEADER_PORT_Y,
        LayoutStageAnchor::RoleInput { role_index } => {
            node.y
                + FIELD_HEADER_HEIGHT
                + (input_role_header_count(node) + role_index) as f32 * FIELD_ROW_HEIGHT
                + FIELD_ROW_HEIGHT / 2.0
        }
        LayoutStageAnchor::FieldInput { field_index }
        | LayoutStageAnchor::FieldOutput { field_index } => {
            node.y
                + FIELD_HEADER_HEIGHT
                + (input_role_header_count(node) + input_role_port_count(node) + field_index) as f32
                    * FIELD_ROW_HEIGHT
                + FIELD_ROW_HEIGHT / 2.0
        }
        LayoutStageAnchor::BranchOutput { branch_index } => {
            node.y
                + FIELD_HEADER_HEIGHT
                + (input_role_header_count(node)
                    + input_role_port_count(node)
                    + field_port_count(node)
                    + branch_index) as f32
                    * FIELD_ROW_HEIGHT
                + FIELD_ROW_HEIGHT / 2.0
        }
    };

    LayoutPoint {
        x: port_x(node, side),
        y,
    }
}

fn port_x(node: &LayoutNode, side: LayoutPortSide) -> f32 {
    match side {
        LayoutPortSide::Input => node.x,
        LayoutPortSide::Output => node.x + node.width,
    }
}

fn field_port_count(node: &LayoutNode) -> usize {
    node.output_ports
        .iter()
        .filter(|port| port.kind == LayoutPortKind::Field)
        .count()
}

fn input_role_port_count(node: &LayoutNode) -> usize {
    node.input_ports
        .iter()
        .filter(|port| port.kind == LayoutPortKind::AggregateGroupKey)
        .count()
}

fn input_role_header_count(node: &LayoutNode) -> usize {
    if input_role_port_count(node) > 0 {
        1
    } else {
        0
    }
}

fn compact_path_points<const N: usize>(points: [LayoutPoint; N]) -> Vec<LayoutPoint> {
    let mut compacted = Vec::with_capacity(N);
    for point in points {
        if compacted
            .last()
            .is_none_or(|previous: &LayoutPoint| *previous != point)
        {
            compacted.push(point);
        }
    }
    compacted
}

fn validate_port_aware_input(view: &PipelineView) -> Option<CanvasLayoutFallback> {
    for (edge_index, connection) in view.connections.iter().enumerate() {
        if connection.from >= view.stages.len() {
            return Some(CanvasLayoutFallback::MissingStage {
                edge_index,
                stage_index: connection.from,
            });
        }
        if connection.to >= view.stages.len() {
            return Some(CanvasLayoutFallback::MissingStage {
                edge_index,
                stage_index: connection.to,
            });
        }
        if let Some(branch_index) = connection.from_branch {
            let stage = &view.stages[connection.from];
            if stage.branches.get(branch_index).is_none() {
                return Some(CanvasLayoutFallback::MissingBranch {
                    edge_index,
                    stage_index: connection.from,
                    branch_index,
                });
            }
        }
    }

    for (edge_index, edge) in view.field_edges.iter().enumerate() {
        if edge.from_node >= view.stages.len() {
            return Some(CanvasLayoutFallback::MissingStage {
                edge_index,
                stage_index: edge.from_node,
            });
        }
        if edge.to_node >= view.stages.len() {
            return Some(CanvasLayoutFallback::MissingStage {
                edge_index,
                stage_index: edge.to_node,
            });
        }
        if view.stages[edge.from_node]
            .field_index(&edge.from_field)
            .is_none()
        {
            return Some(CanvasLayoutFallback::MissingPort {
                edge_index,
                stage_index: edge.from_node,
                side: LayoutPortSide::Output,
                port_id: format!("field:out:{}", edge.from_field),
            });
        }
        if view.stages[edge.to_node]
            .field_index(&edge.to_field)
            .is_none()
        {
            return Some(CanvasLayoutFallback::MissingPort {
                edge_index,
                stage_index: edge.to_node,
                side: LayoutPortSide::Input,
                port_id: format!("field:in:{}", edge.to_field),
            });
        }
    }

    for (edge_index, edge) in view.role_edges.iter().enumerate() {
        if edge.from_node >= view.stages.len() {
            return Some(CanvasLayoutFallback::MissingStage {
                edge_index,
                stage_index: edge.from_node,
            });
        }
        if edge.to_node >= view.stages.len() {
            return Some(CanvasLayoutFallback::MissingStage {
                edge_index,
                stage_index: edge.to_node,
            });
        }
        if view.stages[edge.from_node]
            .field_index(&edge.from_field)
            .is_none()
        {
            return Some(CanvasLayoutFallback::MissingPort {
                edge_index,
                stage_index: edge.from_node,
                side: LayoutPortSide::Output,
                port_id: format!("field:out:{}", edge.from_field),
            });
        }
        if view.stages[edge.to_node]
            .role_port_index(ViewStagePortSide::Input, &edge.to_port)
            .is_none()
        {
            return Some(CanvasLayoutFallback::MissingPort {
                edge_index,
                stage_index: edge.to_node,
                side: LayoutPortSide::Input,
                port_id: format!("role:in:{}", edge.to_port),
            });
        }
    }

    None
}

fn validate_port_aware_graph(graph: &LayoutGraph) -> Option<CanvasLayoutFallback> {
    for (edge_index, edge) in graph.edges.iter().enumerate() {
        let Some(from) = graph.nodes.get(edge.from_node) else {
            return Some(CanvasLayoutFallback::MissingStage {
                edge_index,
                stage_index: edge.from_node,
            });
        };
        let Some(to) = graph.nodes.get(edge.to_node) else {
            return Some(CanvasLayoutFallback::MissingStage {
                edge_index,
                stage_index: edge.to_node,
            });
        };
        if !has_port(from, LayoutPortSide::Output, &edge.from_port) {
            return Some(CanvasLayoutFallback::MissingPort {
                edge_index,
                stage_index: from.stage_index,
                side: LayoutPortSide::Output,
                port_id: edge.from_port.clone(),
            });
        }
        if !has_port(to, LayoutPortSide::Input, &edge.to_port) {
            return Some(CanvasLayoutFallback::MissingPort {
                edge_index,
                stage_index: to.stage_index,
                side: LayoutPortSide::Input,
                port_id: edge.to_port.clone(),
            });
        }
        if edge.path.points.len() < 2 {
            return Some(CanvasLayoutFallback::UnsupportedInput {
                reason: "connector path has fewer than two points",
            });
        }
    }

    None
}

fn has_port(node: &LayoutNode, side: LayoutPortSide, port_id: &str) -> bool {
    let ports = match side {
        LayoutPortSide::Input => &node.input_ports,
        LayoutPortSide::Output => &node.output_ports,
    };
    ports.iter().any(|port| port.id == port_id)
}

fn apply_graph_positions(view: &mut PipelineView, graph: &LayoutGraph) {
    let rank_offsets = rank_center_offsets(graph);
    for node in &graph.nodes {
        let y_offset = rank_offsets
            .get(&node.rank)
            .copied()
            .unwrap_or(COLUMN_CENTER_Y);
        if let Some(stage) = view.stages.get_mut(node.stage_index) {
            stage.canvas_x = LEFT_MARGIN + node.x;
            stage.canvas_y = node.y + y_offset;
        }
    }
}

fn apply_graph_paths(view: &mut PipelineView, graph: &LayoutGraph) {
    let connection_count = view.connections.len();
    let field_edge_count = view.field_edges.len();
    let rank_offsets = rank_center_offsets(graph);
    view.connection_paths = graph
        .edges
        .iter()
        .take(connection_count)
        .map(|edge| canvas_path_for_edge(edge, graph, &rank_offsets).unwrap_or_default())
        .collect();
    view.field_edge_paths = graph
        .edges
        .iter()
        .skip(connection_count)
        .take(field_edge_count)
        .map(|edge| canvas_path_for_edge(edge, graph, &rank_offsets).unwrap_or_default())
        .collect();
    view.role_edge_paths = graph
        .edges
        .iter()
        .skip(connection_count + field_edge_count)
        .take(view.role_edges.len())
        .map(|edge| canvas_path_for_edge(edge, graph, &rank_offsets).unwrap_or_default())
        .collect();
    debug_assert_eq!(view.connection_paths.len(), view.connections.len());
    debug_assert_eq!(view.field_edge_paths.len(), view.field_edges.len());
    debug_assert_eq!(view.role_edge_paths.len(), view.role_edges.len());
}

fn canvas_path_for_edge(
    edge: &LayoutEdge,
    graph: &LayoutGraph,
    rank_offsets: &HashMap<usize, f32>,
) -> Option<CanvasConnectorPath> {
    let from = graph.nodes.get(edge.from_node)?;
    let to = graph.nodes.get(edge.to_node)?;
    let from_offset = rank_offsets
        .get(&from.rank)
        .copied()
        .unwrap_or(COLUMN_CENTER_Y);
    let to_offset = rank_offsets
        .get(&to.rank)
        .copied()
        .unwrap_or(COLUMN_CENTER_Y);
    let start = edge.path.points.first()?;
    let end = edge.path.points.last()?;
    let lane_x = edge
        .path
        .points
        .get(1)
        .map(|point| point.x)
        .unwrap_or((start.x + end.x) / 2.0);

    let start = CanvasPoint {
        x: LEFT_MARGIN + start.x,
        y: from_offset + start.y,
    };
    let end = CanvasPoint {
        x: LEFT_MARGIN + end.x,
        y: to_offset + end.y,
    };
    let lane_x = LEFT_MARGIN + lane_x;

    Some(CanvasConnectorPath {
        points: compact_canvas_path_points([
            start,
            CanvasPoint {
                x: lane_x,
                y: start.y,
            },
            CanvasPoint {
                x: lane_x,
                y: end.y,
            },
            end,
        ]),
    })
}

fn rank_center_offsets(graph: &LayoutGraph) -> HashMap<usize, f32> {
    let mut extents: HashMap<usize, (f32, f32)> = HashMap::new();
    for node in &graph.nodes {
        extents
            .entry(node.rank)
            .and_modify(|(min_y, max_y)| {
                *min_y = min_y.min(node.y);
                *max_y = max_y.max(node.y + node.height);
            })
            .or_insert((node.y, node.y + node.height));
    }

    extents
        .into_iter()
        .map(|(rank, (min_y, max_y))| (rank, COLUMN_CENTER_Y - (min_y + max_y) / 2.0))
        .collect()
}

fn compact_canvas_path_points<const N: usize>(points: [CanvasPoint; N]) -> Vec<CanvasPoint> {
    let mut compacted = Vec::with_capacity(N);
    for point in points {
        if compacted
            .last()
            .is_none_or(|previous: &CanvasPoint| *previous != point)
        {
            compacted.push(point);
        }
    }
    compacted
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline_view::{
        Connection, EdgeNature, FieldEdge, FieldEdgeKind, FieldKind, FieldRow, PipelineView,
        RoleEdge, RouteBranch, StageKind, StagePortKind, StagePortRow, StagePortSide, StageView,
        derive_pipeline_view,
    };
    use clinker_plan::config::parse_config;

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
            role_ports: Vec::new(),
        }
    }

    fn field(name: &str) -> FieldRow {
        FieldRow {
            name: name.to_string(),
            kind: FieldKind::Declared,
            ty: None,
            is_correlation_key: false,
            ..Default::default()
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

    fn fixture_graph(yaml: &str) -> LayoutGraph {
        let config = parse_config(yaml).expect("fixture pipeline parses");
        let view = derive_pipeline_view(&config);
        LayoutGraph::from_pipeline_view(&view)
    }

    fn legacy_longest_path_graph(yaml: &str) -> LayoutGraph {
        let config = parse_config(yaml).expect("fixture pipeline parses");
        let view = derive_pipeline_view(&config);
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
        edges.extend(view.role_edges.iter().map(role_edge));

        let mut graph = LayoutGraph { nodes, edges };
        graph.assign_longest_path_ranks();
        let node_layer_orders = graph.initial_node_layer_orders();
        graph.order_ports_for_crossing_reduction(&node_layer_orders);
        graph.assign_node_positions(&node_layer_orders);
        graph.route_orthogonal_lane_paths();
        graph
    }

    fn rank_of(graph: &LayoutGraph, id: &str) -> usize {
        graph
            .nodes
            .iter()
            .find(|node| node.id == id)
            .map(|node| node.rank)
            .unwrap_or_else(|| panic!("missing layout node {id}"))
    }

    fn assert_edges_flow_left_to_right(graph: &LayoutGraph) {
        for edge in &graph.edges {
            assert!(
                graph.nodes[edge.to_node].rank > graph.nodes[edge.from_node].rank,
                "edge {:?} should flow left-to-right: {}({}) -> {}({})",
                edge.kind,
                graph.nodes[edge.from_node].id,
                graph.nodes[edge.from_node].rank,
                graph.nodes[edge.to_node].id,
                graph.nodes[edge.to_node].rank
            );
        }
    }

    fn assert_sources_left_of_consumers(graph: &LayoutGraph) {
        for edge in &graph.edges {
            if matches!(&graph.nodes[edge.from_node].kind, StageKind::Source) {
                assert!(
                    graph.nodes[edge.from_node].rank < graph.nodes[edge.to_node].rank,
                    "source {} should remain left of consumer {}",
                    graph.nodes[edge.from_node].id,
                    graph.nodes[edge.to_node].id
                );
            }
        }
    }

    fn assert_benchmark_targets(
        yaml: &str,
        legacy_max_span: usize,
        target_max_span: usize,
        crossing_baseline: usize,
    ) {
        let legacy = legacy_longest_path_graph(yaml);
        let legacy_metrics = legacy.metrics();
        assert_eq!(legacy_metrics.max_rank_span, legacy_max_span);

        let graph = fixture_graph(yaml);
        let metrics = graph.metrics();
        assert!(
            metrics.max_rank_span <= target_max_span,
            "expected max rank span <= {target_max_span}, got {metrics:?}"
        );
        assert!(
            metrics.source_skip_rank_edge_count < legacy_metrics.source_skip_rank_edge_count,
            "source skip-rank edges should decrease from legacy metrics {legacy_metrics:?} to {metrics:?}"
        );
        assert!(
            metrics.estimated_crossings <= crossing_baseline,
            "estimated crossings should not exceed baseline {crossing_baseline}: {metrics:?}"
        );
        assert_eq!(metrics.card_overlap_risk, 0);
        assert_edges_flow_left_to_right(&graph);
        assert_sources_left_of_consumers(&graph);
    }

    #[test]
    fn simple_chain_assigns_layers_and_node_ports() {
        let view = PipelineView {
            stages: vec![stage("a"), stage("b"), stage("c")],
            connections: vec![Connection::plain(0, 1), Connection::plain(1, 2)],
            connection_paths: Vec::new(),
            field_edges: Vec::new(),
            field_edge_paths: Vec::new(),
            role_edges: Vec::new(),
            role_edge_paths: Vec::new(),
        };
        let graph = LayoutGraph::from_pipeline_view(&view);

        assert_eq!(graph.layers()[0].nodes, vec![0]);
        assert_eq!(graph.layers()[1].nodes, vec![1]);
        assert_eq!(graph.layers()[2].nodes, vec![2]);
        assert_eq!(graph.edges[0].from_port, "node:out");
        assert_eq!(graph.edges[0].to_port, "node:in");
        assert_eq!(
            graph.edges[0].path.points,
            vec![
                LayoutPoint {
                    x: NODE_WIDTH,
                    y: HEADER_PORT_Y
                },
                LayoutPoint {
                    x: NODE_WIDTH + NODE_GAP / 2.0,
                    y: HEADER_PORT_Y
                },
                LayoutPoint {
                    x: NODE_WIDTH + NODE_GAP,
                    y: HEADER_PORT_Y
                },
            ]
        );
    }

    #[test]
    fn fan_out_keeps_same_rank_consumers_in_input_order() {
        let view = PipelineView {
            stages: vec![stage("source"), stage("left"), stage("right")],
            connections: vec![Connection::plain(0, 1), Connection::plain(0, 2)],
            connection_paths: Vec::new(),
            field_edges: Vec::new(),
            field_edge_paths: Vec::new(),
            role_edges: Vec::new(),
            role_edge_paths: Vec::new(),
        };
        let graph = LayoutGraph::from_pipeline_view(&view);

        assert_eq!(graph.layers()[0].nodes, vec![0]);
        assert_eq!(graph.layers()[1].nodes, vec![1, 2]);
    }

    #[test]
    fn branch_fan_out_orders_consumers_by_branch_port_order() {
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
            connection_paths: Vec::new(),
            field_edges: Vec::new(),
            field_edge_paths: Vec::new(),
            role_edges: Vec::new(),
            role_edge_paths: Vec::new(),
        };

        let graph = LayoutGraph::from_pipeline_view(&view);

        assert_eq!(graph.layers()[1].nodes, vec![3, 2, 1]);
    }

    #[test]
    fn fan_in_places_consumer_after_all_inputs() {
        let view = PipelineView {
            stages: vec![stage("left"), stage("right"), stage("join")],
            connections: vec![Connection::plain(0, 2), Connection::plain(1, 2)],
            connection_paths: Vec::new(),
            field_edges: Vec::new(),
            field_edge_paths: Vec::new(),
            role_edges: Vec::new(),
            role_edge_paths: Vec::new(),
        };
        let graph = LayoutGraph::from_pipeline_view(&view);

        assert_eq!(graph.nodes[0].rank, 0);
        assert_eq!(graph.nodes[1].rank, 0);
        assert_eq!(graph.nodes[2].rank, 1);
        let first_lane_x = graph.edges[0].path.points[1].x;
        let second_lane_x = graph.edges[1].path.points[1].x;
        assert!(
            first_lane_x < second_lane_x,
            "fan-in edges should reserve distinct lanes in source order"
        );
        assert_eq!(
            graph.edges[0].path.points.last(),
            graph.edges[1].path.points.last()
        );
    }

    #[test]
    fn sources_rank_to_first_use_instead_of_dedicated_zero_column() {
        let yaml = r#"
pipeline:
  name: late_source_first_use
nodes:
  - type: source
    name: orders
    config:
      name: orders
      type: csv
      path: ./orders.csv
      schema:
        - { name: id, type: string }
  - type: source
    name: products
    config:
      name: products
      type: csv
      path: ./products.csv
      schema:
        - { name: id, type: string }
  - type: transform
    name: clean
    input: orders
    config:
      cxl: |
        emit id = id
  - type: transform
    name: normalized
    input: clean
    config:
      cxl: |
        emit id = id
  - type: combine
    name: joined
    input:
      orders: normalized
      products: products
    config:
      where: "orders.id == products.id"
      match: first
      on_miss: null_fields
      cxl: |
        emit id = orders.id
      propagate_ck: driver
"#;
        let graph = fixture_graph(yaml);

        assert_eq!(rank_of(&graph, "orders"), 0);
        assert_eq!(rank_of(&graph, "clean"), 1);
        assert_eq!(rank_of(&graph, "normalized"), 2);
        assert_eq!(rank_of(&graph, "products"), 2);
        assert_eq!(rank_of(&graph, "joined"), 3);
        assert_edges_flow_left_to_right(&graph);
        assert_sources_left_of_consumers(&graph);
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
            connection_paths: Vec::new(),
            field_edges: Vec::new(),
            field_edge_paths: Vec::new(),
            role_edges: Vec::new(),
            role_edge_paths: Vec::new(),
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
            connection_paths: Vec::new(),
            field_edges: vec![FieldEdge {
                from_node: 0,
                from_field: "field_119".to_string(),
                to_node: 1,
                to_field: "field_119".to_string(),
                kind: FieldEdgeKind::Passthrough,
                ..Default::default()
            }],
            field_edge_paths: Vec::new(),
            role_edges: Vec::new(),
            role_edge_paths: Vec::new(),
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
        assert_eq!(
            graph.edges[0].path.points.first(),
            Some(&LayoutPoint {
                x: NODE_WIDTH,
                y: FIELD_HEADER_HEIGHT + 119.0 * FIELD_ROW_HEIGHT + FIELD_ROW_HEIGHT / 2.0,
            })
        );
        assert!(
            graph.edges[0].path.points[1].x > NODE_WIDTH
                && graph.edges[0].path.points[1].x < NODE_WIDTH + NODE_GAP,
            "tall-card field edge should route through the inter-rank lane"
        );
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
            connection_paths: Vec::new(),
            field_edges: Vec::new(),
            field_edge_paths: Vec::new(),
            role_edges: Vec::new(),
            role_edge_paths: Vec::new(),
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
        assert_eq!(
            graph
                .edges
                .iter()
                .map(|edge| edge.path.points[0].y)
                .collect::<Vec<_>>(),
            vec![
                FIELD_HEADER_HEIGHT + FIELD_ROW_HEIGHT / 2.0,
                FIELD_HEADER_HEIGHT + FIELD_ROW_HEIGHT + FIELD_ROW_HEIGHT / 2.0,
                FIELD_HEADER_HEIGHT + 2.0 * FIELD_ROW_HEIGHT + FIELD_ROW_HEIGHT / 2.0,
            ]
        );
        assert!(
            graph.edges[0].path.points[1].x < graph.edges[1].path.points[1].x
                && graph.edges[1].path.points[1].x < graph.edges[2].path.points[1].x,
            "branch fan-out should reserve ordered lanes"
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
            connection_paths: Vec::new(),
            field_edges: vec![
                FieldEdge {
                    from_node: 0,
                    from_field: "late".to_string(),
                    to_node: 1,
                    to_field: "omega".to_string(),
                    kind: FieldEdgeKind::Passthrough,
                    ..Default::default()
                },
                FieldEdge {
                    from_node: 0,
                    from_field: "early".to_string(),
                    to_node: 1,
                    to_field: "alpha".to_string(),
                    kind: FieldEdgeKind::Passthrough,
                    ..Default::default()
                },
            ],
            field_edge_paths: Vec::new(),
            role_edges: Vec::new(),
            role_edge_paths: Vec::new(),
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
        assert_eq!(graph.edges[1].kind, LayoutEdgeKind::Field);
        assert_eq!(
            graph.edges[1].path.points.first(),
            Some(&LayoutPoint {
                x: NODE_WIDTH,
                y: FIELD_HEADER_HEIGHT + FIELD_ROW_HEIGHT + FIELD_ROW_HEIGHT / 2.0,
            })
        );
        assert_eq!(
            graph.edges[1].path.points.last(),
            Some(&LayoutPoint {
                x: NODE_WIDTH + NODE_GAP,
                y: FIELD_HEADER_HEIGHT + FIELD_ROW_HEIGHT / 2.0,
            })
        );
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
            connection_paths: Vec::new(),
            field_edges: vec![
                FieldEdge {
                    from_node: 0,
                    from_field: "zeta".to_string(),
                    to_node: 1,
                    to_field: "zeta".to_string(),
                    kind: FieldEdgeKind::Passthrough,
                    ..Default::default()
                },
                FieldEdge {
                    from_node: 0,
                    from_field: "alpha".to_string(),
                    to_node: 1,
                    to_field: "alpha".to_string(),
                    kind: FieldEdgeKind::Passthrough,
                    ..Default::default()
                },
            ],
            field_edge_paths: Vec::new(),
            role_edges: Vec::new(),
            role_edge_paths: Vec::new(),
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
            connection_paths: Vec::new(),
            field_edges: Vec::new(),
            field_edge_paths: Vec::new(),
            role_edges: Vec::new(),
            role_edge_paths: Vec::new(),
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

    #[test]
    fn current_layout_request_preserves_existing_view() {
        let view = PipelineView {
            stages: vec![stage("a"), stage("b")],
            connections: vec![Connection::plain(0, 1)],
            connection_paths: Vec::new(),
            field_edges: Vec::new(),
            field_edge_paths: Vec::new(),
            role_edges: Vec::new(),
            role_edge_paths: Vec::new(),
        };

        let result = apply_canvas_layout(view.clone(), CanvasLayoutEngine::CurrentBarycenter);

        assert_eq!(result.requested, CanvasLayoutEngine::CurrentBarycenter);
        assert_eq!(result.applied, CanvasLayoutEngine::CurrentBarycenter);
        assert_eq!(result.fallback, None);
        assert_eq!(result.view, view);
    }

    #[test]
    fn port_aware_layout_preview_preserves_renderer_edges() {
        let view = PipelineView {
            stages: vec![stage("a"), stage("b"), stage("c")],
            connections: vec![Connection::plain(0, 1), Connection::plain(1, 2)],
            connection_paths: Vec::new(),
            field_edges: Vec::new(),
            field_edge_paths: Vec::new(),
            role_edges: Vec::new(),
            role_edge_paths: Vec::new(),
        };
        let current_positions = view
            .stages
            .iter()
            .map(|stage| (stage.canvas_x, stage.canvas_y))
            .collect::<Vec<_>>();

        let result = apply_canvas_layout(view.clone(), CanvasLayoutEngine::PortAwareSugiyama);

        assert_eq!(result.requested, CanvasLayoutEngine::PortAwareSugiyama);
        assert_eq!(result.applied, CanvasLayoutEngine::PortAwareSugiyama);
        assert_eq!(result.fallback, None);
        assert_eq!(result.view.connections, view.connections);
        assert_eq!(result.view.field_edges, view.field_edges);
        assert_eq!(result.view.connection_paths.len(), view.connections.len());
        assert!(result.view.field_edge_paths.is_empty());
        assert_eq!(
            result
                .view
                .stages
                .iter()
                .map(|stage| stage.id.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b", "c"]
        );
        assert_ne!(
            result
                .view
                .stages
                .iter()
                .map(|stage| (stage.canvas_x, stage.canvas_y))
                .collect::<Vec<_>>(),
            current_positions
        );
        assert!(
            result.view.stages[0].canvas_x < result.view.stages[1].canvas_x
                && result.view.stages[1].canvas_x < result.view.stages[2].canvas_x
        );
        assert_eq!(
            result.view.connection_paths[0].points.first(),
            Some(&CanvasPoint {
                x: result.view.stages[0].port_out().0,
                y: result.view.stages[0].port_out().1,
            })
        );
        assert_eq!(
            result.view.connection_paths[0].points.last(),
            Some(&CanvasPoint {
                x: result.view.stages[1].port_in().0,
                y: result.view.stages[1].port_in().1,
            })
        );
    }

    #[test]
    fn port_aware_layout_preserves_distinct_branch_connector_paths() {
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
                stage("gold_out"),
                stage("silver_out"),
                stage("standard_out"),
            ],
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
                Connection {
                    from: 0,
                    to: 3,
                    from_branch: Some(2),
                },
            ],
            connection_paths: Vec::new(),
            field_edges: Vec::new(),
            field_edge_paths: Vec::new(),
            role_edges: Vec::new(),
            role_edge_paths: Vec::new(),
        };

        let result = apply_canvas_layout(view, CanvasLayoutEngine::PortAwareSugiyama);

        assert_eq!(result.applied, CanvasLayoutEngine::PortAwareSugiyama);
        assert_eq!(result.view.connection_paths.len(), 3);
        let lane_xs = result
            .view
            .connection_paths
            .iter()
            .map(|path| path.points.get(1).map(|point| point.x))
            .collect::<Vec<_>>();
        assert_eq!(lane_xs.len(), 3);
        assert_ne!(lane_xs[0], lane_xs[1]);
        assert_ne!(lane_xs[1], lane_xs[2]);
        assert_eq!(
            result.view.connection_paths[0].points.first(),
            Some(&CanvasPoint {
                x: result.view.stages[0].branch_anchor_out(0).0,
                y: result.view.stages[0].branch_anchor_out(0).1,
            })
        );
    }

    #[test]
    fn port_aware_layout_exports_field_edge_paths_parallel_to_field_edges() {
        let mut source = stage("source");
        source.fields = vec![field("id"), field("total")];
        let mut sink = stage("sink");
        sink.fields = vec![field("id"), field("total")];
        let view = PipelineView {
            stages: vec![source, sink],
            connections: Vec::new(),
            connection_paths: Vec::new(),
            field_edges: vec![
                FieldEdge {
                    from_node: 0,
                    from_field: "id".to_string(),
                    to_node: 1,
                    to_field: "id".to_string(),
                    kind: FieldEdgeKind::Passthrough,
                    ..Default::default()
                },
                FieldEdge {
                    from_node: 0,
                    from_field: "total".to_string(),
                    to_node: 1,
                    to_field: "total".to_string(),
                    kind: FieldEdgeKind::Derive,
                    ..Default::default()
                },
            ],
            field_edge_paths: Vec::new(),
            role_edges: Vec::new(),
            role_edge_paths: Vec::new(),
        };

        let result = apply_canvas_layout(view, CanvasLayoutEngine::PortAwareSugiyama);

        assert_eq!(result.applied, CanvasLayoutEngine::PortAwareSugiyama);
        assert!(result.view.connection_paths.is_empty());
        assert_eq!(
            result.view.field_edge_paths.len(),
            result.view.field_edges.len()
        );
        assert_eq!(
            result.view.field_edge_paths[1].points.first(),
            Some(&CanvasPoint {
                x: result.view.stages[0].field_anchor_out(1).0,
                y: result.view.stages[0].field_anchor_out(1).1,
            })
        );
        assert_eq!(
            result.view.field_edge_paths[1].points.last(),
            Some(&CanvasPoint {
                x: result.view.stages[1].field_anchor_in(1).0,
                y: result.view.stages[1].field_anchor_in(1).1,
            })
        );
    }

    #[test]
    fn aggregate_group_key_role_port_is_routed_as_input_port() {
        let mut source = stage("source");
        source.fields = vec![field("user_id")];
        let mut aggregate = stage("aggregate");
        aggregate.kind = StageKind::Aggregate;
        aggregate.role_ports = vec![StagePortRow {
            id: "group_by:user_id".to_string(),
            label: "user_id".to_string(),
            role: "group_by".to_string(),
            kind: StagePortKind::AggregateGroupKey,
            side: StagePortSide::Input,
        }];
        aggregate.fields = vec![field("user_id")];
        let view = PipelineView {
            stages: vec![source, aggregate],
            connections: Vec::new(),
            connection_paths: Vec::new(),
            field_edges: Vec::new(),
            field_edge_paths: Vec::new(),
            role_edges: vec![RoleEdge {
                from_node: 0,
                from_field: "user_id".to_string(),
                to_node: 1,
                to_port: "group_by:user_id".to_string(),
                kind: FieldEdgeKind::Derive,
            }],
            role_edge_paths: Vec::new(),
        };

        let graph = LayoutGraph::from_pipeline_view(&view);

        assert_eq!(
            input_port_ids(&graph.nodes[1]),
            vec!["node:in", "role:in:group_by:user_id", "field:in:user_id"]
        );
        assert_eq!(graph.edges[0].kind, LayoutEdgeKind::AggregateGroupKey);
        assert_eq!(graph.edges[0].to_port, "role:in:group_by:user_id");
        assert_eq!(
            graph.edges[0].path.points.last(),
            Some(&LayoutPoint {
                x: NODE_WIDTH + NODE_GAP,
                y: FIELD_HEADER_HEIGHT + FIELD_ROW_HEIGHT + FIELD_ROW_HEIGHT / 2.0,
            })
        );

        let result = apply_canvas_layout(view, CanvasLayoutEngine::PortAwareSugiyama);

        assert_eq!(result.applied, CanvasLayoutEngine::PortAwareSugiyama);
        assert_eq!(result.view.role_edge_paths.len(), 1);
        assert_eq!(
            result.view.role_edge_paths[0].points.last(),
            Some(&CanvasPoint {
                x: result.view.stages[1].role_port_anchor_in(0).0,
                y: result.view.stages[1].role_port_anchor_in(0).1,
            })
        );
    }

    #[test]
    fn order_fulfillment_benchmark_shortens_source_edges() {
        assert_benchmark_targets(
            include_str!("../../../../examples/pipelines/order_fulfillment.yaml"),
            2,
            1,
            0,
        );
    }

    #[test]
    fn source_reuse_benchmark_shortens_reused_late_sources() {
        let yaml =
            include_str!("../../../../examples/pipelines/layout_benchmark_source_reuse.yaml");
        assert_benchmark_targets(yaml, 7, 2, 2);

        let graph = fixture_graph(yaml);
        assert_eq!(rank_of(&graph, "orders"), 0);
        assert_eq!(rank_of(&graph, "products"), 2);
        assert_eq!(rank_of(&graph, "audit_events"), 6);
    }

    #[test]
    fn order_lifecycle_benchmark_shortens_late_source_fan_in() {
        let yaml =
            include_str!("../../../../examples/pipelines/layout_benchmark_order_lifecycle.yaml");
        assert_benchmark_targets(yaml, 8, 1, 8);

        let graph = fixture_graph(yaml);
        assert_eq!(rank_of(&graph, "orders"), 0);
        assert_eq!(rank_of(&graph, "products"), 4);
        assert_eq!(rank_of(&graph, "audit_events"), 7);
    }

    #[test]
    fn port_aware_layout_falls_back_for_missing_branch_anchor() {
        let mut route = stage("route");
        route.kind = StageKind::Route;
        let view = PipelineView {
            stages: vec![route, stage("sink")],
            connections: vec![Connection {
                from: 0,
                to: 1,
                from_branch: Some(0),
            }],
            connection_paths: Vec::new(),
            field_edges: Vec::new(),
            field_edge_paths: Vec::new(),
            role_edges: Vec::new(),
            role_edge_paths: Vec::new(),
        };

        let result = apply_canvas_layout(view.clone(), CanvasLayoutEngine::PortAwareSugiyama);

        assert_eq!(result.applied, CanvasLayoutEngine::CurrentBarycenter);
        assert_eq!(
            result.fallback,
            Some(CanvasLayoutFallback::MissingBranch {
                edge_index: 0,
                stage_index: 0,
                branch_index: 0,
            })
        );
        assert_eq!(result.view, view);
    }

    #[test]
    fn port_aware_layout_falls_back_for_missing_field_anchor() {
        let mut sink = stage("sink");
        sink.fields = vec![field("present")];
        let view = PipelineView {
            stages: vec![stage("source"), sink],
            connections: Vec::new(),
            connection_paths: Vec::new(),
            field_edges: vec![FieldEdge {
                from_node: 0,
                from_field: "missing".to_string(),
                to_node: 1,
                to_field: "present".to_string(),
                kind: FieldEdgeKind::Passthrough,
                ..Default::default()
            }],
            field_edge_paths: Vec::new(),
            role_edges: Vec::new(),
            role_edge_paths: Vec::new(),
        };

        let result = apply_canvas_layout(view.clone(), CanvasLayoutEngine::PortAwareSugiyama);

        assert_eq!(result.applied, CanvasLayoutEngine::CurrentBarycenter);
        assert_eq!(
            result.fallback,
            Some(CanvasLayoutFallback::MissingPort {
                edge_index: 0,
                stage_index: 0,
                side: LayoutPortSide::Output,
                port_id: "field:out:missing".to_string(),
            })
        );
        assert_eq!(result.view, view);
    }

    /// #147: the INDIRECT influence kinds Filter/JoinKey/Conditional map to the
    /// zero-weight [`LayoutEdgeKind::IndirectField`] overlay (so they route a
    /// cable without distorting node ranks), while GroupBy — which replaced the
    /// group-key `Derive` one-for-one — maps to the WEIGHTED [`LayoutEdgeKind::Field`]
    /// despite `nature() == Indirect`. Asserts both the kind mapping (via the
    /// `field_edge` constructor) and the rank weight, so it fails if a future
    /// refactor keys the layout weight off `nature()` and silently zeroes GroupBy.
    #[test]
    fn indirect_field_edges_are_zero_weight_overlays_but_group_by_keeps_field_weight() {
        let make = |kind: FieldEdgeKind| {
            field_edge(&FieldEdge {
                from_node: 0,
                from_field: "k".to_string(),
                to_node: 1,
                to_field: "k".to_string(),
                kind,
                ..Default::default()
            })
            .kind
        };

        for kind in [
            FieldEdgeKind::Filter,
            FieldEdgeKind::JoinKey,
            FieldEdgeKind::Conditional,
        ] {
            assert_eq!(
                make(kind),
                LayoutEdgeKind::IndirectField,
                "{kind:?} must map to the zero-weight IndirectField overlay"
            );
        }
        assert_eq!(
            edge_rank_weight(LayoutEdgeKind::IndirectField),
            0,
            "an IndirectField overlay must carry zero rank weight"
        );

        // GroupBy stays a weighted Field edge even though it is INDIRECT by
        // nature — it replaced the group-key Derive one-for-one and must keep its
        // prior layout weight so tuned layouts do not shift.
        assert_eq!(
            make(FieldEdgeKind::GroupBy),
            LayoutEdgeKind::Field,
            "GroupBy must keep the weighted Field kind"
        );
        assert_eq!(
            FieldEdgeKind::GroupBy.nature(),
            EdgeNature::Indirect,
            "GroupBy IS indirect by nature — the weight divergence is intentional"
        );
        assert!(
            edge_rank_weight(LayoutEdgeKind::Field) > 0,
            "the Field kind GroupBy keeps must carry a non-zero rank weight"
        );
    }
}
