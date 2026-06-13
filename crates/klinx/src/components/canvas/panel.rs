use std::cell::RefCell;
use std::rc::Rc;

use dioxus::html::geometry::WheelDelta;
use dioxus::prelude::*;

use crate::pipeline_view::{
    NODE_HEIGHT, NODE_WIDTH, derive_body_view, derive_partial_pipeline_view, derive_pipeline_view,
    fit_transform, layout_bounds,
};
use crate::state::{ChannelViewMode, use_app_state};

use super::connector::Connector;
use super::node::CanvasNode;

// ── Canvas transform constants ───────────────────────────────────────────────
const ZOOM_MIN: f32 = 0.25;
const ZOOM_MAX: f32 = 4.0;
/// Zoom factor applied per scroll-wheel "tick" (for Line/Page delta modes).
const ZOOM_STEP_LINE: f32 = 1.10;
/// Zoom factor per pixel of scroll delta (for Pixel mode).
const ZOOM_STEP_PIXEL: f32 = 0.001;
/// Screen-space padding kept around the node graph when fitting it to view.
const FIT_MARGIN: f32 = 60.0;
/// Fallback viewport dimensions used before the panel reports its real size
/// (the `onmounted` measurement is async). Sized to a typical canvas pane so a
/// fit triggered on the very first frame still produces a sane transform.
const DEFAULT_VIEWPORT_W: f32 = 1000.0;
const DEFAULT_VIEWPORT_H: f32 = 700.0;

// ── Drag state — non-reactive, stored in Rc<RefCell<>> to avoid a signal write
// (and therefore a re-render) on every pointer-move event. ───────────────────
#[derive(Default)]
struct DragState {
    /// Whether a pan drag is currently active.
    active: bool,
    /// Client-space X where the drag began.
    start_x: f32,
    /// Client-space Y where the drag began.
    start_y: f32,
    /// Pan X at drag start — restored as offset when computing current pan.
    start_pan_x: f32,
    /// Pan Y at drag start.
    start_pan_y: f32,
}

/// The infinite-canvas panel rendering the pipeline node graph.
///
/// Pan: left-click-drag anywhere on the canvas background.
/// Zoom: scroll wheel (zoom anchored to cursor position, range 25 %–400 %).
/// Fit-to-view: double-click the canvas, or the toolbar "FIT" button — frames
/// every node in the viewport with a margin. "RESET" re-fits the engine layout.
///
/// Visual layers (back to front):
///   1. Dot grid background (CSS radial-gradient, does NOT transform with content)
///   2. Noise + scanline overlays (CSS ::before / ::after, fixed to panel)
///   3. `klinx-canvas-viewport` div — receives CSS transform(translate + scale)
///      a. SVG connector overlay (absolute, inset: 0, overflow: visible)
///      b. Node cards (absolute, world-space coordinates)
#[component]
pub fn CanvasPanel() -> Element {
    let state = use_app_state();

    let view_mode = *state.channel_view_mode.read();
    let drill_stack = state.composition_drill_stack.read();

    // If drilled into a composition, render the body's nodes instead of top-level.
    let pipeline_view = if let Some(frame) = drill_stack.last() {
        let compiled_guard = state.compiled_plan.read();
        match compiled_guard
            .as_ref()
            .and_then(|plan| plan.body_of(frame.body_id))
        {
            Some(body) => derive_body_view(body),
            None => crate::pipeline_view::PipelineView {
                stages: Vec::new(),
                connections: Vec::new(),
            },
        }
    } else {
        // Top-level: dispatch on view mode
        match view_mode {
            ChannelViewMode::Resolved => {
                let compiled_guard = state.compiled_plan.read();
                match compiled_guard.as_ref() {
                    Some(plan) => derive_pipeline_view(plan.config()),
                    None => match &*(state.pipeline).read() {
                        Some(config) => derive_pipeline_view(config),
                        None => crate::pipeline_view::PipelineView {
                            stages: Vec::new(),
                            connections: Vec::new(),
                        },
                    },
                }
            }
            ChannelViewMode::Raw => match &*(state.pipeline).read() {
                Some(config) => derive_pipeline_view(config),
                None => match &*(state.partial_pipeline).read() {
                    Some(partial) => derive_partial_pipeline_view(partial),
                    None => crate::pipeline_view::PipelineView {
                        stages: Vec::new(),
                        connections: Vec::new(),
                    },
                },
            },
        }
    };
    drop(drill_stack);
    let connections: Vec<_> = pipeline_view
        .connections
        .iter()
        .map(|&(from, to)| {
            (
                pipeline_view.stages[from].clone(),
                pipeline_view.stages[to].clone(),
            )
        })
        .collect();
    let stages = pipeline_view.stages;

    // ── Transform state (local — only the canvas needs these) ────────────────
    let mut pan_x = use_signal(|| 0.0_f32);
    let mut pan_y = use_signal(|| 0.0_f32);
    let mut zoom = use_signal(|| 1.0_f32);

    // ── Viewport pixel size — measured on mount/resize so Fit-to-View can frame
    // the graph against the real pane. Seeded with sane defaults because the
    // `onmounted` measurement is async and may not have run on first fit. ─────
    let mut viewport_w = use_signal(|| DEFAULT_VIEWPORT_W);
    let mut viewport_h = use_signal(|| DEFAULT_VIEWPORT_H);

    // ── Non-reactive drag state — hot path, no re-renders during drag ─────────
    let drag = use_hook(|| Rc::new(RefCell::new(DragState::default())));

    // Bounding box of the current layout (None when empty). `LayoutBounds` is
    // Copy, so each fit handler captures it without cloning the stage list.
    let bounds = layout_bounds(&stages);

    // Fit-to-view shared by the FIT/RESET buttons and the double-click handler.
    // Signals are `Copy`, so each handler captures its own copy and calls this
    // free helper rather than sharing one `FnMut` closure (which can't be moved
    // into multiple handlers). No-op when there are no nodes to frame.
    let fit_to_view =
        move |mut pan_x: Signal<f32>, mut pan_y: Signal<f32>, mut zoom: Signal<f32>| {
            let Some(b) = bounds else { return };
            let (px, py, z) = fit_transform(
                b,
                *viewport_w.peek(),
                *viewport_h.peek(),
                FIT_MARGIN,
                ZOOM_MIN,
                ZOOM_MAX,
            );
            pan_x.set(px);
            pan_y.set(py);
            zoom.set(z);
        };

    // ── Event handler closures ────────────────────────────────────────────────

    let drag_down = {
        let drag = drag.clone();
        move |e: MouseEvent| {
            // Only initiate pan on left-button (button 0) or middle-button (1).
            // Right-click is reserved for a future context menu.
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
                let dx = pos.x as f32 - d.start_x;
                let dy = pos.y as f32 - d.start_y;
                pan_x.set(d.start_pan_x + dx);
                pan_y.set(d.start_pan_y + dy);
            }
        }
    };

    let drag_up = {
        let drag = drag.clone();
        move |_: MouseEvent| {
            drag.borrow_mut().active = false;
        }
    };

    let on_wheel = move |e: WheelEvent| {
        // Compute a zoom multiplier from the wheel delta.
        // Positive delta_y = scroll down = zoom out (< 1).
        // Negative delta_y = scroll up   = zoom in  (> 1).
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

        // Anchor zoom to cursor position (cursor stays fixed in world space).
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

    // SVG overlay bounds — computed from stage bounding box with padding.
    let (svg_w, svg_h) = if stages.is_empty() {
        (1200.0_f32, 400.0_f32)
    } else {
        let max_x = stages
            .iter()
            .map(|s| s.canvas_x + NODE_WIDTH)
            .fold(0.0_f32, f32::max);
        let max_y = stages
            .iter()
            .map(|s| s.canvas_y + NODE_HEIGHT)
            .fold(0.0_f32, f32::max);
        let min_y = stages.iter().map(|s| s.canvas_y).fold(f32::MAX, f32::min);
        // Ensure SVG covers negative-Y nodes (secondary inputs above the chain).
        let _ = min_y; // min_y handled by SVG viewBox if needed; overflow:visible covers it.
        (max_x + 80.0, max_y + 80.0)
    };

    // Channel view mode toggle state
    let tab_mgr = use_context::<crate::state::TabManagerState>();
    let has_channel = tab_mgr
        .channel_state
        .read()
        .as_ref()
        .is_some_and(|cs| cs.active_channel.is_some());
    let is_resolved = view_mode == ChannelViewMode::Resolved;

    rsx! {
        div {
            class: "klinx-canvas-column",

            // ── Breadcrumb bar (composition drill-in navigation) ─────────
            {
                let stack = state.composition_drill_stack.read();
                if !stack.is_empty() {
                    let frames: Vec<_> = stack.iter().map(|f| f.alias.clone()).collect();
                    drop(stack);
                    rsx! {
                        super::breadcrumbs::BreadcrumbBar {
                            frames,
                        }
                    }
                } else {
                    drop(stack);
                    rsx! {}
                }
            }

            // ── Channel view mode toggle bar ─────────────────────────────
            div {
                class: "klinx-canvas-toolbar",

                button {
                    class: if is_resolved { "klinx-view-toggle klinx-view-toggle--active" } else { "klinx-view-toggle" },
                    disabled: !has_channel && !is_resolved,
                    title: if !has_channel && !is_resolved { "Select a channel to enable resolved view" } else if is_resolved { "Switch to Raw view" } else { "Switch to Resolved view" },
                    onclick: move |_| {
                        let mut mode = state.channel_view_mode;
                        let current = *mode.read();
                        mode.set(match current {
                            ChannelViewMode::Raw => ChannelViewMode::Resolved,
                            ChannelViewMode::Resolved => ChannelViewMode::Raw,
                        });
                    },
                    span { class: "klinx-view-toggle-label",
                        if is_resolved { "RESOLVED" } else { "RAW" }
                    }
                }

                // ── Extract as Composition button (enabled with 2+ nodes selected) ──
                {
                    let count = state.selected_stages.read().len();
                    rsx! {
                        button {
                            class: "klinx-view-toggle",
                            disabled: count < 2,
                            title: if count < 2 { "Select 2+ nodes to extract as composition" } else { "Extract selected nodes as a composition" },
                            onclick: move |_| {
                                // TODO: open extraction modal.
                            },
                            span { class: "klinx-view-toggle-label", "EXTRACT" }
                        }
                    }
                }

                // ── Fit to View — frame all nodes in the viewport ──────────
                button {
                    class: "klinx-view-toggle",
                    disabled: bounds.is_none(),
                    title: if bounds.is_none() { "No nodes to fit" } else { "Fit all nodes to the viewport" },
                    onclick: move |_| fit_to_view(pan_x, pan_y, zoom),
                    span { class: "klinx-view-toggle-label", "FIT" }
                }

                // ── Reset Layout — re-run the engine layout and re-fit. With no
                // persisted position overrides yet (issue #3 PR 3b), positions
                // are already recomputed every render, so reset == re-fit. ──
                button {
                    class: "klinx-view-toggle",
                    disabled: bounds.is_none(),
                    title: if bounds.is_none() { "No layout to reset" } else { "Reset to the engine-computed layout" },
                    onclick: move |_| fit_to_view(pan_x, pan_y, zoom),
                    span { class: "klinx-view-toggle-label", "RESET" }
                }
            }

            div {
                class: "klinx-canvas-panel",
            // Measure the pane on mount (guaranteed) and on every resize so
            // Fit-to-View frames the graph against the real viewport size.
            onmounted: move |evt| {
                spawn(async move {
                    if let Ok(rect) = evt.get_client_rect().await {
                        viewport_w.set(rect.size.width.max(1.0) as f32);
                        viewport_h.set(rect.size.height.max(1.0) as f32);
                    }
                });
            },
            onresize: move |evt| {
                if let Ok(size) = evt.get_content_box_size() {
                    viewport_w.set(size.width.max(1.0) as f32);
                    viewport_h.set(size.height.max(1.0) as f32);
                }
            },
            // Events on the outer panel — pointer capture is a future enhancement.
            onmousedown: drag_down,
            onmousemove: drag_move,
            onmouseup: drag_up,
            // Cancel drag if pointer leaves the panel entirely.
            onmouseleave: move |_| { drag.borrow_mut().active = false; },
            onwheel: on_wheel,
            // Double-click empty canvas fits all nodes to view. Node cards
            // stop_propagation() on mousedown, so this fires only on the background.
            ondoubleclick: move |_| fit_to_view(pan_x, pan_y, zoom),
            // Clicking empty canvas deselects any selected node.
            // Node clicks call stop_propagation(), so this only fires on empty space.
            onclick: move |_| {
                let mut sel = state.selected_stages;
                sel.set(std::collections::HashSet::new());
            },

            // ── Transformed viewport ──────────────────────────────────────
            div {
                class: "klinx-canvas-viewport",
                style: "transform: translate({pan_x}px, {pan_y}px) scale({zoom});",

                // SVG connector overlay — rendered first (lower z-index).
                svg {
                    class: "klinx-canvas-svg",
                    width: "{svg_w}",
                    height: "{svg_h}",
                    for (from, to) in connections {
                        Connector {
                            key: "{from.id}-{to.id}",
                            from,
                            to,
                        }
                    }
                }

                // Node cards
                for stage in stages {
                    CanvasNode {
                        key: "{stage.id}",
                        stage,
                    }
                }
            }
        }
        }
    }
}
