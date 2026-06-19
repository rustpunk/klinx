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
//! the full-swap drill stack). The inner sub-canvas gets its OWN pan AND zoom,
//! independent of the parent: a local [`super::panel::PanViewport`] over a
//! locally-derived body view, panned by left/middle drag and zoomed by the
//! header zoom buttons (the engine-independent control) plus a best-effort
//! cursor-anchored `onwheel` mirroring the main canvas — WebKitGTK does not
//! reliably deliver `wheel` to a `position:fixed` overlay, so the buttons are the
//! primary zoom affordance and the wheel is a bonus where it is delivered.
//!
//! Dismissal: Esc (a local `onkeydown` on the focusable overlay root, with an
//! app-root keyboard fallback in [`crate::keyboard`] for WebKitGTK focus quirks),
//! a genuine backdrop click, or the `✕` button — each clears the overlay stack.
//! The "OPEN FULL" button is the escape hatch to the existing full-swap drill: it
//! moves the overlay frames onto the drill stack and clears the overlay.
//!
//! This is the CONTAINED-overlay approach (the brief's bounded fallback): rather
//! than extracting a shared `CanvasSurface` from the ~1240-line `CanvasPanel`
//! (high regression risk to the main canvas), the overlay COMPOSES the existing
//! [`super::node::CanvasNode`] + [`super::connector::Connector`] components against
//! a body view derived here. All view derivation lives in
//! [`super::panel::build_body_canvas`]; the cards/connectors are the same
//! components the main canvas uses. The in-overlay lineage ribbon and the full
//! `CanvasSurface` extraction are a documented Phase-2 follow-up.

use dioxus::html::geometry::WheelDelta;
use dioxus::prelude::*;

use crate::pipeline_view::derive_body_view_unlaid;
use crate::state::{promote_overlay_to_drill, use_app_state};

use super::breadcrumbs::{BreadcrumbBar, BreadcrumbTarget};
use super::connector::Connector;
use super::node::CanvasNode;
use super::panel::{
    PanViewport, ZOOM_MAX, ZOOM_MIN, ZOOM_STEP_LINE, ZOOM_STEP_PIXEL, build_body_canvas,
};
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
    let mut zoom = use_signal(|| 1.0_f32);

    // Non-reactive drag state for the inner pan — a hot path that must not
    // re-render per move, so it lives in a `use_hook` cell, mirroring the main
    // canvas drag. `down_on_backdrop` tracks whether the current gesture STARTED
    // on the backdrop, so a pan that begins inside the sub-canvas and overshoots
    // onto the backdrop on release does not dismiss the overlay (#171 review 3).
    let drag = use_hook(|| std::rc::Rc::new(std::cell::RefCell::new(OverlayDrag::default())));

    // Field-lineage contexts the reused `CanvasNode`/`FieldRowView` consume. The
    // overlay provides its OWN, isolated from the parent canvas's contexts, so a
    // hover inside the overlay never lights the parent and vice versa.
    //
    // Phase 1 does NOT draw the in-overlay field-lineage ribbon, so these contexts
    // are effectively write-only (the cards publish hover/pin into them, but the
    // overlay never reads them back to render cables). They exist because
    // `CanvasNode` requires them to mount. The body-canvas `use_memo` below reads
    // only the stack / plan / zoom signals — NOT these context signals — so a hover
    // does not invalidate it and the O(nodes+edges) layout never re-runs on hover.
    // Wiring the ribbon to read these is a Phase-2 follow-up (the `CanvasSurface`
    // extraction).
    use_context_provider(|| {
        CanvasHover(
            Signal::new(HoverTarget::None),
            Signal::new(HoverTarget::None),
            Signal::new(0),
            Signal::new(false),
        )
    });
    use_context_provider(|| PinnedField(Signal::new(None)));

    // Body render model — an UNCONDITIONAL, REACTIVE `use_memo` (#171 review):
    // it runs every render (above any early return, so it is never a conditional
    // hook), and it READS the selecting signals INSIDE the closure so `Memo` knows
    // to recompute when they go dirty:
    //   - `composition_overlay_stack` → recompute when the displayed frame changes
    //     to a DIFFERENT body (a nested push or a breadcrumb truncate AT THE SAME
    //     zoom must re-derive — keying on a captured `body_id` would miss this and
    //     show the previous body under the new breadcrumb);
    //   - `compiled_plan` → recompute when the plan loads/changes;
    //   - `zoom` → recompute when the Auto display density changes.
    // It deliberately does NOT read the `CanvasHover`/`PinnedField` context signals,
    // so a hover never invalidates it — that preserves the win of NOT re-running the
    // O(nodes+edges) layout + obstacle routing on every hover. Returns `None` only
    // when the overlay is CLOSED (no frame); when a frame exists but the body can't
    // resolve it falls back to an empty canvas (mirroring `current_pipeline_view`),
    // so the overlay chrome still shows. `derive_body_view_unlaid` skips the wasted
    // barycenter pass since `build_body_canvas` re-lays-out anyway.
    let canvas = use_memo(move || {
        let stack = state.composition_overlay_stack.read();
        let frame = stack.last()?;
        let view = {
            let compiled_guard = state.compiled_plan.read();
            compiled_guard
                .as_ref()
                .and_then(|plan| plan.body_of(frame.body_id))
                .map(derive_body_view_unlaid)
                .unwrap_or_default()
        };
        Some(build_body_canvas(view, *zoom.read()))
    });

    // Breadcrumb labels + depth come from the SAME stack read in the component body
    // (so the component re-renders on navigation); the early return below gates on
    // the memo, which is `None` exactly when the overlay is closed.
    let stack = state.composition_overlay_stack.read().clone();
    let breadcrumb_frames: Vec<String> = stack.iter().map(|f| f.alias.clone()).collect();
    let depth = stack.len();
    drop(stack);
    let Some(canvas) = canvas() else {
        // No overlay open — render nothing. The memo (a hook) already ran above, so
        // hook order is identical whether or not the overlay is showing.
        return rsx! {};
    };
    // The memo yields an OWNED `BodyCanvas` (it is `Clone`); destructure it into the
    // pieces the rsx renders. `svg_w`/`svg_h` are `Copy`; `cards`/`connections` are
    // moved into the loops below.
    let super::panel::BodyCanvas {
        cards,
        connections,
        svg_w,
        svg_h,
    } = canvas;

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

    // Inner zoom — cursor-anchored, mirroring the main canvas wheel handler
    // (`panel.rs::on_wheel`) with the same clamp range and step constants, so the
    // overlay zooms independently of the parent (#171 review 1). Anchoring keeps
    // the point under the cursor fixed in world space while scaling.
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

    // Discrete zoom step for the header +/- buttons (#171 review 1). Anchors on
    // the overlay's CENTER (the buttons have no cursor position) so the body grows
    // about the middle of the lightbox; same clamp range as the wheel path. These
    // buttons are the engine-independent zoom control — WebKitGTK does not reliably
    // deliver `wheel` to a `position:fixed` overlay, so the wheel path above is a
    // best-effort on engines that do (the web target), and these always work.
    // `Fn` (not `FnMut`) so it can be shared by both the `+` and `-` buttons: it
    // copies the `Copy` signals locally before mutating, so calling it needs only
    // `&self`. Mirrors the wheel path's clamp, anchored on the overlay center.
    let zoom_step = move |factor: f32| {
        let mut zoom = zoom;
        let mut pan_x = pan_x;
        let mut pan_y = pan_y;
        let old_z = *zoom.peek();
        let new_z = (old_z * factor).clamp(ZOOM_MIN, ZOOM_MAX);
        if (new_z - old_z).abs() < 0.0001 {
            return;
        }
        // Anchor on the overlay region's center. The exact center is unknown
        // without a measurement; the visible body is small and re-centred by the
        // depth re-key, so anchoring on a fixed nominal center keeps it stable.
        let (cx, cy) = (700.0_f32, 450.0_f32);
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
        // Backdrop — fixed full-viewport, sits ABOVE the still-mounted parent
        // canvas. Its blur + semi-opaque fill dim the parent; its pointer-events
        // intercept parent interaction. `tabindex` + `autofocus` make it focusable
        // so the local Esc handler fires; `crate::keyboard` is the fallback path.
        div {
            class: "klinx-body-overlay-backdrop",
            tabindex: "0",
            autofocus: true,
            // Each gesture's mousedown sets `down_on_backdrop` fresh: the BACKDROP
            // mousedown arms it (true), the PANEL mousedown below disarms it (false).
            // Because every gesture begins with exactly one of these two mousedowns,
            // the flag always reflects where the CURRENT gesture STARTED — it can
            // never go stale (Bug 3: a backdrop-press that releases on the panel
            // would otherwise leave it armed for a later pan that overshoots onto the
            // backdrop). So a pan starting in the sub-canvas (panel press → false)
            // never dismisses, and a genuine backdrop press → release dismisses.
            onmousedown: {
                let drag = drag.clone();
                move |_: MouseEvent| {
                    drag.borrow_mut().down_on_backdrop = true;
                }
            },
            onkeydown: move |e: KeyboardEvent| {
                if e.key() == Key::Escape {
                    e.stop_propagation();
                    let mut overlay = state.composition_overlay_stack;
                    overlay.write().clear();
                }
            },
            // Dismiss ONLY when the gesture both pressed and released on the
            // backdrop. The flag is consumed here so the next click re-evaluates.
            onclick: {
                let drag = drag.clone();
                move |_: MouseEvent| {
                    let was_backdrop = {
                        let mut d = drag.borrow_mut();
                        let v = d.down_on_backdrop;
                        d.down_on_backdrop = false;
                        v
                    };
                    if was_backdrop {
                        let mut overlay = state.composition_overlay_stack;
                        overlay.write().clear();
                    }
                }
            },

            // Overlay panel — large, near-fullscreen. `stop_propagation` on both
            // mousedown and click keeps a press/click inside the panel from bubbling
            // to the backdrop. The mousedown ALSO disarms `down_on_backdrop`, so any
            // gesture that starts inside the panel (incl. a sub-canvas pan) can never
            // dismiss on release — even if it overshoots onto the backdrop (Bug 3).
            div {
                class: "klinx-body-overlay",
                onmousedown: {
                    let drag = drag.clone();
                    move |e: MouseEvent| {
                        drag.borrow_mut().down_on_backdrop = false;
                        e.stop_propagation();
                    }
                },
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

                    // ── Independent zoom controls (#171 review 1) ───────────
                    // The overlay sub-canvas zooms independently of the parent.
                    // These buttons drive the LOCAL `zoom` signal; the `onwheel`
                    // on the canvas region below mirrors the main canvas wheel-zoom
                    // where the engine delivers wheel to the overlay.
                    button {
                        class: "klinx-body-overlay-btn",
                        title: "Zoom out",
                        onclick: move |_| zoom_step(1.0 / ZOOM_STEP_LINE),
                        "\u{2212}"
                    }
                    button {
                        class: "klinx-body-overlay-btn",
                        title: "Reset zoom",
                        onclick: reset_zoom,
                        "1:1"
                    }
                    button {
                        class: "klinx-body-overlay-btn",
                        title: "Zoom in",
                        onclick: move |_| zoom_step(ZOOM_STEP_LINE),
                        "+"
                    }

                    // Escape hatch to the existing full-swap drill: move the
                    // overlay frames onto the drill stack, then clear the overlay.
                    // Sequential writes (take + drop each guard in turn) avoid
                    // holding two live `write()` borrows at once (#171 review 6).
                    button {
                        class: "klinx-body-overlay-btn",
                        title: "Open this body in the full canvas",
                        onclick: move |_| {
                            let mut overlay = state.composition_overlay_stack;
                            let mut drill = state.composition_drill_stack;
                            let mut frames = overlay.peek().clone();
                            {
                                let mut drill_frames = drill.write();
                                promote_overlay_to_drill(&mut frames, &mut drill_frames);
                            }
                            overlay.write().clear();
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
                // clips to the overlay region and the world-space cards pan/zoom
                // within the lightbox, not the screen.
                div {
                    class: "klinx-body-overlay-canvas",
                    onmousedown: drag_down,
                    onmousemove: drag_move,
                    onmouseup: drag_up,
                    onmouseleave: drag_leave,
                    onwheel: on_wheel,

                    // Re-key by depth so navigating a breadcrumb level remounts the
                    // viewport (and resets its pan) for the newly-shown body.
                    PanViewport {
                        key: "{depth}",
                        pan_x,
                        pan_y,
                        zoom,

                        // NOTE: this card/connector/SVG rsx is hand-copied from
                        // `panel.rs` (the contained-overlay cost). It hardcodes
                        // `dimmed:false` and DROPS the main canvas's Filter-mode
                        // `filter_keep_nodes` gate and the `--recede` hover class —
                        // Phase 1 draws the body's plain node-level DAG only. The
                        // future `CanvasSurface` extraction supersedes this copy and
                        // restores the reveal behavior in the overlay (Phase 2).
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
/// per-move re-render). `down_on_backdrop` additionally distinguishes a genuine
/// backdrop press from a pan that overshoots onto the backdrop on release, so the
/// latter does not dismiss the overlay (#171 review 3).
#[derive(Default)]
struct OverlayDrag {
    active: bool,
    start_x: f32,
    start_y: f32,
    start_pan_x: f32,
    start_pan_y: f32,
    down_on_backdrop: bool,
}
