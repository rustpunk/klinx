//! In-place composition body render for explode-in-place (#171 Phase 3).
//!
//! The third presentation of a composition body, after the Phase 1 lightbox
//! overlay ([`super::body_overlay`]) and the Phase 2 corner inset
//! ([`super::composition_pip`]). Where those show the body *off to the side* with
//! their OWN pan/zoom, this embeds the body as a mini-DAG **at the parent node's
//! position on the main canvas**: the parent composition node's layout footprint
//! is grown to the body's size (in `panel.rs`, before layout) so sibling nodes
//! reflow around it, and this component fills that reserved footprint — it renders
//! the exploded node AS a bordered frame (header + body interior) in place of the
//! node's ordinary card, which `panel.rs` filters out while it is exploded.
//!
//! Unlike the overlay/inset it has NO [`super::panel::PanViewport`] of its own — it
//! is rendered INSIDE the main canvas's viewport, so it pans and zooms WITH the
//! parent (a single shared transform). Positioning is a plain absolutely-positioned
//! frame placed at the parent card's world position; the reused [`CanvasNode`] /
//! [`Connector`] then lay out at their body-local coordinates relative to the
//! frame's body region — no per-card coordinate offsetting.
//!
//! Like [`super::body_sub_canvas::BodySubCanvas`] this draws the body's plain
//! node-level DAG only (`dimmed: false`, field actions are no-ops). It re-provides
//! fresh [`super::CanvasHover`] / [`super::PinnedField`] contexts so a hover inside
//! the embedded body never lights the parent canvas. Boundary cables binding the
//! parent's fields to the inner ports are a follow-up (#171 Phase 3, PR B).

use dioxus::prelude::*;

use crate::pipeline_view::StageView;
use crate::state::{toggle_composition_explode, use_app_state};

use super::connector::Connector;
use super::node::CanvasNode;
use super::panel::{BodyCanvas, EXPLODE_HEADER_BAND, EXPLODE_PAD};
use super::{CanvasHover, HoverTarget, PinnedField};

/// Render an exploded composition node as a framed mini-DAG at its position on the
/// main canvas (#171 Phase 3).
///
/// `stage` is the laid-out exploded composition node — its `canvas_x`/`canvas_y`
/// and `effective_width`/`effective_height` (the footprint reserved in `panel.rs`)
/// give the frame's world rect, and its label/kind drive the header. `canvas` is
/// the body's [`BodyCanvas`] in body-local coordinates; the frame's body region is
/// an absolutely-positioned container offset by the header band + padding, so the
/// body's local coordinates land inside the frame with no offsetting.
#[component]
pub(super) fn ExplodedBody(stage: StageView, canvas: BodyCanvas) -> Element {
    let state = use_app_state();

    // Isolated field-lineage contexts for the reused `CanvasNode`, exactly like the
    // overlay/inset: the embedded body draws the node-level DAG only, and these keep
    // a hover inside it from lighting the parent canvas (whose card indices differ
    // from the body's local ones).
    use_context_provider(|| {
        CanvasHover(
            Signal::new(HoverTarget::None),
            Signal::new(HoverTarget::None),
            Signal::new(0),
            Signal::new(false),
        )
    });
    use_context_provider(|| PinnedField(Signal::new(None)));

    let BodyCanvas {
        cards,
        connections,
        svg_w,
        svg_h,
    } = canvas;

    let frame_x = stage.canvas_x;
    let frame_y = stage.canvas_y;
    let frame_w = stage.effective_width();
    let frame_h = stage.effective_height();
    let kind_attr = stage.kind.kind_attr();
    let badge = stage.kind.badge_label();
    let label = stage.label.clone();
    let node_id = stage.id.clone();

    rsx! {
        // The exploded frame: an absolutely-positioned bordered container at the
        // parent card's world rect, inside the main `PanViewport` so it shares the
        // parent's pan/zoom. Replaces the node's ordinary card while exploded.
        div {
            key: "{node_id}",
            class: "klinx-exploded-frame",
            "data-stage-kind": kind_attr,
            style: "left: {frame_x}px; top: {frame_y}px; width: {frame_w}px; height: {frame_h}px;",

            // Header band: badge + composition name + collapse control.
            div {
                class: "klinx-exploded-frame-header",
                style: "height: {EXPLODE_HEADER_BAND}px;",
                onmousedown: move |e: MouseEvent| e.stop_propagation(),
                span { class: "klinx-exploded-frame-badge", "{badge}" }
                span { class: "klinx-exploded-frame-label", "{label}" }
                span { style: "flex: 1;" }
                button {
                    class: "klinx-exploded-frame-collapse",
                    title: "Collapse the in-place body",
                    onclick: {
                        let node_id = node_id.clone();
                        move |e: MouseEvent| {
                            e.stop_propagation();
                            let mut set = state.composition_explode_set;
                            let mut next = set.peek().clone();
                            toggle_composition_explode(&mut next, &node_id);
                            set.set(next);
                        }
                    },
                    "\u{229F}"
                }
            }

            // Body region: the body's local coordinate space, offset from the
            // frame's top-left by the header band + padding. The svg + cards render
            // at their body-local coordinates relative to this container.
            div {
                class: "klinx-exploded-frame-body",
                style: "left: {EXPLODE_PAD}px; top: {EXPLODE_HEADER_BAND}px; width: {svg_w}px; height: {svg_h}px;",

                svg {
                    class: "klinx-canvas-svg klinx-canvas-svg--base",
                    width: "{svg_w}",
                    height: "{svg_h}",
                    g {
                        class: "klinx-canvas-edges",
                        for conn in connections {
                            Connector {
                                key: "{conn.from.id}-{conn.to.id}-{conn.from_branch:?}",
                                from: conn.from,
                                to: conn.to,
                                from_branch: conn.from_branch,
                                path: conn.path,
                            }
                        }
                    }
                }

                for (index, (body_stage, display)) in cards.into_iter().enumerate() {
                    CanvasNode {
                        key: "{body_stage.id}",
                        stage: body_stage,
                        index,
                        field_display: display,
                        // Read-only preview, mirroring `BodySubCanvas`: field
                        // search/expand/display actions are no-ops. A nested `▶`
                        // still opens the lightbox overlay (the default drill target).
                        on_field_query: move |_: String| {},
                        on_field_toggle: move |_| {},
                        on_display_action: move |_| {},
                        dimmed: false,
                        highlighted_fields: Vec::new(),
                        highlighted_role_ports: Vec::new(),
                    }
                }
            }
        }
    }
}
