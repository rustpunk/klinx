//! # Klinx IDE — Clinker's Desktop Pipeline Editor
//!
//! Klinx is a Dioxus 0.7 desktop application (`wry` webview) for visually
//! authoring, inspecting, and debugging Clinker ETL pipelines.
//!
//! ## Composition browser
//!
//! The workspace scanner discovers all `.comp.yaml` files and presents them
//! in the activity bar's composition browser. Click any composition to open
//! it in the YAML editor or navigate to its canvas node.
//!
//! ## Composition drill-in and breadcrumb navigation
//!
//! Compositions render as collapsed nodes on the parent canvas. Clicking a
//! composition node descends into a sub-canvas showing the composition body.
//! A breadcrumb bar at the top tracks the navigation stack and allows
//! jumping back to any ancestor level.
//!
//! ## Channel loading and Raw/Resolved toggle
//!
//! Load a `.channel.yaml` file to see how channel overlays affect the
//! pipeline's configuration. The `ChannelViewMode` toggle (`Raw` vs
//! `Resolved`) controls whether the canvas and inspector show raw config from
//! the pipeline definition or resolved config with channel overlays applied.
//! This toggle is independent of drill-in level — inspect resolved values at
//! any nesting depth.
//!
//! ## Provenance panel
//!
//! The inspector's provenance panel shows the full override history for any
//! config field. Each layer (CompositionDefault, ChannelDefault, ChannelFixed,
//! InspectorEdit) is displayed with its source location, value, and whether it
//! won or was shadowed. Use `clinker explain --field` for the CLI equivalent.
//!
//! ## Extract-as-composition
//!
//! Select multiple nodes on the canvas (Shift+click), then click "Extract as
//! Composition" in the action bar. Klinx analyzes the selection boundary to
//! derive input/output ports from crossing edges and proposes config parameters
//! from literal constants. A confirmation modal lets you edit port names,
//! include/exclude config candidates, and set the output path. On confirm,
//! Klinx writes the `.comp.yaml` file and replaces the selection with a
//! composition call site in the parent pipeline.

// Hide the console window on Windows release builds
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod autodoc;
mod commands;
mod components;
mod cxl_bridge;
mod debug_state;
mod file_ops;
mod fs_watcher;
mod hooks;
mod keyboard;
mod notes;
mod parse_diagnostics;
mod perf;
mod pipeline_view;
mod search;
mod state;
mod sync;
mod tab;
mod template;
mod workspace;
mod yaml_patch;

use std::path::PathBuf;
use std::sync::OnceLock;

/// Workspace path passed via `--workspace <path>` CLI arg.
///
/// Highest priority in session restore — overrides last-used workspace and CWD detection.
static CLI_WORKSPACE: OnceLock<PathBuf> = OnceLock::new();

/// Get the CLI-specified workspace path, if any.
pub fn cli_workspace() -> Option<&'static PathBuf> {
    CLI_WORKSPACE.get()
}

fn run() {
    use dioxus::desktop::{Config, LogicalSize, WindowBuilder};

    // On Linux, WebKitGTK's DMABUF renderer (the default on recent versions)
    // silently falls back to software rendering — or paints nothing — on many
    // NVIDIA / Wayland configurations, which surfaces as sluggish or missing
    // frames. Force the stable render path unless the user has explicitly
    // chosen a value. The compositing cost for a 2D desktop UI is negligible
    // next to the stability win. See tauri-apps/tauri#9394, tauri-apps/wry#1315.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        // SAFETY: called at the very top of `run()` before any webview, GTK, or
        // background thread is created, so no other thread can observe the env.
        unsafe {
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        }
    }

    // Parse --workspace <path> from CLI args
    let args: Vec<String> = std::env::args().collect();
    if let Some(idx) = args.iter().position(|a| a == "--workspace")
        && let Some(path_str) = args.get(idx + 1)
    {
        let path = PathBuf::from(path_str);
        let resolved = if path.is_relative() {
            std::env::current_dir()
                .map(|cwd| cwd.join(&path))
                .unwrap_or(path)
        } else {
            path
        };
        let _ = CLI_WORKSPACE.set(resolved);
    }

    // Inlined into <head> via with_custom_head to bypass the wry custom-protocol
    // asset path on Windows (WebView2 IPC marshaling is ~10s for a 173KB file)
    // and to work around DioxusLabs/dioxus#2847 (asset!()-loaded CSS paints late).
    const KLINX_CSS: &str = include_str!("../assets/klinx.css");

    #[cfg_attr(not(target_os = "windows"), allow(unused_mut))]
    let mut cfg = Config::new()
        .with_window(
            WindowBuilder::new()
                .with_title("klinx")
                .with_decorations(false)
                .with_inner_size(LogicalSize::new(1400, 900))
                .with_min_inner_size(LogicalSize::new(800, 600))
                // Start hidden to avoid a white/unstyled flash on cold start.
                // AppShell restores saved geometry and reveals the window once
                // its first frame has mounted (see the `onmounted` on `.klinx-app`).
                .with_visible(false),
        )
        .with_disable_context_menu(true)
        .with_custom_head(format!("<style>{KLINX_CSS}</style>"));

    // Workaround for DioxusLabs/dioxus#2304: keep WebView2's user-data folder
    // in %LOCALAPPDATA% instead of next to the .exe, where ACLs / OneDrive
    // sync can add seconds of cold-start retries.
    #[cfg(target_os = "windows")]
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        cfg = cfg.with_data_directory(PathBuf::from(local).join("Klinx").join("WebView2"));
    }

    dioxus::LaunchBuilder::new()
        .with_cfg(cfg)
        .launch(app::AppShell);
}

fn main() {
    run();
}
