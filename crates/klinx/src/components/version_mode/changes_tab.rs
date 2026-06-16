//! Changes tab — staged/unstaged file list + commit interface.

use dioxus::prelude::*;

use klinx_git::GitOps;

use crate::components::toast::{ToastState, toast_error, toast_success};
use crate::state::TabManagerState;

/// Changes tab component.
#[component]
pub fn ChangesTab() -> Element {
    let mut tab_mgr = use_context::<TabManagerState>();
    let mut commit_msg = use_signal(String::new);
    let git = (tab_mgr.git_state)();
    let Some(ref status) = git else {
        return rsx! { div { "No git status available." } };
    };

    // Separate staged vs unstaged (simplified: treat index status as staged)
    let files = &status.files;
    let has_changes = !files.is_empty();
    let msg = (commit_msg)();

    rsx! {
        div { class: "klinx-changes-tab",
            // ── File list sidebar ───────────────────────────────────
            div { class: "klinx-changes-sidebar",
                if files.is_empty() {
                    div { class: "klinx-changes-sidebar__empty",
                        "No changes detected."
                    }
                }

                if has_changes {
                    div { class: "klinx-changes-sidebar__section",
                        div { class: "klinx-changes-sidebar__header",
                            "CHANGES"
                            span { class: "klinx-changes-sidebar__count", "{files.len()}" }
                        }

                        for file in files {
                            {
                                let path_str = file.path.display().to_string();
                                let letter = file.status.letter();
                                let status_modifier = file.status.css_modifier();

                                rsx! {
                                    div {
                                        class: "klinx-changes-file klinx-changes-file--{status_modifier}",
                                        span { class: "klinx-changes-file__status", "{letter}" }
                                        span { class: "klinx-changes-file__path", "{path_str}" }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // ── Commit area ─────────────────────────────────────────
            div { class: "klinx-changes-commit",
                div { class: "klinx-changes-commit__header", "COMMIT" }

                textarea {
                    class: "klinx-changes-commit__input",
                    placeholder: "Commit message...",
                    value: "{msg}",
                    oninput: move |e: FormEvent| commit_msg.set(e.value()),
                }

                div { class: "klinx-changes-commit__actions",
                    button {
                        class: "klinx-changes-commit__btn klinx-changes-commit__btn--commit",
                        disabled: msg.is_empty(),
                        onclick: {
                            move |_| {
                                do_commit(&mut tab_mgr, &(commit_msg)(), false);
                                commit_msg.set(String::new());
                            }
                        },
                        "Commit"
                    }
                    button {
                        class: "klinx-changes-commit__btn klinx-changes-commit__btn--push",
                        disabled: msg.is_empty(),
                        onclick: {
                            move |_| {
                                do_commit(&mut tab_mgr, &(commit_msg)(), true);
                                commit_msg.set(String::new());
                            }
                        },
                        "Commit & Push"
                    }
                }
            }
        }
    }
}

/// Stage all changes and commit.
fn do_commit(tab_mgr: &mut TabManagerState, message: &str, push: bool) {
    let ws = (tab_mgr.workspace)();
    let Some(ws) = ws else { return };

    let Ok(ops) = klinx_git::GitCliOps::discover(&ws.root) else {
        return;
    };
    let mut toast: Signal<Option<ToastState>> = use_context();

    // Stage all
    if let Err(e) = ops.stage_all() {
        toast_error(&mut toast, format!("stage failed: {e}"));
        return;
    }

    // Commit
    match ops.commit(message) {
        Ok(info) => {
            let short_hash = &info.id[..8.min(info.id.len())];
            toast_success(
                &mut toast,
                format!("Committed: {short_hash} {}", info.subject),
            );

            // Push if requested
            if push && let Err(e) = ops.push() {
                toast_error(&mut toast, format!("push failed: {e}"));
            }
        }
        Err(e) => {
            toast_error(&mut toast, format!("commit failed: {e}"));
        }
    }

    // Refresh git state
    if let Ok(status) = ops.status() {
        tab_mgr.git_state.set(Some(status));
    }
}
