//! PR creation pane — create pull requests from Version Mode.

use dioxus::prelude::*;

use klinx_git::ProviderKind;

use crate::components::toast::{ToastState, toast_error, toast_success};
use crate::state::TabManagerState;

/// PR creation pane component.
#[component]
pub fn PrPane(on_close: EventHandler<()>) -> Element {
    let tab_mgr = use_context::<TabManagerState>();
    let mut title = use_signal(String::new);
    let mut body = use_signal(String::new);
    let mut draft = use_signal(|| false);
    let mut target_branch = use_signal(|| "main".to_string());
    let mut creating = use_signal(|| false);
    let mut created_url = use_signal(|| None::<String>);

    let git = (tab_mgr.git_state)();
    let source_branch = git.as_ref().map(|g| g.branch.clone()).unwrap_or_default();

    // Detect provider from remote
    let ws = (tab_mgr.workspace)();
    let provider = ws
        .as_ref()
        .and_then(|ws| klinx_git::get_remote_url(&ws.root).ok())
        .map(|url| klinx_git::detect_provider(&url))
        .unwrap_or(ProviderKind::Unknown);

    // Auto-suggest title from branch name
    if (title)().is_empty() && !source_branch.is_empty() {
        let suggested = humanize_branch(&source_branch);
        title.set(suggested);
    }

    // Auto-detect target branch
    if let Some(ref ws) = ws {
        let default =
            klinx_git::get_default_branch(&ws.root).unwrap_or_else(|_| "main".to_string());
        if (target_branch)() == "main" && default != "main" {
            target_branch.set(default);
        }
    }

    let created = (created_url)();

    // Show success screen if PR was created
    if let Some(ref url) = created {
        return rsx! {
            div { class: "klinx-pr-success",
                div { class: "klinx-pr-success__icon", "✓" }
                div { class: "klinx-pr-success__title", "PULL REQUEST CREATED" }
                div { class: "klinx-pr-success__meta",
                    "{provider.label()} · {source_branch} → {target_branch}"
                }
                a {
                    class: "klinx-pr-success__url",
                    href: "{url}",
                    "{url}"
                }
                button {
                    class: "klinx-pr-success__close",
                    onclick: move |_| on_close.call(()),
                    "Done"
                }
            }
        };
    }

    rsx! {
        div { class: "klinx-pr-pane",
            div { class: "klinx-pr-pane__header",
                span { class: "klinx-pr-pane__title", "CREATE PULL REQUEST" }
                button {
                    class: "klinx-pr-pane__close",
                    onclick: move |_| on_close.call(()),
                    "×"
                }
            }

            // Provider indicator
            div { class: "klinx-pr-pane__provider",
                span { class: "klinx-pr-pane__provider-label",
                    "Platform: {provider.label()}"
                }
            }

            // Source → Target
            div { class: "klinx-pr-pane__branches",
                div { class: "klinx-pr-pane__branch",
                    span { class: "klinx-pr-pane__branch-icon", "⑂" }
                    span { class: "klinx-pr-pane__branch-name", "{source_branch}" }
                }
                span { class: "klinx-pr-pane__arrow", "→" }
                div { class: "klinx-pr-pane__branch",
                    input {
                        class: "klinx-pr-pane__branch-input",
                        r#type: "text",
                        value: "{target_branch}",
                        oninput: move |e: FormEvent| target_branch.set(e.value()),
                    }
                }
            }

            // Title
            div { class: "klinx-pr-pane__field",
                label { class: "klinx-pr-pane__label", "TITLE" }
                input {
                    class: "klinx-pr-pane__input",
                    r#type: "text",
                    value: "{title}",
                    oninput: move |e: FormEvent| title.set(e.value()),
                }
            }

            // Description
            div { class: "klinx-pr-pane__field",
                label { class: "klinx-pr-pane__label", "DESCRIPTION" }
                textarea {
                    class: "klinx-pr-pane__textarea",
                    placeholder: "Describe your changes...",
                    value: "{body}",
                    oninput: move |e: FormEvent| body.set(e.value()),
                }
            }

            // Options
            div { class: "klinx-pr-pane__options",
                label {
                    class: "klinx-pr-pane__checkbox",
                    input {
                        r#type: "checkbox",
                        checked: (draft)(),
                        onchange: move |e: FormEvent| draft.set(e.value() == "true"),
                    }
                    "Draft PR"
                }
            }

            // Actions
            div { class: "klinx-pr-pane__actions",
                button {
                    class: "klinx-pr-pane__create-btn",
                    disabled: (title)().is_empty() || (creating)(),
                    onclick: {
                        let ws_root = ws.as_ref().map(|w| w.root.clone());
                        move |_| {
                            let Some(ref root) = ws_root else { return };
                            creating.set(true);

                            let params = klinx_git::PrParams {
                                source_branch: source_branch.clone(),
                                target_branch: (target_branch)(),
                                title: (title)(),
                                body: (body)(),
                                draft: (draft)(),
                            };

                            match klinx_git::create_pr(root, &params, provider) {
                                Ok(result) => {
                                    let mut toast: Signal<Option<ToastState>> = use_context();
                                    toast_success(&mut toast, format!("PR #{} created", result.number));
                                    created_url.set(Some(result.url));
                                }
                                Err(e) => {
                                    let mut toast: Signal<Option<ToastState>> = use_context();
                                    toast_error(&mut toast, format!("PR creation failed: {e}"));
                                }
                            }
                            creating.set(false);
                        }
                    },
                    if (draft)() { "Create Draft PR" } else { "Create Pull Request" }
                }
                button {
                    class: "klinx-pr-pane__cancel-btn",
                    onclick: move |_| on_close.call(()),
                    "Cancel"
                }
            }
        }
    }
}

/// Convert a branch name to a human-readable PR title.
fn humanize_branch(branch: &str) -> String {
    // Strip common prefixes
    let name = branch
        .strip_prefix("feat/")
        .or_else(|| branch.strip_prefix("fix/"))
        .or_else(|| branch.strip_prefix("chore/"))
        .or_else(|| branch.strip_prefix("refactor/"))
        .unwrap_or(branch);

    // Replace separators with spaces, capitalize first letter
    let words: String = name.replace(['-', '_'], " ");

    let mut chars = words.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_humanize_branch() {
        assert_eq!(humanize_branch("feat/normalize-stage"), "Normalize stage");
        assert_eq!(humanize_branch("fix/csv-quoting"), "Csv quoting");
        assert_eq!(humanize_branch("main"), "Main");
        assert_eq!(humanize_branch("chore/update_deps"), "Update deps");
    }
}
