//! Workspace file explorer panel — the left-panel (280px) workspace tree.
//!
//! Renders the [`super::model`] tree as a flat list of rows (one component per
//! row, stably keyed) so expand/collapse re-renders only changed rows. The
//! header offers a **Sections | Files** toggle between the discovery-grouped
//! view and the raw filesystem view. Clicking a YAML file opens it as a tab via
//! the shared [`crate::keyboard::open_path`].

use std::collections::HashSet;
use std::path::PathBuf;

use dioxus::prelude::*;

use crate::state::{LeftPanel, TabManagerState};

use super::model::{
    ExplorerTree, ExplorerView, FlatNode, NodeId, NodeKind, build_filesystem, build_sectioned,
    expand_sections, flatten,
};

/// The workspace file explorer component.
#[component]
pub fn FileExplorer() -> Element {
    let mut tab_mgr = use_context::<TabManagerState>();
    let mut view = use_signal(ExplorerView::default);
    let mut expanded = use_signal(HashSet::<NodeId>::new);

    // Build the tree from discovery — rebuilt only when the workspace, schema
    // index, channel state, or view changes (not on expand/collapse).
    let tree = use_memo(move || {
        let Some(ws) = (tab_mgr.workspace)() else {
            return ExplorerTree::default();
        };
        match (view)() {
            ExplorerView::Sections => {
                let idx = (tab_mgr.schema_index)();
                let chans = (tab_mgr.channel_state)();
                build_sectioned(&ws, &idx, chans.as_ref())
            }
            ExplorerView::Files => build_filesystem(&ws.root),
        }
    });

    // Seed default expansion (all sections open) the first time a non-empty
    // sectioned tree appears, so a freshly-opened workspace shows its contents.
    use_effect(move || {
        let t = tree.read();
        if !t.roots.is_empty() && expanded.peek().is_empty() {
            expanded.set(expand_sections(&t));
        }
    });

    // Cheap: re-flatten on every expand/collapse or tree change.
    let rows = use_memo(move || flatten(&tree.read(), &expanded.read()));

    // Read (not clone) the workspace — `()` would clone the whole struct every
    // render; we only need the display name.
    let workspace_name = tab_mgr
        .workspace
        .read()
        .as_ref()
        .map(|ws| ws.display_name())
        .unwrap_or_else(|| "No workspace".to_string());
    let current_view = (view)();

    rsx! {
        div { class: "klinx-file-explorer",

            // ── Header: workspace name + close ──────────────────────────
            div { class: "klinx-file-explorer__header",
                span { class: "klinx-file-explorer__title", "{workspace_name}" }
                button {
                    class: "klinx-file-explorer__close",
                    title: "Close (Alt+B)",
                    onclick: move |_| tab_mgr.left_panel.set(LeftPanel::None),
                    "×"
                }
            }

            // ── View toggle: Sections | Files ───────────────────────────
            div { class: "klinx-file-explorer__toggle",
                button {
                    class: if current_view == ExplorerView::Sections {
                        "klinx-file-explorer__toggle-btn klinx-file-explorer__toggle-btn--active"
                    } else {
                        "klinx-file-explorer__toggle-btn"
                    },
                    onclick: move |_| view.set(ExplorerView::Sections),
                    "Sections"
                }
                button {
                    class: if current_view == ExplorerView::Files {
                        "klinx-file-explorer__toggle-btn klinx-file-explorer__toggle-btn--active"
                    } else {
                        "klinx-file-explorer__toggle-btn"
                    },
                    onclick: move |_| view.set(ExplorerView::Files),
                    "Files"
                }
            }

            // ── Tree rows ───────────────────────────────────────────────
            div { class: "klinx-file-explorer__list",
                if rows.read().is_empty() {
                    div { class: "klinx-file-explorer__empty", "No files discovered." }
                } else {
                    for row in rows.read().iter() {
                        ExplorerRow {
                            key: "{row.id}",
                            row: row.clone(),
                            is_expanded: expanded.read().contains(&row.id),
                            on_toggle: move |id: NodeId| {
                                let mut set = expanded.write();
                                if !set.remove(&id) {
                                    set.insert(id);
                                }
                            },
                            on_open: move |p: PathBuf| {
                                crate::keyboard::open_path(&mut tab_mgr, &p);
                            },
                        }
                    }
                }
            }
        }
    }
}

/// A single explorer row. Its own component so that, with a stable `key:`, only
/// rows whose props change re-render on expand/collapse.
#[component]
fn ExplorerRow(
    row: FlatNode,
    is_expanded: bool,
    on_toggle: EventHandler<NodeId>,
    on_open: EventHandler<PathBuf>,
) -> Element {
    let is_section = matches!(row.kind, NodeKind::Section(_));
    let is_file = matches!(row.kind, NodeKind::File { .. });
    let openable = matches!(row.kind, NodeKind::File { openable: true });

    let indent = 8 + row.depth as i32 * 12;
    let chevron = if row.expandable {
        if is_expanded { "▾" } else { "▸" }
    } else {
        ""
    };

    let mut classes = String::from("klinx-file-explorer__row");
    if is_section {
        classes.push_str(" klinx-file-explorer__row--section");
    }
    if is_file && !openable {
        classes.push_str(" klinx-file-explorer__row--disabled");
    }

    let id = row.id.clone();
    let path = row.path.clone();

    rsx! {
        div {
            class: "{classes}",
            style: "padding-left: {indent}px",
            onclick: move |_| {
                if row.expandable {
                    on_toggle.call(id.clone());
                } else if openable
                    && let Some(p) = path.clone()
                {
                    on_open.call(p);
                }
            },
            span { class: "klinx-file-explorer__chevron", "{chevron}" }
            span { class: "klinx-file-explorer__label", "{row.label}" }
        }
    }
}
