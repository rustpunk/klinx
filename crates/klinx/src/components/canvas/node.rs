use dioxus::prelude::*;

use crate::pipeline_view::{FieldKind, NODE_HEIGHT, NODE_WIDTH, StageKind, StageView};
use crate::state::{CompositionDrillFrame, use_app_state};

use super::HoveredField;

/// A single pipeline stage rendered as a rustpunk node card on the canvas.
///
/// `index` is the stage's position in the current [`crate::pipeline_view::PipelineView`]
/// — the identity a [`crate::pipeline_view::FieldEdge`] uses for its endpoints,
/// so field-row hover reports `(index, field_name)` to the canvas's
/// [`HoveredField`] context. `dimmed` fades the card when a field's lineage is
/// being revealed and this node is outside that closure.
#[component]
pub fn CanvasNode(stage: StageView, index: usize, dimmed: bool) -> Element {
    let state = use_app_state();
    let kind_attr = stage.kind.kind_attr();
    let badge = stage.kind.badge_label();
    let stage_id = stage.id.clone();
    let is_composition = matches!(&stage.kind, StageKind::Composition);

    let is_selected = state.selected_stages.read().contains(stage_id.as_str());
    let is_error = matches!(&stage.kind, StageKind::Error);

    // Hover context for the field-lineage reveal. Acquired once in the component
    // body (hooks must not run inside event handlers/conditionals); the inner
    // `Signal` is `Copy`, so the container's `onmouseleave` captures it directly.
    let mut hovered = use_context::<HoveredField>();

    // Local collapse state — a field-bearing card can be folded back to the
    // compact header-only look. Local (not app-state) because collapse is a
    // pure view concern with no model meaning. Cards without fields never show
    // the toggle, so this signal is inert for them.
    let mut collapsed = use_signal(|| false);
    let has_fields = !stage.fields.is_empty();
    // Route nodes carry output branches (rendered as ports below the columns).
    let has_branches = !stage.branches.is_empty();
    let show_rows = has_fields && !*collapsed.read();
    let show_branches = has_branches && !*collapsed.read();

    let base_class = match (is_selected, is_error, is_composition) {
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
    // Append the dim modifier when this node is outside the revealed field's
    // lineage closure; an undimmed card keeps exactly its classic class string.
    let node_class = if dimmed {
        format!("{base_class} klinx-node--dimmed")
    } else {
        base_class.to_string()
    };

    let border_style = format!(
        "left: {x}px; top: {y}px; width: {w}px; border-top-color: var(--klinx-stage-accent);",
        x = stage.canvas_x,
        y = stage.canvas_y,
        w = NODE_WIDTH
    );

    const BORDER_TOP: f32 = 3.0;
    const PORT_HALF: f32 = 4.0;
    // Node-level port squares sit at the HEADER's vertical center, inline with
    // the node name, matching the cable anchors (`port_in`/`port_out` at
    // NODE_HEIGHT/2 from the card top). Per-column field cables and Route branch
    // cables attach at their own row anchors instead.
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

            // ── Fixed-height header region ───────────────────────────────────
            // Exactly `FIELD_HEADER_HEIGHT` px tall (CSS), so the field-row
            // region below it begins at that offset from the card top and the
            // SVG world-space anchors line up with the rendered dots. `overflow`
            // is clipped in CSS so a long subtitle never grows the header.
            div { class: "klinx-node-header",
                div {
                    class: "klinx-node-badge",
                    style: "color: var(--klinx-stage-accent);",
                    span { class: "klinx-node-type-badge", "{badge}" }
                    // Collapse/expand toggle — on any card that carries field rows
                    // or Route branch ports.
                    if has_fields || has_branches {
                        button {
                            class: "klinx-node-collapse-btn",
                            title: if *collapsed.read() { "Expand fields" } else { "Collapse fields" },
                            onclick: move |e: MouseEvent| {
                                // Toggle only; never select. Stop propagation so the
                                // card's onclick selection handler does not also fire.
                                e.stop_propagation();
                                let now = *collapsed.read();
                                collapsed.set(!now);
                                // Collapsing unmounts the field rows, so their
                                // container `onmouseleave` can never fire to clear a
                                // hover that targets this node — clear it here to
                                // avoid a permanently stuck lineage reveal.
                                if !now
                                    && matches!(hovered.0.peek().as_ref(), Some((n, _)) if *n == index)
                                {
                                    hovered.0.set(None);
                                }
                            },
                            if *collapsed.read() { "▸" } else { "▾" }
                        }
                    }
                }

                div { class: "klinx-node-label", "{stage.label}" }
                hr { class: "klinx-rust-line" }
                // The subtitle clips with an ellipsis at the fixed card width
                // (`.klinx-node-subtitle` is `overflow:hidden; text-overflow:ellipsis`),
                // so a long value (e.g. a Route's "N branches → default") is otherwise
                // unreadable. A native `title` tooltip surfaces the full text on hover.
                div {
                    class: "klinx-node-subtitle",
                    title: "{stage.subtitle}",
                    "{stage.subtitle}"
                }
            }

            // ── Field-row list (variable height) ─────────────────────────────
            if show_rows {
                div {
                    class: "klinx-node-fields",
                    // Clear the revealed field on ONE container-level leave, not
                    // per row. Sweeping row→row would otherwise fire a row's
                    // leave (hover→None) before the next row's enter, flashing a
                    // transient empty closure between every row. Rows abut at the
                    // fixed FIELD_ROW_HEIGHT pitch, so moving within the list
                    // never crosses this boundary; hover stays Some(A)→Some(B)
                    // and only resets when the pointer truly leaves the fields.
                    onmouseleave: move |_| {
                        // Clear only THIS node's hover. Guarding on the node index
                        // (not a bare `is_some`) keeps the clear order-independent:
                        // if the pointer jumped straight onto another node's row,
                        // that row's `onmouseenter` may set `Some(other)` before this
                        // container's leave fires — wiping on `is_some` would then
                        // erase the newer hover. `peek()` reads without subscribing.
                        if matches!(hovered.0.peek().as_ref(), Some((n, _)) if *n == index) {
                            hovered.0.set(None);
                        }
                    },
                    for (i, field) in stage.fields.iter().enumerate() {
                        FieldRowView {
                            key: "{field.name}",
                            node_index: index,
                            row_index: i,
                            name: field.name.clone(),
                            kind: field.kind,
                        }
                    }
                }
            }

            // ── Route branch ports (variable height) ─────────────────────────
            // A Route node's output ports, stacked directly below the column rows
            // at the same FIELD_ROW_HEIGHT pitch so each branch's right anchor
            // lands on `branch_anchor_out(i)` (where the per-branch cable attaches).
            // The default/fallback branch renders distinctly; a condition branch
            // shows its predicate on hover.
            if show_branches {
                div {
                    class: "klinx-node-branches",
                    for branch in stage.branches.iter() {
                        BranchPortView {
                            key: "{branch.name}",
                            name: branch.name.clone(),
                            predicate: branch.predicate.clone(),
                            is_default: branch.is_default,
                        }
                    }
                }
            }

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
            // A Route node's outputs ARE its branch ports — the shared node-level
            // output port is never used, so omit it (your point 2). Every other
            // node keeps its single node-level output port.
            if !has_branches {
                div {
                    class: "klinx-node-port klinx-node-port--out",
                    style: "top: {port_y}px;",
                }
            }
        }
    }
}

/// One field row inside an expanded node card.
///
/// Carries LEFT and RIGHT anchor dots (the visual endpoints field-edge cables
/// connect to) and `data-field-kind` for CSS. Hovering the row publishes
/// `(node_index, name)` to the [`HoveredField`] context so the canvas can reveal
/// that field's lineage closure. The reveal is *cleared* by a single
/// `onmouseleave` on the parent `.klinx-node-fields` container, not per row:
/// sweeping row→row stays `Some(A)→Some(B)` with no transient `None` flash.
#[component]
fn FieldRowView(node_index: usize, row_index: usize, name: String, kind: FieldKind) -> Element {
    let mut hovered = use_context::<HoveredField>();

    let kind_attr = match kind {
        FieldKind::Declared => "declared",
        FieldKind::Emitted => "emitted",
        FieldKind::PassThrough => "passthrough",
    };

    // The row is exactly `FIELD_ROW_HEIGHT` px tall (CSS, box-sizing:border-box)
    // and starts at `FIELD_HEADER_HEIGHT + row_index*FIELD_ROW_HEIGHT` from the
    // card top, so its CSS-centered anchor dots (`top:50%; translateY(-50%)`)
    // sit at exactly `field_row_y(row_index)` in world space. `row_index` is no
    // longer read here (the geometry is fully CSS-driven) but stays on the props
    // as the row's stable identity for keying and future per-row styling.
    let _ = row_index;

    rsx! {
        div {
            class: "klinx-node-field",
            "data-field-kind": kind_attr,
            onmouseenter: {
                let name = name.clone();
                move |_| hovered.0.set(Some((node_index, name.clone())))
            },
            span { class: "klinx-node-field-anchor klinx-node-field-anchor--in" }
            span { class: "klinx-node-field-name", "{name}" }
            span { class: "klinx-node-field-anchor klinx-node-field-anchor--out" }
        }
    }
}

/// One output-branch port on a Route node card.
///
/// Renders below the column rows at the same row pitch, carrying a RIGHT (output)
/// anchor that the downstream cable for `route.name` attaches to (matching
/// [`crate::pipeline_view::StageView::branch_anchor_out`]). The default/fallback
/// branch is styled distinctly via `data-default`. A condition branch surfaces
/// its CXL predicate on hover via a native `title` tooltip (details-on-demand —
/// the predicate is never crammed onto the card).
#[component]
fn BranchPortView(name: String, predicate: Option<String>, is_default: bool) -> Element {
    // Tooltip: the branch's predicate, or a note that the default catches the
    // records no condition matched.
    let tip = match &predicate {
        Some(p) => p.clone(),
        None => "default — records matching no condition".to_string(),
    };

    rsx! {
        div {
            class: "klinx-node-branch",
            "data-default": if is_default { "true" } else { "false" },
            title: "{tip}",
            span { class: "klinx-node-branch-label", "{name}" }
            span { class: "klinx-node-branch-anchor" }
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
