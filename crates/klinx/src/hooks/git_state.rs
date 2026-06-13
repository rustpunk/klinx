//! Git repository state: discover the repo + compute status on workspace
//! change, and run a filesystem watcher that flags git-relevant changes.
//!
//! Owns two side effects, behavior-identical to their former inline form in
//! `AppShell`:
//! - a `use_effect` that re-discovers the git repo and refreshes `git_state`
//!   whenever the workspace changes, and
//! - a `use_effect` that starts a debounced filesystem watcher on the
//!   workspace root and spawns a background thread to observe its events.

use dioxus::prelude::*;
use klinx_git::GitOps;

use crate::workspace::Workspace;

/// Wire up git-status discovery and the filesystem watcher for `AppShell`.
///
/// Reads `workspace` reactively; writes the discovered repo status into
/// `git_state` (consumed by the render body and status bar). Both signals are
/// passed by value (`Signal<T>` is `Copy`).
pub fn use_git_state(
    workspace: Signal<Option<Workspace>>,
    mut git_state: Signal<Option<klinx_git::RepoStatus>>,
) {
    // ── Git: detect repo and compute status on workspace change ──────────
    use_effect(move || {
        let ws = (workspace)();
        if let Some(ref ws) = ws {
            match klinx_git::GitCliOps::discover(&ws.root) {
                Ok(ops) => {
                    if let Ok(status) = ops.status() {
                        git_state.set(Some(status));
                    }
                }
                Err(_) => git_state.set(None),
            }
        } else {
            git_state.set(None);
        }
    });

    // ── Filesystem watcher: auto-refresh git status + schema index ─────
    // Spawns a background watcher on the workspace root. Debounced 500ms.
    use_effect(move || {
        let ws = (workspace)();
        let Some(ref ws) = ws else { return };

        let root = ws.root.clone();
        let Some((_watcher, rx)) = crate::fs_watcher::start_watcher(&root) else {
            return;
        };

        // Spawn a polling loop that checks for debounced changes
        // and refreshes git/schema state.
        std::thread::spawn(move || {
            while let Ok(paths) = rx.recv() {
                if crate::fs_watcher::has_git_relevant_changes(&paths) {
                    // Refresh git status — we can't write to signals from a
                    // non-Dioxus thread directly. The git state is read-refreshed
                    // on next render cycle via the workspace effect above.
                    // For now, this ensures the watcher is running — the actual
                    // refresh is triggered by re-reading workspace signal.
                }
            }
        });
    });
}
