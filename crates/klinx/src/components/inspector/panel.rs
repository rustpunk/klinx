use dioxus::prelude::*;

use crate::state::use_app_state;

use super::cxl_input::CxlInput;
use super::drawer_bar::{ActiveDrawer, DrawerToggleBar};
use super::drawer_docs::DrawerDocs;
use super::drawer_notes::DrawerNotes;
use super::drawer_run::DrawerRun;
use super::scoped_yaml::ScopedYaml;
use super::stage_header::StageHeader;

/// One declarative rule of a Reshape or Cull node, flattened for inspector
/// display. `predicate` is the rule's CXL boolean (`when` on Reshape,
/// `drop_group_when` on Cull); `detail` summarizes the rule's action (the
/// mutate/synthesize shape on Reshape; empty on Cull, whose action is implicit
/// in routing the group to `removed_to`).
#[derive(Clone, PartialEq)]
struct OperatorRule {
    name: String,
    predicate: String,
    detail: String,
}

/// Inspector view-model for the config body of a per-group / framing operator
/// (Reshape, Cull, Envelope). Carries the small set of scalar facts plus the
/// rule list; the panel renders one CONFIG-BODY section from it. The standard
/// CONFIGURATION section above still shows the subtitle.
#[derive(Clone, PartialEq)]
struct OperatorBodyView {
    /// Section heading, e.g. "RESHAPE", "CULL", "ENVELOPE".
    title: &'static str,
    /// Scalar `(label, value)` rows, in display order.
    scalars: Vec<(&'static str, String)>,
    /// Declarative rule rows (empty for Envelope).
    rules: Vec<OperatorRule>,
}

/// Subtitle for a per-group operator: its partition key (the grouping every rule
/// observes), or a note that it is ungrouped when none is declared.
fn partition_subtitle(partition_by: &[String]) -> String {
    if partition_by.is_empty() {
        "ungrouped".to_string()
    } else {
        format!("partition_by: {}", partition_by.join(", "))
    }
}

/// Stable lowercase name of an [`EnvelopeStrategy`] for inspector display.
fn envelope_strategy_name(
    strategy: &clinker_plan::config::pipeline_node::EnvelopeStrategy,
) -> &'static str {
    use clinker_plan::config::pipeline_node::EnvelopeStrategy;
    match strategy {
        EnvelopeStrategy::Preserve => "preserve",
        EnvelopeStrategy::Concat => "concat",
    }
}

/// Render the within-group ordering of a Reshape/Cull body as a compact
/// comma-joined field list (`field asc, other desc`), or "—" when unset.
fn order_by_summary(order_by: &[clinker_plan::config::SortField]) -> String {
    use clinker_plan::config::SortOrder;
    if order_by.is_empty() {
        return "\u{2014}".to_string();
    }
    order_by
        .iter()
        .map(|f| {
            // Mirror the engine's `asc`/`desc` vocabulary so the display matches
            // the authored YAML.
            let dir = match f.order {
                SortOrder::Asc => "asc",
                SortOrder::Desc => "desc",
            };
            format!("{} {dir}", f.field)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Build the inspector config-body view for a per-group / framing operator, or
/// `None` for every other node kind (their config is the standard subtitle +
/// CXL block). Reads the exact `ReshapeBody` / `CullBody` / `EnvelopeBody` field
/// shapes from clinker-plan's `config::pipeline_node`.
fn operator_body_view(node: &clinker_plan::config::PipelineNode) -> Option<OperatorBodyView> {
    use clinker_plan::config::PipelineNode;
    match node {
        PipelineNode::Reshape { config: body, .. } => Some(OperatorBodyView {
            title: "RESHAPE",
            scalars: vec![
                ("partition_by", join_or_dash(&body.partition_by)),
                ("order_by", order_by_summary(&body.order_by)),
            ],
            rules: body
                .rules
                .iter()
                .map(|r| OperatorRule {
                    name: r.name.clone(),
                    predicate: r.when.as_ref().to_string(),
                    // Name which actions the rule applies — the two facts that
                    // distinguish a mutate-only rule from one that also (or only)
                    // synthesizes new rows.
                    detail: reshape_rule_actions(r),
                })
                .collect(),
        }),
        PipelineNode::Cull { config: body, .. } => Some(OperatorBodyView {
            title: "CULL",
            scalars: vec![
                ("partition_by", join_or_dash(&body.partition_by)),
                ("order_by", order_by_summary(&body.order_by)),
                ("removed_to", body.removed_to.clone()),
            ],
            rules: body
                .rules
                .iter()
                .map(|r| OperatorRule {
                    name: r.name.clone(),
                    predicate: r.drop_group_when.as_ref().to_string(),
                    detail: String::new(),
                })
                .collect(),
        }),
        PipelineNode::Envelope { config: body, .. } => Some(OperatorBodyView {
            title: "ENVELOPE",
            scalars: vec![(
                "strategy",
                envelope_strategy_name(&body.strategy).to_string(),
            )],
            rules: Vec::new(),
        }),
        _ => None,
    }
}

/// A Reshape rule's actions named in display order: `mutate`, `synthesize`, both,
/// or "trigger-only" when neither is set (a pure predicate with no action — valid
/// but inert). Surfaces what the rule DOES without re-rendering its full CXL.
fn reshape_rule_actions(rule: &clinker_plan::config::pipeline_node::ReshapeRule) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if rule.mutate.is_some() {
        parts.push("mutate");
    }
    if rule.synthesize.is_some() {
        parts.push("synthesize");
    }
    if parts.is_empty() {
        "trigger-only".to_string()
    } else {
        parts.join(" + ")
    }
}

/// Comma-join a field list, or an em dash when empty — for compact scalar rows.
fn join_or_dash(fields: &[String]) -> String {
    if fields.is_empty() {
        "\u{2014}".to_string()
    } else {
        fields.join(", ")
    }
}

/// Four-concern inspector panel: Config (always visible) + Run/Docs/Notes drawer.
///
/// Keyed on `stage_id` in the parent so selection changes cause a full remount
/// with fresh signals (drawer state resets on selection change).
#[component]
pub fn InspectorPanel(stage_id: String) -> Element {
    let state = use_app_state();
    let mut active_drawer = use_signal(|| ActiveDrawer::None);

    let pipeline_guard = (state.pipeline).read();
    let Some(config) = pipeline_guard.as_ref() else {
        return rsx! {};
    };

    // Dispatch inspector content on the `PipelineNode` variant tag. Every
    // variant is handled explicitly so adding a new one is a compile break
    // here.
    use clinker_plan::config::PipelineNode;
    let Some(node_spanned) = config.nodes.iter().find(|n| n.value.name() == stage_id) else {
        return rsx! {};
    };
    let (kind_label, kind_attr, subtitle, cxl_source) = match &node_spanned.value {
        PipelineNode::Source { config: body, .. } => {
            ("SOURCE", "source", body.source.display_target(), None)
        }
        PipelineNode::Transform { config: body, .. } => (
            "TRANSFORM",
            "transform",
            String::new(),
            Some(body.cxl.as_ref().to_string()),
        ),
        PipelineNode::Aggregate { config: body, .. } => {
            let subtitle = if body.group_by.is_empty() {
                String::new()
            } else {
                format!("group_by: {}", body.group_by.join(", "))
            };
            (
                "AGGREGATE",
                "aggregate",
                subtitle,
                Some(body.cxl.as_ref().to_string()),
            )
        }
        PipelineNode::Route { config: body, .. } => {
            let subtitle = format!(
                "{} branch{} → {}",
                body.conditions.len(),
                if body.conditions.len() == 1 { "" } else { "es" },
                body.default
            );
            ("ROUTE", "route", subtitle, None)
        }
        PipelineNode::Merge { header, .. } => (
            "MERGE",
            "merge",
            format!("{} inputs", header.inputs.len()),
            None,
        ),
        PipelineNode::Combine {
            header,
            config: body,
        } => (
            "COMBINE",
            "combine",
            format!("{} inputs", header.input.len()),
            Some(body.cxl.as_ref().to_string()),
        ),
        PipelineNode::Output { config: body, .. } => {
            ("OUTPUT", "output", body.output.path.clone(), None)
        }
        PipelineNode::Composition {
            r#use, config: _, ..
        } => (
            "COMPOSITION",
            "composition",
            format!("use: {}", r#use.display()),
            None,
        ),
        // Reshape/Cull/Envelope each get their own kind attr (driving the header
        // accent) and a subtitle naming the partition key or framing strategy.
        // Their config-body fields render in a dedicated section below; they carry
        // no editable top-level `cxl:` block (CXL lives inside per-rule fields).
        PipelineNode::Reshape { config: body, .. } => (
            "RESHAPE",
            "reshape",
            partition_subtitle(&body.partition_by),
            None,
        ),
        PipelineNode::Cull { config: body, .. } => {
            ("CULL", "cull", partition_subtitle(&body.partition_by), None)
        }
        PipelineNode::Envelope { config: body, .. } => (
            "ENVELOPE",
            "envelope",
            format!("strategy: {}", envelope_strategy_name(&body.strategy)),
            None,
        ),
    };

    // Config-body card rows for the per-group / framing operators. Each is a
    // labelled section the RSX renders below the standard CONFIGURATION block.
    // `None` for every other variant (their config is the subtitle + CXL above).
    let op_body = operator_body_view(&node_spanned.value);

    // Collect config param names for composition provenance display
    let composition_params: Vec<String> = match &node_spanned.value {
        PipelineNode::Composition { config, .. } => config.keys().cloned().collect(),
        _ => Vec::new(),
    };
    let is_source_or_output = matches!(
        &node_spanned.value,
        PipelineNode::Source { .. } | PipelineNode::Output { .. }
    );
    let drawer_open = (active_drawer)() != ActiveDrawer::None;

    rsx! {
        div {
            class: "klinx-inspector",
            onmousedown: move |e: MouseEvent| e.stop_propagation(),

            // ── Stage header ──────────────────────────────────────────────
            StageHeader {
                stage_id: stage_id.clone(),
                kind_label,
                kind_attr,
                label: stage_id.clone(),
            }

            // ── Config section (upper, always visible) ────────────────────
            div {
                class: "klinx-inspector-config",
                "data-compressed": if drawer_open { "true" } else { "false" },

                div {
                    class: "klinx-inspector-section",

                    div {
                        class: "klinx-section-header",
                        span { class: "klinx-diamond", "\u{25C6}" }
                        span { class: "klinx-section-title", "CONFIGURATION" }
                        span { class: "klinx-section-rule" }
                    }

                    if !subtitle.is_empty() {
                        div {
                            class: "klinx-cxl-field",
                            label { class: "klinx-cxl-label",
                                if is_source_or_output { "PATH" } else { "DESCRIPTION" }
                            }
                            div {
                                class: "klinx-inspector-value",
                                "{subtitle}"
                            }
                        }
                    }

                    if let Some(ref cxl) = cxl_source {
                        CxlInput {
                            key: "{stage_id}-cxl",
                            label: "cxl",
                            initial_value: cxl.clone(),
                        }
                    }
                }

                // ── Operator config-body section (Reshape / Cull / Envelope) ──
                // Reads the variant's `*Body` fields: scalar rows (partition_by,
                // order_by, removed_to, strategy) then the declarative rule list.
                if let Some(ref body) = op_body {
                    div {
                        class: "klinx-inspector-section",

                        div {
                            class: "klinx-section-header",
                            span { class: "klinx-diamond", "\u{25C6}" }
                            span { class: "klinx-section-title", "{body.title}" }
                            span { class: "klinx-section-rule" }
                        }

                        for (field_label, value) in body.scalars.iter() {
                            div {
                                // Scalar labels are unique within a body, so the
                                // label is a stable key for this row.
                                key: "{field_label}",
                                class: "klinx-cxl-field",
                                label { class: "klinx-cxl-label", "{field_label}" }
                                div { class: "klinx-inspector-value", "{value}" }
                            }
                        }

                        if !body.rules.is_empty() {
                            div {
                                class: "klinx-cxl-field",
                                label { class: "klinx-cxl-label", "rules" }
                                for rule in body.rules.iter() {
                                    div {
                                        // Rule names are unique within a body
                                        // (engine-enforced), so they key stably.
                                        key: "{rule.name}",
                                        class: "klinx-op-rule",
                                        div { class: "klinx-op-rule-head",
                                            span { class: "klinx-op-rule-name", "{rule.name}" }
                                            if !rule.detail.is_empty() {
                                                span { class: "klinx-op-rule-detail", "{rule.detail}" }
                                            }
                                        }
                                        div { class: "klinx-op-rule-predicate", "{rule.predicate}" }
                                    }
                                }
                            }
                        }
                    }
                }

                ScopedYaml {
                    stage_id: stage_id.clone(),
                }

                // ── Provenance section (composition nodes only) ──────────
                if !composition_params.is_empty() {
                    div {
                        class: "klinx-inspector-section",

                        div {
                            class: "klinx-section-header",
                            span { class: "klinx-diamond", "\u{25C6}" }
                            span { class: "klinx-section-title", "PROVENANCE" }
                            span { class: "klinx-section-rule" }
                        }

                        for param in composition_params.iter() {
                            {
                                let node = stage_id.clone();
                                let p = param.clone();
                                rsx! {
                                    div {
                                        class: "klinx-provenance-field",
                                        div {
                                            class: "klinx-provenance-field-name",
                                            "{p}"
                                        }
                                        super::provenance::ProvenancePanel {
                                            node_name: node,
                                            param_name: p.clone(),
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // ── Drawer toggle bar (always visible) ────────────────────────
            DrawerToggleBar {
                active: (active_drawer)(),
                on_toggle: move |drawer: ActiveDrawer| {
                    active_drawer.set(drawer);
                },
            }

            // ── Drawer region (expandable) ────────────────────────────────
            div {
                class: "klinx-drawer-region",
                "data-open": if drawer_open { "true" } else { "false" },

                match (active_drawer)() {
                    ActiveDrawer::Run => rsx! { DrawerRun {} },
                    ActiveDrawer::Docs => rsx! { DrawerDocs { stage_id: stage_id.clone() } },
                    ActiveDrawer::Notes => rsx! { DrawerNotes { stage_id: stage_id.clone() } },
                    ActiveDrawer::None => rsx! {},
                }
            }
        }
    }
}
