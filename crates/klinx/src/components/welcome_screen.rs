/// Welcome screen shown when no tabs are open.
///
/// Shows a brand badge, recent files, open/new buttons, and shortcut hints.
use dioxus::prelude::*;

use crate::keyboard;
use crate::state::TabManagerState;
use crate::tab::TabEntry;

/// Welcome / start screen component.
#[component]
pub fn WelcomeScreen() -> Element {
    let mut tab_mgr: TabManagerState = use_context();

    rsx! {
        div {
            class: "klinx-welcome",

            // Brand badge (stacked)
            div {
                class: "klinx-welcome-brand",
                div { class: "klinx-welcome-brand-top", "" }
                div { class: "klinx-welcome-brand-bottom", "KLINX" }
            }

            // Subtitle
            div {
                class: "klinx-welcome-subtitle",
                "pipeline configuration IDE"
            }

            // Rust line divider
            hr { class: "klinx-rust-line" }

            // Action buttons
            div {
                class: "klinx-welcome-actions",

                button {
                    class: "klinx-welcome-btn",
                    onclick: move |_| {
                        keyboard::open_file(&mut tab_mgr);
                    },
                    "Open File"
                }

                button {
                    class: "klinx-welcome-btn",
                    onclick: move |_| {
                        keyboard::open_workspace(&mut tab_mgr);
                    },
                    "Open Workspace"
                }

                button {
                    class: "klinx-welcome-btn",
                    onclick: move |_| {
                        let new_tab = TabEntry::new_untitled(&tab_mgr.tabs.read());
                        let new_id = new_tab.id;
                        tab_mgr.tabs.write().push(new_tab);
                        tab_mgr.active_tab_id.set(Some(new_id));
                    },
                    "New Pipeline"
                }
            }

            // Shortcut hints
            div {
                class: "klinx-welcome-shortcuts",
                div { span { class: "klinx-welcome-key", "Ctrl+O" } " open file" }
                div { span { class: "klinx-welcome-key", "Ctrl+Shift+O" } " open workspace" }
                div { span { class: "klinx-welcome-key", "Ctrl+N" } " new pipeline" }
            }
        }
    }
}
