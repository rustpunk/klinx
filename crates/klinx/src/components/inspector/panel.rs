use std::collections::HashSet;

use dioxus::prelude::*;

use crate::pipeline_view::Precision;
use crate::state::{SelectedField, current_pipeline_view, use_app_state};

use super::drawer_bar::{ActiveDrawer, DrawerToggleBar};
use super::drawer_docs::DrawerDocs;
use super::drawer_notes::DrawerNotes;
use super::drawer_run::DrawerRun;
use super::model::{
    BoundaryHopKind, CxlMentionView, FieldInspectorModel, InspectorBuildContext,
    InspectorDiagnostic, InspectorFact, InspectorRow, InspectorSection, InspectorSelection,
    MissingInspectorModel, NodeInspectorModel, RoleUsageView, SelectedInspectorModel, StatusChip,
    StatusTone, TraceNode, build_selected_inspector, count_field_hops,
};
use super::scoped_yaml::ScopedYamlEditor;

/// Shared inspector for the currently selected node or field.
///
/// The selected item decides the model; the shell, drawer rail, diagnostics,
/// details, and focused YAML editor stay common so node and field inspection do
/// not drift into competing surfaces.
#[component]
pub fn SelectedInspector() -> Element {
    let state = use_app_state();
    let mut active_drawer = use_signal(|| ActiveDrawer::None);

    let selection = selected_item(state);
    let Some(selection) = selection else {
        return rsx! {};
    };

    let view = current_pipeline_view(state);
    let visible_errors = (state.visible_errors)();
    let schema_warning_strings = (state.schema_warnings)()
        .iter()
        .map(|warning| format!("{warning:?}"))
        .collect::<Vec<_>>();
    // #187: composition-binding diagnostics, filtered per-node inside
    // `node_diagnostics` by the offending node name they carry.
    let composition_diagnostics = (state.composition_diagnostics)();
    let channel_mode = *state.channel_view_mode.read();
    // Hold the compiled-plan read guard across the build (#155), mirroring the
    // pipeline guard: the lineage trace's `BodyScopeResolver` borrows the plan to
    // descend into composition bodies. `compiled_plan_available` (the bare bool the
    // node inspector reads) is derived from the same guard.
    let compiled_plan_guard = state.compiled_plan.read();
    let compiled_plan_available = compiled_plan_guard.is_some();
    let pipeline_guard = state.pipeline.read();
    let model = build_selected_inspector(
        selection,
        InspectorBuildContext {
            view: &view,
            config: pipeline_guard.as_ref(),
            plan: compiled_plan_guard.as_deref(),
            channel_mode,
            compiled_plan_available,
            visible_errors: &visible_errors,
            schema_warnings: &schema_warning_strings,
            composition_diagnostics: &composition_diagnostics,
        },
    );
    drop(pipeline_guard);
    drop(compiled_plan_guard);

    let stage_id = model_stage_id(&model);
    let drawer_open = (active_drawer)() != ActiveDrawer::None;

    rsx! {
        div {
            class: "klinx-inspector klinx-selected-inspector",
            onmousedown: move |e: MouseEvent| e.stop_propagation(),

            match &model {
                SelectedInspectorModel::Node(node) => rsx! {
                    InspectorHeader {
                        kind_label: node.kind_label,
                        kind_attr: node.kind_attr,
                        label: node.label.clone(),
                        stage_id: Some(node.stage_id.clone()),
                        is_field: false,
                    }
                    div {
                        class: "klinx-inspector-config",
                        "data-compressed": if drawer_open { "true" } else { "false" },
                        NodeInspectorBody { node: node.as_ref().clone() }
                    }
                },
                SelectedInspectorModel::Field(field) => rsx! {
                    InspectorHeader {
                        kind_label: "FIELD",
                        kind_attr: field.stage_kind_attr,
                        label: field.label.clone(),
                        stage_id: Some(field.selection.stage_id.clone()),
                        is_field: true,
                    }
                    div {
                        class: "klinx-inspector-config",
                        "data-compressed": if drawer_open { "true" } else { "false" },
                        FieldInspectorBody { field: field.as_ref().clone() }
                    }
                },
                SelectedInspectorModel::Missing(missing) => rsx! {
                    InspectorHeader {
                        kind_label: missing.kind_label,
                        kind_attr: missing.kind_attr,
                        label: missing.label.clone(),
                        stage_id: missing.stage_id.clone(),
                        is_field: missing.kind_label == "FIELD",
                    }
                    div {
                        class: "klinx-inspector-config",
                        "data-compressed": if drawer_open { "true" } else { "false" },
                        MissingInspectorBody { missing: missing.clone() }
                    }
                },
            }

            DrawerToggleBar {
                active: (active_drawer)(),
                on_toggle: move |drawer: ActiveDrawer| {
                    active_drawer.set(drawer);
                },
            }

            div {
                class: "klinx-drawer-region",
                "data-open": if drawer_open { "true" } else { "false" },

                match (active_drawer)() {
                    ActiveDrawer::Run => rsx! { DrawerRun {} },
                    ActiveDrawer::Docs => {
                        if let Some(stage_id) = stage_id.clone() {
                            rsx! { DrawerDocs { stage_id } }
                        } else {
                            rsx! { DrawerUnavailable { label: "Docs" } }
                        }
                    },
                    ActiveDrawer::Notes => {
                        if let Some(stage_id) = stage_id.clone() {
                            rsx! { DrawerNotes { stage_id } }
                        } else {
                            rsx! { DrawerUnavailable { label: "Notes" } }
                        }
                    },
                    ActiveDrawer::None => rsx! {},
                }
            }
        }
    }
}

fn selected_item(state: crate::state::AppState) -> Option<InspectorSelection> {
    if let Some(field) = state.selected_field.read().clone() {
        return Some(InspectorSelection::Field(field));
    }

    let stages = state.selected_stages.read();
    if stages.len() == 1 {
        stages.iter().next().cloned().map(InspectorSelection::Node)
    } else {
        None
    }
}

fn model_stage_id(model: &SelectedInspectorModel) -> Option<String> {
    match model {
        SelectedInspectorModel::Node(node) => Some(node.stage_id.clone()),
        SelectedInspectorModel::Field(field) => Some(field.selection.stage_id.clone()),
        SelectedInspectorModel::Missing(missing) => missing.stage_id.clone(),
    }
}

#[component]
fn InspectorHeader(
    kind_label: &'static str,
    kind_attr: &'static str,
    label: String,
    stage_id: Option<String>,
    is_field: bool,
) -> Element {
    let state = use_app_state();

    rsx! {
        div {
            class: "klinx-inspector-header",
            "data-stage-kind": kind_attr,
            style: "border-top: 3px solid var(--klinx-stage-accent);",

            span {
                class: "klinx-inspector-badge",
                style: "color: var(--klinx-stage-accent); border-color: var(--klinx-stage-accent);",
                "{kind_label}"
            }
            span { class: "klinx-inspector-label", "{label}" }
            span { style: "flex: 1;" }
            if let Some(stage_id) = stage_id {
                button {
                    class: "klinx-inspector-close",
                    onclick: move |_| {
                        if is_field {
                            let mut selected_field = state.selected_field;
                            selected_field.set(None);
                        } else {
                            let mut stages = state.selected_stages;
                            let mut next = (*stages.peek()).clone();
                            next.remove(&stage_id);
                            stages.set(next);
                        }
                    },
                    "\u{00D7}"
                }
            }
        }
    }
}

#[component]
fn NodeInspectorBody(node: NodeInspectorModel) -> Element {
    rsx! {
        div { class: "klinx-inspector-selected-body",
            OverviewSection {
                title: "OVERVIEW",
                facts: node.overview.clone(),
                chips: node.status_chips.clone(),
            }

            DiagnosticsSection { diagnostics: node.diagnostics.clone() }

            for section in node.sections.iter() {
                InspectorSectionView {
                    key: "{section.title}",
                    section: section.clone(),
                }
            }

            if !node.composition_params.is_empty() {
                div { class: "klinx-inspector-section",
                    SectionHeader { title: "PROVENANCE" }
                    for param in node.composition_params.iter() {
                        {
                            let stage_id = node.stage_id.clone();
                            let param_name = param.clone();
                            rsx! {
                                div {
                                    key: "{param_name}",
                                    class: "klinx-provenance-field",
                                    div { class: "klinx-provenance-field-name", "{param_name}" }
                                    super::provenance::ProvenancePanel {
                                        node_name: stage_id,
                                        param_name,
                                    }
                                }
                            }
                        }
                    }
                }
            }

            ScopedYamlEditor { stage_id: node.stage_id.clone() }
        }
    }
}

#[component]
fn FieldInspectorBody(field: FieldInspectorModel) -> Element {
    rsx! {
        div { class: "klinx-inspector-selected-body klinx-field-inspector",
            div { class: "klinx-inspector-section",
                SectionHeader { title: "FIELD" }
                div { class: "klinx-field-summary",
                    div { class: "klinx-field-summary-name", "{field.field_name}" }
                    div { class: "klinx-field-summary-meta",
                        StatusChipView {
                            chip: StatusChip {
                                label: field.field_kind_label.to_string(),
                                tone: StatusTone::Info,
                            }
                        }
                        StatusChipView {
                            chip: StatusChip {
                                label: field.stage_kind_label.to_string(),
                                tone: StatusTone::Info,
                            }
                        }
                        // Per-field precision badge (#148): grades how faithful the
                        // field's lineage is, toned so a degraded tier draws the eye.
                        span {
                            class: "klinx-field-precision-badge",
                            "data-precision": "{field.lineage_precision.precision_attr()}",
                            title: "{field.precision_reason}",
                            "{field.lineage_precision.precision_label()}"
                        }
                        for badge in field.badges.iter() {
                            StatusChipView {
                                key: "{badge}",
                                chip: StatusChip {
                                    label: badge.clone(),
                                    tone: StatusTone::Ok,
                                }
                            }
                        }
                    }
                }
                FactGrid { facts: field.context.clone() }
            }

            div { class: "klinx-inspector-section",
                SectionHeader { title: "EXPLANATION" }
                div { class: "klinx-inspector-empty", "{field.explanation}" }
                if let Some(annotation) = field.annotation.as_ref() {
                    div { class: "klinx-field-warning", "{annotation}" }
                }
            }

            CxlMentionsSection { mentions: field.cxl_mentions.clone() }

            LineageSection { field: field.clone() }

            ScopedYamlEditor { stage_id: field.selection.stage_id.clone() }
        }
    }
}

#[component]
fn MissingInspectorBody(missing: MissingInspectorModel) -> Element {
    rsx! {
        div { class: "klinx-inspector-selected-body",
            div { class: "klinx-inspector-section",
                SectionHeader { title: "UNAVAILABLE" }
                div { class: "klinx-field-warning", "{missing.reason}" }
            }
            if let Some(stage_id) = missing.stage_id {
                ScopedYamlEditor { stage_id }
            }
        }
    }
}

#[component]
fn OverviewSection(
    title: &'static str,
    facts: Vec<InspectorFact>,
    chips: Vec<StatusChip>,
) -> Element {
    rsx! {
        div { class: "klinx-inspector-section",
            SectionHeader { title }
            if !chips.is_empty() {
                div { class: "klinx-field-summary-meta klinx-inspector-chip-row",
                    for chip in chips.iter() {
                        StatusChipView {
                            key: "{chip.label}",
                            chip: chip.clone(),
                        }
                    }
                }
            }
            FactGrid { facts }
        }
    }
}

#[component]
fn DiagnosticsSection(diagnostics: Vec<InspectorDiagnostic>) -> Element {
    rsx! {
        div { class: "klinx-inspector-section",
            SectionHeader { title: "DIAGNOSTICS" }
            if diagnostics.is_empty() {
                div { class: "klinx-inspector-empty",
                    "No parse, schema, or stage diagnostics are visible for this selection."
                }
            } else {
                div { class: "klinx-inspector-row-list",
                    for diagnostic in diagnostics.iter() {
                        div {
                            key: "{diagnostic.label}-{diagnostic.message}",
                            class: "klinx-inspector-detail-row",
                            "data-tone": diagnostic.tone.as_attr(),
                            span { class: "klinx-inspector-detail-label", "{diagnostic.label}" }
                            span { class: "klinx-inspector-detail-value", "{diagnostic.message}" }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn InspectorSectionView(section: InspectorSection) -> Element {
    rsx! {
        div { class: "klinx-inspector-section",
            SectionHeader { title: section.title }
            if !section.facts.is_empty() {
                FactGrid { facts: section.facts.clone() }
            }
            if !section.rows.is_empty() {
                RowList { rows: section.rows.clone() }
            }
            if let Some(reason) = section.unavailable.as_ref() {
                div { class: "klinx-inspector-empty", "{reason}" }
            }
        }
    }
}

#[component]
fn FactGrid(facts: Vec<InspectorFact>) -> Element {
    if facts.is_empty() {
        return rsx! {};
    }

    rsx! {
        div { class: "klinx-field-property-grid",
            for fact in facts.iter() {
                div {
                    key: "{fact.label}",
                    class: "klinx-field-property",
                    span { class: "klinx-field-property-label", "{fact.label}" }
                    span { class: "klinx-field-property-value", "{fact.value}" }
                }
            }
        }
    }
}

#[component]
fn RowList(rows: Vec<InspectorRow>) -> Element {
    rsx! {
        div { class: "klinx-inspector-row-list",
            for row in rows.iter() {
                div {
                    key: "{row.label}-{row.value}",
                    class: "klinx-inspector-detail-row",
                    "data-tone": row.tone.map(StatusTone::as_attr).unwrap_or("none"),
                    span { class: "klinx-inspector-detail-label", "{row.label}" }
                    span { class: "klinx-inspector-detail-value", "{row.value}" }
                }
            }
        }
    }
}

#[component]
fn SectionHeader(title: &'static str) -> Element {
    rsx! {
        div {
            class: "klinx-section-header",
            span { class: "klinx-diamond", "\u{25C6}" }
            span { class: "klinx-section-title", "{title}" }
            span { class: "klinx-section-rule" }
        }
    }
}

#[component]
fn StatusChipView(chip: StatusChip) -> Element {
    rsx! {
        span {
            class: "klinx-field-chip",
            "data-kind": chip.tone.as_attr(),
            "{chip.label}"
        }
    }
}

#[component]
fn CxlMentionsSection(mentions: Vec<CxlMentionView>) -> Element {
    rsx! {
        div { class: "klinx-inspector-section",
            SectionHeader { title: "CXL STATEMENTS" }
            if mentions.is_empty() {
                div { class: "klinx-inspector-empty",
                    "No CXL statement reads or writes are available for this field."
                }
            } else {
                div { class: "klinx-inspector-row-list",
                    for mention in mentions.iter() {
                        div {
                            key: "{mention.kind}-{mention.expression}",
                            class: "klinx-inspector-detail-row",
                            "data-tone": "info",
                            span { class: "klinx-inspector-detail-label", "{mention.kind}" }
                            span { class: "klinx-inspector-detail-value",
                                "reads [{mention.reads}] writes [{mention.writes}] :: {mention.expression}"
                            }
                        }
                    }
                }
            }
        }
    }
}

/// A stable expand-state key for a trace hop: `(stage_id, field_name, hop, scope_depth)`
/// (#153, scope component added in #156). Keying expansion by the hop's IDENTITY — not
/// its position in the flattened row list — keeps a branch's open/closed state stable
/// across re-renders (selecting a new field, toggling the INDIRECT filter), since the
/// same hop keeps the same key even as rows shift around it.
///
/// `scope_depth` — the number of composition walls (`Enter`/`Recursive` crossings)
/// open above-and-including this hop — disambiguates two same-named nodes living in
/// DIFFERENT scopes at the same hop (#156). A body node and an outer node can share a
/// `(stage_id, field_name, hop)` triple because the body view reuses unqualified stage
/// ids; without the scope component, collapsing one would collapse the other. The trace
/// BFS itself already keeps these distinct (it dedups on `(scope_id, node, field)`); this
/// mirrors that scope-awareness into the panel's expand-state key.
type TraceKey = (String, String, usize, usize);

/// Build a hop's [`TraceKey`] at a known scope depth. The depth is supplied by the
/// caller (the tree walk accumulates it) because a [`TraceNode`] alone does not carry
/// its path's open-composition count.
fn trace_key_at(node: &TraceNode, scope_depth: usize) -> TraceKey {
    (
        node.endpoint.stage_id.clone(),
        node.endpoint.field_name.clone(),
        node.endpoint.hop,
        scope_depth,
    )
}

/// The Dioxus list `key:` string for a trace row — the same four identity components as
/// [`trace_key_at`], dash-joined. Built from one helper so the render key cannot drift
/// from the expand-state [`TraceKey`]: `scope_depth` MUST be present, or two visible
/// rows sharing `(stage_id, field, hop)` across composition scopes collide and Dioxus
/// mis-associates them (#156).
fn trace_render_key(node: &TraceNode, scope_depth: usize) -> String {
    format!(
        "{}-{}-{}-{}",
        node.endpoint.stage_id, node.endpoint.field_name, node.endpoint.hop, scope_depth
    )
}

/// The scope depth ON this node, given its parent's scope depth: a node whose hop is an
/// `Enter`/`Recursive` composition crossing opens one more wall, so it and its
/// descendants live one scope deeper. An `Exit` resurfaces but is counted at the depth
/// it is reached at — the crossing-count on the path only ever grows, so two distinct
/// scopes reached via different Enter chains can never collide on depth for the SAME hop
/// number.
///
/// The `Enter | Recursive => +1` increment MUST stay in sync with
/// [`super::model::max_scope_depth`], which owns the same rule for the "originated N
/// deep" summary; if one changes which crossings deepen a scope, the other must match or
/// the panel's per-row depth and the summary figure diverge.
fn node_scope_depth(parent_depth: usize, node: &TraceNode) -> usize {
    match node.endpoint.boundary {
        Some(BoundaryHopKind::Enter(_)) | Some(BoundaryHopKind::Recursive(_)) => parent_depth + 1,
        _ => parent_depth,
    }
}

/// Singular/plural noun for the "originated N compositions deep" summary (#156), so a
/// single crossing reads "1 composition deep" rather than "1 compositions deep".
fn composition_word(depth: usize) -> &'static str {
    if depth == 1 {
        "composition"
    } else {
        "compositions"
    }
}

/// One flattened, depth-tagged row of a trace tree, ready to render in document
/// order (#153). `depth` is the VISUAL indent depth; `scope_depth` is the composition
/// nesting (#156), carried so the row reconstructs the same [`TraceKey`] the walk
/// inserted. `has_children` drives the caret; `expanded` whether this row's children
/// follow.
#[derive(Clone, PartialEq)]
struct TraceRow {
    node: TraceNode,
    depth: usize,
    scope_depth: usize,
    has_children: bool,
    expanded: bool,
}

/// Flatten a trace forest into visible rows in pre-order (parent before its
/// children), honoring the collapsed set (#153). A collapsed node still appears as
/// a row — only its descendants are withheld — so its caret can re-expand it.
///
/// `scope_depth` is the composition nesting of `nodes`' parent; each node's own scope
/// depth (#156) is derived via [`node_scope_depth`] and folded into its [`TraceKey`],
/// so a body node and an outer node sharing `(stage_id, field, hop)` are keyed apart.
fn flatten_trace(
    nodes: &[TraceNode],
    expanded: &HashSet<TraceKey>,
    depth: usize,
    scope_depth: usize,
    out: &mut Vec<TraceRow>,
) {
    for node in nodes {
        let has_children = !node.children.is_empty();
        let node_scope = node_scope_depth(scope_depth, node);
        let is_expanded = expanded.contains(&trace_key_at(node, node_scope));
        out.push(TraceRow {
            node: node.clone(),
            depth,
            scope_depth: node_scope,
            has_children,
            expanded: is_expanded,
        });
        if has_children && is_expanded {
            flatten_trace(&node.children, expanded, depth + 1, node_scope, out);
        }
    }
}

/// The default-expanded set for a freshly-built tree (#153): every hop-1 (direct) child
/// of the selected root EXCEPT a composition `Enter` crossing, so the first hop is open
/// and deeper hops start collapsed. Descending INTO a composition is opt-in (#156): an
/// `Enter` hop stays collapsed even at hop 1, so the reader expands one wall per click
/// rather than being dropped into a body's internals. `Recursive`/`Exit` and ordinary
/// hop-1 nodes open as before.
fn default_expanded(upstream: &[TraceNode], downstream: &[TraceNode]) -> HashSet<TraceKey> {
    upstream
        .iter()
        .chain(downstream.iter())
        .filter(|node| !matches!(node.endpoint.boundary, Some(BoundaryHopKind::Enter(_))))
        .map(|node| trace_key_at(node, node_scope_depth(0, node)))
        .collect()
}

/// The LINEAGE section: the per-hop trace TREE plus the INDIRECT include/exclude
/// toggle (#153). Owns the toggle and expand-state signals so the trees rebuild
/// reactively; the model stays free of UI state — the INDIRECT filter prunes the
/// already-built tree here.
#[component]
fn LineageSection(field: FieldInspectorModel) -> Element {
    // INDIRECT include/exclude toggle (#153). Default ON so nothing regresses; when
    // off, influence-rooted subtrees are pruned from the built tree.
    let mut include_indirect = use_signal(|| true);
    // Expand state keyed by hop identity, so toggling one branch (or the INDIRECT
    // filter) never collapses the others. Re-seeded (hop-1 open, deeper collapsed)
    // whenever the SELECTED field changes — the component instance persists across
    // field navigation, so the previous field's keys must be replaced, and seeding
    // by field identity (not "set is empty") lets a user deliberately collapse every
    // hop-1 node within a field without it springing back open.
    let mut expanded = use_signal(HashSet::<TraceKey>::new);
    let mut seeded_for = use_signal(|| None::<SelectedField>);

    // The toggle selects between the two precomputed trees the model built (#153):
    // the full tree (INDIRECT included) and the direct-only tree. Selecting — rather
    // than pruning the full tree in the panel — keeps a dual-role column (carried AND
    // an influence) visible as a DIRECT hop when the toggle is off; a prune would drop
    // it, since the full tree's worst-precision dedup tags that hop INDIRECT.
    let upstream = use_memo(use_reactive!(|field| {
        if (include_indirect)() {
            field.upstream.clone()
        } else {
            field.upstream_direct.clone()
        }
    }));
    let downstream = use_memo(use_reactive!(|field| {
        if (include_indirect)() {
            field.downstream.clone()
        } else {
            field.downstream_direct.clone()
        }
    }));

    {
        // Seed the default-open set from the FULL trees (toggle-independent), so the
        // hop-1 rows are open by default regardless of the toggle's state when the
        // field was selected — re-enabling INDIRECT never leaves a freshly-revealed
        // hop-1 influence row collapsed. Keys for currently-pruned nodes are inert.
        let selection = field.selection.clone();
        let seed = default_expanded(&field.upstream, &field.downstream);
        use_effect(use_reactive!(|(selection, seed)| {
            if seeded_for.peek().as_ref() != Some(&selection) {
                expanded.set(seed.clone());
                seeded_for.set(Some(selection));
            }
        }));
    }

    // The LINEAGE summary counts the field's FULL lineage (toggle-independent), so it
    // agrees with the `lineage` context fact and presents one source of truth; the
    // INDIRECT toggle filters which hops the tree below DISPLAYS, not the count. It
    // counts real source/consumer FIELDS, excluding the synthetic composition-boundary
    // crossings #155 inserts (`count_field_hops`), so the figure matches the `lineage`
    // fact built in `build_field_detail`.
    let upstream_count = count_field_hops(&field.upstream);
    let downstream_count = count_field_hops(&field.downstream);
    let indirect_on = include_indirect();

    rsx! {
        div { class: "klinx-inspector-section",
            div { class: "klinx-field-lineage-header",
                SectionHeader { title: "LINEAGE" }
                button {
                    class: if indirect_on {
                        "klinx-field-trace-toggle klinx-field-trace-toggle--active"
                    } else {
                        "klinx-field-trace-toggle"
                    },
                    "aria-pressed": if indirect_on { "true" } else { "false" },
                    title: "Show or hide INDIRECT influence hops (filter / group-by / join-key / branch)",
                    onclick: move |_| {
                        let next = !*include_indirect.peek();
                        include_indirect.set(next);
                    },
                    "INDIRECT"
                }
            }
            div { class: "klinx-field-lineage-summary",
                span { "{upstream_count} upstream" }
                span { "{downstream_count} downstream" }
                span { "{field.role_usages.len()} role uses" }
                // Cross-boundary depth (#156): how many composition walls the deepest
                // trace path crossed. Shown only when the trace descended at all (> 0);
                // a flat trace keeps the summary unchanged from before #156.
                if field.max_scope_depth > 0 {
                    span {
                        class: "klinx-field-lineage-depth",
                        "originated {field.max_scope_depth} {composition_word(field.max_scope_depth)} deep"
                    }
                }
            }
            // A field with no lineage edges shows the preserved empty-state
            // message; an edged field whose precision is degraded surfaces the
            // reason as a warning so the over-approximation is visible (#148).
            if field.lineage_empty {
                div { class: "klinx-field-warning", "{field.precision_reason}" }
            } else if field.lineage_precision != Precision::Exact {
                div {
                    class: "klinx-field-precision-note",
                    "data-precision": "{field.lineage_precision.precision_attr()}",
                    "{field.precision_reason}"
                }
            }
            TraceTree {
                title: "UPSTREAM",
                nodes: upstream.read().clone(),
                empty: "No upstream fields.",
                expanded,
            }
            TraceTree {
                title: "DOWNSTREAM",
                nodes: downstream.read().clone(),
                empty: "No downstream fields.",
                expanded,
            }
            RoleUsageList { usages: field.role_usages.clone() }
        }
    }
}

/// An expandable hop-by-hop trace tree (#153), replacing the former flat
/// `TraceList`. Renders the tree as a depth-indented flat row list (one stably-keyed
/// row per visible hop) so expand/collapse re-renders only the changed rows, reusing
/// the file-explorer pattern.
#[component]
fn TraceTree(
    title: &'static str,
    nodes: Vec<TraceNode>,
    empty: &'static str,
    expanded: Signal<HashSet<TraceKey>>,
) -> Element {
    let is_empty = nodes.is_empty();
    let rows = use_memo(use_reactive!(|(nodes, expanded)| {
        let mut rows = Vec::new();
        flatten_trace(&nodes, &expanded.read(), 0, 0, &mut rows);
        rows
    }));

    rsx! {
        div {
            class: "klinx-field-trace-group",
            div { class: "klinx-field-trace-title", "{title}" }
            if is_empty {
                div { class: "klinx-field-trace-empty", "{empty}" }
            } else {
                div {
                    class: "klinx-field-trace-list",
                    for row in rows.read().iter() {
                        TraceTreeRow {
                            key: "{trace_render_key(&row.node, row.scope_depth)}",
                            row: row.clone(),
                            on_toggle: move |key: TraceKey| {
                                let mut set = expanded.write();
                                if !set.remove(&key) {
                                    set.insert(key);
                                }
                            },
                        }
                    }
                }
            }
        }
    }
}

/// A single trace-tree row: its own component with a stable `key:` so only rows
/// whose props change re-render on expand/collapse (#153). A click on the caret
/// toggles the branch; a click elsewhere selects that hop's field on the canvas.
#[component]
fn TraceTreeRow(row: TraceRow, on_toggle: EventHandler<TraceKey>) -> Element {
    let state = use_app_state();
    let entry = &row.node.endpoint;
    // Indent children by depth, mirroring the file-explorer tree.
    let indent = 10 + row.depth as i32 * 14;
    let caret = if row.has_children {
        if row.expanded { "\u{25BE}" } else { "\u{25B8}" }
    } else {
        ""
    };
    let key = trace_key_at(&row.node, row.scope_depth);
    // The precision badge's tooltip (#156). On a boundary hop, surface WHY the tier is
    // what it is — a Recursive crossing is Approximate because the composition recurses,
    // so the reason names the boundary rather than leaving the bare tier unexplained.
    // Reuses the existing `entry.precision` badge (no new badge) and the shared
    // `BoundaryHopKind` verb/label methods (no re-spelled strings); only the tooltip is
    // enriched. Ordinary hops keep the plain tier label.
    let precision_title = match &entry.boundary {
        Some(kind) => format!(
            "{} — {} {}",
            entry.precision.precision_label(),
            kind.verb(),
            kind.label(),
        ),
        None => entry.precision.precision_label().to_string(),
    };

    rsx! {
        div {
            class: "klinx-field-trace-row klinx-field-trace-row--tree klinx-field-trace-row--selectable",
            style: "padding-left: {indent}px",
            "data-stage-kind": "{entry.stage_kind_attr}",
            // Clicking a hop selects that field on the canvas (#151): it writes the
            // shared `SelectedField`, which the canvas reveal effect resolves to a
            // node + reveals, and from which the inspector rebuilds onto the new
            // field. Field selection supersedes any node selection, mirroring the
            // canvas field-row click.
            onclick: {
                let target = entry.to_selected_field();
                move |_| {
                    let mut selected_field = state.selected_field;
                    let mut selected_stages = state.selected_stages;
                    selected_field.set(Some(target.clone()));
                    selected_stages.set(std::collections::HashSet::new());
                }
            },
            // Caret toggles expansion without selecting the field; `stop_propagation`
            // keeps the row's select handler from also firing.
            span {
                class: "klinx-field-trace-caret",
                onclick: move |e: MouseEvent| {
                    e.stop_propagation();
                    if row.has_children {
                        on_toggle.call(key.clone());
                    }
                },
                "{caret}"
            }
            span { class: "klinx-field-trace-hop", "h{entry.hop}" }
            span { class: "klinx-field-trace-main",
                span { class: "klinx-field-trace-stage", "{entry.stage_label}" }
                span { class: "klinx-field-trace-field", "{entry.field_name}" }
                // Composition-boundary marker (#156): when this hop crosses a
                // composition wall, name the crossing (↳ enters / ↥ exits / ↺
                // recursive) so a cross-boundary trace reads legibly. Glyph/verb/slug
                // come from the shared `BoundaryHopKind` methods so the marker and the
                // precision tooltip cannot drift; `data-boundary` tints the base
                // `.klinx-field-trace-boundary` rule per crossing kind.
                if let Some(kind) = entry.boundary.as_ref() {
                    span {
                        class: "klinx-field-trace-boundary",
                        "data-boundary": "{kind.data_slug()}",
                        span { class: "klinx-field-trace-boundary-glyph", "{kind.glyph()}" }
                        span { class: "klinx-field-trace-boundary-text", "{kind.verb()} {kind.label()}" }
                    }
                }
                // Per-hop CXL attribution (#153): the responsible statement(s) on
                // this hop's own stage. Absent for a non-CXL stage, where the edge
                // kind + precision badge is the attribution.
                for mention in row.node.cxl_mentions.iter() {
                    div {
                        key: "{mention.kind}-{mention.expression}",
                        class: "klinx-field-trace-cxl",
                        span { class: "klinx-field-trace-cxl-kind", "{mention.kind}" }
                        span { class: "klinx-field-trace-cxl-expr", "{mention.expression}" }
                    }
                }
            }
            span {
                class: "klinx-field-trace-kind",
                "data-kind": "{entry.edge_kind_attr}",
                "{entry.edge_kind_label}"
            }
            // Per-hop precision badge (#148): the tier of the edge taken to reach
            // this hop, so a reader sees where the trace becomes an
            // over-approximation.
            span {
                class: "klinx-field-trace-precision",
                "data-precision": "{entry.precision.precision_attr()}",
                title: "{precision_title}",
                "{entry.precision.precision_label()}"
            }
        }
    }
}

#[component]
fn RoleUsageList(usages: Vec<RoleUsageView>) -> Element {
    if usages.is_empty() {
        return rsx! {};
    }

    rsx! {
        div {
            class: "klinx-field-trace-group",
            div { class: "klinx-field-trace-title", "ROLE USES" }
            div {
                class: "klinx-field-trace-list",
                for usage in usages.iter() {
                    div {
                        key: "{usage.stage_label}-{usage.port_label}",
                        class: "klinx-field-trace-row",
                        "data-stage-kind": "{usage.stage_kind_attr}",
                        span { class: "klinx-field-trace-hop", "role" }
                        span { class: "klinx-field-trace-main",
                            span { class: "klinx-field-trace-stage", "{usage.stage_label}" }
                            span { class: "klinx-field-trace-field", "{usage.port_label}" }
                        }
                        span {
                            class: "klinx-field-trace-kind",
                            "data-kind": "{usage.edge_kind_attr}",
                            title: "{usage.role}",
                            "{usage.edge_kind_label}"
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn DrawerUnavailable(label: &'static str) -> Element {
    rsx! {
        div { class: "klinx-drawer-content",
            div { class: "klinx-drawer-placeholder",
                "{label} is unavailable for this selection."
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::model::{BoundaryHopKind, TraceEndpointView};
    use super::*;

    /// A test `TraceNode` at `(stage, field, hop)` with the given children. Edge
    /// kind/precision are fixed (DIRECT derive, Exact) since these tests exercise the
    /// flatten / expand-key logic, not per-hop attribution.
    fn node(stage: &str, field: &str, hop: usize, children: Vec<TraceNode>) -> TraceNode {
        TraceNode {
            endpoint: TraceEndpointView {
                stage_id: stage.to_string(),
                stage_label: stage.to_string(),
                stage_kind_label: "Transform",
                stage_kind_attr: "transform",
                field_name: field.to_string(),
                edge_kind_label: "derive",
                edge_kind_attr: "derive",
                precision: Precision::Exact,
                hop,
                boundary: None,
            },
            cxl_mentions: Vec::new(),
            children,
        }
    }

    /// A test `TraceNode` carrying a composition-boundary crossing (#156): the edge
    /// kind is `boundary` and the precision is the caller's, so the same fixture builds
    /// an Exact `Enter` hop or an Approximate `Recursive` hop.
    fn boundary_node(
        stage: &str,
        field: &str,
        hop: usize,
        boundary: BoundaryHopKind,
        precision: Precision,
        children: Vec<TraceNode>,
    ) -> TraceNode {
        TraceNode {
            endpoint: TraceEndpointView {
                stage_id: stage.to_string(),
                stage_label: stage.to_string(),
                stage_kind_label: "Composition",
                stage_kind_attr: "composition",
                field_name: field.to_string(),
                edge_kind_label: "boundary",
                edge_kind_attr: "boundary",
                precision,
                hop,
                boundary: Some(boundary),
            },
            cxl_mentions: Vec::new(),
            children,
        }
    }

    /// #153: the expand key is the hop's STABLE identity `(stage_id, field_name, hop,
    /// scope_depth)`, NOT its position in the flattened list — so a branch's open/closed
    /// state survives re-renders that reorder rows. Two distinct hops yield distinct
    /// keys; the same hop yields the same key regardless of siblings. The #156
    /// scope-depth component is fixed at 0 here (a flat, boundary-free trace).
    #[test]
    fn trace_key_is_stable_hop_identity() {
        let a = node("mid", "y", 1, vec![]);
        let b = node("src", "x", 2, vec![]);
        assert_eq!(
            trace_key_at(&a, 0),
            ("mid".to_string(), "y".to_string(), 1, 0)
        );
        assert_ne!(trace_key_at(&a, 0), trace_key_at(&b, 0));
        // A clone (what a re-render produces) keeps the identical key.
        assert_eq!(trace_key_at(&a, 0), trace_key_at(&a.clone(), 0));
    }

    /// #156: two same-named nodes reached in DIFFERENT composition scopes — a body
    /// node and an outer node sharing `(stage_id, field, hop)` — get DISTINCT expand
    /// keys via the scope-depth component, so collapsing the body node never collapses
    /// the outer one. The body view reuses unqualified stage ids, so without the scope
    /// component these would alias and one caret would drive both.
    #[test]
    fn trace_key_distinguishes_same_name_across_scopes() {
        // `inner.v` at hop 2 reached at the top scope (depth 0)…
        let outer = node("inner", "v", 2, vec![]);
        // …versus the SAME `(inner, v, 2)` reached one composition wall deeper.
        let inner = node("inner", "v", 2, vec![]);
        assert_ne!(
            trace_key_at(&outer, 0),
            trace_key_at(&inner, 1),
            "the scope-depth component must keep same-named cross-scope hops distinct"
        );

        // The whole-tree walk derives those depths itself: an Enter crossing pushes its
        // body one scope deeper, so the body's `inner.v` is keyed at depth 1 while an
        // outer `inner.v` stays at depth 0 — they never collide in the expand set.
        let forest = vec![
            // Outer occurrence of inner.v at the top scope.
            node("inner", "v", 2, vec![]),
            // Enter a composition, whose body ALSO has an inner.v at the same hop.
            boundary_node(
                "comp",
                "v",
                1,
                BoundaryHopKind::Enter("comp".to_string()),
                Precision::Exact,
                vec![node("inner", "v", 2, vec![])],
            ),
        ];
        let mut expanded = HashSet::new();
        // Open every node so the deep inner.v is visible.
        expanded.insert(trace_key_at(&forest[0], 0));
        let comp_scope = node_scope_depth(0, &forest[1]);
        expanded.insert(trace_key_at(&forest[1], comp_scope));
        let mut rows = Vec::new();
        flatten_trace(&forest, &expanded, 0, 0, &mut rows);
        let inner_rows: Vec<&TraceRow> = rows
            .iter()
            .filter(|r| r.node.endpoint.stage_id == "inner")
            .collect();
        assert_eq!(inner_rows.len(), 2, "both inner.v rows are present");
        let inner_keys: Vec<TraceKey> = inner_rows
            .iter()
            .map(|r| trace_key_at(&r.node, r.scope_depth))
            .collect();
        assert_ne!(
            inner_keys[0], inner_keys[1],
            "the outer and in-body inner.v rows carry distinct expand-state keys"
        );
        // The Dioxus list `key:` string MUST be distinct in lockstep — otherwise the two
        // visible same-name rows collide and Dioxus mis-associates them (#156).
        let render_keys: Vec<String> = inner_rows
            .iter()
            .map(|r| trace_render_key(&r.node, r.scope_depth))
            .collect();
        assert_ne!(
            render_keys[0], render_keys[1],
            "the render `key:` string must carry scope_depth so the rows do not collide"
        );
    }

    /// #153: `default_expanded` opens ONLY the hop-1 (direct) children, so the first
    /// hop is visible and deeper hops start collapsed.
    #[test]
    fn default_expanded_opens_only_first_hop() {
        let up = vec![node("mid", "y", 1, vec![node("src", "x", 2, vec![])])];
        let down: Vec<TraceNode> = vec![];
        let expanded = default_expanded(&up, &down);
        assert!(expanded.contains(&("mid".to_string(), "y".to_string(), 1, 0)));
        assert!(
            !expanded.contains(&("src".to_string(), "x".to_string(), 2, 0)),
            "deeper hops start collapsed"
        );
    }

    /// #156: shallow-by-default descent — `default_expanded` does NOT open a hop-1
    /// `Enter` boundary hop, so descending INTO a composition is opt-in (one caret
    /// click per wall). An ordinary hop-1 node and a hop-1 `Recursive` leaf still open
    /// as before; only `Enter` (a wall with a body behind it) stays collapsed.
    #[test]
    fn default_expanded_skips_enter_boundary_hops() {
        let up = vec![
            // An ordinary hop-1 hop opens by default.
            node("mid", "y", 1, vec![]),
            // A hop-1 Enter crossing stays collapsed — its body is opt-in.
            boundary_node(
                "comp",
                "a",
                1,
                BoundaryHopKind::Enter("comp".to_string()),
                Precision::Exact,
                vec![node("body", "a", 2, vec![])],
            ),
        ];
        let down: Vec<TraceNode> = vec![];
        let expanded = default_expanded(&up, &down);
        assert!(
            expanded.contains(&("mid".to_string(), "y".to_string(), 1, 0)),
            "an ordinary hop-1 node opens by default"
        );
        assert!(
            !expanded.contains(&trace_key_at(&up[1], node_scope_depth(0, &up[1]))),
            "a hop-1 Enter crossing stays collapsed: descending is opt-in"
        );
    }

    /// #156: the precision tier of a boundary hop reaches the flattened row UNCHANGED,
    /// so the existing per-hop precision badge renders "approximate" on a degraded
    /// (Recursive) crossing without any new badge logic. The boundary marker itself is
    /// also carried on the row's endpoint for the row to render.
    #[test]
    fn approximate_boundary_hop_reaches_row_with_precision() {
        // A Recursive crossing is the degraded case — it carries Approximate.
        let forest = vec![boundary_node(
            "comp",
            "v",
            1,
            BoundaryHopKind::Recursive("comp".to_string()),
            Precision::Approximate,
            vec![],
        )];
        let expanded = HashSet::new();
        let mut rows = Vec::new();
        flatten_trace(&forest, &expanded, 0, 0, &mut rows);
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(
            row.node.endpoint.precision,
            Precision::Approximate,
            "the boundary hop's precision reaches the row unchanged for the badge"
        );
        assert_eq!(
            row.node.endpoint.boundary,
            Some(BoundaryHopKind::Recursive("comp".to_string())),
            "the row carries the Recursive marker for the row to render"
        );
    }

    /// #156: an `Enter` hop carries its marker through `flatten_trace` to the row, and
    /// expanding it (one click) reveals its in-body child one scope deeper.
    #[test]
    fn enter_boundary_marker_and_child_reach_rows() {
        let forest = vec![boundary_node(
            "comp",
            "a",
            1,
            BoundaryHopKind::Enter("comp".to_string()),
            Precision::Exact,
            vec![node("body", "a", 2, vec![])],
        )];
        // Expand the Enter hop (the opt-in click).
        let mut expanded = HashSet::new();
        expanded.insert(trace_key_at(&forest[0], node_scope_depth(0, &forest[0])));
        let mut rows = Vec::new();
        flatten_trace(&forest, &expanded, 0, 0, &mut rows);

        assert_eq!(
            rows[0].node.endpoint.boundary,
            Some(BoundaryHopKind::Enter("comp".to_string())),
            "the Enter marker reaches the row"
        );
        assert_eq!(rows[0].scope_depth, 1, "the Enter hop lives one wall deep");
        // Its in-body child is revealed once the Enter hop is expanded.
        assert!(
            rows.iter()
                .any(|r| r.node.endpoint.stage_id == "body" && r.scope_depth == 1),
            "expanding the Enter hop reveals its in-body child one scope deeper"
        );
    }

    /// #153: `flatten_trace` emits a collapsed node as a row but withholds its
    /// descendants; expanding the node reveals them, indented one level deeper.
    /// Toggling one branch never affects an unrelated branch's expansion.
    #[test]
    fn flatten_respects_collapsed_set_and_depth() {
        // Two independent hop-1 branches, each with a hop-2 child.
        let forest = vec![
            node("midA", "a", 1, vec![node("srcA", "a0", 2, vec![])]),
            node("midB", "b", 1, vec![node("srcB", "b0", 2, vec![])]),
        ];

        // Expand only branch A's hop-1 node: A's child shows, B's stays hidden.
        let mut expanded = HashSet::new();
        expanded.insert(("midA".to_string(), "a".to_string(), 1, 0));
        let mut rows = Vec::new();
        flatten_trace(&forest, &expanded, 0, 0, &mut rows);

        let visible: Vec<_> = rows
            .iter()
            .map(|r| (r.node.endpoint.stage_id.as_str(), r.depth))
            .collect();
        // Both hop-1 roots at depth 0; only A's hop-2 child (depth 1) is revealed.
        assert_eq!(
            visible,
            vec![("midA", 0), ("srcA", 1), ("midB", 0)],
            "collapsing branch B hides its child without affecting branch A"
        );
        // The hidden child must not leak in.
        assert!(
            !rows.iter().any(|r| r.node.endpoint.stage_id == "srcB"),
            "a collapsed branch withholds its descendants"
        );
    }

    /// #153: the trace-tree caret + indent CSS exists so the expandable tree renders
    /// with affordances. Asserts the new `klinx-field-trace-caret` and lineage
    /// header rules, following the `css_rule_block` pattern.
    #[test]
    fn trace_tree_css_rules_present() {
        let css = include_str!("../../../assets/klinx.css");
        assert!(
            css_rule_block(css, ".klinx-field-trace-caret").is_some(),
            "the trace caret needs a CSS rule"
        );
        assert!(
            css_rule_block(css, ".klinx-field-lineage-header").is_some(),
            "the lineage header (with the INDIRECT toggle) needs a CSS rule"
        );
        assert!(
            css_rule_block(css, ".klinx-field-trace-toggle").is_some(),
            "the INDIRECT toggle button needs a CSS rule"
        );
        assert!(
            css_rule_block(css, ".klinx-field-trace-cxl").is_some(),
            "the per-hop CXL line needs a CSS rule"
        );
        // #156: the composition-boundary marker needs its base rule, and the
        // "originated N deep" depth summary needs its accent rule.
        assert!(
            css_rule_block(css, ".klinx-field-trace-boundary").is_some(),
            "the composition-boundary marker needs a CSS rule"
        );
        assert!(
            css_rule_block(css, ".klinx-field-lineage-depth").is_some(),
            "the cross-boundary depth summary needs a CSS rule"
        );
    }

    /// #156: the "originated N compositions deep" summary uses singular/plural nouns.
    /// The RSX builds the string from `FieldInspectorModel.max_scope_depth` behind a
    /// `> 0` gate (so it is absent at depth 0); this exercises the noun helper and the
    /// exact rendered strings the gate emits at depth 1 and 2.
    #[test]
    fn depth_summary_word_and_string() {
        assert_eq!(composition_word(1), "composition");
        assert_eq!(composition_word(2), "compositions");
        let depth = 2usize;
        assert_eq!(
            format!("originated {depth} {} deep", composition_word(depth)),
            "originated 2 compositions deep"
        );
        let depth = 1usize;
        assert_eq!(
            format!("originated {depth} {} deep", composition_word(depth)),
            "originated 1 composition deep"
        );
    }

    /// Local copy of the canvas test's CSS-rule-block extractor — returns the body
    /// between the first `{` after `selector` and its closing `}`.
    fn css_rule_block<'a>(css: &'a str, selector: &str) -> Option<&'a str> {
        let start = css.find(selector)?;
        let open = css[start..].find('{')? + start;
        let close = css[open..].find('}')? + open;
        Some(&css[open + 1..close])
    }
}
