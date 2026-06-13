use dioxus::prelude::*;

use crate::autodoc::{ConfigCategory, StageDoc};
use crate::notes::StageNotes;

/// A single stage detail card in the Schematics content area.
///
/// Full technical documentation: CXL code, schema tables, field analysis,
/// contracts, config grids, composition sub-cards. The card IS the
/// documentation, not a pointer to it.
#[component]
pub fn StageCard(
    index: usize,
    stage_id: String,
    kind_attr: &'static str,
    badge: &'static str,
    doc: StageDoc,
    notes: StageNotes,
) -> Element {
    let idx = format!("{:02}", index);
    let delay = index as f64 * 0.1;

    // Group config entries by category
    let config_groups = group_config_entries(&doc);

    rsx! {
        div {
            class: "klinx-stage-card",
            "data-stage-kind": kind_attr,
            style: "border-top-color: var(--klinx-stage-accent); animation: blueprintIn 0.5s ease {delay}s both;",

            // Title block stamp
            span {
                class: "klinx-stage-card-stamp",
                "{badge}"
            }

            // Header: index + label + type badge
            div {
                class: "klinx-stage-card-header",
                span { class: "klinx-stage-card-index", "{idx}" }
                span { class: "klinx-stage-card-label", "{stage_id}" }
                span {
                    class: "klinx-stage-card-badge",
                    style: "color: var(--klinx-stage-accent); border-color: color-mix(in srgb, var(--klinx-stage-accent) 25%, transparent); \
                            background: color-mix(in srgb, var(--klinx-stage-accent) 12%, transparent);",
                    "{badge}"
                }
            }

            hr { class: "klinx-stage-card-rule" }

            // ── Summary ──────────────────────────────────────────────────
            div {
                class: "klinx-stage-card-description",
                "{doc.summary}"
            }

            // ── User note ────────────────────────────────────────────────
            if !notes.stage_note.is_empty() {
                div {
                    class: "klinx-stage-card-note",
                    span { class: "klinx-stage-card-note-label", "NOTE" }
                    div {
                        class: "klinx-stage-card-note-block",
                        "{notes.stage_note}"
                    }
                }
            }

            // ── Channel override ─────────────────────────────────────────
            if let Some(ref co) = doc.channel_override {
                div {
                    class: "klinx-card-section",
                    span { class: "klinx-card-section-label", "CHANNEL OVERRIDE" }
                    div { class: "klinx-card-meta",
                        span { class: "klinx-card-meta-key", "{co.override_kind}" }
                        span { class: "klinx-card-meta-value", "via {co.override_source}" }
                    }
                }
            }

            // ── CXL source code ──────────────────────────────────────────
            if let Some(ref cxl) = doc.cxl_source {
                div {
                    class: "klinx-card-section",
                    span { class: "klinx-card-section-label", "CXL" }
                    pre {
                        class: "klinx-card-cxl-block",
                        "{cxl.trim()}"
                    }
                }
            }

            // ── CXL analysis ─────────────────────────────────────────────
            if let Some(ref analysis) = doc.cxl_analysis {
                div {
                    class: "klinx-card-section",
                    span { class: "klinx-card-section-label", "FIELD ANALYSIS" }
                    for (i, stmt) in analysis.statements.iter().enumerate() {
                        div { class: "klinx-card-stmt",
                            key: "stmt-{i}",
                            span {
                                class: "klinx-docs-stmt-badge",
                                "data-kind": stmt.kind.label().to_lowercase(),
                                "{stmt.kind.label()}"
                            }
                            if let Some(ref out) = stmt.output_field {
                                span { class: "klinx-card-emit-field", "+{out}" }
                            }
                            if !stmt.field_refs.is_empty() {
                                span { class: "klinx-card-reads",
                                    "reads "
                                    for (j, r) in stmt.field_refs.iter().enumerate() {
                                        if j > 0 { ", " }
                                        span { class: "klinx-card-field-ref", "{r}" }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // ── Schema table ─────────────────────────────────────────────
            if let Some(ref schema) = doc.schema {
                div {
                    class: "klinx-card-section",
                    span { class: "klinx-card-section-label",
                        match &schema.source {
                            crate::autodoc::SchemaOrigin::File(path) => format!("SCHEMA ({})", path),
                            crate::autodoc::SchemaOrigin::Inline => "SCHEMA".to_string(),
                            crate::autodoc::SchemaOrigin::OverridesOnly => "SCHEMA OVERRIDES".to_string(),
                            crate::autodoc::SchemaOrigin::None => "FIELDS".to_string(),
                        }
                    }
                    if !schema.fields.is_empty() {
                        div { class: "klinx-card-schema",
                            div { class: "klinx-card-schema-header",
                                span { "Name" }
                                span { "Type" }
                                span { "Req" }
                            }
                            for field in schema.fields.iter() {
                                div { class: "klinx-card-schema-row",
                                    key: "sf-{field.name}",
                                    span { class: "klinx-card-schema-name", "{field.name}" }
                                    span { {field.field_type.as_deref().unwrap_or("—")} }
                                    span { if field.required { "yes" } else { "—" } }
                                }
                            }
                        }
                    }
                }
            }

            // ── Contract ─────────────────────────────────────────────────
            if let Some(ref contract) = doc.contract {
                div {
                    class: "klinx-card-section",
                    span { class: "klinx-card-section-label", "CONTRACT" }
                    if !contract.requires.is_empty() {
                        div { class: "klinx-card-contract-group",
                            span { class: "klinx-card-contract-heading", "REQUIRES" }
                            for f in contract.requires.iter() {
                                span { class: "klinx-card-contract-field",
                                    key: "req-{f.name}",
                                    "{f.name}: {f.field_type}"
                                }
                            }
                        }
                    }
                    if !contract.produces.is_empty() {
                        div { class: "klinx-card-contract-group",
                            span { class: "klinx-card-contract-heading", "PRODUCES" }
                            for f in contract.produces.iter() {
                                span { class: "klinx-card-contract-field",
                                    key: "prod-{f.name}",
                                    "{f.name}: {f.field_type}"
                                }
                            }
                        }
                    }
                }
            }

            // ── Composition sub-stages ───────────────────────────────────
            if !doc.sub_stages.is_empty() {
                div {
                    class: "klinx-card-section",
                    span { class: "klinx-card-section-label", "TRANSFORMS" }
                    for (si, sub) in doc.sub_stages.iter().enumerate() {
                        {
                            let _sub_name = sub.summary.split(':').next()
                                .or_else(|| sub.summary.split('.').next())
                                .unwrap_or(&sub.summary);
                            // Find the actual stage name from config entries
                            let _sub_stage_name = sub.config.entries.iter()
                                .find(|e| e.key == "TYPE")
                                .map(|_| {
                                    // Use provenance or fallback
                                    sub.provenance.as_ref()
                                        .map(|p| p.composition_name.clone())
                                        .unwrap_or_default()
                                })
                                .unwrap_or_default();

                            rsx! {
                                div { class: "klinx-card-substage",
                                    key: "sub-{si}",

                                    // Sub-stage header
                                    div { class: "klinx-card-substage-header",
                                        span { class: "klinx-card-substage-index", "{si:02}" }
                                        span { class: "klinx-card-substage-summary", "{sub.summary}" }
                                    }

                                    // CXL source
                                    if let Some(ref cxl) = sub.cxl_source {
                                        pre {
                                            class: "klinx-card-cxl-block",
                                            "{cxl.trim()}"
                                        }
                                    }

                                    // CXL analysis
                                    if let Some(ref analysis) = sub.cxl_analysis {
                                        div { class: "klinx-card-substage-analysis",
                                            for (i, stmt) in analysis.statements.iter().enumerate() {
                                                div { class: "klinx-card-stmt",
                                                    key: "sub-stmt-{i}",
                                                    span {
                                                        class: "klinx-docs-stmt-badge",
                                                        "data-kind": stmt.kind.label().to_lowercase(),
                                                        "{stmt.kind.label()}"
                                                    }
                                                    if let Some(ref out) = stmt.output_field {
                                                        span { class: "klinx-card-emit-field", "+{out}" }
                                                    }
                                                    if !stmt.field_refs.is_empty() {
                                                        span { class: "klinx-card-reads",
                                                            "reads "
                                                            for (j, r) in stmt.field_refs.iter().enumerate() {
                                                                if j > 0 { ", " }
                                                                span { class: "klinx-card-field-ref", "{r}" }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }

                                    // Provenance (override info)
                                    if let Some(ref prov) = sub.provenance {
                                        if prov.is_overridden {
                                            div { class: "klinx-card-override-indicator",
                                                "overridden"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // ── Provenance ───────────────────────────────────────────────
            if let Some(ref prov) = doc.provenance {
                div {
                    class: "klinx-card-section",
                    span { class: "klinx-card-section-label", "PROVENANCE" }
                    div { class: "klinx-card-meta",
                        span { class: "klinx-card-meta-key", "from" }
                        span { class: "klinx-card-meta-value", "{prov.composition_name}" }
                    }
                    div { class: "klinx-card-meta",
                        span { class: "klinx-card-meta-key", "path" }
                        span { class: "klinx-card-meta-value klinx-card-meta-path", "{prov.composition_path}" }
                    }
                    if prov.is_overridden {
                        div { class: "klinx-card-meta",
                            span { class: "klinx-card-meta-key", "status" }
                            span { class: "klinx-card-meta-value klinx-card-override-indicator", "overridden" }
                        }
                    }
                }
            }

            // ── Config grid ──────────────────────────────────────────────
            if !config_groups.is_empty() {
                div {
                    class: "klinx-card-section",
                    span { class: "klinx-card-section-label", "CONFIGURATION" }
                    for (label, entries) in config_groups.iter() {
                        div { class: "klinx-card-config-group",
                            if config_groups.len() > 1 {
                                span { class: "klinx-card-config-category", "{label}" }
                            }
                            for entry in entries.iter() {
                                div { class: "klinx-card-meta",
                                    key: "cfg-{entry.key}",
                                    span { class: "klinx-card-meta-key", "{entry.key}" }
                                    span { class: "klinx-card-meta-value", "{entry.value}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Group config entries by category for rendering.
fn group_config_entries(doc: &StageDoc) -> Vec<(String, Vec<&crate::autodoc::ConfigEntry>)> {
    let mut groups: Vec<(ConfigCategory, Vec<&crate::autodoc::ConfigEntry>)> = Vec::new();

    for entry in &doc.config.entries {
        if let Some(group) = groups.iter_mut().find(|(cat, _)| *cat == entry.category) {
            group.1.push(entry);
        } else {
            groups.push((entry.category.clone(), vec![entry]));
        }
    }

    groups
        .into_iter()
        .map(|(cat, entries)| (cat.label().to_string(), entries))
        .collect()
}
