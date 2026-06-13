use dioxus::prelude::*;

use crate::components::yaml_sidebar::YamlLine;
use crate::components::yaml_sidebar::tokenizer::tokenize;
use crate::state::use_app_state;
use crate::sync::compute_yaml_ranges;

/// Bottom section of the inspector showing the selected stage's YAML block
/// with absolute line numbers from the full YAML.
#[component]
pub fn ScopedYaml(stage_id: String) -> Element {
    let state = use_app_state();
    let text = (state.yaml_text)();
    let pipeline_guard = (state.pipeline).read();

    let Some(config) = pipeline_guard.as_ref() else {
        return rsx! {};
    };

    let ranges = compute_yaml_ranges(&text, config);
    let Some(&(start, end)) = ranges.get(&stage_id) else {
        return rsx! {};
    };

    let all_lines = tokenize(&text);
    let start_idx = start.saturating_sub(1);
    let end_idx = end.min(all_lines.len());
    let scoped_lines = &all_lines[start_idx..end_idx];

    let section_title = "STAGE YAML";

    rsx! {
        div {
            class: "klinx-inspector-yaml",

            div {
                class: "klinx-section-header",
                span { class: "klinx-diamond", "\u{25C6}" }
                span { class: "klinx-section-title", "{section_title}" }
                span { class: "klinx-section-rule" }
            }

            div {
                class: "klinx-yaml-code-area klinx-inspector-yaml-area",

                // Gutter — absolute line numbers
                div {
                    class: "klinx-yaml-gutter",
                    for (i, _) in scoped_lines.iter().enumerate() {
                        {
                            let line_num = start + i;
                            rsx! {
                                div {
                                    key: "gutter-{i}",
                                    class: "klinx-yaml-line-num",
                                    "{line_num}"
                                }
                            }
                        }
                    }
                }

                // Code column
                div {
                    class: "klinx-yaml-code",
                    for (i, line_tokens) in scoped_lines.iter().enumerate() {
                        YamlLine {
                            key: "scoped-{i}",
                            tokens: line_tokens.clone(),
                            selected: false,
                        }
                    }
                }
            }
        }
    }
}
