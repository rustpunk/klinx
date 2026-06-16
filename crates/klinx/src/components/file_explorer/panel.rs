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
use klinx_git::StatusKind;

use crate::state::{LeftPanel, TabManagerState};

use super::model::{
    ExplorerTree, ExplorerView, FlatNode, NodeId, NodeKind, build_filesystem, build_sectioned,
    expand_sections, flatten, row_git_status,
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

    // ── Row decorations ──────────────────────────────────────────────────
    // Each memo recomputes only when its own source changes, so editing YAML
    // or expanding a node never rebuilds the others (and a Memo only re-renders
    // subscribers when its output actually changes).

    // Changed files from git status; suffix-matched per row by
    // `model::row_git_status`. Holds only changed files, so the scan is cheap.
    let git_files = use_memo(move || {
        tab_mgr
            .git_state
            .read()
            .as_ref()
            .map(|gs| gs.files.clone())
            .unwrap_or_default()
    });

    // Absolute paths of every file-backed open tab.
    let open_paths = use_memo(move || {
        tab_mgr
            .tabs
            .read()
            .iter()
            .filter_map(|t| t.file_path.clone())
            .collect::<HashSet<PathBuf>>()
    });

    // Absolute path of the active tab's file, if it is file-backed.
    let active_path = use_memo(move || {
        let active = (tab_mgr.active_tab_id)()?;
        tab_mgr
            .tabs
            .read()
            .iter()
            .find(|t| t.id == active)
            .and_then(|t| t.file_path.clone())
    });

    // Read (not clone) the workspace — `()` would clone the whole struct every
    // render; we only need the display name.
    let workspace_name = tab_mgr
        .workspace
        .read()
        .as_ref()
        .map(|ws| ws.display_name())
        .unwrap_or_else(|| "No workspace".to_string());
    let current_view = (view)();

    let git_files = git_files.read();
    let open_paths = open_paths.read();
    let active_path = active_path.read();

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
                        {
                            let git_status = row_git_status(row, &git_files);
                            // Open/active apply to file leaves only.
                            let file_path = match row.kind {
                                NodeKind::File { .. } => row.path.as_deref(),
                                _ => None,
                            };
                            let is_open = file_path.is_some_and(|p| open_paths.contains(p));
                            // `is_some_and` (not a bare `==`) guards against a
                            // non-file row's `None` path matching an untitled
                            // tab's `None` active path.
                            let is_active =
                                file_path.is_some_and(|p| Some(p) == active_path.as_deref());
                            rsx! {
                                ExplorerRow {
                                    key: "{row.id}",
                                    row: row.clone(),
                                    is_expanded: expanded.read().contains(&row.id),
                                    git_status,
                                    is_open,
                                    is_active,
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
    }
}

/// A single explorer row. Its own component so that, with a stable `key:`, only
/// rows whose props change re-render on expand/collapse.
#[component]
fn ExplorerRow(
    row: FlatNode,
    is_expanded: bool,
    git_status: Option<StatusKind>,
    is_open: bool,
    is_active: bool,
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
    if let Some(status) = git_status {
        classes.push(' ');
        classes.push_str("klinx-file-explorer__row--");
        classes.push_str(status.css_modifier());
    }
    if is_open {
        classes.push_str(" klinx-file-explorer__row--open");
    }
    if is_active {
        classes.push_str(" klinx-file-explorer__row--active");
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
            if let Some(status) = git_status {
                span { class: "klinx-file-explorer__git", "{status.letter()}" }
            }
        }
    }
}
