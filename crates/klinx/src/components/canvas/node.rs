use dioxus::prelude::*;

use crate::pipeline_view::{
    FieldKind, HEADER_PORT_Y, NODE_WIDTH, Precision, StageKind, StagePortKind, StagePortRow,
    StagePortSide, StageView,
};
use crate::state::SelectedField;
use crate::state::{resolve_composition_frame, use_app_state};

use super::{CanvasHover, LineageTarget, PinnedField};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum GlobalNodeDisplayMode {
    #[default]
    Auto,
    Compact,
    Preview,
    Schema,
    Full,
}

impl GlobalNodeDisplayMode {
    pub(super) const ALL: [Self; 5] = [
        Self::Auto,
        Self::Compact,
        Self::Preview,
        Self::Schema,
        Self::Full,
    ];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Auto => "AUTO",
            Self::Compact => "COMPACT",
            Self::Preview => "PREVIEW",
            Self::Schema => "SCHEMA",
            Self::Full => "FULL",
        }
    }

    pub(super) fn title(self) -> &'static str {
        match self {
            Self::Auto => "Adaptive node detail",
            Self::Compact => "Header and topology ports",
            Self::Preview => "Ranked high-signal fields",
            Self::Schema => "Schema rows with wide-field cap",
            Self::Full => "All known fields",
        }
    }

    pub(super) fn resolved(self) -> Option<ResolvedNodeDisplayMode> {
        match self {
            Self::Auto => None,
            Self::Compact => Some(ResolvedNodeDisplayMode::Compact),
            Self::Preview => Some(ResolvedNodeDisplayMode::Preview),
            Self::Schema => Some(ResolvedNodeDisplayMode::Schema),
            Self::Full => Some(ResolvedNodeDisplayMode::Full),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum ResolvedNodeDisplayMode {
    #[default]
    Compact,
    Preview,
    Schema,
    Full,
}

impl ResolvedNodeDisplayMode {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Compact => "Compact",
            Self::Preview => "Preview",
            Self::Schema => "Schema",
            Self::Full => "Full",
        }
    }

    fn short_label(self) -> &'static str {
        match self {
            Self::Compact => "C",
            Self::Preview => "P",
            Self::Schema => "S",
            Self::Full => "F",
        }
    }

    pub(super) fn next_override(self) -> Option<Self> {
        match self {
            Self::Compact => Some(Self::Preview),
            Self::Preview => Some(Self::Schema),
            Self::Schema => Some(Self::Full),
            Self::Full => None,
        }
    }

    fn as_data_attr(self) -> &'static str {
        match self {
            Self::Compact => "compact",
            Self::Preview => "preview",
            Self::Schema => "schema",
            Self::Full => "full",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NodeDisplayAction {
    CycleOverride,
    ClearOverride,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct FieldDisplayInfo {
    pub total_count: usize,
    pub matching_count: usize,
    pub hidden_count: usize,
    pub next_count: usize,
    pub temporary_fields: Vec<String>,
    pub query: String,
    pub searchable: bool,
    pub can_reduce: bool,
    pub mode: ResolvedNodeDisplayMode,
    pub global_mode: GlobalNodeDisplayMode,
    pub override_mode: Option<ResolvedNodeDisplayMode>,
}

/// A single pipeline stage rendered as a rustpunk node card on the canvas.
///
/// `index` is the stage's position in the current [`crate::pipeline_view::PipelineView`]
/// — the identity a [`crate::pipeline_view::FieldEdge`] uses for its endpoints, so
/// a field-row hover reports `HoverTarget::Field(index, name)` to the canvas's
/// [`super::CanvasHover`] context after a cold-entry dwell, then instantly while
/// warm. Plain card hover does not reveal field connectors. `dimmed` fades the
/// card when a field's lineage is being revealed and this node is outside that
/// closure (Highlight mode). `hidden` removes the card entirely when Filter mode
/// is suppressing off-path nodes (#123); the two are mutually exclusive — Filter
/// mode hides instead of dimming.
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
    on_display_action: EventHandler<NodeDisplayAction>,
    dimmed: bool,
    #[props(default = false)] hidden: bool,
    highlighted_fields: Vec<String>,
    highlighted_role_ports: Vec<String>,
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

    let input_role_ports: Vec<StagePortRow> =
        stage.role_ports_on(StagePortSide::Input).cloned().collect();
    let input_role_section = role_port_section_attr(&input_role_ports);
    let input_role_has_header = input_role_section == "group-by";
    let has_fields = field_display.total_count > 0;
    let has_input_roles = !input_role_ports.is_empty();
    // Node-level lineage precision (#148): the WORST tier across the card's field
    // rows, so a single degraded field marks the whole node. Rendered as a subtle
    // hatched corner shown ONLY on selection/hover (CSS-gated — see
    // `.klinx-node-precision-corner`); a node with no degraded field stays Exact
    // and the corner reads as a faint clean marker rather than a warning. `None`
    // for a card with NO field rows (no lineage data at all) — suppressing the
    // corner there avoids implying a precision verdict for a node that has none.
    // Computed once from the static `stage.fields`, so it adds no signal
    // subscription and does not defeat the card's `PartialEq` memoization.
    let node_precision = stage
        .fields
        .iter()
        .map(|field| field.lineage_precision)
        .reduce(Precision::worst);
    // Route and Cull nodes carry extra output ports (rendered as branch ports
    // below the columns): a Route's condition/default branches, or a Cull's
    // `removed_to` side-output.
    let has_branches = !stage.branches.is_empty();
    // A Route's outputs ARE its branch ports, so it has no node-level output
    // port; a Cull keeps its node-level main output ALONGSIDE the side-output.
    let keeps_node_out = stage.keeps_node_output_port();
    let show_input_roles = has_input_roles;
    let show_rows = !stage.fields.is_empty();
    let show_branches = has_branches;
    let show_field_input_anchors = !matches!(&stage.kind, StageKind::Source);
    let show_field_output_anchors = !matches!(&stage.kind, StageKind::Output);
    let field_tools_visible = field_display.searchable;
    let toggle_display = field_display.hidden_count > 0 || field_display.can_reduce;
    let toggle_label = if field_display.hidden_count > 0 {
        format!("Show {} more", field_display.next_count)
    } else {
        "Show fewer".to_string()
    };
    let toggle_title = if field_display.hidden_count > 0 {
        "Show more matching fields"
    } else {
        "Return to capped field list"
    };
    let visible_field_count = field_display
        .matching_count
        .saturating_sub(field_display.hidden_count);
    let count_label = if field_display.query.trim().is_empty()
        || field_display.matching_count == field_display.total_count
    {
        format!("{visible_field_count}/{}", field_display.total_count)
    } else {
        format!("{visible_field_count}/{}", field_display.matching_count)
    };
    let count_title = if field_display.query.trim().is_empty()
        || field_display.matching_count == field_display.total_count
    {
        format!(
            "{visible_field_count} of {} fields shown",
            field_display.total_count
        )
    } else {
        format!(
            "{visible_field_count} of {} matching fields shown",
            field_display.matching_count
        )
    };

    // Lineage-endpoint lookup (#87): build the set ONCE per card, so testing each
    // field row is O(1) rather than scanning `highlighted_fields` per row. Empty
    // (allocation-free) when nothing is hovered or this card has no endpoints.
    let highlighted: std::collections::HashSet<&str> =
        highlighted_fields.iter().map(String::as_str).collect();
    let highlighted_role_ports: std::collections::HashSet<&str> =
        highlighted_role_ports.iter().map(String::as_str).collect();
    let temporary: std::collections::HashSet<&str> = field_display
        .temporary_fields
        .iter()
        .map(String::as_str)
        .collect();

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
    // lineage closure (Highlight mode); an undimmed card keeps exactly its classic
    // class string. Filter mode hides the card instead (#123) — `--hidden` sets
    // `display:none`, removing it from layout while keeping its key/identity stable
    // so the component memoizes and re-appears unchanged when the reveal clears.
    let mut node_class = base_class.to_string();
    if hidden {
        node_class.push_str(" klinx-node--hidden");
    }
    if dimmed {
        node_class.push_str(" klinx-node--dimmed");
    }
    if field_display.override_mode.is_some() {
        node_class.push_str(" klinx-node--display-override");
    }

    let display_button_label = field_display
        .override_mode
        .map_or("A", ResolvedNodeDisplayMode::short_label);
    let display_button_title = match field_display.override_mode {
        Some(mode) => format!(
            "Node display override: {}. Click to cycle; Shift-click to use Auto.",
            mode.label()
        ),
        None => format!(
            "Node display: Auto → {} from global {}. Click to override.",
            field_display.mode.label(),
            field_display.global_mode.label()
        ),
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
            "data-node-display": field_display.mode.as_data_attr(),
            style: "{border_style}",
            onmousedown: move |e: MouseEvent| e.stop_propagation(),
            // Leaving the card clears only THIS node's hover. The node-index
            // guard keeps the reset order-independent: if the pointer jumped onto
            // another card whose enter already set the new target, this stale
            // leave does not clobber it. `peek` reads without subscribing.
            onmouseleave: move |_| {
                hovered.close_if_node(index);
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
                    let mut selected_field = state.selected_field;
                    selected_field.set(None);
                }
            },

            // ── Lineage-precision corner affordance (#148) ───────────────────
            // A subtle hatched top-right corner conveying the node's worst field
            // precision. Hidden by default and revealed ONLY on selection or hover
            // via CSS (`.klinx-node--selected`/`.klinx-node:hover`), so it never
            // adds always-on badge fatigue; `data-precision` drives the hatch hue.
            // Omitted entirely for a card with no field rows (no lineage data).
            if let Some(node_precision) = node_precision {
                div {
                    class: "klinx-node-precision-corner",
                    "data-precision": node_precision.precision_attr(),
                    title: "lineage precision: {node_precision.precision_label()}",
                }
            }

            // ── Fixed-height header region ───────────────────────────────────
            // Exactly `FIELD_HEADER_HEIGHT` px tall (CSS), so the field-row
            // region below it begins at that offset from the card top and the
            // SVG world-space anchors line up with the rendered dots. `overflow`
            // is clipped in CSS so a long subtitle never grows the header.
            div { class: if field_tools_visible { "klinx-node-header klinx-node-header--with-field-tools" } else { "klinx-node-header" },
                div {
                    class: "klinx-node-badge",
                    style: "color: var(--klinx-stage-accent);",
                    span { class: "klinx-node-type-badge", "{badge}" }
                    // Display-mode control. The panel owns the actual projection so
                    // card geometry, anchors, and rendered rows stay synchronized.
                    if has_fields || has_input_roles || has_branches {
                        button {
                            class: "klinx-node-display-btn",
                            title: "{display_button_title}",
                            onclick: move |e: MouseEvent| {
                                e.stop_propagation();
                                let action = if e.data().modifiers().shift() {
                                    NodeDisplayAction::ClearOverride
                                } else {
                                    NodeDisplayAction::CycleOverride
                                };
                                on_display_action.call(action);
                            },
                            "{display_button_label}"
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

                if field_tools_visible {
                    div {
                        class: "klinx-node-field-tools",
                        input {
                            class: "klinx-node-field-search",
                            r#type: "search",
                            value: "{field_display.query}",
                            placeholder: "filter fields",
                            title: "Filter fields",
                            onmousedown: move |e: MouseEvent| e.stop_propagation(),
                            onclick: move |e: MouseEvent| e.stop_propagation(),
                            oninput: move |e: FormEvent| on_field_query.call(e.value()),
                        }
                    }
                }
            }

            // ── Input role ports (variable height) ───────────────────────────
            if show_input_roles {
                div {
                    class: "klinx-node-role-ports",
                    "data-role-section": input_role_section,
                    onmouseleave: move |_| {
                        hovered.close_if_node(index);
                    },
                    if input_role_has_header {
                        div {
                            class: "klinx-node-role-section-header",
                            "GROUP_BY"
                        }
                    }
                    for port in input_role_ports.iter() {
                        RolePortRowView {
                            key: "{port.id}",
                            node_index: index,
                            port: port.clone(),
                            highlighted: highlighted_role_ports.contains(port.id.as_str()),
                        }
                    }
                }
            }

            // ── Field-row list (variable height) ─────────────────────────────
            if show_rows {
                div {
                    class: "klinx-node-fields",
                    // Clear/cancel on ONE container-level leave, not per row.
                    // Sweeping row→row would otherwise fire a row's leave before
                    // the next row's enter, flashing a transient empty closure
                    // between every row. Rows abut at the fixed FIELD_ROW_HEIGHT
                    // pitch, so moving within the list never crosses this
                    // boundary; an active/warm field hover can move row-to-row
                    // instantly until the pointer leaves the field area.
                    //
                    // Leaving the list means the pointer is on node chrome or has
                    // left the card; neither should reveal field connectors.
                    // `close_if_node` checks both active and pending hover targets,
                    // so stale leave events cannot cancel a newer row hover on a
                    // different card.
                    onmouseleave: move |_| {
                        hovered.close_if_node(index);
                    },
                    for (i, field) in stage.fields.iter().enumerate() {
                        FieldRowView {
                            key: "{field.name}",
                            stage_id: stage_id.clone(),
                            node_index: index,
                            row_index: i,
                            name: field.name.clone(),
                            kind: field.kind,
                            ty: field.ty.clone(),
                            // Tint this cell when it is a lineage endpoint of the
                            // active hover (#87). O(1) lookup in this card's
                            // pre-built endpoint set.
                            highlighted: highlighted.contains(field.name.as_str()),
                            temporary: temporary.contains(field.name.as_str()),
                            is_correlation_key: field.is_correlation_key,
                            show_input_anchor: show_field_input_anchors,
                            show_output_anchor: show_field_output_anchors,
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

            if show_rows && toggle_display {
                div {
                    class: "klinx-node-field-footer",
                    button {
                        class: "klinx-node-more-btn",
                        title: "{toggle_title}",
                        onclick: move |e: MouseEvent| {
                            e.stop_propagation();
                            on_field_toggle.call(());
                        },
                        "{toggle_label}"
                    }
                    span {
                        class: "klinx-node-field-footer-count",
                        title: "{count_title}",
                        "{count_label}"
                    }
                }
            }

            // Drill-in button for composition nodes — opens the in-context body
            // overlay (#171). The full-swap drill lives behind the overlay's
            // "OPEN FULL" escape hatch, so the card needs no second button.
            if is_composition {
                button {
                    class: "klinx-node-drill-btn",
                    title: "Open composition body",
                    onclick: {
                        let stage_id = stage.id.clone();
                        move |e: MouseEvent| {
                            e.stop_propagation();
                            open_composition_overlay(&state, &stage_id);
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
/// after a cold-entry dwell so the canvas can reveal that field's lineage closure.
/// On leaving the row list a single `onmouseleave` on the parent
/// `.klinx-node-fields` container clears the hover, not per row: sweeping row→row
/// stays instant once the field hover is warm, with no transient flash.
///
/// `highlighted` tints THIS cell (`klinx-node-field--lineage`) when the row is a
/// lineage endpoint of the active hover reveal (#87) — a calm, static background
/// tint, no motion and no layout shift, so it coexists with the whole-node dim
/// and the inline datatype label.
///
/// `is_correlation_key` (#88) highlights the row's field ports only on marked
/// rows. Unmarked rows reserve no gutter, keeping wide-schema field names close
/// to the left port while preserving source schema order.
///
/// The Aggregate group-by grain is no longer a per-row flag: it is represented
/// exactly once as the INDIRECT `GroupBy` field edge (#147), revealed (dashed /
/// ghosted) when the group-key field is selected, so this row carries no
/// separate grain marker.
#[component]
fn FieldRowView(
    stage_id: String,
    node_index: usize,
    row_index: usize,
    name: String,
    kind: FieldKind,
    ty: Option<String>,
    highlighted: bool,
    temporary: bool,
    is_correlation_key: bool,
    show_input_anchor: bool,
    show_output_anchor: bool,
) -> Element {
    let state = use_app_state();
    let mut hovered = use_context::<CanvasHover>();
    let mut pinned = use_context::<PinnedField>();

    // This row is the pinned (clicked-to-select) anchor when the pin names it.
    // `read` subscribes the row so it restyles when the pin is set/cleared.
    let is_pinned = matches!(
        &*pinned.0.read(),
        Some(LineageTarget::Field(n, f)) if *n == node_index && f.as_str() == name
    );

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
    let mut tip = match ty.as_ref() {
        Some(t) => format!("{name} : {t}"),
        None => name.clone(),
    };
    if is_correlation_key {
        tip.push_str(" · source correlation key");
    }
    if temporary {
        tip.push_str(" · temporarily revealed");
    }

    // Append modifiers only when active; an inert row keeps exactly its classic
    // class string. `--lineage` tints a hover/pin endpoint cell (#87); `--pinned`
    // marks the clicked-to-select anchor (#75) so the user can see WHICH column
    // the persistent full-lineage reveal is anchored on.
    let mut row_class = String::from("klinx-node-field");
    if temporary {
        row_class.push_str(" klinx-node-field--temporary");
    }
    if highlighted {
        row_class.push_str(" klinx-node-field--lineage");
    }
    if is_pinned {
        row_class.push_str(" klinx-node-field--pinned");
    }
    if is_correlation_key {
        row_class.push_str(" klinx-node-field--correlation-key");
    }

    rsx! {
        div {
            class: "{row_class}",
            "data-field-kind": kind_attr,
            title: "{tip}",
            onmouseenter: {
                // Row-scope reveal (#72): request this field's 1-hop closure.
                // Cold entry uses a short dwell; once warm, row-to-row movement
                // applies immediately.
                let name = name.clone();
                move |_| hovered.request_field(node_index, name.clone())
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
                        &*pinned.0.peek(),
                        Some(LineageTarget::Field(n, f)) if *n == node_index && f.as_str() == name
                    );
                    if already {
                        pinned.0.set(None);
                        let mut selected_field = state.selected_field;
                        selected_field.set(None);
                    } else {
                        pinned
                            .0
                            .set(Some(LineageTarget::Field(node_index, name.clone())));
                        let mut selected_field = state.selected_field;
                        selected_field
                            .set(Some(SelectedField::new(stage_id.clone(), name.clone())));
                        let mut selected_stages = state.selected_stages;
                        selected_stages.set(std::collections::HashSet::new());
                    }
                }
            },
            if show_input_anchor {
                span { class: "klinx-node-field-anchor klinx-node-field-anchor--in" }
            }
            span { class: "klinx-node-field-name", "{name}" }
            // Compact datatype suffix (e.g. `: float`) when known. Declared and
            // carried columns have a type; emitted columns don't yet (Phase 2b).
            if let Some(t) = ty.as_ref() {
                span { class: "klinx-node-field-type", "{t}" }
            }
            if show_output_anchor {
                span { class: "klinx-node-field-anchor klinx-node-field-anchor--out" }
            }
        }
    }
}

fn role_port_section_attr(ports: &[StagePortRow]) -> &'static str {
    if ports
        .iter()
        .all(|port| matches!(port.kind, StagePortKind::AggregateGroupKey))
    {
        "group-by"
    } else {
        "mixed"
    }
}

/// One semantic input role row inside an expanded node card.
///
/// Role rows explain how an operator consumes an input field without implying
/// that the role itself is an output column. Aggregate `group_by` keys use these
/// rows so the producer field can connect to `group_by.<field>` and separately
/// to the grouped output field row.
#[component]
fn RolePortRowView(node_index: usize, port: StagePortRow, highlighted: bool) -> Element {
    let mut hovered = use_context::<CanvasHover>();
    let mut pinned = use_context::<PinnedField>();

    let is_pinned = matches!(
        &*pinned.0.read(),
        Some(LineageTarget::RolePort(n, p)) if *n == node_index && p.as_str() == port.id
    );
    let kind_attr = match port.kind {
        StagePortKind::AggregateGroupKey => "aggregate-group-key",
    };
    let tip = format!("{}: {}", port.role, port.label);

    let mut row_class = String::from("klinx-node-role-port");
    if highlighted {
        row_class.push_str(" klinx-node-role-port--lineage");
    }
    if is_pinned {
        row_class.push_str(" klinx-node-role-port--pinned");
    }

    rsx! {
        div {
            class: "{row_class}",
            "data-role-kind": kind_attr,
            title: "{tip}",
            onmouseenter: {
                let port_id = port.id.clone();
                move |_| hovered.request_role_port(node_index, port_id.clone())
            },
            onclick: {
                let port_id = port.id.clone();
                move |e: MouseEvent| {
                    e.stop_propagation();
                    let already = matches!(
                        &*pinned.0.peek(),
                        Some(LineageTarget::RolePort(n, p))
                            if *n == node_index && p.as_str() == port_id
                    );
                    if already {
                        pinned.0.set(None);
                    } else {
                        pinned
                            .0
                            .set(Some(LineageTarget::RolePort(node_index, port_id.clone())));
                    }
                }
            },
            span { class: "klinx-node-role-port-anchor klinx-node-role-port-anchor--in" }
            span { class: "klinx-node-role-port-role", "{port.role}" }
            span { class: "klinx-node-role-port-name", "{port.label}" }
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

/// The `▶` drill action for a composition node card (#171).
///
/// A composition node's `▶` always pushes the in-context OVERLAY stack — on the
/// top-level canvas it opens a fresh lightbox; inside the overlay a nested `▶`
/// pushes the same stack so drilling stays in the lightbox (the overlay re-mounts
/// against the new top frame). The full-swap drill remains reachable only via the
/// overlay's "OPEN FULL" escape hatch, never directly from a node card.
///
/// Shared frame resolution lives in [`resolve_composition_frame`]; this only
/// routes the resolved frame onto the overlay stack. A no-op when the compiled
/// plan has no body for `node_name` — the same silent no-op the drill has always
/// had when no plan is compiled (`compiled_plan` is `None`).
fn open_composition_overlay(state: &crate::state::AppState, node_name: &str) {
    let Some(frame) = ({
        let compiled_guard = state.compiled_plan.read();
        compiled_guard
            .as_ref()
            .and_then(|plan| resolve_composition_frame(plan, node_name))
    }) else {
        return;
    };
    let mut overlay = state.composition_overlay_stack;
    overlay.write().push(frame);
}
