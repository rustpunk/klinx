use dioxus::prelude::*;

use crate::pipeline_view::{NODE_HEIGHT, NODE_WIDTH, StageKind, StageView};
use crate::state::{CompositionDrillFrame, use_app_state};

/// A single pipeline stage rendered as a rustpunk node card on the canvas.
#[component]
pub fn CanvasNode(stage: StageView) -> Element {
    let state = use_app_state();
    let kind_attr = stage.kind.kind_attr();
    let badge = stage.kind.badge_label();
    let stage_id = stage.id.clone();
    let is_composition = matches!(&stage.kind, StageKind::Composition);

    let is_selected = state.selected_stages.read().contains(stage_id.as_str());
    let is_error = matches!(&stage.kind, StageKind::Error);

    let node_class = match (is_selected, is_error, is_composition) {
        (_, _, true) => {
            if is_selected {
                "klinx-node klinx-node--selected klinx-node--composition"
            } else {
                "klinx-node klinx-node--composition"
            }
        }
        (true, true, _) => "klinx-node klinx-node--selected klinx-node--error",
        (false, true, _) => "klinx-node klinx-node--error",
        (true, false, _) => "klinx-node klinx-node--selected",
        (false, false, _) => "klinx-node",
    };

    let border_style = format!(
        "left: {x}px; top: {y}px; width: {w}px; border-top-color: var(--klinx-stage-accent);",
        x = stage.canvas_x,
        y = stage.canvas_y,
        w = NODE_WIDTH
    );

    const BORDER_TOP: f32 = 3.0;
    const PORT_HALF: f32 = 4.0;
    let port_y = NODE_HEIGHT / 2.0 - PORT_HALF - BORDER_TOP;

    rsx! {
        div {
            key: "{stage.id}",
            class: "{node_class}",
            "data-stage-kind": kind_attr,
            style: "{border_style}",
            onmousedown: move |e: MouseEvent| e.stop_propagation(),
            onclick: {
                let stage_id = stage_id.clone();
                move |e: MouseEvent| {
                    e.stop_propagation();
                    let mut sel = state.selected_stages;
                    let shift = e.data().modifiers().shift();
                    if shift {
                        // Shift+click: toggle this node in the multi-select set.
                        let mut set = sel.write();
                        if set.contains(stage_id.as_str()) {
                            set.remove(stage_id.as_str());
                        } else {
                            set.insert(stage_id.clone());
                        }
                    } else {
                        // Regular click: single-select toggle.
                        let current = sel.read().clone();
                        if current.len() == 1 && current.contains(stage_id.as_str()) {
                            sel.set(std::collections::HashSet::new());
                        } else {
                            let mut set = std::collections::HashSet::new();
                            set.insert(stage_id.clone());
                            sel.set(set);
                        }
                    }
                }
            },

            div {
                class: "klinx-node-badge",
                style: "color: var(--klinx-stage-accent);",
                span { class: "klinx-node-type-badge", "{badge}" }
            }

            div { class: "klinx-node-label", "{stage.label}" }
            hr { class: "klinx-rust-line" }
            div { class: "klinx-node-subtitle", "{stage.subtitle}" }

            // Drill-in button for composition nodes
            if is_composition {
                button {
                    class: "klinx-node-drill-btn",
                    title: "Drill into composition",
                    onclick: {
                        let stage_id = stage.id.clone();
                        let subtitle = stage.subtitle.clone();
                        move |e: MouseEvent| {
                            e.stop_propagation();
                            drill_into_composition(&state, &stage_id, &subtitle);
                        }
                    },
                    "▶"
                }
            }

            div {
                class: "klinx-node-port klinx-node-port--in",
                style: "top: {port_y}px;",
            }
            div {
                class: "klinx-node-port klinx-node-port--out",
                style: "top: {port_y}px;",
            }
        }
    }
}

/// Push a drill frame for a composition node onto the drill stack.
fn drill_into_composition(state: &crate::state::AppState, node_name: &str, _subtitle: &str) {
    let compiled_guard = state.compiled_plan.read();
    let Some(plan) = compiled_guard.as_ref() else {
        return;
    };

    // Look up the body ID from the compilation artifacts
    let Some(&body_id) = plan.artifacts().composition_body_assignments.get(node_name) else {
        return;
    };

    // Get the use_path from the body
    let use_path = plan
        .body_of(body_id)
        .map(|b| b.signature_path.clone())
        .unwrap_or_default();

    drop(compiled_guard);

    let mut drill = state.composition_drill_stack;
    drill.write().push(CompositionDrillFrame {
        body_id,
        alias: node_name.to_string(),
        use_path,
    });
}
