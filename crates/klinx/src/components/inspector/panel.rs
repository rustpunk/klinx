use dioxus::prelude::*;

use crate::pipeline_view::Precision;
use crate::state::{current_pipeline_view, use_app_state};

use super::drawer_bar::{ActiveDrawer, DrawerToggleBar};
use super::drawer_docs::DrawerDocs;
use super::drawer_notes::DrawerNotes;
use super::drawer_run::DrawerRun;
use super::model::{
    CxlMentionView, FieldInspectorModel, InspectorBuildContext, InspectorDiagnostic, InspectorFact,
    InspectorRow, InspectorSection, InspectorSelection, MissingInspectorModel, NodeInspectorModel,
    RoleUsageView, SelectedInspectorModel, StatusChip, StatusTone, TraceEndpointView,
    build_selected_inspector,
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
    let channel_mode = *state.channel_view_mode.read();
    let compiled_plan_available = state.compiled_plan.read().is_some();
    let pipeline_guard = state.pipeline.read();
    let model = build_selected_inspector(
        selection,
        InspectorBuildContext {
            view: &view,
            config: pipeline_guard.as_ref(),
            channel_mode,
            compiled_plan_available,
            visible_errors: &visible_errors,
            schema_warnings: &schema_warning_strings,
        },
    );
    drop(pipeline_guard);

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

            div { class: "klinx-inspector-section",
                SectionHeader { title: "LINEAGE" }
                div { class: "klinx-field-lineage-summary",
                    span { "{field.upstream.len()} upstream" }
                    span { "{field.downstream.len()} downstream" }
                    span { "{field.role_usages.len()} role uses" }
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
                TraceList {
                    title: "UPSTREAM",
                    entries: field.upstream.clone(),
                    empty: "No upstream fields."
                }
                TraceList {
                    title: "DOWNSTREAM",
                    entries: field.downstream.clone(),
                    empty: "No downstream fields."
                }
                RoleUsageList { usages: field.role_usages.clone() }
            }

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

#[component]
fn TraceList(title: &'static str, entries: Vec<TraceEndpointView>, empty: &'static str) -> Element {
    let state = use_app_state();
    rsx! {
        div {
            class: "klinx-field-trace-group",
            div { class: "klinx-field-trace-title", "{title}" }
            if entries.is_empty() {
                div { class: "klinx-field-trace-empty", "{empty}" }
            } else {
                div {
                    class: "klinx-field-trace-list",
                    for entry in entries.iter() {
                        // Clicking a hop selects that field on the canvas (#151):
                        // it writes the shared `SelectedField`, which the canvas
                        // reveal effect resolves to a node + reveals, and from
                        // which the inspector rebuilds onto the new field. Field
                        // selection supersedes any node selection, mirroring the
                        // canvas field-row click.
                        div {
                            key: "{entry.stage_id}-{entry.field_name}-{entry.hop}",
                            class: "klinx-field-trace-row klinx-field-trace-row--selectable",
                            "data-stage-kind": "{entry.stage_kind_attr}",
                            onclick: {
                                let target = entry.to_selected_field();
                                move |_| {
                                    let mut selected_field = state.selected_field;
                                    let mut selected_stages = state.selected_stages;
                                    selected_field.set(Some(target.clone()));
                                    selected_stages.set(std::collections::HashSet::new());
                                }
                            },
                            span { class: "klinx-field-trace-hop", "h{entry.hop}" }
                            span { class: "klinx-field-trace-main",
                                span { class: "klinx-field-trace-stage", "{entry.stage_label}" }
                                span { class: "klinx-field-trace-field", "{entry.field_name}" }
                            }
                            span {
                                class: "klinx-field-trace-kind",
                                "data-kind": "{entry.edge_kind_attr}",
                                "{entry.edge_kind_label}"
                            }
                            // Per-hop precision badge (#148): the tier of the edge
                            // taken to reach this hop, so a reader sees where the
                            // trace becomes an over-approximation.
                            span {
                                class: "klinx-field-trace-precision",
                                "data-precision": "{entry.precision.precision_attr()}",
                                "{entry.precision.precision_label()}"
                            }
                        }
                    }
                }
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
