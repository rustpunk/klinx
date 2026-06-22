//! Picture-in-picture composition body inset (#171, Phase 2).
//!
//! The non-modal sibling of the Phase 1 lightbox [`super::body_overlay`]. Where
//! the overlay is a modal lightbox over a dimmed, non-interactive parent, the
//! inset is a small panel pinned to a corner with **no backdrop** — the parent
//! canvas stays fully interactive while the body stays visible. The user docks the
//! lightbox to the corner (the overlay's dock button moves its frames here) and
//! can keep panning/selecting/drilling the parent; the inset's `⤢` button expands
//! the body back to the full lightbox, and `✕` closes it.
//!
//! It reads [`crate::state::AppState::composition_pip_stack`]: its top frame names
//! the body to show. Like the overlay it has its OWN local pan/zoom (independent
//! of the parent and of the overlay), an in-inset breadcrumb that navigates the
//! PiP stack ([`BreadcrumbTarget::Pip`]), and isolated `CanvasHover`/`PinnedField`
//! contexts. It additionally provides [`CompositionDrillTarget::Pip`] so a nested
//! `▶` on a body card keeps drilling INSIDE the inset rather than spawning a
//! lightbox over it. The shared sub-canvas render is [`BodySubCanvas`].
//!
//! Being non-modal, there is no backdrop to click-dismiss and no Esc handler (Esc
//! stays bound to the modal overlay) — the inset is dismissed only by its own `✕`,
//! by "expand", or by the canvas view-swap effect on a tab/pipeline switch.

use dioxus::html::geometry::WheelDelta;
use dioxus::prelude::*;

use crate::pipeline_view::PipelineView;
use crate::state::{move_composition_frames, use_app_state};

use super::body_sub_canvas::BodySubCanvas;
use super::breadcrumbs::{BreadcrumbBar, BreadcrumbTarget};
use super::panel::{
    ZOOM_MAX, ZOOM_MIN, ZOOM_STEP_LINE, ZOOM_STEP_PIXEL, build_body_canvas, build_body_canvas_for,
};
use super::{CanvasHover, CompositionDrillTarget, HoverTarget, PinnedField};

/// The picture-in-picture composition body inset (#171 Phase 2).
///
/// Hook-light, mirroring [`super::body_overlay::BodyOverlay`]: every hook runs
/// unconditionally at the top and the "no inset open" early return happens only
/// after them, so hook order is stable across renders. The pan/zoom signals are
/// LOCAL, giving the inset a transform independent of the parent canvas and the
/// lightbox.
#[component]
pub fn CompositionPip() -> Element {
    let state = use_app_state();

    // Inner sub-canvas transform — LOCAL to the inset.
    let mut pan_x = use_signal(|| 0.0_f32);
    let mut pan_y = use_signal(|| 0.0_f32);
    let mut zoom = use_signal(|| 1.0_f32);

    // Non-reactive drag state for the inset pan (hot path; no per-move re-render).
    // Simpler than the overlay's: there is no backdrop, so no overshoot tracking —
    // a drag that starts on the canvas pans, full stop.
    let drag = use_hook(|| std::rc::Rc::new(std::cell::RefCell::new(PipDrag::default())));

    // Isolated field-lineage contexts for the reused `CanvasNode` (write-only in
    // this phase, exactly like the overlay — the inset draws the node-level DAG,
    // not the field ribbon). Kept separate so a hover inside the inset never lights
    // the parent canvas or the lightbox.
    use_context_provider(|| {
        CanvasHover(
            Signal::new(HoverTarget::None),
            Signal::new(HoverTarget::None),
            Signal::new(0),
            Signal::new(false),
        )
    });
    use_context_provider(|| PinnedField(Signal::new(None)));
    // Route a nested `▶` to the PiP stack so drilling stays in the inset.
    use_context_provider(|| CompositionDrillTarget::Pip);

    // Body render model — unconditional, reactive `use_memo`, identical in spirit
    // to the overlay's: reads the PiP stack + compiled plan + zoom inside the
    // closure so it recomputes on a nested push / breadcrumb truncate / plan load /
    // Auto-density change, but NOT on a hover. `None` only when the inset is closed.
    let canvas = use_memo(move || {
        let stack = state.composition_pip_stack.read();
        let frame = stack.last()?;
        let zoom = *zoom.read();
        Some(match state.compiled_plan.read().as_ref() {
            Some(plan) => build_body_canvas_for(plan, frame.body_id, zoom),
            None => build_body_canvas(PipelineView::default(), zoom),
        })
    });

    // Breadcrumb labels + depth from the same stack read in the body (so the
    // component re-renders on navigation); the early return gates on the memo.
    let stack = state.composition_pip_stack.read().clone();
    let breadcrumb_frames: Vec<String> = stack.iter().map(|f| f.alias.clone()).collect();
    let depth = stack.len();
    drop(stack);
    let Some(canvas) = canvas() else {
        return rsx! {};
    };

    // ── Inner pan handlers (left/middle-drag on the inset background) ──────
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

    // Inner zoom — cursor-anchored, mirroring the overlay/main-canvas wheel
    // handler with the same clamp range + step constants.
    let on_wheel = move |e: WheelEvent| {
        let factor = match e.delta() {
            WheelDelta::Pixels(data) => {
                let dy = data.y as f32;
                if dy == 0.0 {
                    return;
                }
                1.0 - dy * ZOOM_STEP_PIXEL
            }
            WheelDelta::Lines(data) => {
                let dy = data.y as f32;
                if dy == 0.0 {
                    return;
                }
                if dy < 0.0 {
                    ZOOM_STEP_LINE
                } else {
                    1.0 / ZOOM_STEP_LINE
                }
            }
            WheelDelta::Pages(data) => {
                let dy = data.y as f32;
                if dy == 0.0 {
                    return;
                }
                if dy < 0.0 {
                    ZOOM_STEP_LINE * ZOOM_STEP_LINE
                } else {
                    1.0 / (ZOOM_STEP_LINE * ZOOM_STEP_LINE)
                }
            }
        };

        let old_z = *zoom.peek();
        let new_z = (old_z * factor).clamp(ZOOM_MIN, ZOOM_MAX);
        if (new_z - old_z).abs() < 0.0001 {
            return;
        }

        let cursor = e.client_coordinates();
        let cx = cursor.x as f32;
        let cy = cursor.y as f32;
        let old_px = *pan_x.peek();
        let old_py = *pan_y.peek();
        let ratio = new_z / old_z;
        pan_x.set(cx - (cx - old_px) * ratio);
        pan_y.set(cy - (cy - old_py) * ratio);
        zoom.set(new_z);
    };

    // Discrete zoom step for the header +/- buttons. Anchored on the inset's
    // nominal center (the buttons carry no cursor position); same clamp as wheel.
    let zoom_step = move |factor: f32| {
        let mut zoom = zoom;
        let mut pan_x = pan_x;
        let mut pan_y = pan_y;
        let old_z = *zoom.peek();
        let new_z = (old_z * factor).clamp(ZOOM_MIN, ZOOM_MAX);
        if (new_z - old_z).abs() < 0.0001 {
            return;
        }
        // Nominal center of the small inset region (≈ the CSS panel size / 2).
        let (cx, cy) = (210.0_f32, 150.0_f32);
        let old_px = *pan_x.peek();
        let old_py = *pan_y.peek();
        let ratio = new_z / old_z;
        pan_x.set(cx - (cx - old_px) * ratio);
        pan_y.set(cy - (cy - old_py) * ratio);
        zoom.set(new_z);
    };
    let reset_zoom = move |_| {
        zoom.set(1.0);
        pan_x.set(0.0);
        pan_y.set(0.0);
    };

    rsx! {
        // Full-viewport, `pointer-events: none` container holding the corner panel
        // (flex-positioned bottom-right). Mirrors the lightbox's `inset: 0`
        // backdrop — but transparent and click-through, so the parent canvas behind
        // it stays FULLY interactive. The window itself re-enables pointer events.
        // (`inset: 0` rather than a bare `right`/`bottom` offset keeps positioning
        // robust regardless of the fixed-position containing block.)
        div {
            class: "klinx-composition-pip",

            // The actual inset window. `stop_propagation` on mousedown keeps a
            // press inside it from reaching the parent canvas underneath.
            div {
                class: "klinx-composition-pip-window",
                onmousedown: move |e: MouseEvent| e.stop_propagation(),
                onclick: move |e: MouseEvent| e.stop_propagation(),

            // ── Header: breadcrumb + zoom + expand + close ─────────────────
            div {
                class: "klinx-composition-pip-header",

                BreadcrumbBar {
                    frames: breadcrumb_frames,
                    target: BreadcrumbTarget::Pip,
                    root_label: "body".to_string(),
                }

                div { class: "klinx-composition-pip-header-spacer" }

                button {
                    class: "klinx-composition-pip-btn",
                    title: "Zoom out",
                    onclick: move |_| zoom_step(1.0 / ZOOM_STEP_LINE),
                    "\u{2212}"
                }
                button {
                    class: "klinx-composition-pip-btn",
                    title: "Reset zoom",
                    onclick: reset_zoom,
                    "1:1"
                }
                button {
                    class: "klinx-composition-pip-btn",
                    title: "Zoom in",
                    onclick: move |_| zoom_step(ZOOM_STEP_LINE),
                    "+"
                }

                // Expand back to the modal lightbox: hand the inset frames back to
                // the overlay stack at the same depth, then clear the inset.
                button {
                    class: "klinx-composition-pip-btn",
                    title: "Expand to the lightbox",
                    onclick: move |_| {
                        let mut pip = state.composition_pip_stack;
                        let mut overlay = state.composition_overlay_stack;
                        let mut frames = pip.peek().clone();
                        {
                            let mut overlay_frames = overlay.write();
                            move_composition_frames(&mut frames, &mut overlay_frames);
                        }
                        pip.write().clear();
                    },
                    "\u{2922}"
                }

                button {
                    class: "klinx-composition-pip-close",
                    title: "Close",
                    onclick: move |_| {
                        let mut pip = state.composition_pip_stack;
                        pip.write().clear();
                    },
                    "\u{2715}"
                }
            }

            // ── Inner sub-canvas: own PanViewport + independent pan/zoom ────
            div {
                class: "klinx-composition-pip-canvas",
                onmousedown: drag_down,
                onmousemove: drag_move,
                onmouseup: drag_up,
                onmouseleave: drag_leave,
                onwheel: on_wheel,

                BodySubCanvas { canvas, pan_x, pan_y, zoom, depth }
            }
            }
        }
    }
}

/// Non-reactive drag state for the inset sub-canvas pan (hot path; no per-move
/// re-render). Simpler than the overlay's `OverlayDrag` — the inset has no
/// backdrop, so there is no overshoot case to disambiguate.
#[derive(Default)]
struct PipDrag {
    active: bool,
    start_x: f32,
    start_y: f32,
    start_pan_x: f32,
    start_pan_y: f32,
}
