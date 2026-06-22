//! Shared inner sub-canvas render for the composition body lightbox overlay
//! (#171 Phase 1) and the picture-in-picture inset (#171 Phase 2).
//!
//! Both surfaces project a composition body to a [`BodyCanvas`] (via
//! [`super::panel::build_body_canvas`]) and render it inside a [`PanViewport`]
//! with their OWN local pan/zoom, reusing the main canvas's [`CanvasNode`] +
//! [`Connector`] components — no card/connector reimplementation. This component
//! is that shared render: the card/connector/SVG block that was hand-copied from
//! `panel.rs` into the overlay now lives once and is consumed by both the overlay
//! and the inset.
//!
//! Like the overlay's original copy, this draws the body's plain node-level DAG
//! only (`dimmed: false`, no Filter-mode `--recede`/keep gate, field actions are
//! no-ops). The larger `CanvasSurface` extraction that would also restore the main
//! canvas's reveal behavior in these surfaces remains a follow-up.

use dioxus::prelude::*;

use super::connector::Connector;
use super::node::CanvasNode;
use super::panel::{BodyCanvas, PanViewport};

/// Render a derived [`BodyCanvas`] inside a [`PanViewport`] driven by the caller's
/// pan/zoom signals.
///
/// `depth` is the caller's navigation depth: it keys the inner [`PanViewport`] so
/// a breadcrumb push/truncate remounts the viewport for the newly-shown body
/// (matching the overlay's original behavior). The caller is responsible for
/// providing the [`super::CanvasHover`] / [`super::PinnedField`] contexts the
/// reused [`CanvasNode`] requires, and (for the inset) the
/// [`super::CompositionDrillTarget`] context that routes a nested `▶`.
#[component]
pub(super) fn BodySubCanvas(
    canvas: BodyCanvas,
    pan_x: ReadSignal<f32>,
    pan_y: ReadSignal<f32>,
    zoom: ReadSignal<f32>,
    depth: usize,
) -> Element {
    let BodyCanvas {
        cards,
        connections,
        svg_w,
        svg_h,
    } = canvas;

    rsx! {
        // Re-key by depth so navigating a breadcrumb level remounts the viewport
        // for the newly-shown body.
        PanViewport {
            key: "{depth}",
            pan_x,
            pan_y,
            zoom,

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

            for (index, (stage, display)) in cards.into_iter().enumerate() {
                CanvasNode {
                    key: "{stage.id}",
                    stage,
                    index,
                    field_display: display,
                    // The body sub-canvas is a read-only preview: field
                    // search/expand/display actions are no-ops. A nested `▶`
                    // still works — it lives inside CanvasNode and pushes the
                    // stack named by the surrounding CompositionDrillTarget context.
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
