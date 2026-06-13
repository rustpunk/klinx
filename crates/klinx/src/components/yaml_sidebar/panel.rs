use dioxus::prelude::*;

use crate::perf::perf_trace;
use crate::state::use_app_state;
// The blame components read git state from the tab manager directly.
use crate::state::TabManagerState;

use super::highlight::EditorPane;
use super::tokenizer::tokenize;

/// Full-height YAML sidebar with syntax-highlighted overlay and editable textarea.
///
/// Architecture: a transparent `<textarea>` sits on top of a syntax-coloured
/// `<pre>` that shows the same text with tokenized `<span>` elements. The
/// textarea captures keystrokes; the pre shows the colours. Both scroll
/// together via a shared container.
///
/// Blame gutter: toggleable column showing per-line git blame (author, time, hash).
#[component]
pub fn YamlSidebar() -> Element {
    let state = use_app_state();
    let errors = (state.parse_errors)();

    // Tokenize only when the YAML text changes — not on every re-render (e.g.
    // selection or error-bar churn). `tokenize` is a pure fn of the text.
    let raw_lines = use_memo(move || {
        let text = (state.yaml_text)();
        perf_trace!(tokenize(&text), "tokenize: {} bytes", text.len())
    });
    let line_count = raw_lines.read().len().max(1);

    // Selected stage's YAML line range — recomputed only when the text,
    // selection, or parsed pipeline changes.
    let selected_range = use_memo(move || -> Option<(usize, usize)> {
        let text = (state.yaml_text)();
        let stages = state.selected_stages.read();
        let single_selected = if stages.len() == 1 {
            stages.iter().next().cloned()
        } else {
            None
        };
        drop(stages);
        let pipeline_guard = (state.pipeline).read();
        match (single_selected.as_deref(), pipeline_guard.as_ref()) {
            (Some(stage_id), Some(config)) => crate::sync::compute_yaml_ranges(&text, config)
                .get(stage_id)
                .copied(),
            _ => None,
        }
    });
    let sel_range = selected_range();

    // Schema warnings for YAML squiggles
    let _warnings = (state.schema_warnings)();

    let is_editable = true;

    // Blame visibility state — provided as context for blame sub-components.
    let blame_visible = use_signal(|| false);
    use_context_provider(|| blame_visible);

    let section_title = "PIPELINE YAML";

    rsx! {
        div {
            class: "kiln-yaml-sidebar",

            // Section header with blame toggle
            div {
                class: "kiln-section-header",
                span { class: "kiln-diamond", "\u{25C6}" }
                span { class: "kiln-section-title", "{section_title}" }
                span { class: "kiln-section-rule" }

                BlameToggle {}
            }

            // Code area: blame gutter + line numbers + editor
            div {
                class: "kiln-yaml-code-area",

                BlameGutter { line_count }

                // Line-number gutter
                div {
                    class: "kiln-yaml-gutter",
                    for i in 0..line_count {
                        {
                            let line_num = i + 1;
                            let in_range = sel_range
                                .is_some_and(|(s, e)| line_num >= s && line_num <= e);
                            // Check for schema warnings on this line
                            let _has_warning = false; // TODO: map warnings to line numbers
                            rsx! {
                                div {
                                    key: "gutter-{i}",
                                    class: "kiln-yaml-line-num",
                                    "data-selected": if in_range { "true" },
                                    "{line_num}"
                                }
                            }
                        }
                    }
                }

                // Editor container: virtualized syntax overlay + textarea.
                // Lives in its own component so scrolling re-renders only the
                // editor, not the gutters.
                EditorPane {
                    lines: raw_lines,
                    selected_range: sel_range,
                    editable: is_editable,
                }
            }

            // Parse error bar
            if !errors.is_empty() {
                div {
                    class: "kiln-yaml-errors",
                    for (i, err) in errors.iter().enumerate() {
                        {
                            let err_text = err.clone();
                            let err_display = err.clone();
                            rsx! {
                                div {
                                    key: "err-{i}",
                                    class: "kiln-yaml-error",
                                    span {
                                        class: "kiln-yaml-error-text",
                                        "{err_display}"
                                    }
                                    button {
                                        class: "kiln-yaml-error-copy",
                                        title: "Copy error to clipboard",
                                        onclick: move |_| {
                                            let text = err_text.clone();
                                            let js = format!(
                                                "navigator.clipboard.writeText({})",
                                                serde_json::to_string(&text).unwrap_or_default()
                                            );
                                            document::eval(&js);
                                        },
                                        "\u{2398}"
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

// ── Blame components ────────────────────────────────────────────────────

/// Blame toggle button — rendered when git is available for the workspace.
#[component]
fn BlameToggle() -> Element {
    let tab_mgr = use_context::<TabManagerState>();
    let mut blame_visible = use_context::<Signal<bool>>();

    // Initialize blame context if not already provided
    let show_blame = (blame_visible)();

    if (tab_mgr.git_state)().is_some() {
        rsx! {
            button {
                class: if show_blame {
                    "kiln-blame-toggle kiln-blame-toggle--active"
                } else {
                    "kiln-blame-toggle"
                },
                onclick: move |_| {
                    blame_visible.set(!show_blame);
                },
                "⑂ BLAME"
            }
        }
    } else {
        rsx! {}
    }
}

/// Blame gutter — shows per-line git blame data.
#[component]
fn BlameGutter(line_count: usize) -> Element {
    use klinx_git::BlameLine;

    let tab_mgr = use_context::<TabManagerState>();
    let mut blame_data = use_signal(Vec::<BlameLine>::new);
    let blame_visible = use_context::<Signal<bool>>();
    let show_blame = (blame_visible)();

    // Load blame data when toggled on
    if show_blame && (blame_data)().is_empty() {
        load_blame(&tab_mgr, &mut blame_data);
    }

    if !show_blame {
        return rsx! {};
    }

    rsx! {
        div {
            class: "kiln-blame-gutter",
            for i in 0..line_count {
                {
                    let bl = (blame_data)();
                    let blame = bl.iter().find(|b| b.line == i + 1).cloned();
                    let prev_blame = if i > 0 {
                        bl.iter().find(|b| b.line == i).cloned()
                    } else {
                        None
                    };
                    let is_group_start = blame.as_ref().map(|b| {
                        prev_blame.as_ref().map(|pb| pb.commit_id != b.commit_id).unwrap_or(true)
                    }).unwrap_or(false);

                    if let Some(ref b) = blame {
                        let author = if b.author.len() > 8 { b.author[..8].to_string() } else { b.author.clone() };
                        let time = relative_time_short(b.timestamp);
                        let hash = b.commit_id.clone();

                        rsx! {
                            div {
                                key: "blame-{i}",
                                class: "kiln-blame-line",
                                if is_group_start {
                                    span { class: "kiln-blame-author", "{author}" }
                                    span { class: "kiln-blame-time", "{time}" }
                                    span { class: "kiln-blame-hash", "{hash}" }
                                } else {
                                    span { class: "kiln-blame-continuation", "│" }
                                }
                            }
                        }
                    } else {
                        rsx! {
                            div {
                                key: "blame-{i}",
                                class: "kiln-blame-line",
                                span { class: "kiln-blame-uncommitted", "uncommitted" }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Load blame data for the current file.
fn load_blame(tab_mgr: &TabManagerState, blame_data: &mut Signal<Vec<klinx_git::BlameLine>>) {
    use klinx_git::GitOps;

    let ws = (tab_mgr.workspace)();
    let Some(ws) = ws else { return };

    // Get the active tab's file path
    let active_id = (tab_mgr.active_tab_id)();
    let tabs = tab_mgr.tabs.read();
    let active_tab = active_id.and_then(|id| tabs.iter().find(|t| t.id == id));
    let Some(tab) = active_tab else { return };
    let Some(ref file_path) = tab.file_path else {
        return;
    };

    // Make path relative to repo root
    let relative = file_path.strip_prefix(&ws.root).unwrap_or(file_path);

    if let Ok(ops) = klinx_git::GitCliOps::discover(&ws.root)
        && let Ok(lines) = ops.blame(relative)
    {
        blame_data.set(lines);
    }
}

/// Short relative time for blame gutter (2h, 3d, 1w, 2mo).
fn relative_time_short(timestamp: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let diff = now - timestamp;

    if diff < 3600 {
        format!("{}m", diff / 60)
    } else if diff < 86400 {
        format!("{}h", diff / 3600)
    } else if diff < 604800 {
        format!("{}d", diff / 86400)
    } else if diff < 2592000 {
        format!("{}w", diff / 604800)
    } else {
        format!("{}mo", diff / 2592000)
    }
}
