/// Tab bar: horizontal strip below the title bar showing open pipeline tabs.
///
/// Each tab carries active/inactive styling, a dirty dot, and a close
/// button; a trailing [+] opens a new tab.
use dioxus::prelude::*;

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
                        tab.file_path.as_ref().and_then(|fp| {
                            gs.files.iter().find(|f| fp.ends_with(&f.path)).map(|f| f.status)
                        })
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
                                    let css_class = match status {
                                        klinx_git::StatusKind::Modified => "klinx-tab-git klinx-tab-git--modified",
                                        klinx_git::StatusKind::Added => "klinx-tab-git klinx-tab-git--added",
                                        klinx_git::StatusKind::Deleted => "klinx-tab-git klinx-tab-git--deleted",
                                        klinx_git::StatusKind::Renamed => "klinx-tab-git klinx-tab-git--renamed",
                                        klinx_git::StatusKind::Untracked => "klinx-tab-git klinx-tab-git--untracked",
                                    };
                                    rsx! {
                                        span { class: "{css_class}", "{letter}" }
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
