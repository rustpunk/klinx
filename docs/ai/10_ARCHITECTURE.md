# Architecture

## Project Overview

**Verified.** Klinx is a Rust 2024 workspace containing a native Dioxus 0.7 desktop IDE for authoring Clinker YAML pipeline configurations. It runs as a `wry` webview desktop app and consumes Clinker engine crates through git-pinned workspace dependencies at rev `997ea7d`.

## Major Subsystems

- **Desktop app crate (`crates/klinx`)**: Dioxus shell, app state, tabs, workspace/session persistence, pipeline parsing, canvas model, UI components, search, templates, debug/run data models.
- **Git abstraction crate (`crates/klinx-git`)**: CLI-backed `GitOps` trait implementation plus provider helpers for remote parsing and GitHub PR creation.
- **Examples (`examples/pipelines`)**: ready-to-open Clinker workspace with pipelines, compositions, channel overlays, CSV data, and retraction demo.
- **Tooling and CI**: cargo fmt, two clippy passes, cargo test, cargo deny, and Dioxus desktop bundle build across Linux/macOS/Windows.

## Data And Control Flow

1. `main.rs` configures Dioxus desktop, parses optional `--workspace <path>`, inlines CSS, and launches `app::AppShell`.
2. `AppShell` owns top-level Dioxus signals. Per-tab editor/canvas signals are swapped into and out of `TabEntry::snapshot`.
3. Workspace restore loads `kiln.toml`, `.kiln-state.json`, last-workspace state, or CLI workspace state through `workspace.rs`.
4. YAML edits update `yaml_text` with an `EditSource`. `hooks/pipeline_sync.rs` debounces parsing and syncs parsed models back into active tab state.
5. `sync.rs` routes YAML through pipeline or composition parsing, resolves imports when a workspace root is available, and can produce partial views for invalid YAML.
6. `pipeline_view.rs` and `pipeline_view/field_lineage.rs` derive canvas-ready models, layout, connections, branch ports, and field lineage. Compiled composition drill-in uses `derive_body_view` over `BoundBody`; body field rows come from `BoundBody::body_rows` and missing rows degrade to node-level body connectors. `pipeline_view/layout_model.rs` is a pure Rust scaffold for future port-aware layered layout; the visible canvas still uses `layout_positions`.
7. Components consume `AppState` and `TabManagerState` contexts to render canvas, YAML editor, inspector, schemas, search, git, and overlays.
8. `klinx-git` shells out to `git` and `gh` for version-control operations used by version-mode UI and git status hooks.

## Important Boundaries

- **Clinker engine boundary**: engine types and parsers are external git dependencies. Do not vendor or casually replace them.
- **Editor text boundary**: YAML text is authoritative. Parsed `PipelineConfig` is a model, not the saved document.
- **UI/model boundary**: `pipeline_view` creates UI-safe view models; components should not reimplement graph derivation.
- **Layout migration boundary**: `layout_model` can represent ordered node, field-row, and branch ports with ranked layers and connector paths, but it is not yet the default renderer layout. Prior research in `docs/research/2026-06-13-field-lineage-ui.md` and `docs/research/2026-06-14-route-node-visualization.md` points toward a Rust Sugiyama-style layered pass with port-aware crossing minimization and orthogonal routing; this scaffold captures the data shape before the visible canvas migration.
- **Git boundary**: `klinx-git` owns repository operations; UI should avoid ad hoc shelling out.
- **Desktop-only boundary**: no wasm/web target or Playwright browser target is documented.

## Public API Surfaces Or Entry Points

- App: `app::AppShell`, `main::cli_workspace`, `state::{AppState, TabManagerState}`.
- Workspace/session: `workspace::{load_workspace, restore_session, save_full_session, build_state_snapshot}`.
- Parsing/view: `sync::{parse_yaml, try_parse_yaml, parse_composition}`, `pipeline_view::{derive_pipeline_view, derive_composition_view, derive_body_view, derive_partial_pipeline_view, layout_model}`.
- YAML patching: `yaml_patch::{patch_yaml_preserving_nodes, serialize_yaml_full}`.
- Git: `klinx_git::{GitOps, GitCliOps, RepoStatus, create_pr, detect_provider}`.
- Commands: `dx serve --package klinx --platform desktop`, `cargo test --workspace`, CI commands in `.github/workflows/ci.yml`.

## Ownership, State, And Concurrency

**Verified.** `AppShell` owns all reactive signals to avoid Dioxus signal scope issues. Tabs hold plain data snapshots. Hook calls must remain unconditional and stable.

**Verified.** Parse work is debounced around editor input. Visible YAML errors settle on a longer delay than parsing to avoid flicker.

**Strong inference.** Background watchers and git refresh are intentionally conservative because Dioxus signals cannot be updated directly from arbitrary watcher threads.

## Configuration, Serialization, And Resource Loading

- Workspace config is `kiln.toml`; persisted UI/session state is `.kiln-state.json`.
- CSS is included with `include_str!("../assets/klinx.css")` and injected into the desktop webview head.
- Template YAML files are bundled in `crates/klinx/src/templates`.
- Example workspaces live under `examples/pipelines`.
- `saphyr-parser-bw` is directly pinned to match the span-tracking parser already used by `serde-saphyr`.

## Error Handling Strategy

**Verified.** UI-facing app helpers often return `Result<T, String>` for file and parse operations, while `klinx-git` uses a custom `GitError`. Parse diagnostics are refined in `parse_diagnostics.rs`.

**Strong inference.** Expected user-facing failures should surface as diagnostics, toasts, or result errors rather than panics. Some tests use `unwrap`/`expect` for fixtures and invariants.

## Extension Or Plugin Boundaries

No plugin system was found. The clearest extension boundaries are:

- Clinker engine dependency pin in root `Cargo.toml`.
- `GitOps` trait for future git backend replacement.
- Template YAML files and example workspaces.
- Component modules that consume shared contexts.

## Open Question Routing

Current unresolved architecture questions are tracked in `docs/ai/80_OPEN_QUESTIONS.md`. Check that registry before changing README dependency prose, placeholder UI surfaces, git status watcher refresh, or the future of the `gix_backend.rs` module.
