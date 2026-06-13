//! Template gallery — centered overlay showing available pipeline templates.
//!
//! A 560px centered overlay with a card grid, format tabs, and search.
//! Opened via Ctrl+Shift+N, command palette, or welcome screen.

use dioxus::prelude::*;

use crate::state::TabManagerState;
use crate::tab::TabEntry;
use crate::template::{self, Template, TemplateSource};

/// Template gallery overlay component.
#[component]
pub fn TemplateGallery() -> Element {
    let mut tab_mgr = use_context::<TabManagerState>();
    let mut filter = use_signal(|| "All".to_string());
    let mut search = use_signal(String::new);

    let ws_root = (tab_mgr.workspace)().map(|w| w.root.clone());
    let all_templates = template::load_all_templates(ws_root.as_deref());

    let current_filter = (filter)();
    let current_search = (search)();

    let filtered: Vec<&Template> = all_templates
        .iter()
        .filter(|t| template::format_filter_matches(&current_filter, &t.format_category))
        .filter(|t| {
            if current_search.is_empty() {
                return true;
            }
            let q = current_search.to_lowercase();
            t.metadata.name.to_lowercase().contains(&q)
                || t.metadata.description.to_lowercase().contains(&q)
                || t.metadata
                    .tags
                    .iter()
                    .any(|tag| tag.to_lowercase().contains(&q))
        })
        .collect();

    let close = move |_| {
        tab_mgr.show_template_gallery.set(false);
    };

    rsx! {
        // Backdrop
        div {
            class: "klinx-gallery-backdrop",
            onclick: close,
        }

        // Gallery overlay
        div {
            class: "klinx-gallery",
            onclick: move |e: MouseEvent| e.stop_propagation(),

            // ── Header ──────────────────────────────────────────────
            div { class: "klinx-gallery__header",
                span { class: "klinx-gallery__title", "NEW FROM TEMPLATE" }
                button {
                    class: "klinx-gallery__close",
                    onclick: close,
                    "×"
                }
            }

            // ── Search ──────────────────────────────────────────────
            div { class: "klinx-gallery__search",
                input {
                    class: "klinx-gallery__search-input",
                    r#type: "text",
                    placeholder: "Search templates...",
                    value: "{search}",
                    oninput: move |e: FormEvent| search.set(e.value()),
                }
            }

            // ── Format tabs ─────────────────────────────────────────
            div { class: "klinx-gallery__tabs",
                for cat in template::FORMAT_CATEGORIES {
                    button {
                        class: if current_filter == *cat {
                            "klinx-gallery-tab klinx-gallery-tab--active"
                        } else {
                            "klinx-gallery-tab"
                        },
                        onclick: {
                            let cat = cat.to_string();
                            move |_| filter.set(cat.clone())
                        },
                        "{cat}"
                    }
                }
            }

            // ── Card grid ───────────────────────────────────────────
            div { class: "klinx-gallery__grid",
                if filtered.is_empty() {
                    div { class: "klinx-gallery__empty",
                        "No templates match the current filter."
                    }
                }

                for tmpl in filtered {
                    TemplateCard {
                        key: "{tmpl.metadata.name}",
                        template: tmpl.clone(),
                        on_use: {
                        move |yaml: String| {
                            // Strip _template block and open as new tab
                            let clean_yaml = template::strip_template_block(&yaml);
                            let new_tab = TabEntry::new_from_yaml(
                                &tab_mgr.tabs.read(),
                                clean_yaml,
                            );
                            let new_id = new_tab.id;
                            tab_mgr.tabs.write().push(new_tab);
                            tab_mgr.active_tab_id.set(Some(new_id));
                            tab_mgr.show_template_gallery.set(false);
                        }
                    },
                    }
                }
            }
        }
    }
}

/// A single template card in the gallery grid.
#[component]
fn TemplateCard(template: Template, on_use: EventHandler<String>) -> Element {
    let name = template.metadata.name.clone();
    let desc = template.metadata.description.clone();
    let tags = template.metadata.tags.clone();
    let format = template.format_category.clone();
    let source_label = match template.source {
        TemplateSource::Bundled => "built-in",
        TemplateSource::Workspace => "workspace",
    };
    let format_class = format!("klinx-gallery-card--{format}");
    let raw_yaml = template.raw_yaml.clone();

    rsx! {
        div {
            class: "klinx-gallery-card {format_class}",

            div { class: "klinx-gallery-card__header",
                span { class: "klinx-gallery-card__name", "{name}" }
                span { class: "klinx-gallery-card__source", "{source_label}" }
            }

            div { class: "klinx-gallery-card__desc", "{desc}" }

            div { class: "klinx-gallery-card__tags",
                for tag in tags {
                    span { class: "klinx-gallery-card__tag", "{tag}" }
                }
            }

            button {
                class: "klinx-gallery-card__use-btn",
                onclick: move |_| on_use.call(raw_yaml.clone()),
                "Use Template"
            }
        }
    }
}
