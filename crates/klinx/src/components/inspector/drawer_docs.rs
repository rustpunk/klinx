use dioxus::prelude::*;

use crate::autodoc::{ConfigCategory, StageDoc, generate_stage_doc};
use crate::notes::parse_notes;
use crate::state::use_app_state;

/// Docs drawer — full stage documentation with Blueprint sub-aesthetic.
///
/// Content:
/// 1. Summary + user description
/// 2. Schema table (fields, types, constraints)
/// 3. Lineage table (emit field → input refs)
/// 4. Contract section (requires/produces)
/// 5. Config section (grouped by category)
/// 6. Provenance section (composition origin, override diff)
/// 7. Channel override section
/// 8. Footer: "AUTODOC"
#[component]
pub fn DrawerDocs(stage_id: String) -> Element {
    let state = use_app_state();

    let pipeline_guard = (state.pipeline).read();
    let config = match pipeline_guard.as_ref() {
        Some(c) => c,
        None => {
            return rsx! {
                div {
                    class: "klinx-drawer-content klinx-drawer-content--docs",
                    div { class: "klinx-drawer-placeholder", "No pipeline loaded" }
                }
            };
        }
    };

    let Some(doc) = generate_stage_doc(config, &stage_id) else {
        return rsx! {
            div {
                class: "klinx-drawer-content klinx-drawer-content--docs",
                div { class: "klinx-drawer-placeholder", "No documentation for this stage" }
            }
        };
    };

    // Get the stage note from _notes (if any)
    let notes_value = config.stage_notes(&stage_id);
    let notes = parse_notes(notes_value);

    // Group config entries by category
    let config_groups = group_config_entries(&doc);

    rsx! {
        div {
            class: "klinx-drawer-content klinx-drawer-content--docs",

            // ── Summary + user description ───────────────────────────────
            div {
                class: "klinx-docs-description",
                style: "position: relative;",
                span {
                    class: "klinx-stage-card-stamp",
                    "autodoc"
                }
                "{doc.summary}"
            }

            if let Some(ref desc) = doc.user_description {
                div {
                    class: "klinx-docs-user-desc",
                    "{desc}"
                }
            }

            // ── User-authored stage note (when present) ──────────────────
            if !notes.stage_note.is_empty() {
                div {
                    class: "klinx-docs-note-section",
                    span { class: "klinx-docs-note-label", "NOTE" }
                    div {
                        class: "klinx-docs-note-block",
                        "{notes.stage_note}"
                    }
                }
            }

            // ── Channel override section ─────────────────────────────────
            if let Some(ref co) = doc.channel_override {
                div {
                    class: "klinx-docs-section",
                    span { class: "klinx-docs-section-label", "CHANNEL OVERRIDE" }
                    div {
                        class: "klinx-docs-metadata",
                        div { class: "klinx-docs-meta-row",
                            span { class: "klinx-docs-meta-key", "CHANNEL" }
                            span { class: "klinx-docs-meta-value", "{co.channel_id}" }
                        }
                        div { class: "klinx-docs-meta-row",
                            span { class: "klinx-docs-meta-key", "ACTION" }
                            span { class: "klinx-docs-meta-value klinx-docs-meta-value--badge", "{co.override_kind}" }
                        }
                        div { class: "klinx-docs-meta-row",
                            span { class: "klinx-docs-meta-key", "SOURCE" }
                            span { class: "klinx-docs-meta-value", "{co.override_source}" }
                        }
                        div { class: "klinx-docs-meta-row",
                            span { class: "klinx-docs-meta-key", "FILE" }
                            span { class: "klinx-docs-meta-value klinx-docs-meta-value--path", "{co.override_file}" }
                        }
                    }
                }
            }

            // ── Schema table ─────────────────────────────────────────────
            if let Some(ref schema) = doc.schema {
                div {
                    class: "klinx-docs-section",
                    span { class: "klinx-docs-section-label",
                        match &schema.source {
                            crate::autodoc::SchemaOrigin::File(path) => format!("SCHEMA (from {})", path),
                            crate::autodoc::SchemaOrigin::Inline => "SCHEMA (inline)".to_string(),
                            crate::autodoc::SchemaOrigin::OverridesOnly => "SCHEMA (overrides)".to_string(),
                            crate::autodoc::SchemaOrigin::None => "SCHEMA".to_string(),
                        }
                    }
                    if !schema.fields.is_empty() {
                        div {
                            class: "klinx-docs-schema-table",
                            // Header row
                            div { class: "klinx-docs-schema-row klinx-docs-schema-row--header",
                                span { class: "klinx-docs-schema-cell", "Name" }
                                span { class: "klinx-docs-schema-cell", "Type" }
                                span { class: "klinx-docs-schema-cell", "Required" }
                                span { class: "klinx-docs-schema-cell", "Format" }
                                span { class: "klinx-docs-schema-cell", "Default" }
                            }
                            for field in schema.fields.iter() {
                                div { class: "klinx-docs-schema-row",
                                    key: "schema-{field.name}",
                                    span { class: "klinx-docs-schema-cell klinx-docs-schema-cell--name", "{field.name}" }
                                    span { class: "klinx-docs-schema-cell",
                                        {field.field_type.as_deref().unwrap_or("—")}
                                    }
                                    span { class: "klinx-docs-schema-cell",
                                        if field.required { "yes" } else { "—" }
                                    }
                                    span { class: "klinx-docs-schema-cell",
                                        {field.format.as_deref().unwrap_or("—")}
                                    }
                                    span { class: "klinx-docs-schema-cell",
                                        {field.default_value.as_deref().unwrap_or("—")}
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // ── CXL analysis ─────────────────────────────────────────────
            if let Some(ref analysis) = doc.cxl_analysis {
                div {
                    class: "klinx-docs-section",
                    span { class: "klinx-docs-section-label", "CXL ANALYSIS" }

                    // All fields referenced summary
                    if !analysis.all_field_refs.is_empty() {
                        div { class: "klinx-docs-field-refs-summary",
                            span { class: "klinx-docs-lineage-refs-label", "fields referenced: " }
                            for r in analysis.all_field_refs.iter() {
                                span { class: "klinx-docs-lineage-ref", "{r}" }
                            }
                        }
                    }

                    // Classified statements
                    div {
                        class: "klinx-docs-lineage-table",
                        for (i, stmt) in analysis.statements.iter().enumerate() {
                            div { class: "klinx-docs-lineage-row",
                                key: "stmt-{i}",
                                // Kind badge + output field
                                div { class: "klinx-docs-lineage-field",
                                    span {
                                        class: "klinx-docs-stmt-badge",
                                        "data-kind": stmt.kind.label().to_lowercase(),
                                        "{stmt.kind.label()}"
                                    }
                                    if let Some(ref out) = stmt.output_field {
                                        span { class: "klinx-docs-column-tag klinx-docs-column-tag--added",
                                            "+{out}"
                                        }
                                    }
                                }
                                // Expression
                                div { class: "klinx-docs-lineage-expr",
                                    code { class: "klinx-docs-lineage-code", "{stmt.expression}" }
                                }
                                // Field refs
                                if !stmt.field_refs.is_empty() {
                                    div { class: "klinx-docs-lineage-refs",
                                        span { class: "klinx-docs-lineage-refs-label", "reads: " }
                                        for r in stmt.field_refs.iter() {
                                            span { class: "klinx-docs-lineage-ref", "{r}" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // ── Contract section ─────────────────────────────────────────
            if let Some(ref contract) = doc.contract {
                div {
                    class: "klinx-docs-section",
                    span { class: "klinx-docs-section-label",
                        "CONTRACT ({contract.composition_name})"
                    }
                    if !contract.requires.is_empty() {
                        div { class: "klinx-docs-contract-group",
                            span { class: "klinx-docs-contract-heading", "REQUIRES" }
                            for f in contract.requires.iter() {
                                div { class: "klinx-docs-contract-field",
                                    key: "req-{f.name}",
                                    span { class: "klinx-docs-contract-name", "{f.name}" }
                                    span { class: "klinx-docs-contract-type", "{f.field_type}" }
                                }
                            }
                        }
                    }
                    if !contract.produces.is_empty() {
                        div { class: "klinx-docs-contract-group",
                            span { class: "klinx-docs-contract-heading", "PRODUCES" }
                            for f in contract.produces.iter() {
                                div { class: "klinx-docs-contract-field",
                                    key: "prod-{f.name}",
                                    span { class: "klinx-docs-contract-name", "{f.name}" }
                                    span { class: "klinx-docs-contract-type", "{f.field_type}" }
                                }
                            }
                        }
                    }
                }
            }

            // ── Config section (grouped by category) ─────────────────────
            for (label, entries) in config_groups.iter() {
                div {
                    class: "klinx-docs-section",
                    span { class: "klinx-docs-section-label", "{label}" }
                    div {
                        class: "klinx-docs-metadata",
                        for entry in entries.iter() {
                            div {
                                key: "cfg-{entry.key}",
                                class: "klinx-docs-meta-row",
                                span { class: "klinx-docs-meta-key", "{entry.key}" }
                                span { class: "klinx-docs-meta-value", "{entry.value}" }
                            }
                        }
                    }
                }
            }

            // ── Provenance section ───────────────────────────────────────
            if let Some(ref prov) = doc.provenance {
                div {
                    class: "klinx-docs-section",
                    span { class: "klinx-docs-section-label", "PROVENANCE" }
                    div {
                        class: "klinx-docs-metadata",
                        div { class: "klinx-docs-meta-row",
                            span { class: "klinx-docs-meta-key", "COMPOSITION" }
                            span { class: "klinx-docs-meta-value", "{prov.composition_name}" }
                        }
                        div { class: "klinx-docs-meta-row",
                            span { class: "klinx-docs-meta-key", "PATH" }
                            span { class: "klinx-docs-meta-value klinx-docs-meta-value--path", "{prov.composition_path}" }
                        }
                        if let Some(ref ver) = prov.composition_version {
                            div { class: "klinx-docs-meta-row",
                                span { class: "klinx-docs-meta-key", "VERSION" }
                                span { class: "klinx-docs-meta-value", "{ver}" }
                            }
                        }
                        if prov.is_overridden {
                            div { class: "klinx-docs-meta-row",
                                span { class: "klinx-docs-meta-key", "STATUS" }
                                span { class: "klinx-docs-meta-value klinx-docs-meta-value--badge", "OVERRIDDEN" }
                            }
                        }
                    }

                    // Override diff
                    if prov.is_overridden {
                        if let Some(ref original) = prov.original_cxl {
                            div { class: "klinx-docs-diff",
                                span { class: "klinx-docs-diff-label", "ORIGINAL CXL" }
                                pre { class: "klinx-docs-diff-block klinx-docs-diff-block--original", "{original}" }
                            }
                        }
                        if let Some(ref current) = prov.current_cxl {
                            div { class: "klinx-docs-diff",
                                span { class: "klinx-docs-diff-label", "CURRENT CXL" }
                                pre { class: "klinx-docs-diff-block klinx-docs-diff-block--current", "{current}" }
                            }
                        }
                    }
                }
            }

            // ── Footer ───────────────────────────────────────────────────
            div {
                class: "klinx-docs-footer",
                span { class: "klinx-docs-footer-rule" }
                span { class: "klinx-docs-footer-label", "AUTODOC" }
                span { class: "klinx-docs-footer-rule" }
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
