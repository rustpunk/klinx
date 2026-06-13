//! Syntax-highlight rendering for the YAML editor: a memoized per-line
//! component and the virtualized, scrollable editor pane.

use dioxus::prelude::*;
use dioxus_nox_virtualize::VirtualViewport;

use crate::state::use_app_state;
use crate::sync::EditSource;

use super::tokenizer::Token;

/// Line height in px. MUST match `.klinx-yaml-line` / `.klinx-yaml-textarea`
/// `line-height` in `klinx.css` — virtualization positions lines by this
/// constant, so a mismatch would drift the overlay off the textarea.
const LINE_HEIGHT: u32 = 20;

/// One syntax-highlighted YAML line: token `<span>`s inside a `klinx-yaml-line`
/// div.
///
/// A `#[component]` so Dioxus memoizes it on its props — a line whose `tokens`
/// and `selected` state are unchanged is skipped on re-render. Editing one line
/// (or a pure selection change, or a scroll) therefore no longer rebuilds every
/// line's spans.
#[component]
pub(crate) fn YamlLine(tokens: Vec<Token>, selected: bool) -> Element {
    rsx! {
        div {
            class: "klinx-yaml-line",
            "data-selected": if selected { "true" },
            for (j, token) in tokens.iter().enumerate() {
                span {
                    key: "tok-{j}",
                    "data-token": token.kind.as_data_attr(),
                    "{token.text}"
                }
            }
            // Preserve the line's height when it has no visible glyphs.
            if tokens.iter().all(|t| t.text.is_empty()) {
                "\u{00A0}"
            }
        }
    }
}

/// The scrollable editor pane: a virtualized syntax-highlight overlay stacked
/// under the transparent textarea.
///
/// Owns its own scroll/viewport state, so scrolling re-renders only this pane,
/// not the surrounding gutters. Only the visible line window (plus overscan) is
/// rendered; top/bottom spacer divs reproduce the height of the off-screen lines
/// exactly (`LINE_HEIGHT` each), so the editor's scroll height is unchanged and
/// the overlay stays pixel-aligned with the full-height textarea above it.
#[component]
pub(crate) fn EditorPane(
    /// Tokenized lines (memoized by the parent); read here for the visible window.
    lines: ReadSignal<Vec<Vec<Token>>>,
    /// Inclusive 1-based YAML line range of the selected stage, if any.
    selected_range: Option<(usize, usize)>,
    editable: bool,
) -> Element {
    let state = use_app_state();
    let mut scroll_top = use_signal(|| 0u32);
    // Editor client height, corrected by `onresize`. Generous default so the
    // first paint (before the resize observer fires) still covers a tall pane.
    let mut viewport_h = use_signal(|| 1200u32);

    let text = (state.yaml_text)();

    let line_count = lines.read().len();
    let viewport = VirtualViewport {
        item_count: line_count,
        item_height: LINE_HEIGHT,
        viewport_height: viewport_h(),
        scroll_top: scroll_top(),
        overscan: 8,
    };
    let (start, end) = viewport.visible_range();
    let top_spacer = viewport.top_spacer_height();
    let bottom_spacer = viewport.bottom_spacer_height();

    rsx! {
        div {
            class: "klinx-yaml-editor",
            onscroll: move |evt| scroll_top.set(evt.scroll_top().max(0.0) as u32),
            // Measure the real pane height on mount (guaranteed) and on every
            // resize, so the visible window covers the viewport even if the
            // resize observer never fires.
            onmounted: move |evt| {
                spawn(async move {
                    if let Ok(rect) = evt.get_client_rect().await {
                        viewport_h.set(rect.size.height.max(0.0) as u32);
                    }
                });
            },
            onresize: move |evt| {
                if let Ok(size) = evt.get_content_box_size() {
                    viewport_h.set(size.height.max(0.0) as u32);
                }
            },

            // Syntax-highlighted overlay (read-only visual layer)
            div {
                class: "klinx-yaml-highlight",
                div { style: "height: {top_spacer}px;" }
                for i in start..end {
                    YamlLine {
                        key: "line-{i}",
                        tokens: lines.read()[i].clone(),
                        selected: selected_range.is_some_and(|(s, e)| (s..=e).contains(&(i + 1))),
                    }
                }
                div { style: "height: {bottom_spacer}px;" }
            }

            // Transparent textarea (captures input), sits on top, full height.
            if editable {
                textarea {
                    class: "klinx-yaml-textarea",
                    spellcheck: "false",
                    value: "{text}",
                    oninput: move |e: FormEvent| {
                        // Keep yaml_text immediate (textarea echo + undo); only
                        // flip edit_source on a real transition so it doesn't
                        // churn the parse/snapshot effects per key.
                        let mut src = state.edit_source;
                        if *src.peek() != EditSource::Yaml {
                            src.set(EditSource::Yaml);
                        }
                        let mut yaml = state.yaml_text;
                        yaml.set(e.value());
                    },
                }
            } else {
                textarea {
                    class: "klinx-yaml-textarea klinx-yaml-textarea--readonly",
                    spellcheck: "false",
                    readonly: true,
                    value: "{text}",
                }
            }
        }
    }
}
