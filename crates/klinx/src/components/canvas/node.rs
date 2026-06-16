use dioxus::prelude::*;

use crate::pipeline_view::{FieldKind, HEADER_PORT_Y, NODE_WIDTH, StageKind, StageView};
use crate::state::{CompositionDrillFrame, use_app_state};

use super::{CanvasHover, HoverTarget, PinnedField};

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct FieldDisplayInfo {
    pub total_count: usize,
    pub hidden_count: usize,
    pub query: String,
    pub searchable: bool,
    pub can_reduce: bool,
}

/// A single pipeline stage rendered as a rustpunk node card on the canvas.
///
/// `index` is the stage's position in the current [`crate::pipeline_view::PipelineView`]
/// — the identity a [`crate::pipeline_view::FieldEdge`] uses for its endpoints, so
/// a card hover reports `HoverTarget::Node(index)` and a field-row hover reports
/// `HoverTarget::Field(index, name)` to the canvas's [`super::CanvasHover`]
/// context. `dimmed` fades the card when a field's lineage is being revealed and
/// this node is outside that closure.
///
/// `highlighted_fields` names THIS card's own field rows that are lineage
/// endpoints of the active hover reveal (#87) — the rows to tint so a reader of
/// a multi-field node sees *which* row is the source/target, not just that the
/// card participates. The canvas pre-groups endpoints by node and hands each card
/// exactly its own names (a small, sorted, de-duplicated list), so there is no
/// per-node scan of a global set. Empty when nothing is hovered (or none of this
/// card's rows are endpoints). Looked up via a `HashSet<&str>` built once below.
#[component]
pub fn CanvasNode(
    stage: StageView,
    index: usize,
    field_display: FieldDisplayInfo,
    on_field_query: EventHandler<String>,
    on_field_toggle: EventHandler<()>,
    dimmed: bool,
    highlighted_fields: Vec<String>,
) -> Element {
    let state = use_app_state();
    let kind_attr = stage.kind.kind_attr();
    let badge = stage.kind.badge_label();
    let stage_id = stage.id.clone();
    let is_composition = matches!(&stage.kind, StageKind::Composition);

    let is_selected = state.selected_stages.read().contains(stage_id.as_str());
    let is_error = matches!(&stage.kind, StageKind::Error);

    // Hover + pin contexts for the field-lineage reveal. Acquired once in the
    // component body (hooks must not run inside event handlers/conditionals); the
    // inner `Signal` is `Copy`, so the handlers capture them directly.
    let mut hovered = use_context::<CanvasHover>();
    let mut pinned = use_context::<PinnedField>();

    // Local collapse state — a field-bearing card can be folded back to the
    // compact header-only look. Local (not app-state) because collapse is a
    // pure view concern with no model meaning. Cards without fields never show
    // the toggle, so this signal is inert for them.
    let mut collapsed = use_signal(|| false);
    let has_fields = field_display.total_count > 0;
    // Route and Cull nodes carry extra output ports (rendered as branch ports
    // below the columns): a Route's condition/default branches, or a Cull's
    // `removed_to` side-output.
    let has_branches = !stage.branches.is_empty();
    // A Route's outputs ARE its branch ports, so it has no node-level output
    // port; a Cull keeps its node-level main output ALONGSIDE the side-output.
    let keeps_node_out = stage.keeps_node_output_port();
    let show_rows = !stage.fields.is_empty() && !*collapsed.read();
    let show_branches = has_branches && !*collapsed.read();
    let toggle_display = field_display.hidden_count > 0 || field_display.can_reduce;
    let toggle_label = if field_display.hidden_count > 0 {
        format!("+{} more", field_display.hidden_count)
    } else {
        "less".to_string()
    };
    let toggle_title = if field_display.hidden_count > 0 {
        "Show all matching fields"
    } else {
        "Return to capped field list"
    };

    // Lineage-endpoint lookup (#87): build the set ONCE per card, so testing each
    // field row is O(1) rather than scanning `highlighted_fields` per row. Empty
    // (allocation-free) when nothing is hovered or this card has no endpoints.
    let highlighted: std::collections::HashSet<&str> =
        highlighted_fields.iter().map(String::as_str).collect();

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
    // Node-level port squares sit on the node-name label's mid-line, inline with
    // the name, matching the cable anchors (`port_in`/`port_out` at
    // `HEADER_PORT_Y` from the card top): the square's center lands at
    // `BORDER_TOP + port_y + PORT_HALF == HEADER_PORT_Y`. Per-column field cables
    // and Route branch cables attach at their own row anchors instead.
    let port_y = HEADER_PORT_Y - PORT_HALF - BORDER_TOP;

    rsx! {
        div {
            key: "{stage.id}",
            class: "{node_class}",
            "data-stage-kind": kind_attr,
            style: "{border_style}",
            onmousedown: move |e: MouseEvent| e.stop_propagation(),
            // Node-scope lineage reveal (#72): entering the card (off any row)
            // reveals this node's identity carries — but only when the card shows
            // field rows (a collapsed/field-less card has no rendered anchors, so
            // its carries would draw to phantom positions → reveal nothing).
            //
            // The `node() != Some(index)` guard is load-bearing: when the pointer
            // arrives directly on a field row, BOTH this card's `onmouseenter`
            // (Node) and the row's `onmouseenter` (Field) fire, and their relative
            // order is NOT guaranteed under WebKitGTK (it fires the card enter even
            // on a move that lands on a child). Without the guard a card-enter that
            // runs after the row-enter clobbers the row's `Field` reveal back to the
            // whole-node overview. The guard makes card-enter claim node-scope only
            // when the hover is not already anchored on this card, so a row's
            // `Field` always wins regardless of event order.
            onmouseenter: move |_| {
                if !show_rows {
                    hovered.0.set(HoverTarget::None);
                } else if hovered.0.peek().node() != Some(index) {
                    hovered.0.set(HoverTarget::Node(index));
                }
            },
            // Leaving the card clears only THIS node's hover. The node-index
            // guard keeps the reset order-independent: if the pointer jumped onto
            // another card whose enter already set the new target, this stale
            // leave does not clobber it. `peek` reads without subscribing.
            onmouseleave: move |_| {
                if hovered.0.peek().node() == Some(index) {
                    hovered.0.set(HoverTarget::None);
                }
            },
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
                    if field_display.searchable {
                        input {
                            class: "klinx-node-field-search",
                            r#type: "search",
                            value: "{field_display.query}",
                            placeholder: "filter",
                            title: "Filter fields",
                            onmousedown: move |e: MouseEvent| e.stop_propagation(),
                            onclick: move |e: MouseEvent| e.stop_propagation(),
                            oninput: move |e: FormEvent| on_field_query.call(e.value()),
                        }
                    }
                    if toggle_display {
                        button {
                            class: "klinx-node-more-btn",
                            title: "{toggle_title}",
                            onclick: move |e: MouseEvent| {
                                e.stop_propagation();
                                on_field_toggle.call(());
                            },
                            "{toggle_label}"
                        }
                    }
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
                                // hover that targets this node — and a Node-scope or
                                // pinned reveal would now point at phantom row
                                // anchors. Clear both here to avoid a stuck lineage
                                // reveal; a later pointer move / click re-establishes.
                                if !now {
                                    if hovered.0.peek().node() == Some(index) {
                                        hovered.0.set(HoverTarget::None);
                                    }
                                    if matches!(&*pinned.0.peek(), Some((n, _)) if *n == index) {
                                        pinned.0.set(None);
                                    }
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
                    // Downgrade Field→Node on ONE container-level leave, not per
                    // row. Sweeping row→row would otherwise fire a row's leave
                    // before the next row's enter, flashing a transient empty
                    // closure between every row. Rows abut at the fixed
                    // FIELD_ROW_HEIGHT pitch, so moving within the list never
                    // crosses this boundary; the field hover stays Field(A)→Field(B).
                    //
                    // Leaving the list means the pointer is either still inside
                    // this card (moved off the rows onto the header/padding) →
                    // downgrade to the node-scope carry reveal; or it left the
                    // card entirely → the card's own `onmouseleave` fires too and
                    // clears to None. Guarding on `Field(index, _)` makes this
                    // order-independent against the card-leave and against a jump
                    // straight onto another card's row (which already set the new
                    // target). `peek()` reads without subscribing.
                    onmouseleave: move |_| {
                        if matches!(&*hovered.0.peek(), HoverTarget::Field(n, _) if *n == index) {
                            hovered.0.set(HoverTarget::Node(index));
                        }
                    },
                    for (i, field) in stage.fields.iter().enumerate() {
                        FieldRowView {
                            key: "{field.name}",
                            node_index: index,
                            row_index: i,
                            name: field.name.clone(),
                            kind: field.kind,
                            ty: field.ty.clone(),
                            // Tint this cell when it is a lineage endpoint of the
                            // active hover (#87). O(1) lookup in this card's
                            // pre-built endpoint set.
                            highlighted: highlighted.contains(field.name.as_str()),
                            is_correlation_key: field.is_correlation_key,
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
            // A Route node's outputs ARE its branch ports, so its node-level
            // output port is omitted. A Cull keeps its node-level main output
            // (the unremoved groups) alongside the `removed_to` side-output
            // branch port; every non-branching node keeps its single output port.
            if keeps_node_out {
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
/// `HoverTarget::Field(node_index, name)` to the [`super::CanvasHover`] context
/// (upgrading the card's `Node` hover) so the canvas can reveal that field's
/// lineage closure. On leaving the row list a single `onmouseleave` on the parent
/// `.klinx-node-fields` container downgrades back to `Node`, not per row: sweeping
/// row→row stays `Field(A)→Field(B)` with no transient flash.
///
/// `highlighted` tints THIS cell (`klinx-node-field--lineage`) when the row is a
/// lineage endpoint of the active hover reveal (#87) — a calm, static background
/// tint, no motion and no layout shift, so it coexists with the whole-node dim
/// and the inline datatype label.
///
/// `is_correlation_key` (#88) draws a calm, always-on key glyph in a reserved
/// LEADING slot before the field name when the field drives a Source's
/// correlation key. The slot is always present (fixed width) so marked and
/// unmarked rows align identically — no layout jitter when the flag flips.
#[component]
fn FieldRowView(
    node_index: usize,
    row_index: usize,
    name: String,
    kind: FieldKind,
    ty: Option<String>,
    highlighted: bool,
    is_correlation_key: bool,
) -> Element {
    let mut hovered = use_context::<CanvasHover>();
    let mut pinned = use_context::<PinnedField>();

    // This row is the pinned (clicked-to-select) anchor when the pin names it.
    // `read` subscribes the row so it restyles when the pin is set/cleared.
    let is_pinned =
        matches!(&*pinned.0.read(), Some((n, f)) if *n == node_index && f.as_str() == name);

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

    // Full `name : type` for the native tooltip — the name and the type each clip
    // with an ellipsis at the card width, so hovering reveals the untruncated text.
    let tip = match ty.as_ref() {
        Some(t) => format!("{name} : {t}"),
        None => name.clone(),
    };

    // Append modifiers only when active; an inert row keeps exactly its classic
    // class string. `--lineage` tints a hover/pin endpoint cell (#87); `--pinned`
    // marks the clicked-to-select anchor (#75) so the user can see WHICH column
    // the persistent full-lineage reveal is anchored on.
    let mut row_class = String::from("klinx-node-field");
    if highlighted {
        row_class.push_str(" klinx-node-field--lineage");
    }
    if is_pinned {
        row_class.push_str(" klinx-node-field--pinned");
    }

    rsx! {
        div {
            class: "{row_class}",
            "data-field-kind": kind_attr,
            title: "{tip}",
            onmouseenter: {
                // Row-scope reveal (#72): upgrade the card's Node hover to this
                // field's 1-hop closure. `mouseenter` does not bubble, so the
                // card's enter does not re-fire and clobber this.
                let name = name.clone();
                move |_| hovered.0.set(HoverTarget::Field(node_index, name.clone()))
            },
            onclick: {
                // Click-to-select (#75): pin this column to reveal its FULL
                // transitive pipeline lineage, sticky across pointer moves.
                // Clicking the pinned column again unpins it. `stop_propagation`
                // keeps the click from bubbling to the card's node-select handler —
                // a field click selects the FIELD, not the node.
                let name = name.clone();
                move |e: MouseEvent| {
                    e.stop_propagation();
                    let already = matches!(
                        &*pinned.0.peek(), Some((n, f)) if *n == node_index && f.as_str() == name
                    );
                    if already {
                        pinned.0.set(None);
                    } else {
                        pinned.0.set(Some((node_index, name.clone())));
                    }
                }
            },
            span { class: "klinx-node-field-anchor klinx-node-field-anchor--in" }
            // Correlation-key marker (#88): a reserved LEADING slot whose width
            // is fixed in CSS so every row aligns whether or not it carries a
            // key. The shape (a monochrome key glyph in `currentColor`), not
            // color alone, carries the meaning (WCAG 1.4.1); color only
            // reinforces. The glyph (and its `title`/`aria-label`) render only
            // for a CK row; the empty slot still reserves its width so unmarked
            // rows do not shift.
            span {
                class: "klinx-node-field-ck",
                // `role="img"` makes the slot an accessible image so its
                // `aria-label` is reliably exposed (a roleless span does not
                // reliably surface its accessible name); set only on a CK row,
                // alongside the label, so the empty slot stays roleless.
                role: if is_correlation_key { Some("img") } else { None },
                title: if is_correlation_key { Some("Correlation key") } else { None },
                "aria-label": if is_correlation_key { Some("Correlation key") } else { None },
                if is_correlation_key {
                    // Inline SVG (not a 🔑 emoji) for consistent cross-platform,
                    // single-hue rendering. `currentColor` defers the hue to the
                    // CSS class so the marker stays a calm, desaturated tone.
                    svg {
                        class: "klinx-node-field-ck-glyph",
                        view_box: "0 0 16 16",
                        width: "11",
                        height: "11",
                        // Decorative: the meaning is conveyed by the parent
                        // span's `aria-label`, so assistive tech skips the glyph.
                        "aria-hidden": "true",
                        // A key: round bow (ring) on the left, shaft to the right
                        // with two bit teeth. Stroked in `currentColor`.
                        circle {
                            cx: "5",
                            cy: "8",
                            r: "3",
                            fill: "none",
                            stroke: "currentColor",
                            stroke_width: "1.5",
                        }
                        path {
                            d: "M8 8 H14 M12 8 V10.5 M14 8 V11",
                            fill: "none",
                            stroke: "currentColor",
                            stroke_width: "1.5",
                            stroke_linecap: "round",
                            stroke_linejoin: "round",
                        }
                    }
                }
            }
            span { class: "klinx-node-field-name", "{name}" }
            // Compact datatype suffix (e.g. `: float`) when known. Declared and
            // carried columns have a type; emitted columns don't yet (Phase 2b).
            if let Some(t) = ty.as_ref() {
                span { class: "klinx-node-field-type", "{t}" }
            }
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
