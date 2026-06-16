use dioxus::prelude::*;

use crate::pipeline_view::{FieldEdgeKind, StageView};

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
            // Node-level cables keep their per-kind stage accent, inlined so the
            // stroke follows `--klinx-stage-accent` from the `data-stage-kind`.
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
}

#[component]
pub fn FieldConnector(props: FieldConnectorProps) -> Element {
    let (sx, sy) = props.start;
    let (tx, ty) = props.end;
    // Each relationship kind reads as a distinct hue: a pure pass-through is the
    // quietest, an accessed carry a warm highlight, a derive the active accent.
    let extra_class = match props.kind {
        FieldEdgeKind::Passthrough => "klinx-field-edge klinx-field-edge--passthrough",
        FieldEdgeKind::Access => "klinx-field-edge klinx-field-edge--access",
        FieldEdgeKind::Derive => "klinx-field-edge klinx-field-edge--derive",
    }
    .to_string();

    rsx! {
        ConnectorPath {
            sx,
            sy,
            tx,
            ty,
            kind_attr: props.kind_attr.clone(),
            extra_class,
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
    /// Whether to inline `stroke: var(--klinx-stage-accent)` on each path. Node
    /// connectors set this so their stroke follows the per-kind accent; field
    /// connectors clear it so the `.klinx-field-edge--*` CSS classes own the
    /// stroke colour (inline styles outrank class rules, so the inline stroke
    /// must be ABSENT for the class colour to apply).
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
        inline_accent_stroke,
    } = props;

    // Empty when the CSS class owns the stroke (field edges); the accent style
    // otherwise (node edges). An empty `style` leaves the path's stroke to be
    // inherited from the `<g>`'s class-set value.
    let stroke_style = if inline_accent_stroke {
        // Fallback to the transform accent so a node kind that is ever missing a
        // `--klinx-stage-accent` definition still renders a visible cable rather
        // than a strokeless (invisible) one.
        "stroke: var(--klinx-stage-accent, var(--klinx-accent-transform));"
    } else {
        ""
    };

    let path = rounded_orthogonal_path(sx, sy, tx, ty);

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
}
