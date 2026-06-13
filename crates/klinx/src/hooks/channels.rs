//! Channel discovery: scan the workspace for `.channel.yaml` bindings and
//! restore the persisted active/recent selection.
//!
//! Owns one side effect, behavior-identical to its former inline form in
//! `AppShell`.

use dioxus::prelude::*;

use crate::state::ChannelState;
use crate::workspace::{self, Workspace};

/// Discover channels for the active workspace and restore persisted selection.
///
/// Reads `workspace` reactively; writes the discovered state into
/// `channel_state` (cleared to `None` when no workspace is loaded). The
/// persisted `active`/`recent` selection from the workspace state overlays the
/// freshly-discovered bindings. Both signals are passed by value (`Signal<T>`
/// is `Copy`).
pub fn use_channels(
    workspace: Signal<Option<Workspace>>,
    mut channel_state: Signal<Option<ChannelState>>,
) {
    // ── Channel state: discover channels when workspace changes ────────
    use_effect(move || {
        let ws = (workspace)();
        if let Some(ref ws) = ws {
            let mut state = workspace::discover_channels(ws);
            // Restore persisted channel selection
            if let Some(ref mut cs) = state
                && let Some(ref chan_persist) = ws.state.channels
            {
                cs.active_channel = chan_persist.active.clone();
                cs.recent_channels = chan_persist.recent.clone();
            }
            channel_state.set(state);
        } else {
            channel_state.set(None);
        }
    });
}
