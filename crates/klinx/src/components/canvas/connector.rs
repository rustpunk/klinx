use std::collections::BTreeMap;

use dioxus::prelude::*;

use crate::pipeline_view::{
    CanvasConnectorPath, CanvasPoint, EdgeNature, FieldEdgeKind, StageView,
};

const CHANNEL_LANE_SPACING: f32 = 14.0;
const CHANNEL_NODE_MARGIN: f32 = 18.0;
const CHANNEL_OBSTACLE_CLEARANCE: f32 = 18.0;
const AXIS_EPSILON: f32 = 0.5;
const LANE_SPACING_EPSILON: f32 = 0.01;
const LANE_INTERSECTION_PENALTY: f32 = 10_000.0;
const ROUTE_LANE_DEVIATION_PENALTY: f32 = 0.03;
const NODE_CONNECTOR_STROKE_STYLE: &str = "stroke: var(--klinx-accent-transform);";

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ConnectorEndpoints {
    pub sx: f32,
    pub sy: f32,
    pub tx: f32,
    pub ty: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ConnectorObstacle {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[cfg(test)]
fn centered_channel_paths(endpoints: &[ConnectorEndpoints]) -> Vec<CanvasConnectorPath> {
    obstacle_aware_channel_paths(endpoints, &[])
}

pub(crate) fn obstacle_aware_channel_paths(
    endpoints: &[ConnectorEndpoints],
    obstacles: &[ConnectorObstacle],
) -> Vec<CanvasConnectorPath> {
    let mut lane_xs = vec![0.0; endpoints.len()];
    let mut groups: BTreeMap<(i32, i32), Vec<usize>> = BTreeMap::new();
    for (index, endpoint) in endpoints.iter().enumerate() {
        groups
            .entry(channel_key(*endpoint))
            .or_default()
            .push(index);
    }

    for indices in groups.values_mut() {
        indices.sort_by(|&a, &b| {
            endpoint_sort_key(endpoints[a]).total_cmp(&endpoint_sort_key(endpoints[b]))
        });
        let group_lane_xs = lane_xs_for_group(endpoints, indices, obstacles);
        for (&endpoint_index, lane_x) in indices.iter().zip(group_lane_xs) {
            lane_xs[endpoint_index] = lane_x;
        }
    }

    separate_independent_lane_collisions(endpoints, obstacles, &mut lane_xs);

    assign_obstacle_aware_paths(endpoints, obstacles, &lane_xs)
}

fn channel_key(endpoint: ConnectorEndpoints) -> (i32, i32) {
    (endpoint.sx.round() as i32, endpoint.tx.round() as i32)
}

fn endpoint_sort_key(endpoint: ConnectorEndpoints) -> f32 {
    (endpoint.sy + endpoint.ty) / 2.0
}

fn lane_xs_for_group(
    endpoints: &[ConnectorEndpoints],
    indices: &[usize],
    obstacles: &[ConnectorObstacle],
) -> Vec<f32> {
    let lane_count = indices.len();
    if lane_count == 0 {
        return Vec::new();
    }

    let base = LaneSpan::from_endpoints(endpoints, indices);
    let free_spans = free_lane_spans(base, obstacles);
    if lane_count > 1 && free_spans.len() > 1 {
        let group_center = base.center();
        if let Some(best_free) =
            best_lane_span_in_candidates(&free_spans, endpoints, indices, obstacles, group_center)
        {
            let best_free_lane_xs = lane_xs_in_span(best_free, lane_count);
            if span_supports_full_spacing(best_free, lane_count) {
                return best_free_lane_xs;
            }

            return spaced_lane_xs_across_spans(base, &free_spans, endpoints, indices, obstacles)
                .unwrap_or(best_free_lane_xs);
        }
    }

    let best = best_lane_span(endpoints, indices, obstacles);
    let best_lane_xs = lane_xs_in_span(best, lane_count);
    if lane_count <= 1 || span_supports_full_spacing(best, lane_count) {
        return best_lane_xs;
    }

    if free_spans.len() <= 1 {
        return best_lane_xs;
    }

    spaced_lane_xs_across_spans(base, &free_spans, endpoints, indices, obstacles)
        .unwrap_or(best_lane_xs)
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct LaneSpan {
    start: f32,
    end: f32,
}

impl LaneSpan {
    fn from_endpoints(endpoints: &[ConnectorEndpoints], indices: &[usize]) -> Self {
        let mut min_x = f32::MAX;
        let mut max_x = f32::MIN;
        for &index in indices {
            let endpoint = endpoints[index];
            min_x = min_x.min(endpoint.sx.min(endpoint.tx));
            max_x = max_x.max(endpoint.sx.max(endpoint.tx));
        }

        lane_span_from_bounds(min_x, max_x)
    }

    fn from_endpoint(endpoint: ConnectorEndpoints) -> Self {
        lane_span_from_bounds(endpoint.sx.min(endpoint.tx), endpoint.sx.max(endpoint.tx))
    }

    fn center(self) -> f32 {
        (self.start + self.end) / 2.0
    }

    fn width(self) -> f32 {
        (self.end - self.start).max(0.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct SpanScore {
    intersections: usize,
    center_distance: f32,
    start: f32,
}

fn lane_span_from_bounds(min_x: f32, max_x: f32) -> LaneSpan {
    let center = (min_x + max_x) / 2.0;
    if max_x - min_x <= 2.0 * CHANNEL_NODE_MARGIN {
        return LaneSpan {
            start: center,
            end: center,
        };
    }

    LaneSpan {
        start: min_x + CHANNEL_NODE_MARGIN,
        end: max_x - CHANNEL_NODE_MARGIN,
    }
}

fn best_lane_span(
    endpoints: &[ConnectorEndpoints],
    indices: &[usize],
    obstacles: &[ConnectorObstacle],
) -> LaneSpan {
    let base = LaneSpan::from_endpoints(endpoints, indices);
    let group_center = base.center();
    let mut best = base;
    let best_score = score_lane_span(base, endpoints, indices, obstacles, group_center);

    if let Some(candidate) = best_lane_span_in_candidates(
        &free_lane_spans(base, obstacles),
        endpoints,
        indices,
        obstacles,
        group_center,
    ) {
        let score = score_lane_span(candidate, endpoints, indices, obstacles, group_center);
        if better_span_score(score, best_score) {
            best = candidate;
        }
    }

    best
}

fn best_lane_span_in_candidates(
    candidates: &[LaneSpan],
    endpoints: &[ConnectorEndpoints],
    indices: &[usize],
    obstacles: &[ConnectorObstacle],
    group_center: f32,
) -> Option<LaneSpan> {
    candidates.iter().copied().min_by(|a, b| {
        let a_score = score_lane_span(*a, endpoints, indices, obstacles, group_center);
        let b_score = score_lane_span(*b, endpoints, indices, obstacles, group_center);
        if better_span_score(a_score, b_score) {
            std::cmp::Ordering::Less
        } else if better_span_score(b_score, a_score) {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    })
}

fn free_lane_spans(base: LaneSpan, obstacles: &[ConnectorObstacle]) -> Vec<LaneSpan> {
    let mut blockers = obstacles
        .iter()
        .filter(|obstacle| obstacle.is_valid())
        .filter_map(|obstacle| {
            let start = (obstacle.left() - CHANNEL_OBSTACLE_CLEARANCE).max(base.start);
            let end = (obstacle.right() + CHANNEL_OBSTACLE_CLEARANCE).min(base.end);
            (start < end).then_some((start, end))
        })
        .collect::<Vec<_>>();
    blockers.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.total_cmp(&b.1)));

    let mut spans = Vec::new();
    let mut cursor = base.start;
    for (block_start, block_end) in blockers {
        if cursor < block_start {
            spans.push(LaneSpan {
                start: cursor,
                end: block_start,
            });
        }
        cursor = cursor.max(block_end);
    }
    if cursor < base.end {
        spans.push(LaneSpan {
            start: cursor,
            end: base.end,
        });
    }

    spans
}

fn score_lane_span(
    span: LaneSpan,
    endpoints: &[ConnectorEndpoints],
    indices: &[usize],
    obstacles: &[ConnectorObstacle],
    group_center: f32,
) -> SpanScore {
    let lane_count = indices.len();
    let intersections = indices
        .iter()
        .enumerate()
        .map(|(lane_index, &endpoint_index)| {
            let lane_x = lane_x_in_span(span, lane_index, lane_count);
            let points = path_points_for_lane(endpoints[endpoint_index], lane_x);
            path_intersection_count(&points, obstacles)
        })
        .sum();

    SpanScore {
        intersections,
        center_distance: (span.center() - group_center).abs(),
        start: span.start,
    }
}

fn better_span_score(candidate: SpanScore, current: SpanScore) -> bool {
    let distance_order = candidate
        .center_distance
        .total_cmp(&current.center_distance);
    candidate.intersections < current.intersections
        || (candidate.intersections == current.intersections && distance_order.is_lt())
        || (candidate.intersections == current.intersections
            && distance_order.is_eq()
            && candidate.start.total_cmp(&current.start).is_lt())
}

fn lane_x_in_span(span: LaneSpan, lane_index: usize, lane_count: usize) -> f32 {
    let center = span.center();
    if lane_count <= 1 {
        return center;
    }

    let spacing = CHANNEL_LANE_SPACING.min(span.width() / (lane_count - 1) as f32);
    if spacing <= 0.0 {
        return center;
    }

    let centered_index = lane_index as f32 - (lane_count - 1) as f32 / 2.0;
    (center + centered_index * spacing).clamp(span.start, span.end)
}

fn lane_xs_in_span(span: LaneSpan, lane_count: usize) -> Vec<f32> {
    (0..lane_count)
        .map(|lane_index| lane_x_in_span(span, lane_index, lane_count))
        .collect()
}

fn span_supports_full_spacing(span: LaneSpan, lane_count: usize) -> bool {
    lane_count <= 1
        || span.width() + LANE_SPACING_EPSILON >= CHANNEL_LANE_SPACING * (lane_count - 1) as f32
}

fn spaced_lane_xs_across_spans(
    base: LaneSpan,
    free_spans: &[LaneSpan],
    endpoints: &[ConnectorEndpoints],
    indices: &[usize],
    obstacles: &[ConnectorObstacle],
) -> Option<Vec<f32>> {
    let lane_count = indices.len();
    let capacity = free_spans
        .iter()
        .copied()
        .map(full_spacing_lane_capacity)
        .sum::<usize>();
    if capacity < lane_count {
        return None;
    }

    let candidates = lane_candidates_from_spans(free_spans);
    if candidates.len() < lane_count {
        return None;
    }

    let desired_lane_xs = lane_xs_in_span(base, lane_count);
    select_spaced_lane_xs(&candidates, &desired_lane_xs, endpoints, indices, obstacles)
}

fn lane_candidates_from_spans(spans: &[LaneSpan]) -> Vec<f32> {
    let mut candidates = Vec::new();
    for span in spans.iter().copied() {
        let capacity = full_spacing_lane_capacity(span);
        for lane_index in 0..capacity {
            candidates.push(lane_x_in_span(span, lane_index, capacity));
        }
    }

    candidates.sort_by(f32::total_cmp);
    candidates.dedup_by(|a, b| (*a - *b).abs() < AXIS_EPSILON);
    candidates
}

fn full_spacing_lane_capacity(span: LaneSpan) -> usize {
    if !span.start.is_finite() || !span.end.is_finite() || span.end < span.start {
        return 0;
    }

    (span.width() / CHANNEL_LANE_SPACING).floor() as usize + 1
}

fn select_spaced_lane_xs(
    candidates: &[f32],
    desired_lane_xs: &[f32],
    endpoints: &[ConnectorEndpoints],
    indices: &[usize],
    obstacles: &[ConnectorObstacle],
) -> Option<Vec<f32>> {
    let lane_count = desired_lane_xs.len();
    let candidate_count = candidates.len();
    if lane_count == 0 {
        return Some(Vec::new());
    }

    let prev_limits = previous_spaced_candidate_limits(candidates);
    let assignment_costs =
        lane_assignment_costs(candidates, desired_lane_xs, endpoints, indices, obstacles);
    let width = lane_count + 1;
    let table_index =
        |candidate_prefix: usize, lane_prefix: usize| candidate_prefix * width + lane_prefix;
    let mut costs = vec![f32::INFINITY; (candidate_count + 1) * width];
    let mut take = vec![false; (candidate_count + 1) * width];

    for candidate_prefix in 0..=candidate_count {
        costs[table_index(candidate_prefix, 0)] = 0.0;
    }

    for candidate_prefix in 1..=candidate_count {
        for lane_prefix in 1..=lane_count {
            let skip_cost = costs[table_index(candidate_prefix - 1, lane_prefix)];
            let candidate_index = candidate_prefix - 1;
            let prev_prefix = prev_limits[candidate_index];
            let take_cost = costs[table_index(prev_prefix, lane_prefix - 1)]
                + assignment_costs[lane_prefix - 1][candidate_index];

            if take_cost < skip_cost {
                costs[table_index(candidate_prefix, lane_prefix)] = take_cost;
                take[table_index(candidate_prefix, lane_prefix)] = true;
            } else {
                costs[table_index(candidate_prefix, lane_prefix)] = skip_cost;
            }
        }
    }

    if !costs[table_index(candidate_count, lane_count)].is_finite() {
        return None;
    }

    let mut selected = Vec::with_capacity(lane_count);
    let mut candidate_prefix = candidate_count;
    let mut lane_prefix = lane_count;
    while lane_prefix > 0 && candidate_prefix > 0 {
        if take[table_index(candidate_prefix, lane_prefix)] {
            let candidate_index = candidate_prefix - 1;
            selected.push(candidates[candidate_index]);
            candidate_prefix = prev_limits[candidate_index];
            lane_prefix -= 1;
        } else {
            candidate_prefix -= 1;
        }
    }

    if lane_prefix > 0 {
        return None;
    }

    selected.reverse();
    Some(selected)
}

fn previous_spaced_candidate_limits(candidates: &[f32]) -> Vec<usize> {
    let mut limits = Vec::with_capacity(candidates.len());
    let mut next = 0;
    for (index, &candidate) in candidates.iter().enumerate() {
        while next < index
            && candidates[next] <= candidate - CHANNEL_LANE_SPACING + LANE_SPACING_EPSILON
        {
            next += 1;
        }
        limits.push(next);
    }
    limits
}

fn lane_assignment_costs(
    candidates: &[f32],
    desired_lane_xs: &[f32],
    endpoints: &[ConnectorEndpoints],
    indices: &[usize],
    obstacles: &[ConnectorObstacle],
) -> Vec<Vec<f32>> {
    desired_lane_xs
        .iter()
        .enumerate()
        .map(|(lane_index, desired_lane_x)| {
            let endpoint = endpoints[indices[lane_index]];
            candidates
                .iter()
                .map(|&lane_x| {
                    let points = route_points_for_lane(endpoint, lane_x, obstacles);
                    let intersections = path_intersection_count(&points, obstacles) as f32;
                    intersections * LANE_INTERSECTION_PENALTY + (lane_x - desired_lane_x).abs()
                })
                .collect()
        })
        .collect()
}

fn separate_independent_lane_collisions(
    endpoints: &[ConnectorEndpoints],
    obstacles: &[ConnectorObstacle],
    lane_xs: &mut [f32],
) {
    let mut reserved = Vec::<usize>::new();
    let mut indices = (0..endpoints.len()).collect::<Vec<_>>();
    indices.sort_by(|&a, &b| {
        let a_span = LaneSpan::from_endpoint(endpoints[a]);
        let b_span = LaneSpan::from_endpoint(endpoints[b]);
        a_span.start.total_cmp(&b_span.start).then_with(|| {
            endpoint_sort_key(endpoints[a]).total_cmp(&endpoint_sort_key(endpoints[b]))
        })
    });

    for index in indices {
        if lane_conflicts_with_reserved(index, lane_xs[index], endpoints, lane_xs, &reserved)
            && let Some(lane_x) = alternate_lane_x(
                index,
                endpoints,
                obstacles,
                lane_xs[index],
                lane_xs,
                &reserved,
            )
        {
            lane_xs[index] = lane_x;
        }
        reserved.push(index);
    }
}

fn alternate_lane_x(
    index: usize,
    endpoints: &[ConnectorEndpoints],
    obstacles: &[ConnectorObstacle],
    current_lane_x: f32,
    lane_xs: &[f32],
    reserved: &[usize],
) -> Option<f32> {
    let endpoint = endpoints[index];
    let base = LaneSpan::from_endpoint(endpoint);
    let free_spans = free_lane_spans(base, obstacles);
    let spans = if free_spans.is_empty() {
        vec![base]
    } else {
        free_spans
    };
    let mut candidates = lane_candidates_from_spans(&spans);
    candidates.push(base.center());
    candidates.push(current_lane_x);
    candidates.sort_by(f32::total_cmp);
    candidates.dedup_by(|a, b| (*a - *b).abs() < AXIS_EPSILON);
    candidates.sort_by(|a, b| {
        lane_candidate_cost(endpoint, *a, current_lane_x, obstacles).total_cmp(
            &lane_candidate_cost(endpoint, *b, current_lane_x, obstacles),
        )
    });

    candidates.into_iter().find(|&candidate| {
        !lane_conflicts_with_reserved(index, candidate, endpoints, lane_xs, reserved)
    })
}

fn lane_candidate_cost(
    endpoint: ConnectorEndpoints,
    lane_x: f32,
    current_lane_x: f32,
    obstacles: &[ConnectorObstacle],
) -> f32 {
    let points = route_points_for_lane(endpoint, lane_x, obstacles);
    let intersections = path_intersection_count(&points, obstacles) as f32;
    let base = LaneSpan::from_endpoint(endpoint);
    intersections * LANE_INTERSECTION_PENALTY
        + (lane_x - current_lane_x).abs()
        + (lane_x - base.center()).abs() * 0.1
}

fn lane_conflicts_with_reserved(
    index: usize,
    lane_x: f32,
    endpoints: &[ConnectorEndpoints],
    lane_xs: &[f32],
    reserved: &[usize],
) -> bool {
    reserved.iter().any(|&reserved_index| {
        independent_lane_collision(
            endpoints[index],
            lane_x,
            endpoints[reserved_index],
            lane_xs[reserved_index],
        )
    })
}

fn independent_lane_collision(
    endpoint: ConnectorEndpoints,
    lane_x: f32,
    other: ConnectorEndpoints,
    other_lane_x: f32,
) -> bool {
    !endpoints_share_endpoint(endpoint, other)
        && (lane_x - other_lane_x).abs() < CHANNEL_LANE_SPACING - LANE_SPACING_EPSILON
        && vertical_lane_ranges_overlap(endpoint, other)
}

fn vertical_lane_ranges_overlap(a: ConnectorEndpoints, b: ConnectorEndpoints) -> bool {
    open_ranges_overlap(
        a.sy.min(a.ty),
        a.sy.max(a.ty),
        b.sy.min(b.ty),
        b.sy.max(b.ty),
    )
}

fn path_points_for_lane(endpoint: ConnectorEndpoints, lane_x: f32) -> Vec<CanvasPoint> {
    compact_points(&[
        CanvasPoint {
            x: endpoint.sx,
            y: endpoint.sy,
        },
        CanvasPoint {
            x: lane_x,
            y: endpoint.sy,
        },
        CanvasPoint {
            x: lane_x,
            y: endpoint.ty,
        },
        CanvasPoint {
            x: endpoint.tx,
            y: endpoint.ty,
        },
    ])
}

#[derive(Clone, Debug)]
struct ReservedPath {
    endpoint: ConnectorEndpoints,
    points: Vec<CanvasPoint>,
}

fn assign_obstacle_aware_paths(
    endpoints: &[ConnectorEndpoints],
    obstacles: &[ConnectorObstacle],
    lane_xs: &[f32],
) -> Vec<CanvasConnectorPath> {
    let mut paths = vec![CanvasConnectorPath::default(); endpoints.len()];
    let mut reserved = Vec::<ReservedPath>::new();
    let mut indices = (0..endpoints.len()).collect::<Vec<_>>();
    indices.sort_by(|&a, &b| {
        endpoint_sort_key(endpoints[a]).total_cmp(&endpoint_sort_key(endpoints[b]))
    });

    for index in indices {
        let points =
            route_points_for_lane_avoiding(endpoints[index], lane_xs[index], obstacles, &reserved);
        reserved.push(ReservedPath {
            endpoint: endpoints[index],
            points: points.clone(),
        });
        paths[index] = CanvasConnectorPath { points };
    }

    paths
}

fn route_points_for_lane(
    endpoint: ConnectorEndpoints,
    lane_x: f32,
    obstacles: &[ConnectorObstacle],
) -> Vec<CanvasPoint> {
    route_points_for_lane_avoiding(endpoint, lane_x, obstacles, &[])
}

fn route_points_for_lane_avoiding(
    endpoint: ConnectorEndpoints,
    lane_x: f32,
    obstacles: &[ConnectorObstacle],
    reserved: &[ReservedPath],
) -> Vec<CanvasPoint> {
    let simple = path_points_for_lane(endpoint, lane_x);
    if path_intersection_count(&simple, obstacles) == 0
        && !path_conflicts_with_reserved(endpoint, &simple, reserved)
    {
        return simple;
    }

    if let Some(points) = free_space_route_points(endpoint, lane_x, obstacles, reserved)
        .filter(|points| route_is_valid(endpoint, points, obstacles, reserved))
    {
        return points;
    }

    exterior_detour_points(endpoint, lane_x, obstacles, reserved)
        .unwrap_or_else(|| unroutable_path(endpoint))
}

fn free_space_route_points(
    endpoint: ConnectorEndpoints,
    lane_x: f32,
    obstacles: &[ConnectorObstacle],
    reserved: &[ReservedPath],
) -> Option<Vec<CanvasPoint>> {
    let (xs, ys) = route_grid_axes(endpoint, lane_x, obstacles, reserved);
    let start = grid_index_for_point(&xs, &ys, endpoint.sx, endpoint.sy)?;
    let end = grid_index_for_point(&xs, &ys, endpoint.tx, endpoint.ty)?;
    let lane_x_index = xs.iter().position(|x| (*x - lane_x).abs() < AXIS_EPSILON)?;
    let (start_costs, start_previous) =
        grid_shortest_paths(&xs, &ys, start, endpoint, lane_x, obstacles, reserved);
    let (end_costs, end_previous) =
        grid_shortest_paths(&xs, &ys, end, endpoint, lane_x, obstacles, reserved);

    let mid_y = (endpoint.sy + endpoint.ty) / 2.0;
    let best_lane_waypoint = ys
        .iter()
        .enumerate()
        .filter_map(|(y_index, &y)| {
            let index = grid_index(lane_x_index, y_index, xs.len());
            let cost = start_costs[index] + end_costs[index] + (y - mid_y).abs() * 0.01;
            cost.is_finite().then_some((index, cost))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(index, _)| index);

    if let Some(waypoint) = best_lane_waypoint {
        let mut first = reconstruct_grid_path(&xs, &ys, &start_previous, start, waypoint)?;
        let mut second = reconstruct_grid_path(&xs, &ys, &end_previous, end, waypoint)?;
        second.reverse();
        first.extend(second.into_iter().skip(1));
        return Some(compact_orthogonal_points(&first));
    }

    reconstruct_grid_path(&xs, &ys, &start_previous, start, end)
        .map(|points| compact_orthogonal_points(&points))
}

fn route_grid_axes(
    endpoint: ConnectorEndpoints,
    lane_x: f32,
    obstacles: &[ConnectorObstacle],
    reserved: &[ReservedPath],
) -> (Vec<f32>, Vec<f32>) {
    let mut xs = vec![endpoint.sx, endpoint.tx, lane_x];
    let mut ys = vec![endpoint.sy, endpoint.ty];

    for obstacle in obstacles
        .iter()
        .copied()
        .filter(|obstacle| obstacle.is_valid())
    {
        xs.push(obstacle.left() - CHANNEL_OBSTACLE_CLEARANCE);
        xs.push(obstacle.right() + CHANNEL_OBSTACLE_CLEARANCE);
        ys.push(obstacle.top() - CHANNEL_OBSTACLE_CLEARANCE);
        ys.push(obstacle.bottom() + CHANNEL_OBSTACLE_CLEARANCE);
    }

    for reserved_path in reserved {
        for segment in reserved_path.points.windows(2) {
            let [from, to] = segment else { continue };
            if (from.x - to.x).abs() < AXIS_EPSILON {
                xs.push(from.x - CHANNEL_LANE_SPACING);
                xs.push(from.x + CHANNEL_LANE_SPACING);
            }
            if (from.y - to.y).abs() < AXIS_EPSILON {
                ys.push(from.y - CHANNEL_LANE_SPACING);
                ys.push(from.y + CHANNEL_LANE_SPACING);
            }
        }
    }

    sort_dedup_axis(&mut xs);
    sort_dedup_axis(&mut ys);
    (xs, ys)
}

fn sort_dedup_axis(axis: &mut Vec<f32>) {
    axis.retain(|value| value.is_finite());
    axis.sort_by(f32::total_cmp);
    axis.dedup_by(|a, b| (*a - *b).abs() < AXIS_EPSILON);
}

fn grid_shortest_paths(
    xs: &[f32],
    ys: &[f32],
    source: usize,
    endpoint: ConnectorEndpoints,
    lane_x: f32,
    obstacles: &[ConnectorObstacle],
    reserved: &[ReservedPath],
) -> (Vec<f32>, Vec<Option<usize>>) {
    let node_count = xs.len() * ys.len();
    let mut costs = vec![f32::INFINITY; node_count];
    let mut previous = vec![None; node_count];
    let mut visited = vec![false; node_count];
    costs[source] = 0.0;

    for _ in 0..node_count {
        let Some(current) = (0..node_count)
            .filter(|&index| !visited[index])
            .min_by(|&a, &b| costs[a].total_cmp(&costs[b]))
        else {
            break;
        };
        if !costs[current].is_finite() {
            break;
        }
        visited[current] = true;

        for next in grid_neighbors(current, xs.len(), ys.len()) {
            if visited[next] {
                continue;
            }
            let from = grid_point(current, xs, ys);
            let to = grid_point(next, xs, ys);
            if segment_intersects_any_obstacle(from, to, obstacles) {
                continue;
            }
            if segment_conflicts_with_reserved(endpoint, from, to, reserved) {
                continue;
            }
            let next_cost = costs[current] + route_segment_cost(from, to, lane_x);
            if next_cost < costs[next] {
                costs[next] = next_cost;
                previous[next] = Some(current);
            }
        }
    }

    (costs, previous)
}

fn grid_neighbors(index: usize, width: usize, height: usize) -> Vec<usize> {
    let x = index % width;
    let y = index / width;
    let mut neighbors = Vec::with_capacity(4);
    if x > 0 {
        neighbors.push(grid_index(x - 1, y, width));
    }
    if x + 1 < width {
        neighbors.push(grid_index(x + 1, y, width));
    }
    if y > 0 {
        neighbors.push(grid_index(x, y - 1, width));
    }
    if y + 1 < height {
        neighbors.push(grid_index(x, y + 1, width));
    }
    neighbors
}

fn route_segment_cost(from: CanvasPoint, to: CanvasPoint, lane_x: f32) -> f32 {
    let mut cost = segment_len(from, to);
    if (from.x - to.x).abs() < AXIS_EPSILON {
        cost += (from.x - lane_x).abs() * ROUTE_LANE_DEVIATION_PENALTY;
    }

    cost
}

fn exterior_detour_points(
    endpoint: ConnectorEndpoints,
    lane_x: f32,
    obstacles: &[ConnectorObstacle],
    reserved: &[ReservedPath],
) -> Option<Vec<CanvasPoint>> {
    let mut min_y = endpoint.sy.min(endpoint.ty);
    let mut max_y = endpoint.sy.max(endpoint.ty);
    for obstacle in obstacles
        .iter()
        .copied()
        .filter(|obstacle| obstacle.is_valid())
    {
        min_y = min_y.min(obstacle.top());
        max_y = max_y.max(obstacle.bottom());
    }
    for reserved_path in reserved {
        for point in &reserved_path.points {
            min_y = min_y.min(point.y);
            max_y = max_y.max(point.y);
        }
    }

    let clearance = CHANNEL_OBSTACLE_CLEARANCE + CHANNEL_LANE_SPACING;
    let candidates = [
        min_y - clearance,
        max_y + clearance,
        min_y - clearance * 2.0,
        max_y + clearance * 2.0,
    ];
    let flow_dir = if endpoint.tx >= endpoint.sx {
        1.0
    } else {
        -1.0
    };
    let source_stub_x = endpoint.sx + flow_dir * CHANNEL_OBSTACLE_CLEARANCE;
    let target_stub_x = endpoint.tx - flow_dir * CHANNEL_OBSTACLE_CLEARANCE;

    candidates
        .into_iter()
        .map(|detour_y| {
            compact_orthogonal_points(&[
                CanvasPoint {
                    x: endpoint.sx,
                    y: endpoint.sy,
                },
                CanvasPoint {
                    x: source_stub_x,
                    y: endpoint.sy,
                },
                CanvasPoint {
                    x: source_stub_x,
                    y: detour_y,
                },
                CanvasPoint {
                    x: lane_x,
                    y: detour_y,
                },
                CanvasPoint {
                    x: target_stub_x,
                    y: detour_y,
                },
                CanvasPoint {
                    x: target_stub_x,
                    y: endpoint.ty,
                },
                CanvasPoint {
                    x: endpoint.tx,
                    y: endpoint.ty,
                },
            ])
        })
        .filter(|points| route_is_valid(endpoint, points, obstacles, reserved))
        .min_by(|a, b| route_len(a).total_cmp(&route_len(b)))
}

fn route_is_valid(
    endpoint: ConnectorEndpoints,
    points: &[CanvasPoint],
    obstacles: &[ConnectorObstacle],
    reserved: &[ReservedPath],
) -> bool {
    path_intersection_count(points, obstacles) == 0
        && !path_conflicts_with_reserved(endpoint, points, reserved)
}

fn route_len(points: &[CanvasPoint]) -> f32 {
    points
        .windows(2)
        .map(|segment| segment_len(segment[0], segment[1]))
        .sum()
}

fn unroutable_path(endpoint: ConnectorEndpoints) -> Vec<CanvasPoint> {
    vec![
        CanvasPoint {
            x: endpoint.sx,
            y: endpoint.sy,
        },
        CanvasPoint {
            x: endpoint.sx,
            y: endpoint.sy,
        },
    ]
}

fn reconstruct_grid_path(
    xs: &[f32],
    ys: &[f32],
    previous: &[Option<usize>],
    source: usize,
    target: usize,
) -> Option<Vec<CanvasPoint>> {
    let mut route = vec![grid_point(target, xs, ys)];
    let mut current = target;
    while current != source {
        current = previous[current]?;
        route.push(grid_point(current, xs, ys));
    }
    route.reverse();
    Some(route)
}

fn grid_index_for_point(xs: &[f32], ys: &[f32], x: f32, y: f32) -> Option<usize> {
    let x_index = xs
        .iter()
        .position(|candidate| (*candidate - x).abs() < AXIS_EPSILON)?;
    let y_index = ys
        .iter()
        .position(|candidate| (*candidate - y).abs() < AXIS_EPSILON)?;
    Some(grid_index(x_index, y_index, xs.len()))
}

fn grid_index(x_index: usize, y_index: usize, width: usize) -> usize {
    y_index * width + x_index
}

fn grid_point(index: usize, xs: &[f32], ys: &[f32]) -> CanvasPoint {
    CanvasPoint {
        x: xs[index % xs.len()],
        y: ys[index / xs.len()],
    }
}

fn segment_intersects_any_obstacle(
    from: CanvasPoint,
    to: CanvasPoint,
    obstacles: &[ConnectorObstacle],
) -> bool {
    obstacles
        .iter()
        .any(|obstacle| segment_intersects_obstacle(from, to, obstacle))
}

fn path_intersection_count(points: &[CanvasPoint], obstacles: &[ConnectorObstacle]) -> usize {
    points
        .windows(2)
        .map(|segment| {
            obstacles
                .iter()
                .filter(|obstacle| segment_intersects_obstacle(segment[0], segment[1], obstacle))
                .count()
        })
        .sum()
}

#[cfg(test)]
fn path_intersects_obstacle(points: &[CanvasPoint], obstacle: &ConnectorObstacle) -> bool {
    points
        .windows(2)
        .any(|segment| segment_intersects_obstacle(segment[0], segment[1], obstacle))
}

fn path_conflicts_with_reserved(
    endpoint: ConnectorEndpoints,
    points: &[CanvasPoint],
    reserved: &[ReservedPath],
) -> bool {
    reserved.iter().any(|reserved_path| {
        !endpoints_share_endpoint(endpoint, reserved_path.endpoint)
            && paths_share_segment(points, &reserved_path.points)
    })
}

fn segment_conflicts_with_reserved(
    endpoint: ConnectorEndpoints,
    from: CanvasPoint,
    to: CanvasPoint,
    reserved: &[ReservedPath],
) -> bool {
    reserved.iter().any(|reserved_path| {
        !endpoints_share_endpoint(endpoint, reserved_path.endpoint)
            && reserved_path
                .points
                .windows(2)
                .any(|segment| segments_overlap(from, to, segment[0], segment[1]))
    })
}

#[cfg(test)]
fn paths_share_segment(a: &[CanvasPoint], b: &[CanvasPoint]) -> bool {
    paths_share_segment_impl(a, b)
}

#[cfg(not(test))]
fn paths_share_segment(a: &[CanvasPoint], b: &[CanvasPoint]) -> bool {
    paths_share_segment_impl(a, b)
}

fn paths_share_segment_impl(a: &[CanvasPoint], b: &[CanvasPoint]) -> bool {
    a.windows(2).any(|a_segment| {
        b.windows(2).any(|b_segment| {
            segments_overlap(a_segment[0], a_segment[1], b_segment[0], b_segment[1])
        })
    })
}

fn segments_overlap(
    a_from: CanvasPoint,
    a_to: CanvasPoint,
    b_from: CanvasPoint,
    b_to: CanvasPoint,
) -> bool {
    let a_horizontal = (a_from.y - a_to.y).abs() < AXIS_EPSILON;
    let b_horizontal = (b_from.y - b_to.y).abs() < AXIS_EPSILON;
    if a_horizontal && b_horizontal && (a_from.y - b_from.y).abs() < AXIS_EPSILON {
        return open_ranges_overlap(
            a_from.x.min(a_to.x),
            a_from.x.max(a_to.x),
            b_from.x.min(b_to.x),
            b_from.x.max(b_to.x),
        );
    }

    let a_vertical = (a_from.x - a_to.x).abs() < AXIS_EPSILON;
    let b_vertical = (b_from.x - b_to.x).abs() < AXIS_EPSILON;
    a_vertical
        && b_vertical
        && (a_from.x - b_from.x).abs() < AXIS_EPSILON
        && open_ranges_overlap(
            a_from.y.min(a_to.y),
            a_from.y.max(a_to.y),
            b_from.y.min(b_to.y),
            b_from.y.max(b_to.y),
        )
}

fn endpoints_share_endpoint(a: ConnectorEndpoints, b: ConnectorEndpoints) -> bool {
    points_nearly_equal(
        CanvasPoint { x: a.sx, y: a.sy },
        CanvasPoint { x: b.sx, y: b.sy },
    ) || points_nearly_equal(
        CanvasPoint { x: a.sx, y: a.sy },
        CanvasPoint { x: b.tx, y: b.ty },
    ) || points_nearly_equal(
        CanvasPoint { x: a.tx, y: a.ty },
        CanvasPoint { x: b.sx, y: b.sy },
    ) || points_nearly_equal(
        CanvasPoint { x: a.tx, y: a.ty },
        CanvasPoint { x: b.tx, y: b.ty },
    )
}

fn points_nearly_equal(a: CanvasPoint, b: CanvasPoint) -> bool {
    (a.x - b.x).abs() < AXIS_EPSILON && (a.y - b.y).abs() < AXIS_EPSILON
}

fn segment_intersects_obstacle(
    from: CanvasPoint,
    to: CanvasPoint,
    obstacle: &ConnectorObstacle,
) -> bool {
    if !obstacle.is_valid() {
        return false;
    }

    let seg_min_x = from.x.min(to.x);
    let seg_max_x = from.x.max(to.x);
    let seg_min_y = from.y.min(to.y);
    let seg_max_y = from.y.max(to.y);

    if (from.y - to.y).abs() < AXIS_EPSILON {
        let crosses_body = from.y > obstacle.top() && from.y < obstacle.bottom();
        let runs_on_border = (from.y - obstacle.top()).abs() < AXIS_EPSILON
            || (from.y - obstacle.bottom()).abs() < AXIS_EPSILON;
        return (crosses_body || runs_on_border)
            && open_ranges_overlap(seg_min_x, seg_max_x, obstacle.left(), obstacle.right());
    }

    if (from.x - to.x).abs() < AXIS_EPSILON {
        let crosses_body = from.x > obstacle.left() && from.x < obstacle.right();
        let runs_on_border = (from.x - obstacle.left()).abs() < AXIS_EPSILON
            || (from.x - obstacle.right()).abs() < AXIS_EPSILON;
        return (crosses_body || runs_on_border)
            && open_ranges_overlap(seg_min_y, seg_max_y, obstacle.top(), obstacle.bottom());
    }

    open_ranges_overlap(seg_min_x, seg_max_x, obstacle.left(), obstacle.right())
        && open_ranges_overlap(seg_min_y, seg_max_y, obstacle.top(), obstacle.bottom())
}

fn open_ranges_overlap(a_start: f32, a_end: f32, b_start: f32, b_end: f32) -> bool {
    a_start.max(b_start) < a_end.min(b_end)
}

impl ConnectorObstacle {
    fn is_valid(self) -> bool {
        self.width > 0.0 && self.height > 0.0
    }

    fn left(self) -> f32 {
        self.x
    }

    fn right(self) -> f32 {
        self.x + self.width
    }

    fn top(self) -> f32 {
        self.y
    }

    fn bottom(self) -> f32 {
        self.y + self.height
    }
}

/// Three-layer SVG connector between two adjacent pipeline stages.
///
/// Renders a single `<g>` element containing three `<path>` elements:
/// 1. Glow layer   — wide stroke at 10% opacity for a soft halo effect.
/// 2. Core cable   — dashed stroke at 70% opacity (8px dash, 4px gap).
/// 3. Bright centre — hairline solid stroke at 90% opacity (hot-wire effect).
///
///    Plus an open chevron arrowhead at the target port.
#[derive(Props, Clone, PartialEq)]
pub struct ConnectorProps {
    pub from: StageView,
    pub to: StageView,
    /// When the edge leaves a Route node, the index of the source branch in
    /// `from.branches` — the cable then anchors at that branch's output port
    /// rather than the shared node-level port. `None` for ordinary edges.
    pub from_branch: Option<usize>,
    /// Optional layout-provided route. When present, it preserves port-aware
    /// lane assignment; otherwise the renderer derives the classic midpoint
    /// orthogonal path from `from`/`to` anchors.
    pub path: Option<CanvasConnectorPath>,
}

#[component]
pub fn Connector(props: ConnectorProps) -> Element {
    // Source anchor: a Route-branch edge leaves the specific branch port; every
    // other edge leaves the node-level mid-height port. The target always enters
    // at the consumer's node-level port. This is the DEFAULT canvas view — one
    // cable per `(from, to)` connection.
    let (sx, sy) = match props.from_branch {
        Some(i) => props.from.branch_anchor_out(i),
        None => props.from.port_out(),
    };
    let (tx, ty) = props.to.port_in();
    let kind_attr = props.from.kind.kind_attr();

    rsx! {
        ConnectorPath {
            sx,
            sy,
            tx,
            ty,
            kind_attr: kind_attr.to_string(),
            extra_class: String::new(),
            routed_path: props.path.clone(),
            // Node-level DAG cables use one pipe colour; field cables keep the
            // CSS-class semantic colours for lineage kind.
            inline_accent_stroke: true,
        }
    }
}

/// Field-level connector between two explicit anchor points.
///
/// Used only on hover-reveal: a hover scope's edge set draws one of these per
/// participating [`crate::pipeline_view::FieldEdge`], from the producer row's
/// RIGHT anchor to the consumer row's LEFT anchor. `kind` drives the visual —
/// the three relationship flavours read as distinct stroke colours (#72).
#[derive(Props, Clone, PartialEq)]
pub struct FieldConnectorProps {
    pub start: (f32, f32),
    pub end: (f32, f32),
    /// `data-stage-kind` of the producer node, so the cable inherits its accent.
    pub kind_attr: String,
    /// The relationship the edge expresses ([`FieldEdgeKind`]) — selects the CSS
    /// class and therefore the stroke colour: pure carry, accessed carry, or
    /// derive.
    pub kind: FieldEdgeKind,
    /// Optional layout-provided route for the field edge.
    pub path: Option<CanvasConnectorPath>,
    /// Whether this edge is the current local focus inside a larger pinned
    /// lineage reveal.
    pub spotlight: bool,
}

#[component]
pub fn FieldConnector(props: FieldConnectorProps) -> Element {
    let (sx, sy) = props.start;
    let (tx, ty) = props.end;
    // Each relationship kind reads as a distinct hue: a pure pass-through is the
    // quietest, an accessed carry a warm highlight, a derive the active accent.
    // The four INDIRECT influence kinds (#147) additionally carry the
    // `--indirect` modifier — sourced from [`FieldEdgeKind::nature`] so the
    // value/influence split has a single source of truth — which the CSS renders
    // ghosted/dashed so influence reads differently from value.
    let kind_class = match props.kind {
        FieldEdgeKind::Passthrough => "klinx-field-edge--passthrough",
        FieldEdgeKind::Access => "klinx-field-edge--access",
        FieldEdgeKind::Derive => "klinx-field-edge--derive",
        FieldEdgeKind::Filter => "klinx-field-edge--filter",
        FieldEdgeKind::GroupBy => "klinx-field-edge--groupby",
        FieldEdgeKind::JoinKey => "klinx-field-edge--joinkey",
        FieldEdgeKind::Conditional => "klinx-field-edge--conditional",
    };
    let mut extra_class = format!("klinx-field-edge {kind_class}");
    if props.kind.nature() == EdgeNature::Indirect {
        extra_class.push_str(" klinx-field-edge--indirect");
    }
    if props.spotlight {
        extra_class.push_str(" klinx-field-edge--spotlight");
    }

    rsx! {
        ConnectorPath {
            sx,
            sy,
            tx,
            ty,
            kind_attr: props.kind_attr.clone(),
            extra_class,
            routed_path: props.path.clone(),
            // Field cables do NOT inline a stroke: the CSS classes
            // `.klinx-field-edge--derive` / `--access` / `--passthrough` own the
            // stroke COLOUR (set on the `<g>`, inherited by each path), so the
            // three kinds read as distinct hues — not just distinct opacity. An
            // inline stroke would override the class and erase that distinction.
            inline_accent_stroke: false,
        }
    }
}

/// Shared three-layer cable + chevron between two explicit world-space points.
///
/// Both [`Connector`] (node ports) and [`FieldConnector`] (field anchors) render
/// through this so the cable styling stays identical regardless of endpoint
/// source. `extra_class` lets the field path opt into hover/dim styling.
#[derive(Props, Clone, PartialEq)]
struct ConnectorPathProps {
    sx: f32,
    sy: f32,
    tx: f32,
    ty: f32,
    kind_attr: String,
    extra_class: String,
    routed_path: Option<CanvasConnectorPath>,
    /// Whether to inline the uniform node-level DAG stroke. Field connectors
    /// clear it so the `.klinx-field-edge--*` CSS classes own the stroke colour
    /// (inline styles outrank class rules, so the inline stroke must be ABSENT
    /// for the class colour to apply).
    inline_accent_stroke: bool,
}

#[component]
fn ConnectorPath(props: ConnectorPathProps) -> Element {
    let ConnectorPathProps {
        sx,
        sy,
        tx,
        ty,
        kind_attr,
        extra_class,
        routed_path,
        inline_accent_stroke,
    } = props;

    // Empty when the CSS class owns the stroke (field edges). Node-level DAG
    // pipes use one transform-orange stroke regardless of source node kind.
    let stroke_style = connector_stroke_style(inline_accent_stroke);

    let path = connector_path_data(sx, sy, tx, ty, routed_path.as_ref());

    // Open chevron arrowhead pointing right, positioned at target anchor.
    let arrow = format!(
        "M {:.1},{:.1} L {tx:.1},{ty:.1} L {:.1},{:.1}",
        tx - 8.0,
        ty - 5.0,
        tx - 8.0,
        ty + 5.0,
    );

    rsx! {
        g {
            "data-stage-kind": "{kind_attr}",
            class: "{extra_class}",
            // Layer 1 — glow
            path {
                d: "{path}",
                fill: "none",
                stroke_width: "5",
                stroke_opacity: "0.1",
                style: "{stroke_style}",
            }
            // Layer 2 — dashed core cable
            path {
                d: "{path}",
                fill: "none",
                stroke_width: "2",
                stroke_dasharray: "8 4",
                stroke_opacity: "0.7",
                style: "{stroke_style}",
            }
            // Layer 3 — bright centre hairline
            path {
                d: "{path}",
                fill: "none",
                stroke_width: "0.75",
                stroke_opacity: "0.9",
                style: "{stroke_style}",
            }
            // Open chevron arrowhead
            path {
                d: "{arrow}",
                fill: "none",
                stroke_width: "1.5",
                stroke_opacity: "0.8",
                stroke_linejoin: "round",
                stroke_linecap: "round",
                style: "{stroke_style}",
            }
        }
    }
}

fn connector_stroke_style(inline_accent_stroke: bool) -> &'static str {
    if inline_accent_stroke {
        NODE_CONNECTOR_STROKE_STYLE
    } else {
        ""
    }
}

fn connector_path_data(
    sx: f32,
    sy: f32,
    tx: f32,
    ty: f32,
    routed_path: Option<&CanvasConnectorPath>,
) -> String {
    match routed_path {
        Some(path) => reanchor_routed_points(&path.points, sx, sy, tx, ty)
            .as_deref()
            .and_then(rounded_orthogonal_polyline_path)
            .unwrap_or_else(|| degenerate_connector_path(sx, sy)),
        None => rounded_orthogonal_path(sx, sy, tx, ty),
    }
}

fn rounded_orthogonal_path(sx: f32, sy: f32, tx: f32, ty: f32) -> String {
    let dx = tx - sx;
    let dy = ty - sy;
    if dy.abs() < 0.5 {
        return format!("M {sx:.1},{sy:.1} L {tx:.1},{ty:.1}");
    }

    let mid_x = sx + dx / 2.0;
    let dir_x = if dx >= 0.0 { 1.0 } else { -1.0 };
    let dir_y = if dy >= 0.0 { 1.0 } else { -1.0 };
    let radius = 12.0_f32
        .min((mid_x - sx).abs())
        .min((tx - mid_x).abs())
        .min(dy.abs() / 2.0);

    if radius < 0.5 {
        return format!(
            "M {sx:.1},{sy:.1} L {mid_x:.1},{sy:.1} L {mid_x:.1},{ty:.1} L {tx:.1},{ty:.1}",
        );
    }

    format!(
        "M {sx:.1},{sy:.1} \
         L {:.1},{sy:.1} \
         Q {mid_x:.1},{sy:.1} {mid_x:.1},{:.1} \
         L {mid_x:.1},{:.1} \
         Q {mid_x:.1},{ty:.1} {:.1},{ty:.1} \
         L {tx:.1},{ty:.1}",
        mid_x - dir_x * radius,
        sy + dir_y * radius,
        ty - dir_y * radius,
        mid_x + dir_x * radius,
    )
}

fn degenerate_connector_path(sx: f32, sy: f32) -> String {
    format!("M {sx:.1},{sy:.1}")
}

fn rounded_orthogonal_polyline_path(points: &[CanvasPoint]) -> Option<String> {
    let points = compact_points(points);
    let first = points.first()?;
    if points.len() == 1 {
        return None;
    }

    let mut path = format!("M {:.1},{:.1}", first.x, first.y);
    for window in points.windows(3) {
        let [prev, corner, next] = window else {
            continue;
        };
        let Some((before, after)) = rounded_corner_points(*prev, *corner, *next) else {
            path.push_str(&format!(" L {:.1},{:.1}", corner.x, corner.y));
            continue;
        };
        path.push_str(&format!(" L {:.1},{:.1}", before.x, before.y));
        path.push_str(&format!(
            " Q {:.1},{:.1} {:.1},{:.1}",
            corner.x, corner.y, after.x, after.y
        ));
    }

    let last = points.last()?;
    path.push_str(&format!(" L {:.1},{:.1}", last.x, last.y));
    Some(path)
}

fn reanchor_routed_points(
    points: &[CanvasPoint],
    sx: f32,
    sy: f32,
    tx: f32,
    ty: f32,
) -> Option<Vec<CanvasPoint>> {
    let mut points = compact_points(points);
    if points.len() < 2 {
        return None;
    }

    if points_nearly_equal(points[0], CanvasPoint { x: sx, y: sy })
        && points_nearly_equal(*points.last()?, CanvasPoint { x: tx, y: ty })
    {
        return Some(points);
    }

    let last_index = points.len() - 1;
    points[0] = CanvasPoint { x: sx, y: sy };
    points[last_index] = CanvasPoint { x: tx, y: ty };

    if points.len() >= 4 {
        points[1].y = sy;
        points[last_index - 1].y = ty;
    }

    Some(points)
}

fn rounded_corner_points(
    prev: CanvasPoint,
    corner: CanvasPoint,
    next: CanvasPoint,
) -> Option<(CanvasPoint, CanvasPoint)> {
    let incoming = axis_direction(prev, corner)?;
    let outgoing = axis_direction(corner, next)?;
    if incoming == outgoing {
        return None;
    }

    let incoming_len = segment_len(prev, corner);
    let outgoing_len = segment_len(corner, next);
    let radius = 12.0_f32.min(incoming_len / 2.0).min(outgoing_len / 2.0);
    if radius < 0.5 {
        return None;
    }

    Some((
        CanvasPoint {
            x: corner.x - incoming.0 * radius,
            y: corner.y - incoming.1 * radius,
        },
        CanvasPoint {
            x: corner.x + outgoing.0 * radius,
            y: corner.y + outgoing.1 * radius,
        },
    ))
}

fn axis_direction(from: CanvasPoint, to: CanvasPoint) -> Option<(f32, f32)> {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    if dx.abs() < 0.5 && dy.abs() < 0.5 {
        None
    } else if dx.abs() >= dy.abs() {
        Some((dx.signum(), 0.0))
    } else {
        Some((0.0, dy.signum()))
    }
}

fn segment_len(from: CanvasPoint, to: CanvasPoint) -> f32 {
    (to.x - from.x).abs().max((to.y - from.y).abs())
}

fn compact_points(points: &[CanvasPoint]) -> Vec<CanvasPoint> {
    let mut compacted = Vec::with_capacity(points.len());
    for &point in points {
        if compacted
            .last()
            .is_none_or(|previous: &CanvasPoint| *previous != point)
        {
            compacted.push(point);
        }
    }
    compacted
}

fn compact_orthogonal_points(points: &[CanvasPoint]) -> Vec<CanvasPoint> {
    let points = compact_points(points);
    let mut compacted = Vec::with_capacity(points.len());
    for point in points {
        if compacted.len() >= 2 {
            let previous = compacted[compacted.len() - 1];
            let before_previous = compacted[compacted.len() - 2];
            if points_are_collinear(before_previous, previous, point) {
                let last_index = compacted.len() - 1;
                compacted[last_index] = point;
                continue;
            }
        }
        compacted.push(point);
    }
    compacted
}

fn points_are_collinear(a: CanvasPoint, b: CanvasPoint, c: CanvasPoint) -> bool {
    ((a.x - b.x).abs() < AXIS_EPSILON && (b.x - c.x).abs() < AXIS_EPSILON)
        || ((a.y - b.y).abs() < AXIS_EPSILON && (b.y - c.y).abs() < AXIS_EPSILON)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounded_orthogonal_path_rounds_both_elbows() {
        let path = rounded_orthogonal_path(100.0, 40.0, 300.0, 120.0);

        assert_eq!(
            path,
            "M 100.0,40.0 L 188.0,40.0 Q 200.0,40.0 200.0,52.0 L 200.0,108.0 Q 200.0,120.0 212.0,120.0 L 300.0,120.0"
        );
    }

    #[test]
    fn rounded_orthogonal_path_clamps_radius_for_tight_vertical_gap() {
        let path = rounded_orthogonal_path(100.0, 40.0, 300.0, 50.0);

        assert_eq!(
            path,
            "M 100.0,40.0 L 195.0,40.0 Q 200.0,40.0 200.0,45.0 L 200.0,45.0 Q 200.0,50.0 205.0,50.0 L 300.0,50.0"
        );
    }

    #[test]
    fn rounded_orthogonal_path_uses_straight_line_for_aligned_ports() {
        let path = rounded_orthogonal_path(100.0, 40.0, 300.0, 40.2);

        assert_eq!(path, "M 100.0,40.0 L 300.0,40.2");
    }

    #[test]
    fn rounded_orthogonal_polyline_path_uses_layout_lanes() {
        let path = rounded_orthogonal_polyline_path(&[
            CanvasPoint { x: 160.0, y: 103.0 },
            CanvasPoint { x: 220.0, y: 103.0 },
            CanvasPoint { x: 220.0, y: 220.0 },
            CanvasPoint { x: 300.0, y: 220.0 },
        ]);

        assert_eq!(
            path.as_deref(),
            Some(
                "M 160.0,103.0 L 208.0,103.0 Q 220.0,103.0 220.0,115.0 L 220.0,208.0 Q 220.0,220.0 232.0,220.0 L 300.0,220.0"
            )
        );
    }

    #[test]
    fn rounded_orthogonal_polyline_path_rejects_degenerate_path() {
        assert_eq!(rounded_orthogonal_polyline_path(&[]), None);
        assert_eq!(
            rounded_orthogonal_polyline_path(&[CanvasPoint { x: 1.0, y: 2.0 }]),
            None
        );
    }

    #[test]
    fn reanchor_routed_points_preserves_lane_xs_and_updates_endpoints() {
        let points = reanchor_routed_points(
            &[
                CanvasPoint { x: 160.0, y: 103.0 },
                CanvasPoint { x: 220.0, y: 103.0 },
                CanvasPoint { x: 220.0, y: 220.0 },
                CanvasPoint { x: 300.0, y: 220.0 },
            ],
            160.0,
            147.0,
            300.0,
            264.0,
        )
        .expect("valid path");

        assert_eq!(
            points,
            vec![
                CanvasPoint { x: 160.0, y: 147.0 },
                CanvasPoint { x: 220.0, y: 147.0 },
                CanvasPoint { x: 220.0, y: 264.0 },
                CanvasPoint { x: 300.0, y: 264.0 },
            ]
        );
    }

    #[test]
    fn connector_path_data_keeps_duplicate_routed_path_degenerate() {
        let routed_path = CanvasConnectorPath {
            points: vec![
                CanvasPoint { x: 160.0, y: 147.0 },
                CanvasPoint { x: 160.0, y: 147.0 },
            ],
        };

        let path = connector_path_data(160.0, 147.0, 300.0, 264.0, Some(&routed_path));

        assert_eq!(path, "M 160.0,147.0");
        assert_ne!(path, rounded_orthogonal_path(160.0, 147.0, 300.0, 264.0));
    }

    #[test]
    fn rounded_orthogonal_polyline_path_accepts_two_point_path() {
        let path = rounded_orthogonal_polyline_path(&[
            CanvasPoint { x: 10.0, y: 20.0 },
            CanvasPoint { x: 40.0, y: 20.0 },
        ]);

        assert_eq!(path.as_deref(), Some("M 10.0,20.0 L 40.0,20.0"));
    }

    #[test]
    fn node_connector_stroke_style_uses_uniform_transform_orange() {
        assert_eq!(
            connector_stroke_style(true),
            "stroke: var(--klinx-accent-transform);"
        );
        assert_eq!(connector_stroke_style(false), "");
    }

    #[test]
    fn centered_channel_paths_puts_single_visible_pipe_in_center() {
        let paths = centered_channel_paths(&[ConnectorEndpoints {
            sx: 100.0,
            sy: 20.0,
            tx: 300.0,
            ty: 80.0,
        }]);

        assert_eq!(paths[0].points[1].x, 200.0);
    }

    #[test]
    fn centered_channel_paths_fans_two_visible_pipes_around_center() {
        let endpoints = [
            ConnectorEndpoints {
                sx: 100.0,
                sy: 20.0,
                tx: 300.0,
                ty: 80.0,
            },
            ConnectorEndpoints {
                sx: 100.0,
                sy: 40.0,
                tx: 300.0,
                ty: 100.0,
            },
        ];

        let paths = centered_channel_paths(&endpoints);

        assert_eq!(paths[0].points[1].x, 193.0);
        assert_eq!(paths[1].points[1].x, 207.0);
    }

    #[test]
    fn centered_channel_paths_fans_five_visible_pipes_from_center() {
        let endpoints = (0..5)
            .map(|idx| ConnectorEndpoints {
                sx: 100.0,
                sy: 20.0 + idx as f32 * 10.0,
                tx: 300.0,
                ty: 80.0 + idx as f32 * 10.0,
            })
            .collect::<Vec<_>>();

        let paths = centered_channel_paths(&endpoints);
        let lane_xs = paths
            .iter()
            .map(|path| path.points[1].x)
            .collect::<Vec<_>>();

        assert_eq!(lane_xs, vec![172.0, 186.0, 200.0, 214.0, 228.0]);
    }

    #[test]
    fn centered_channel_paths_keep_lane_points_inside_available_gap() {
        let sx = 160.0;
        let tx = 320.0;
        let endpoints = (0..5)
            .map(|idx| ConnectorEndpoints {
                sx,
                sy: 40.0 + idx as f32 * 12.0,
                tx,
                ty: 120.0 + idx as f32 * 12.0,
            })
            .collect::<Vec<_>>();

        let paths = centered_channel_paths(&endpoints);

        for path in paths {
            let internal_points = &path.points[1..path.points.len() - 1];
            assert!(
                internal_points
                    .iter()
                    .all(|point| point.x >= sx + CHANNEL_NODE_MARGIN
                        && point.x <= tx - CHANNEL_NODE_MARGIN),
                "internal lane points should stay in the inter-node gap: {path:?}"
            );
        }
    }

    #[test]
    fn obstacle_aware_channel_paths_moves_skip_rank_lane_outside_intermediate_card() {
        let endpoints = [ConnectorEndpoints {
            sx: 220.0,
            sy: 80.0,
            tx: 620.0,
            ty: 260.0,
        }];
        let obstacle = ConnectorObstacle {
            x: 300.0,
            y: 120.0,
            width: 160.0,
            height: 100.0,
        };
        let midpoint_path = path_points_for_lane(endpoints[0], 420.0);
        assert!(
            path_intersects_obstacle(&midpoint_path, &obstacle),
            "test setup should block the previous midpoint lane"
        );

        let paths = obstacle_aware_channel_paths(&endpoints, &[obstacle]);
        let lane_x = paths[0].points[1].x;

        assert!(
            lane_x > obstacle.right() || lane_x < obstacle.left(),
            "lane {lane_x} should sit outside the intermediate card: {obstacle:?}"
        );
        assert_eq!(lane_x, 540.0);
    }

    #[test]
    fn obstacle_aware_channel_paths_spreads_crowded_group_across_free_spans() {
        let endpoints = (0..6)
            .map(|idx| ConnectorEndpoints {
                sx: 100.0,
                sy: 20.0 + idx as f32 * 10.0,
                tx: 482.0,
                ty: 250.0 + idx as f32 * 10.0,
            })
            .collect::<Vec<_>>();
        let obstacles = [
            ConnectorObstacle {
                x: 185.0,
                y: 100.0,
                width: 50.0,
                height: 120.0,
            },
            ConnectorObstacle {
                x: 335.0,
                y: 100.0,
                width: 50.0,
                height: 120.0,
            },
        ];

        let paths = obstacle_aware_channel_paths(&endpoints, &obstacles);
        let lane_xs = paths
            .iter()
            .map(|path| path.points[1].x)
            .collect::<Vec<_>>();

        assert!(
            lane_xs
                .windows(2)
                .all(|pair| pair[1] - pair[0] >= CHANNEL_LANE_SPACING - LANE_SPACING_EPSILON),
            "lanes should keep full channel spacing when clean slots exist: {lane_xs:?}"
        );
        assert!(
            lane_xs.iter().any(|lane_x| *lane_x > 403.0),
            "crowded group should use a second free span instead of squeezing into one: {lane_xs:?}"
        );
        for path in paths {
            for obstacle in obstacles {
                assert!(
                    !path_intersects_obstacle(&path.points, &obstacle),
                    "routed path should avoid obstacle interior: {path:?}"
                );
            }
        }
    }

    #[test]
    fn obstacle_aware_channel_paths_separates_independent_overlapping_groups() {
        let endpoints = [
            ConnectorEndpoints {
                sx: 100.0,
                sy: 20.0,
                tx: 300.0,
                ty: 140.0,
            },
            ConnectorEndpoints {
                sx: 80.0,
                sy: 40.0,
                tx: 320.0,
                ty: 160.0,
            },
        ];

        let paths = obstacle_aware_channel_paths(&endpoints, &[]);
        let lane_xs = paths
            .iter()
            .map(|path| path.points[1].x)
            .collect::<Vec<_>>();

        assert!(
            (lane_xs[0] - lane_xs[1]).abs() >= CHANNEL_LANE_SPACING - LANE_SPACING_EPSILON,
            "independent node pipes with overlapping vertical runs should reserve separate lanes: {lane_xs:?}"
        );
    }

    #[test]
    fn obstacle_aware_channel_paths_do_not_intersect_intermediate_card_segments() {
        let endpoints = [ConnectorEndpoints {
            sx: 220.0,
            sy: 80.0,
            tx: 620.0,
            ty: 260.0,
        }];
        let obstacle = ConnectorObstacle {
            x: 300.0,
            y: 120.0,
            width: 160.0,
            height: 100.0,
        };

        let paths = obstacle_aware_channel_paths(&endpoints, &[obstacle]);

        assert!(
            !path_intersects_obstacle(&paths[0].points, &obstacle),
            "routed path should avoid obstacle interior: {:?}",
            paths[0]
        );
    }

    #[test]
    fn obstacle_aware_channel_paths_detours_horizontal_legs_around_intermediate_card() {
        let endpoints = [ConnectorEndpoints {
            sx: 220.0,
            sy: 150.0,
            tx: 620.0,
            ty: 180.0,
        }];
        let obstacle = ConnectorObstacle {
            x: 300.0,
            y: 120.0,
            width: 160.0,
            height: 100.0,
        };
        let midpoint_path = path_points_for_lane(endpoints[0], 540.0);
        assert!(
            path_intersects_obstacle(&midpoint_path, &obstacle),
            "test setup should block the old lane-only route"
        );

        let paths = obstacle_aware_channel_paths(&endpoints, &[obstacle]);

        assert!(
            !path_intersects_obstacle(&paths[0].points, &obstacle),
            "all routed polyline segments should avoid the intermediate card: {:?}",
            paths[0]
        );
        assert!(
            paths[0].points.len() > midpoint_path.len(),
            "blocked horizontal legs should add an orthogonal detour: {:?}",
            paths[0]
        );
    }

    #[test]
    fn obstacle_aware_channel_paths_do_not_stack_unrelated_independent_polylines() {
        let endpoints = [
            ConnectorEndpoints {
                sx: 100.0,
                sy: 100.0,
                tx: 300.0,
                ty: 100.0,
            },
            ConnectorEndpoints {
                sx: 120.0,
                sy: 100.0,
                tx: 320.0,
                ty: 100.0,
            },
        ];
        let old_first = path_points_for_lane(endpoints[0], 200.0);
        let old_second = path_points_for_lane(endpoints[1], 220.0);
        assert!(
            paths_share_segment(&old_first, &old_second),
            "test setup should reproduce the old stacked horizontal segment"
        );

        let paths = obstacle_aware_channel_paths(&endpoints, &[]);

        assert!(
            !paths_share_segment(&paths[0].points, &paths[1].points),
            "unrelated independent connectors should not reuse a full segment: {paths:?}"
        );
        assert!(
            paths[1].points.iter().any(|point| point.y != 100.0),
            "the second independent connector should take a small detour: {:?}",
            paths[1]
        );
    }

    #[test]
    fn segment_intersects_obstacle_counts_border_runs_not_port_touches() {
        let obstacle = ConnectorObstacle {
            x: 100.0,
            y: 100.0,
            width: 120.0,
            height: 80.0,
        };

        assert!(
            segment_intersects_obstacle(
                CanvasPoint { x: 130.0, y: 100.0 },
                CanvasPoint { x: 180.0, y: 100.0 },
                &obstacle,
            ),
            "running along a node border obscures the card edge and should be blocked"
        );
        assert!(
            !segment_intersects_obstacle(
                CanvasPoint { x: 220.0, y: 140.0 },
                CanvasPoint { x: 260.0, y: 140.0 },
                &obstacle,
            ),
            "leaving a port perpendicularly should only touch the card at one point"
        );
    }
}
