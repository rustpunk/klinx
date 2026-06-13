/// Root application shell: owns all reactive signals.
///
/// Per-tab state is stored as plain data in `TabEntry::snapshot`. AppShell
/// owns one set of signals (yaml_text, pipeline, etc.) that always reflect
/// the active tab. On tab switch, the departing tab's snapshot is updated
/// from the signals, and the arriving tab's snapshot is loaded into them.
///
/// This avoids Dioxus signal scope-ownership issues — all signals live in
/// AppShell's scope, which outlives every child component.
///
/// Navigation uses a two-level model:
/// - `NavigationContext` selects the active page (Pipeline, Channels, Git, Docs, Runs)
/// - `PipelineLayoutMode` selects the view within Pipeline (Canvas, Hybrid, Editor)
use crate::components::confirm_dialog::{ConfirmAction, ConfirmDialog, PendingConfirm};
use crate::components::toast::ToastState;
use crate::components::{
    activity_bar::ActivityBar, canvas::CanvasPanel, command_palette::CommandPalette,
    inspector::InspectorPanel, placeholder_page::PlaceholderPage, run_log::RunLogDrawer,
    schema_panel::SchemaPanel, schematics::SchematicsPanel, search_panel::SearchPanel,
    status_bar::StatusBar, tab_bar::TabBar, template_gallery::TemplateGallery, title_bar::TitleBar,
    toast::ToastOverlay, welcome_screen::WelcomeScreen, yaml_sidebar::YamlSidebar,
};
use clinker_schema::SchemaIndex;
use dioxus::desktop::tao::dpi::{PhysicalPosition, PhysicalSize};
use dioxus::desktop::tao::event::Event;
use dioxus::desktop::{WindowEvent, use_window, use_wry_event_handler};
use dioxus::prelude::*;

use crate::keyboard::handle_keyboard;
use crate::state::{
    AppState, ChannelViewMode, KilnTheme, LeftPanel, NavigationContext, PipelineLayoutMode,
    TabManagerState, use_app_state,
};
use crate::sync::EditSource;
use crate::tab::{TabEntry, TabId};
use crate::workspace;

#[component]
pub fn AppShell() -> Element {
    // ── Global signals (shared across all tabs) ──────────────────────────
    let run_log_expanded = use_signal(|| false);

    // ── Per-tab signals (owned here, swapped on tab switch) ──────────────
    let mut yaml_text = use_signal(String::new);
    let mut pipeline = use_signal(|| None);
    let mut parse_errors = use_signal(Vec::new);
    let mut edit_source = use_signal(|| EditSource::None);
    let mut selected_stages = use_signal(std::collections::HashSet::<String>::new);
    let schema_warnings = use_signal(Vec::new);
    let mut partial_pipeline = use_signal(|| None);
    // Debounced parse: keystrokes coalesce into one parse ~150ms after typing
    // stops. The debounce effect (below) bumps `parse_trigger`, which the parse
    // effect keys on; `debounce_gen` identifies the latest pending keystroke so
    // only the most recent debounce task fires.
    let mut parse_trigger = use_signal(|| 0u64);
    let mut debounce_gen = use_signal(|| 0u64);
    let channel_view_mode = use_signal(|| ChannelViewMode::Raw);
    let compiled_plan: Signal<Option<std::sync::Arc<clinker_core::plan::CompiledPlan>>> =
        use_signal(|| None);
    let composition_drill_stack: Signal<Vec<crate::state::CompositionDrillFrame>> =
        use_signal(Vec::new);

    // ── Session restore (single call on first mount per use_signal) ─────
    // restore_session() is called in the first use_signal closure. The result
    // is cached — subsequent closures read from the same signal. On re-renders,
    // use_signal closures don't re-execute, so disk I/O happens only once.
    let mut session_data: Signal<Option<workspace::SessionInit>> =
        use_signal(|| Some(workspace::restore_session()));

    // ── Navigation signals ──────────────────────────────────────────────
    let active_context = use_signal(|| {
        session_data
            .peek()
            .as_ref()
            .map(|s| s.context)
            .unwrap_or_default()
    });
    let pipeline_layout = use_signal(|| {
        session_data
            .peek()
            .as_ref()
            .map(|s| s.pipeline_layout)
            .unwrap_or_default()
    });
    let activity_bar_visible = use_signal(|| true);
    let nav_history: Signal<Vec<NavigationContext>> = use_signal(Vec::new);

    let mut tabs: Signal<Vec<TabEntry>> = use_signal(|| {
        session_data
            .write()
            .as_mut()
            .map(|s| std::mem::take(&mut s.tabs))
            .unwrap_or_default()
    });
    let active_tab_id: Signal<Option<TabId>> =
        use_signal(|| session_data.peek().as_ref().and_then(|s| s.active_tab_id));
    let mut prev_tab_id: Signal<Option<TabId>> = use_signal(|| None);
    let workspace: Signal<Option<workspace::Workspace>> = use_signal(|| {
        session_data
            .write()
            .as_mut()
            .and_then(|s| s.workspace.take())
    });

    // ── Left panel + schema index + template gallery ──────────────────────
    let left_panel: Signal<LeftPanel> = use_signal(|| LeftPanel::None);
    let schema_index: Signal<SchemaIndex> = use_signal(SchemaIndex::default);
    let show_template_gallery: Signal<bool> = use_signal(|| false);
    let git_state: Signal<Option<klinx_git::RepoStatus>> = use_signal(|| None);
    let show_command_palette: Signal<bool> = use_signal(|| false);
    let show_settings: Signal<bool> = use_signal(|| false);
    let channel_state: Signal<Option<crate::state::ChannelState>> = use_signal(|| None);
    let theme: Signal<KilnTheme> = use_signal(|| {
        session_data
            .peek()
            .as_ref()
            .and_then(|s| s.theme)
            .unwrap_or_default()
    });

    // ── Window: cache live geometry; restore + reveal after first paint ──
    // The window is created hidden (`with_visible(false)` in main.rs) so users
    // never see an unstyled white flash. Live geometry is cached from the
    // event-loop thread via `use_wry_event_handler` — never by querying the
    // window from the async autosave task, which is unsound on some platforms.
    // The cached value feeds session save; the restored value is applied just
    // before the window is revealed (see `onmounted` on `.kiln-app`).
    let desktop = use_window();
    // Geometry restored from the saved session, captured once at mount. The
    // live `window_geom` below (which feeds session save) must NOT drive the
    // restore: events emitted while the hidden window is laid out at its default
    // size would otherwise clobber the saved value before `onmounted` applies it.
    let restored_geom: Option<workspace::WindowGeometry> = use_hook(|| {
        workspace
            .peek()
            .as_ref()
            .and_then(|ws| ws.state.window.clone())
    });
    let mut window_geom: Signal<Option<workspace::WindowGeometry>> =
        use_signal(|| restored_geom.clone());
    {
        let desktop = desktop.clone();
        use_wry_event_handler(move |event, _| {
            if let Event::WindowEvent { event, .. } = event
                && matches!(event, WindowEvent::Moved(_) | WindowEvent::Resized(_))
                && let Ok(pos) = desktop.outer_position()
            {
                // While maximized, the reported frame is the maximized one, not
                // the user's normal window — keep the last normal geometry and
                // only record the flag, so un-maximizing on the next launch
                // returns to a sensible size rather than the full-screen size.
                let maximized = desktop.is_maximized();
                let (x, y, width, height) =
                    if maximized && let Some(g) = window_geom.peek().as_ref() {
                        (g.x, g.y, g.width, g.height)
                    } else {
                        let size = desktop.inner_size();
                        (pos.x, pos.y, size.width, size.height)
                    };
                window_geom.set(Some(workspace::WindowGeometry {
                    x,
                    y,
                    width,
                    height,
                    maximized,
                }));
            }
        });
    }

    // ── Git: discover repo + compute status, and run the FS watcher ──────
    crate::hooks::use_git_state(workspace, git_state);

    // ── Schema index: rebuild when workspace changes ─────────────────────
    crate::hooks::use_schema_index(workspace, schema_index);

    // ── Channel state: discover channels + restore persisted selection ───
    crate::hooks::use_channels(workspace, channel_state);

    // ── Workspace available: re-parse active tab to resolve compositions ──
    // On startup, tabs may restore before the workspace signal is set.
    // When workspace becomes available, trigger a re-parse so resolve_imports()
    // can find .comp.yaml files relative to the workspace root.
    {
        use_effect(move || {
            let ws = (workspace)();
            let text = (yaml_text)();
            let source = (edit_source)();
            if ws.is_some() && !text.is_empty() && source == EditSource::None {
                edit_source.set(EditSource::Yaml);
            }
        });
    }

    // ── Session persistence: save on quit, 5s autosave, tab-snapshot sync ─
    crate::hooks::use_session_persistence(
        workspace,
        tabs,
        active_tab_id,
        active_context,
        pipeline_layout,
        activity_bar_visible,
        channel_state,
        theme,
        window_geom,
        yaml_text,
        pipeline,
        partial_pipeline,
        parse_errors,
        selected_stages,
        edit_source,
    );

    // ── Toast + confirm dialog ───────────────────────────────────────────
    let toast_message: Signal<Option<ToastState>> = use_signal(|| None);
    let pending_confirm: Signal<Option<PendingConfirm>> = use_signal(|| None);

    // ── Tab switch: snapshot departing tab, load arriving tab ─────────────
    let current_active = (active_tab_id)();
    let previous = (prev_tab_id)();

    if current_active != previous {
        // Save departing tab's state from signals → snapshot
        if let Some(old_id) = previous {
            let mut tabs_w = tabs.write();
            if let Some(old_tab) = tabs_w.iter_mut().find(|t| t.id == old_id) {
                old_tab.snapshot.yaml_text = (yaml_text)();
                old_tab.snapshot.pipeline = (pipeline)();
                old_tab.snapshot.partial_pipeline = (partial_pipeline)();
                old_tab.snapshot.parse_errors = (parse_errors)();
                old_tab.snapshot.edit_source = (edit_source)();
                old_tab.snapshot.selected_stage = selected_stages.read().iter().next().cloned();
            }
        }

        // Load arriving tab's snapshot → signals
        if let Some(new_id) = current_active {
            let tabs_r = tabs.read();
            if let Some(new_tab) = tabs_r.iter().find(|t| t.id == new_id) {
                yaml_text.set(new_tab.snapshot.yaml_text.clone());
                pipeline.set(new_tab.snapshot.pipeline.clone());
                partial_pipeline.set(new_tab.snapshot.partial_pipeline.clone());
                parse_errors.set(new_tab.snapshot.parse_errors.clone());
                selected_stages.set(new_tab.snapshot.selected_stage.iter().cloned().collect());

                // If tab was loaded from disk (edit_source == None) and has content,
                // trigger a full re-parse to resolve _import compositions.
                // parse_yaml_raw_path() used during tab creation skips resolve_imports().
                if new_tab.snapshot.edit_source == EditSource::None
                    && !new_tab.snapshot.yaml_text.is_empty()
                {
                    edit_source.set(EditSource::Yaml);
                } else {
                    edit_source.set(new_tab.snapshot.edit_source);
                }
            }
        } else {
            // No active tab — clear signals
            yaml_text.set(String::new());
            pipeline.set(None);
            partial_pipeline.set(None);
            parse_errors.set(Vec::new());
            edit_source.set(EditSource::None);
            selected_stages.set(std::collections::HashSet::new());
        }

        prev_tab_id.set(current_active);

        // Save workspace state on tab switch
        if let Some(ref ws) = *workspace.read() {
            let active_file = current_active.and_then(|id| {
                tabs.read()
                    .iter()
                    .find(|t| t.id == id)
                    .and_then(|t| t.file_path.as_ref())
                    .map(|p| p.display().to_string())
            });
            let state = workspace::build_state_snapshot(
                &tabs.read(),
                active_file.as_deref(),
                (active_context)(),
                (pipeline_layout)(),
                (activity_bar_visible)(),
                &(channel_state)(),
                (theme)(),
                window_geom.peek().clone(),
            );
            workspace::save_workspace_state(&ws.root, &state);
            workspace::save_last_workspace(&ws.root);
        }
    }

    // ── Build AppState ───────────────────────────────────────────────────
    let current_app_state = AppState {
        active_context,
        pipeline_layout,
        run_log_expanded,
        selected_stages,
        yaml_text,
        pipeline,
        partial_pipeline,
        parse_errors,
        edit_source,
        schema_warnings,
        channel_view_mode,
        compiled_plan,
        composition_drill_stack,
    };

    let mut app_state_signal = use_signal(|| current_app_state);
    *app_state_signal.write() = current_app_state;

    // ── Provide contexts ─────────────────────────────────────────────────
    use_context_provider(|| app_state_signal);
    use_context_provider(|| TabManagerState {
        tabs,
        active_tab_id,
        workspace,
        left_panel,
        schema_index,
        show_template_gallery,
        git_state,
        show_command_palette,
        show_settings,
        activity_bar_visible,
        nav_history,
        channel_state,
        theme,
    });
    // ── Debug state context ────────────────────────────────────────────
    {
        use crate::debug_state::{DebugRunState, DebugState, DebugTab};
        use std::collections::{HashMap, HashSet};

        let debug_run_state = use_signal(|| DebugRunState::Idle);
        let debug_breakpoints = use_signal(HashSet::new);
        let debug_cond_breakpoints = use_signal(HashMap::new);
        let debug_drawer_open = use_signal(|| false);
        let debug_drawer_stage = use_signal(|| None::<String>);
        let debug_drawer_tab = use_signal(DebugTab::default);
        let debug_stage_cache = use_signal(HashMap::new);
        let debug_watches = use_signal(Vec::new);
        let debug_watch_collapsed = use_signal(|| false);
        let debug_editing_bp_stage = use_signal(|| None::<String>);
        let debug_downstream_dim_set = use_signal(HashSet::new);

        use_context_provider(|| DebugState {
            run_state: debug_run_state,
            breakpoints: debug_breakpoints,
            cond_breakpoints: debug_cond_breakpoints,
            drawer_open: debug_drawer_open,
            drawer_stage: debug_drawer_stage,
            drawer_tab: debug_drawer_tab,
            stage_cache: debug_stage_cache,
            watches: debug_watches,
            watch_collapsed: debug_watch_collapsed,
            editing_bp_stage: debug_editing_bp_stage,
            downstream_dim_set: debug_downstream_dim_set,
        });
    }

    use_context_provider(move || toast_message);
    use_context_provider(move || pending_confirm);

    // ── Keyboard handler ─────────────────────────────────────────────────
    let mut kb_tab_mgr = TabManagerState {
        tabs,
        active_tab_id,
        workspace,
        left_panel,
        schema_index,
        show_template_gallery,
        git_state,
        show_command_palette,
        show_settings,
        activity_bar_visible,
        nav_history,
        channel_state,
        theme,
    };

    // ── Debounce: bump `parse_trigger` ~150ms after the last YAML keystroke ─
    // The textarea echoes keystrokes natively, so trailing the parse has no
    // perceptible cost but collapses a burst of keystrokes into one
    // parse+validate+canvas-rebuild.
    {
        use_effect(move || {
            let _ = (yaml_text)(); // re-arm on every keystroke
            // Only debounce-parse YAML-sourced edits; inspector-driven yaml_text
            // writes carry EditSource::Inspector and must not schedule a parse.
            if *edit_source.peek() != EditSource::Yaml {
                return;
            }
            let generation = *debounce_gen.peek() + 1;
            debounce_gen.set(generation);
            spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                // Fire only if no newer keystroke arrived during the wait.
                if *debounce_gen.peek() == generation {
                    let next = *parse_trigger.peek() + 1;
                    parse_trigger.set(next);
                }
            });
        });
    }

    // ── Edit-sync: YAML ↔ pipeline ↔ schema validation ───────────────────
    // The EditSource guards inside this hook break the YAML↔inspector feedback
    // loop; the debounce effect above re-arms the parse via `parse_trigger`.
    crate::hooks::use_pipeline_sync(
        parse_trigger,
        edit_source,
        yaml_text,
        workspace,
        pipeline,
        partial_pipeline,
        parse_errors,
        schema_index,
        schema_warnings,
    );

    let has_active_tab = current_active.is_some();
    let current_ctx = (active_context)();

    rsx! {
        document::Title { "klinx" }

        div {
            class: "kiln-app",
            "data-theme": (theme)().as_data_attr(),
            tabindex: "0",
            // Reveal the window once its first frame has mounted, applying any
            // restored geometry while still hidden so there is no visible jump.
            onmounted: move |_| {
                if let Some(g) = restored_geom.as_ref() {
                    // Only restore the position if it still lands on a connected
                    // monitor; a coordinate from a now-disconnected display would
                    // strand the frameless (un-draggable) window off-screen.
                    let on_screen = desktop.available_monitors().any(|m| {
                        let mp = m.position();
                        let ms = m.size();
                        g.x >= mp.x
                            && g.y >= mp.y
                            && g.x < mp.x + ms.width as i32
                            && g.y < mp.y + ms.height as i32
                    });
                    if on_screen {
                        desktop.set_outer_position(PhysicalPosition::new(g.x, g.y));
                    }
                    desktop.set_inner_size(PhysicalSize::new(g.width, g.height));
                    if g.maximized {
                        desktop.set_maximized(true);
                    }
                }
                desktop.set_visible(true);
            },
            onkeydown: move |e: KeyboardEvent| {
                if handle_keyboard(&e, &mut kb_tab_mgr) {
                    e.prevent_default();
                }
            },

            TitleBar {}

            // ── Body: activity bar + content area ──
            div {
                class: "kiln-body",

                ActivityBar {}

                div {
                    class: "kiln-content-area",

                    // Tab bar: Pipeline context only
                    if current_ctx == NavigationContext::Pipeline {
                        TabBar {}
                    }

                    // Context content dispatch with cross-fade
                    div {
                        class: "kiln-context-content",
                        key: "{current_ctx.as_data_attr()}",
                        "data-context": current_ctx.as_data_attr(),

                        match current_ctx {
                            NavigationContext::Pipeline => rsx! {
                                if has_active_tab {
                                    ActiveTabContent {
                                        key: "{current_active.map(|id| id.to_string()).unwrap_or_default()}",
                                    }
                                } else {
                                    WelcomeScreen {}
                                }
                            },
                            NavigationContext::Git => rsx! {
                                GitContextPage {}
                            },
                            NavigationContext::Docs => rsx! {
                                PlaceholderPage {
                                    name: "Technical Guide",
                                    description: "Klinx user guide, CXL reference, and pipeline authoring documentation.",
                                }
                            },
                            NavigationContext::Channels => rsx! {
                                PlaceholderPage {
                                    name: "Channels",
                                    description: "Channel management: load .channel.yaml files and toggle between Raw and Resolved views on the canvas.",
                                }
                            },
                            NavigationContext::Runs => rsx! {
                                PlaceholderPage {
                                    name: "Run History",
                                    description: "Pipeline execution history, filtering, and run detail breakdowns.",
                                }
                            },
                        }
                    }
                }
            }

            RunLogDrawer {}
            StatusBar {}
            ToastOverlay {}

            // ── Template gallery overlay ──────────────────────────────────
            if (show_template_gallery)() {
                TemplateGallery {}
            }

            // ── Command palette overlay ─────────────────────────────────
            if (show_command_palette)() {
                CommandPalette {}
            }

            if let Some(pending) = (pending_confirm)() {
                ConfirmDialog {
                    pending: pending.clone(),
                    on_action: move |action: ConfirmAction| {
                        let mut confirm = pending_confirm;
                        let tab_id = pending.tab_id;
                        match action {
                            ConfirmAction::Save => {
                                crate::keyboard::save_and_close_tab(
                                    &mut kb_tab_mgr, tab_id,
                                );
                                confirm.set(None);
                            }
                            ConfirmAction::Discard => {
                                crate::keyboard::force_close_tab(
                                    &mut kb_tab_mgr, tab_id,
                                );
                                confirm.set(None);
                            }
                            ConfirmAction::Cancel => {
                                confirm.set(None);
                            }
                        }
                    },
                }
            }
        }
    }
}

/// Git context page — renders the version-control workspace.
#[component]
fn GitContextPage() -> Element {
    use crate::components::version_mode::VersionMode;
    rsx! { VersionMode {} }
}

/// Active tab's content area — Pipeline context only.
///
/// Renders canvas, inspector, YAML sidebar, and left panel slot.
/// Keyed on tab ID for clean remount.
#[component]
fn ActiveTabContent() -> Element {
    let state = use_app_state();
    let pipeline_layout = state.pipeline_layout;
    let selected_stages = state.selected_stages;
    let tab_mgr = use_context::<TabManagerState>();
    let left_panel = (tab_mgr.left_panel)();

    // Schematics layout mode: full-pipeline documentation view
    if *pipeline_layout.read() == PipelineLayoutMode::Schematics {
        return rsx! {
            div {
                class: "kiln-main",
                "data-layout": "schematics",
                SchematicsPanel {}
            }
        };
    }

    rsx! {
        div {
            class: "kiln-main",
            "data-layout": pipeline_layout.read().as_data_attr(),

            // ── Left panel slot (280px, shared between Search, Schemas, Compositions) ──
            match left_panel {
                LeftPanel::Search => rsx! {
                    SearchPanel {}
                },
                LeftPanel::Schemas => rsx! {
                    SchemaPanel {}
                },
                LeftPanel::Compositions => rsx! {},
                LeftPanel::None => rsx! {},
            }

            CanvasPanel {}

            // Inspector shows for any selected stage, even when drilled into a composition.
            // Drilled transforms exist in state.pipeline (expanded during resolution).
            {
                let stages = selected_stages.read();
                if stages.len() == 1 {
                    let stage_id = stages.iter().next().unwrap().clone();
                    drop(stages);
                    rsx! {
                        InspectorPanel {
                            key: "{stage_id}",
                            stage_id: stage_id.clone(),
                        }
                    }
                } else {
                    rsx! {}
                }
            }

            YamlSidebar {}
        }
    }
}
