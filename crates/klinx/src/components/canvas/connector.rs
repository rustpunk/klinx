use dioxus::prelude::*;

use crate::pipeline_view::StageView;

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
}

#[component]
pub fn Connector(props: ConnectorProps) -> Element {
    // Node-level connector: anchor at the two stages' mid-height ports. This is
    // the DEFAULT canvas view — one cable per `(from, to)` connection.
    let (sx, sy) = props.from.port_out();
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
        }
    }
}

/// Field-level connector between two explicit anchor points.
///
/// Used only on hover-reveal: a single field's lineage closure draws one of
/// these per participating [`crate::pipeline_view::FieldEdge`], from the
/// producer row's RIGHT anchor to the consumer row's LEFT anchor. `passthrough`
/// drives the visual (a carry vs. a derive); `dimmed` fades edges outside the
/// hovered closure.
#[derive(Props, Clone, PartialEq)]
pub struct FieldConnectorProps {
    pub start: (f32, f32),
    pub end: (f32, f32),
    /// `data-stage-kind` of the producer node, so the cable inherits its accent.
    pub kind_attr: String,
    /// Identity carry (`col` → same `col`) vs. a derivation. Distinguishes the
    /// CSS treatment of the two field-edge flavours.
    pub passthrough: bool,
}

#[component]
pub fn FieldConnector(props: FieldConnectorProps) -> Element {
    let (sx, sy) = props.start;
    let (tx, ty) = props.end;
    // A passthrough carry reads as a quieter line than a compute/derive edge.
    let extra_class = if props.passthrough {
        "klinx-field-edge klinx-field-edge--passthrough".to_string()
    } else {
        "klinx-field-edge klinx-field-edge--derive".to_string()
    };

    rsx! {
        ConnectorPath {
            sx,
            sy,
            tx,
            ty,
            kind_attr: props.kind_attr.clone(),
            extra_class,
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
    } = props;

    // Cubic S-curve: control points at 1/3 of horizontal distance from each end.
    let cp_offset = (tx - sx).abs() / 3.0;
    let path = format!(
        "M {sx:.1},{sy:.1} C {:.1},{sy:.1} {:.1},{ty:.1} {tx:.1},{ty:.1}",
        sx + cp_offset,
        tx - cp_offset,
    );

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
                style: "stroke: var(--klinx-stage-accent);",
            }
            // Layer 2 — dashed core cable
            path {
                d: "{path}",
                fill: "none",
                stroke_width: "2",
                stroke_dasharray: "8 4",
                stroke_opacity: "0.7",
                style: "stroke: var(--klinx-stage-accent);",
            }
            // Layer 3 — bright centre hairline
            path {
                d: "{path}",
                fill: "none",
                stroke_width: "0.75",
                stroke_opacity: "0.9",
                style: "stroke: var(--klinx-stage-accent);",
            }
            // Open chevron arrowhead
            path {
                d: "{arrow}",
                fill: "none",
                stroke_width: "1.5",
                stroke_opacity: "0.8",
                stroke_linejoin: "round",
                stroke_linecap: "round",
                style: "stroke: var(--klinx-stage-accent);",
            }
        }
    }
}
