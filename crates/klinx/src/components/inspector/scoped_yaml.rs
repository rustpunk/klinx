use dioxus::prelude::*;

use crate::state::use_app_state;
use crate::sync::{EditSource, YamlNodeRange, compute_yaml_node_ranges, splice_yaml_node_block};

/// Writable node-scoped YAML editor.
///
/// The full YAML buffer remains authoritative. Edits splice this node block
/// back into `state.yaml_text`, mark the source as YAML, and let the normal
/// debounced parse path reconcile the model.
#[component]
pub fn ScopedYamlEditor(stage_id: String) -> Element {
    let state = use_app_state();
    let mut last_range = use_signal(|| None::<YamlNodeRange>);

    let current_range = use_memo(move || {
        let text = (state.yaml_text)();
        let pipeline_guard = state.pipeline.read();
        pipeline_guard.as_ref().and_then(|config| {
            compute_yaml_node_ranges(&text, config)
                .get(&stage_id)
                .copied()
        })
    });

    use_effect(move || {
        if let Some(range) = current_range() {
            last_range.set(Some(range));
        }
    });

    let text = (state.yaml_text)();
    let range = current_range().or_else(|| *last_range.read());
    let Some(range) = range else {
        return rsx! {
            div { class: "klinx-inspector-yaml",
                div {
                    class: "klinx-section-header",
                    span { class: "klinx-diamond", "\u{25C6}" }
                    span { class: "klinx-section-title", "STAGE YAML" }
                    span { class: "klinx-section-rule" }
                }
                div { class: "klinx-inspector-empty",
                    "YAML range is not available for this selected node."
                }
            }
        };
    };

    let block = text
        .get(range.start_byte..range.end_byte)
        .unwrap_or_default()
        .to_string();
    let line_count = scoped_line_count(&block);
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
                class: "klinx-yaml-code-area klinx-inspector-yaml-area klinx-scoped-yaml-editor",

                div {
                    class: "klinx-yaml-gutter",
                    for i in 0..line_count {
                        {
                            let line_num = range.start_line + i;
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

                textarea {
                    class: "klinx-scoped-yaml-textarea",
                    spellcheck: "false",
                    value: "{block}",
                    oninput: move |event: FormEvent| {
                        let draft = event.value();
                        let current = state.yaml_text.peek().clone();
                        let active_range = current_range()
                            .or_else(|| *last_range.peek())
                            .unwrap_or(range);
                        let next = splice_yaml_node_block(&current, active_range, &draft);
                        let next_range = range_after_replacement(
                            active_range,
                            &draft,
                            active_range.end_byte < current.len(),
                        );

                        last_range.set(Some(next_range));
                        let mut edit_source = state.edit_source;
                        if *edit_source.peek() != EditSource::Yaml {
                            edit_source.set(EditSource::Yaml);
                        }
                        let mut yaml = state.yaml_text;
                        yaml.set(next);
                    },
                }
            }
        }
    }
}

fn scoped_line_count(text: &str) -> usize {
    text.lines().count().max(1)
}

fn range_after_replacement(
    previous: YamlNodeRange,
    replacement: &str,
    needs_trailing_newline: bool,
) -> YamlNodeRange {
    let mut byte_len = replacement.len();
    if needs_trailing_newline && !replacement.ends_with('\n') {
        byte_len += 1;
    }
    let line_count = scoped_line_count(replacement);
    YamlNodeRange {
        start_line: previous.start_line,
        end_line: previous.start_line + line_count.saturating_sub(1),
        start_byte: previous.start_byte,
        end_byte: previous.start_byte + byte_len,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoped_range_after_replacement_tracks_grow_and_shrink() {
        let range = YamlNodeRange {
            start_line: 10,
            end_line: 12,
            start_byte: 100,
            end_byte: 140,
        };

        let grown = range_after_replacement(range, "a\nb\nc\nd", true);
        assert_eq!(grown.start_line, 10);
        assert_eq!(grown.end_line, 13);
        assert_eq!(grown.end_byte, 108);

        let shrunk = range_after_replacement(grown, "a", true);
        assert_eq!(shrunk.end_line, 10);
        assert_eq!(shrunk.end_byte, 102);
    }
}
