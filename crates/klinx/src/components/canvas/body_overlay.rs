//! In-context composition body overlay (#171, Phase 1).
//!
//! Drilling into a composition node opens its body in an OVERLAY — a "lightbox"
//! over the dimmed/blurred-but-visible parent canvas — instead of swapping the
//! whole canvas. The parent stays mounted and untouched; the fixed full-viewport
//! backdrop above it (semi-opaque fill + `backdrop-filter: blur`) is what dims it
//! and intercepts parent interaction (`pointer-events`).
//!
//! The overlay reads [`crate::state::AppState::composition_overlay_stack`]: its
//! top frame names the body to show. An in-overlay breadcrumb navigates nested
//! compositions WITHIN the overlay (pushing/truncating the overlay stack, never
//! the full-swap drill stack). The inner sub-canvas gets its OWN pan/zoom,
//! independent of the parent, by mounting a second [`super::panel::PanViewport`]
//! over a locally-derived body view.
//!
//! Dismissal: Esc (a local `onkeydown` on the focusable overlay root), backdrop
//! click, or the `✕` button — each clears the overlay stack. The "OPEN FULL"
//! button is the escape hatch to the existing full-swap drill: it moves the
//! overlay frames onto the drill stack and clears the overlay.
//!
//! This is the CONTAINED-overlay approach (the brief's bounded fallback): rather
//! than extracting a shared `CanvasSurface` from the ~1240-line `CanvasPanel`
//! (high regression risk to the main canvas), the overlay COMPOSES the existing
//! [`super::node::CanvasNode`] + [`super::connector::Connector`] components against
//! a body view derived here. All view derivation lives in
//! [`super::panel::build_body_canvas`]; the cards/connectors are the same
//! components the main canvas uses. The in-overlay lineage ribbon and the full
//! `CanvasSurface` extraction are a documented Phase-2 follow-up.

use dioxus::prelude::*;

use crate::pipeline_view::derive_body_view;
use crate::state::{promote_overlay_to_drill, use_app_state};

use super::breadcrumbs::{BreadcrumbBar, BreadcrumbTarget};
use super::connector::Connector;
use super::node::CanvasNode;
use super::panel::{PanViewport, build_body_canvas};
use super::{CanvasHover, HoverTarget, PinnedField};

/// The in-context composition body overlay (#171).
///
/// Hook-light by design: every hook runs unconditionally at the top, and the
/// "no overlay open" early return happens only AFTER them, so hook order is
/// stable across renders (per `components/AGENTS.md`). The inner pan/zoom signals
/// are LOCAL to this component, giving the sub-canvas pan/zoom independent of the
/// parent canvas's own transform. Re-keyed by overlay depth so navigating a level
/// resets the pan/zoom for the newly-shown body.
#[component]
pub fn BodyOverlay() -> Element {
    let state = use_app_state();

    // Inner sub-canvas transform — LOCAL, so the overlay pans/zooms independently
    // of the parent canvas (its transform lives on `CanvasPanel`, untouched here).
    let mut pan_x = use_signal(|| 0.0_f32);
    let mut pan_y = use_signal(|| 0.0_f32);
    let zoom = use_signal(|| 1.0_f32);

    // Non-reactive drag state for the inner pan — a hot path that must not
    // re-render per move, so it lives in a `use_hook` cell, mirroring the main
    // canvas drag.
    let drag = use_hook(|| std::rc::Rc::new(std::cell::RefCell::new(OverlayDrag::default())));

    // Field-lineage contexts the reused `CanvasNode`/`FieldRowView` consume. The
    // overlay provides its OWN, isolated from the parent canvas's contexts, so a
    // hover inside the overlay never lights the parent and vice versa. Phase 1
    // does not draw the field-lineage overlay inside the lightbox, but the cards
    // still require these contexts to mount.
    use_context_provider(|| {
        CanvasHover(
            Signal::new(HoverTarget::None),
            Signal::new(HoverTarget::None),
            Signal::new(0),
            Signal::new(false),
        )
    });
    use_context_provider(|| PinnedField(Signal::new(None)));

    // Read the overlay stack: its frames drive the breadcrumb and its top frame
    // names the body to render. Subscribes so a push/truncate re-renders.
    let stack = state.composition_overlay_stack.read().clone();
    let Some(frame) = stack.last().cloned() else {
        // No overlay open — render nothing. AFTER all hooks above, so hook order
        // is identical whether or not the overlay is showing.
        return rsx! {};
    };
    let breadcrumb_frames: Vec<String> = stack.iter().map(|f| f.alias.clone()).collect();
    let depth = stack.len();

    // Derive the body view for the top frame, mirroring `current_pipeline_view`'s
    // missing-body fallback (no compiled plan / unknown body → empty canvas).
    let view = {
        let compiled_guard = state.compiled_plan.read();
        compiled_guard
            .as_ref()
            .and_then(|plan| plan.body_of(frame.body_id))
            .map(derive_body_view)
            .unwrap_or_default()
    };
    let canvas = build_body_canvas(view, *zoom.read());

    // ── Inner pan handlers (left/middle-drag on the sub-canvas background) ──
    let drag_down = {
        let drag = drag.clone();
        move |e: MouseEvent| {
            if e.trigger_button() == Some(dioxus::html::input_data::MouseButton::Primary)
                || e.trigger_button() == Some(dioxus::html::input_data::MouseButton::Auxiliary)
            {
                let pos = e.client_coordinates();
                let mut d = drag.borrow_mut();
                d.active = true;
                d.start_x = pos.x as f32;
                d.start_y = pos.y as f32;
                d.start_pan_x = *pan_x.peek();
                d.start_pan_y = *pan_y.peek();
            }
        }
    };
    let drag_move = {
        let drag = drag.clone();
        move |e: MouseEvent| {
            let d = drag.borrow();
            if d.active {
                let pos = e.client_coordinates();
                pan_x.set(d.start_pan_x + (pos.x as f32 - d.start_x));
                pan_y.set(d.start_pan_y + (pos.y as f32 - d.start_y));
            }
        }
    };
    let drag_up = {
        let drag = drag.clone();
        move |_: MouseEvent| {
            drag.borrow_mut().active = false;
        }
    };
    let drag_leave = {
        let drag = drag.clone();
        move |_: MouseEvent| {
            drag.borrow_mut().active = false;
        }
    };

    rsx! {
        // Backdrop — fixed full-viewport, sits ABOVE the still-mounted parent
        // canvas. Its blur + semi-opaque fill dim the parent; its pointer-events
        // intercept parent interaction. Clicking it dismisses the overlay.
        // `tabindex` + `autofocus` make it focusable so the local Esc handler
        // fires without a central keyboard arm.
        div {
            class: "klinx-body-overlay-backdrop",
            tabindex: "0",
            autofocus: true,
            onkeydown: move |e: KeyboardEvent| {
                if e.key() == Key::Escape {
                    e.stop_propagation();
                    let mut overlay = state.composition_overlay_stack;
                    overlay.write().clear();
                }
            },
            onclick: move |_| {
                let mut overlay = state.composition_overlay_stack;
                overlay.write().clear();
            },

            // Overlay panel — large, near-fullscreen. `stop_propagation` keeps a
            // click inside the panel from bubbling to the backdrop's dismiss.
            div {
                class: "klinx-body-overlay",
                onclick: move |e: MouseEvent| e.stop_propagation(),

                // ── Header: in-overlay breadcrumb + OPEN FULL + close ──────
                div {
                    class: "klinx-body-overlay-header",

                    // In-overlay breadcrumb — navigates the OVERLAY stack, with a
                    // body-appropriate root label, so per-level back-navigation
                    // stays inside the lightbox (Esc closes the whole overlay).
                    BreadcrumbBar {
                        frames: breadcrumb_frames,
                        target: BreadcrumbTarget::Overlay,
                        root_label: "body".to_string(),
                    }

                    div { class: "klinx-body-overlay-header-spacer" }

                    // Escape hatch to the existing full-swap drill: move the
                    // overlay frames onto the drill stack, then clear the overlay.
                    button {
                        class: "klinx-body-overlay-btn",
                        title: "Open this body in the full canvas",
                        onclick: move |_| {
                            let mut overlay = state.composition_overlay_stack;
                            let mut drill = state.composition_drill_stack;
                            let mut overlay_frames = overlay.write();
                            let mut drill_frames = drill.write();
                            promote_overlay_to_drill(&mut overlay_frames, &mut drill_frames);
                        },
                        "OPEN FULL"
                    }

                    button {
                        class: "klinx-body-overlay-close",
                        title: "Close (Esc)",
                        onclick: move |_| {
                            let mut overlay = state.composition_overlay_stack;
                            overlay.write().clear();
                        },
                        "✕"
                    }
                }

                // ── Inner sub-canvas: own PanViewport + independent pan/zoom ──
                // `overflow:hidden` + `position:relative` (CSS) so the viewport
                // clips to the overlay region and the cards' world-space cards
                // pan/zoom within the lightbox, not the screen.
                div {
                    class: "klinx-body-overlay-canvas",
                    onmousedown: drag_down,
                    onmousemove: drag_move,
                    onmouseup: drag_up,
                    onmouseleave: drag_leave,

                    // Re-key by depth so navigating a breadcrumb level remounts the
                    // viewport (and resets its pan) for the newly-shown body.
                    PanViewport {
                        key: "{depth}",
                        pan_x,
                        pan_y,
                        zoom,

                        svg {
                            class: "klinx-canvas-svg klinx-canvas-svg--base",
                            width: "{canvas.svg_w}",
                            height: "{canvas.svg_h}",
                            g {
                                class: "klinx-canvas-edges",
                                for conn in canvas.connections {
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

                        for (index, (stage, display)) in canvas.cards.into_iter().enumerate() {
                            CanvasNode {
                                key: "{stage.id}",
                                stage,
                                index,
                                field_display: display,
                                // Phase 1: the overlay sub-canvas is a read-only
                                // body preview — field search/expand/display
                                // actions are no-ops here (a Phase-2 follow-up
                                // wires the in-overlay reveal). A nested `▶` still
                                // works: it lives inside CanvasNode and pushes the
                                // overlay stack via the shared AppState context.
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
    }
}

/// Non-reactive drag state for the overlay sub-canvas pan (hot path; no
/// per-move re-render).
#[derive(Default)]
struct OverlayDrag {
    active: bool,
    start_x: f32,
    start_y: f32,
    start_pan_x: f32,
    start_pan_y: f32,
}
