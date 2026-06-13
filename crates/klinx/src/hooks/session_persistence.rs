//! Session persistence: save-on-quit, periodic autosave, and active-tab
//! snapshot mirroring.
//!
//! Owns three side effects, all behavior-identical to their former inline
//! form in `AppShell`:
//! - a `use_drop` that flushes the full session when `AppShell`'s scope drops
//!   (window close, app exit),
//! - a 5-second `use_future` autosave loop, and
//! - a `use_effect` that mirrors the active tab's snapshot from the live edit
//!   signals so `is_dirty()`/save paths see current state.

use dioxus::prelude::*;

use crate::state::{ChannelState, KilnTheme, NavigationContext, PipelineLayoutMode};
use crate::sync::EditSource;
use crate::tab::{TabEntry, TabId};
use crate::workspace::{self, WindowGeometry, Workspace};

/// Wire up full-session persistence for `AppShell`.
///
/// Registers the save-on-drop, 5s autosave, and active-tab snapshot effects.
/// All signals are passed by value (`Signal<T>` is `Copy`); the snapshot effect
/// writes `tabs`, while the save paths read every persisted signal via `.peek()`.
#[allow(clippy::too_many_arguments)]
pub fn use_session_persistence(
    workspace: Signal<Option<Workspace>>,
    mut tabs: Signal<Vec<TabEntry>>,
    active_tab_id: Signal<Option<TabId>>,
    active_context: Signal<NavigationContext>,
    pipeline_layout: Signal<PipelineLayoutMode>,
    activity_bar_visible: Signal<bool>,
    channel_state: Signal<Option<ChannelState>>,
    theme: Signal<KilnTheme>,
    window_geom: Signal<Option<WindowGeometry>>,
    yaml_text: Signal<String>,
    pipeline: Signal<Option<clinker_core::config::PipelineConfig>>,
    partial_pipeline: Signal<Option<clinker_core::partial::PartialPipelineConfig>>,
    parse_errors: Signal<Vec<String>>,
    selected_stages: Signal<std::collections::HashSet<String>>,
    edit_source: Signal<EditSource>,
) {
    // ── Session persistence: save on quit + periodic autosave ─────────────
    // use_drop fires when AppShell's scope drops (window close, app exit).
    use_drop(move || {
        workspace::save_full_session(
            &workspace.peek(),
            &tabs.peek(),
            *active_tab_id.peek(),
            *active_context.peek(),
            *pipeline_layout.peek(),
            *activity_bar_visible.peek(),
            &channel_state.peek(),
            *theme.peek(),
            window_geom.peek().clone(),
        );
    });

    // Periodic autosave: flush state to disk every 5 seconds.
    // Catches all state changes even if the user never switches tabs.
    // Worst case on force-kill: lose last 5 seconds of layout/tab state.
    use_future(move || async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            workspace::save_full_session(
                &workspace.peek(),
                &tabs.peek(),
                *active_tab_id.peek(),
                *active_context.peek(),
                *pipeline_layout.peek(),
                *activity_bar_visible.peek(),
                &channel_state.peek(),
                *theme.peek(),
                window_geom.peek().clone(),
            );
        }
    });

    // ── Sync active tab snapshot from signals (debounced) ────────────────
    // Keeps the active tab snapshot current for is_dirty()/save. Subscribes to
    // the parse OUTPUTS (pipeline/partial/errors) — which the parse effect
    // already debounces — plus selection, so `tabs` is written ~once per typing
    // pause instead of per keystroke (no tab-bar re-render on every character).
    // yaml_text/edit_source are read non-reactively. The dirty dot therefore
    // appears ~150ms after typing starts; save/close paths call
    // `flush_active_snapshot` first so they never act on stale text (keyboard.rs).
    use_effect(move || {
        let pl = (pipeline)();
        let pp = (partial_pipeline)();
        let errs = (parse_errors)();
        let sel = selected_stages.read().iter().next().cloned();

        if let Some(active_id) = (active_tab_id)() {
            let text = yaml_text.peek().clone();
            let src = *edit_source.peek();
            let mut tabs_w = tabs.write();
            if let Some(tab) = tabs_w.iter_mut().find(|t| t.id == active_id) {
                tab.snapshot.yaml_text = text;
                tab.snapshot.pipeline = pl;
                tab.snapshot.partial_pipeline = pp;
                tab.snapshot.parse_errors = errs;
                tab.snapshot.edit_source = src;
                tab.snapshot.selected_stage = sel;
            }
        }
    });
}
