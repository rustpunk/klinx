/// Tab bar: horizontal strip below the title bar showing open pipeline tabs.
///
/// Each tab carries active/inactive styling, a dirty dot, and a close
/// button; a trailing [+] opens a new tab.
use dioxus::prelude::*;
use klinx_git::git_status_for_path;

use crate::keyboard;
use crate::state::TabManagerState;
use crate::tab::TabEntry;

/// Tab bar component spanning the full viewport width.
#[component]
pub fn TabBar() -> Element {
    let mut tab_mgr: TabManagerState = use_context();
    let mut tabs = tab_mgr.tabs;
    let active_id = tab_mgr.active_tab_id;

    let current_active = (active_id)();

    rsx! {
        div {
            class: "klinx-tab-bar",

            for tab in tabs.read().iter() {
                {
                    let tab_id = tab.id;
                    let is_active = current_active == Some(tab_id);
                    let is_dirty = tab.is_dirty();
                    let name = tab.display_name();

                    // Git status badge per tab.
                    let git_status = (tab_mgr.git_state)().as_ref().and_then(|gs| {
                        tab.file_path
                            .as_deref()
                            .and_then(|fp| git_status_for_path(fp, &gs.files))
                    });
                    let class = if is_active {
                        "klinx-tab klinx-tab--active"
                    } else {
                        "klinx-tab"
                    };

                    rsx! {
                        div {
                            key: "{tab_id}",
                            class: "{class}",
                            onclick: move |_| {
                                let mut active = tab_mgr.active_tab_id;
                                active.set(Some(tab_id));
                            },
                            title: "{tab.full_path().unwrap_or_default()}",

                            if is_dirty {
                                span { class: "klinx-tab-dirty", "\u{25CF} " }
                            }
                            span { class: "klinx-tab-name", "{name}" }
                            if let Some(status) = git_status {
                                {
                                    let letter = status.letter();
                                    let css_modifier = status.css_modifier();
                                    rsx! {
                                        span { class: "klinx-tab-git klinx-tab-git--{css_modifier}", "{letter}" }
                                    }
                                }
                            }

                            button {
                                class: "klinx-tab-close",
                                onclick: move |e: MouseEvent| {
                                    e.stop_propagation();
                                    keyboard::request_close_tab(&mut tab_mgr, tab_id);
                                },
                                "\u{00D7}"
                            }
                        }
                    }
                }
            }

            // [+] New tab button
            button {
                class: "klinx-tab-new",
                onclick: move |_| {
                    let new_tab = TabEntry::new_untitled(&tabs.read());
                    let new_id = new_tab.id;
                    tabs.write().push(new_tab);
                    tab_mgr.active_tab_id.set(Some(new_id));
                },
                "+"
            }
        }
    }
}
