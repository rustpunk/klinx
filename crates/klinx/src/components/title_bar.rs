use dioxus::desktop::use_window;
use dioxus::prelude::*;

use crate::keyboard;
use crate::state::{NavigationContext, PipelineLayoutMode, TabManagerState, use_app_state};
use crate::tab::TabEntry;

/// Custom frameless title bar with context-aware content.
///
/// Common elements (always visible):
///   [klinx]  |  workspace-name  |  validation LED
///
/// Pipeline context:
///   [New][Open][Save]  |  filename  |  [Canvas|Hybrid|Editor]
///
/// Other contexts:
///   Context label  |  context-specific actions
#[component]
pub fn TitleBar() -> Element {
    // Native window handle drives the frameless drag region.
    let window = use_window();
    let state = use_app_state();
    let mut tab_mgr: TabManagerState = use_context();
    let current_ctx = (state.active_context)();

    // Derive filename + dirty state from active tab
    let active_id = (tab_mgr.active_tab_id)();
    let tabs = tab_mgr.tabs.read();
    let active_tab = active_id.and_then(|id| tabs.iter().find(|t| t.id == id));
    let filename = active_tab.map(|t| t.display_name()).unwrap_or_default();
    let is_dirty = active_tab.map(|t| t.is_dirty()).unwrap_or(false);
    let has_active_tab = active_tab.is_some();
    let ws_name = (tab_mgr.workspace)().as_ref().map(|ws| ws.display_name());

    // Validation state (only relevant in Pipeline context)
    let has_errors = !(state.parse_errors)().is_empty();
    let led_class = if has_errors || !has_active_tab {
        "klinx-led-dot klinx-led-dot--err"
    } else {
        "klinx-led-dot klinx-led-dot--ok"
    };
    let led_label = if !has_active_tab {
        ""
    } else if has_errors {
        "ERROR"
    } else {
        "VALID"
    };

    // Mutable signal copy for layout mode switching
    let mut pipeline_layout = state.pipeline_layout;

    // Git state for the Git-context title bar branch label.
    let git_branch = (tab_mgr.git_state)().as_ref().map(|gs| gs.branch.clone());

    rsx! {
        div {
            class: "klinx-title-bar",
            onmousedown: move |_| {
                // Drag the frameless window from the title-bar region.
                window.drag();
            },

            // Brand badge — always visible
            div {
                class: "klinx-brand",
                onmousedown: move |e| e.stop_propagation(),
                span { class: "klinx-brand-label", "" }
                span { class: "klinx-brand-value", "klinx" }
            }

            span { class: "klinx-title-divider" }

            // ── Pipeline context: file actions + filename ────────────────
            if current_ctx == NavigationContext::Pipeline {
                div {
                    class: "klinx-file-actions",
                    onmousedown: move |e| e.stop_propagation(),

                    button {
                        class: "klinx-file-btn",
                        title: "New pipeline (Ctrl+N)",
                        onclick: move |_| {
                            let new_tab = TabEntry::new_untitled(&tab_mgr.tabs.read());
                            let new_id = new_tab.id;
                            tab_mgr.tabs.write().push(new_tab);
                            tab_mgr.active_tab_id.set(Some(new_id));
                        },
                        "New"
                    }
                    button {
                        class: "klinx-file-btn",
                        title: "Open file (Ctrl+O)",
                        onclick: move |_| {
                            keyboard::open_file(&mut tab_mgr);
                        },
                        "Open"
                    }
                    button {
                        class: "klinx-file-btn",
                        title: "Open workspace (Ctrl+Shift+O)",
                        onclick: move |_| {
                            keyboard::open_workspace(&mut tab_mgr);
                        },
                        "Workspace"
                    }
                    if has_active_tab {
                        button {
                            class: "klinx-file-btn",
                            title: "Save (Ctrl+S)",
                            onclick: move |_| {
                                keyboard::save_active_tab(&mut tab_mgr, false);
                            },
                            "Save"
                        }
                    }
                }

                span { class: "klinx-title-divider" }
            }

            // ── Non-Pipeline contexts: context label ────────────────────
            if current_ctx != NavigationContext::Pipeline {
                span {
                    class: "klinx-title-context-label",
                    "{current_ctx.label()}"
                }
                span { class: "klinx-title-divider" }
            }

            // Workspace name (if in workspace mode)
            if let Some(ref name) = ws_name {
                span {
                    class: "klinx-title-workspace",
                    "{name}"
                }
                span { class: "klinx-title-divider" }
            }

            // Pipeline context: filename with dirty indicator
            if current_ctx == NavigationContext::Pipeline {
                span {
                    class: "klinx-title-filename",
                    if is_dirty { "\u{25CF} " } else { "" }
                    "{filename}"
                }
            }

            // Git context: branch name
            if current_ctx == NavigationContext::Git {
                if let Some(ref branch) = git_branch {
                    span {
                        class: "klinx-title-branch",
                        "⑂ {branch}"
                    }
                }
            }

            // Flex spacer
            span { class: "klinx-title-spacer" }

            // ── Pipeline context: layout mode switcher ───────────────────
            if current_ctx == NavigationContext::Pipeline && has_active_tab {
                div {
                    class: "klinx-layout-switcher",
                    onmousedown: move |e| e.stop_propagation(),

                    for mode in PipelineLayoutMode::ALL {
                        {
                            let is_active = (state.pipeline_layout)() == mode;

                            rsx! {
                                button {
                                    key: "{mode.label()}",
                                    class: "klinx-layout-btn",
                                    "data-active": if is_active { "true" } else { "false" },
                                    onclick: move |_| {
                                        pipeline_layout.set(mode);
                                    },
                                    "{mode.label()}"
                                }
                            }
                        }
                    }
                }
            }

            // Validation LED — always visible when Pipeline has active tab
            if current_ctx == NavigationContext::Pipeline && has_active_tab {
                div {
                    class: "klinx-validation-led",
                    onmousedown: move |e| e.stop_propagation(),
                    span { class: "{led_class}" }
                    span { class: "klinx-led-label", "{led_label}" }
                }
            }
        }

    }
}
