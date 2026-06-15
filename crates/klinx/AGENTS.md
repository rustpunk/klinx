# AGENTS.md

## Purpose

This crate is the Klinx Dioxus desktop IDE binary. It owns the app shell, workspace/session state, tabs, YAML parse/sync, pipeline view models, templates, search, debug models, components, CSS, and desktop launch config.

## Responsibilities

- Launch the desktop app through `main.rs` and `app::AppShell`.
- Maintain active-tab state, global workspace/tab/git state, and session persistence.
- Parse Clinker YAML and derive canvas/inspector models.
- Preserve user-authored YAML text through editing and saving.
- Render and coordinate the IDE UI under `src/components`.

## Important Public APIs Or Entry Points

- `app::AppShell`
- `state::{AppState, TabManagerState, NavigationContext, PipelineLayoutMode}`
- `workspace::{load_workspace, restore_session, save_full_session}`
- `sync::{try_parse_yaml, parse_composition}`
- `pipeline_view::{derive_pipeline_view, derive_composition_view, derive_partial_pipeline_view}`
- `yaml_patch::patch_yaml_preserving_nodes`

## Internal Module Map

- `main.rs`: desktop configuration, CLI workspace argument, CSS injection.
- `app.rs`: root signals, contexts, layout dispatch, parse debounce.
- `state.rs`, `tab.rs`, `workspace.rs`: state and persistence models.
- `sync.rs`, `pipeline_view.rs`, `yaml_patch.rs`, `cxl_bridge.rs`, `autodoc.rs`: pipeline semantics.
- `components/**`: Dioxus rendering layer.
- `hooks/**`: side effects for sync, git, schema, channels, and session save.

## Dependency Rules

Use root workspace dependencies and current Clinker pins from `Cargo.toml`. Do not bump Dioxus, Clinker, or add dependencies without explicit approval.

## Important Invariants

- `AppShell` owns long-lived Dioxus signals.
- Hooks must remain unconditional and stable.
- Inactive tabs store plain `TabEntry` snapshots.
- `EditSource` guards parse/sync loops.
- YAML text is authoritative for saves.
- Use node-preserving YAML patching for inspector-driven edits.

## Common Mistakes To Avoid

- Serializing `PipelineConfig` for normal saves.
- Moving hooks into conditionals or event handlers.
- Updating `yaml_text` without considering `EditSource`.
- Treating README dependency prose as more current than `Cargo.toml`.
- Assuming desktop UI has browser automation coverage.

## Local Commands

- `cargo test -p klinx <filter>`
- `cargo test -p klinx pipeline_view`
- `cargo test -p klinx sync`
- `cargo test -p klinx yaml_patch`
- `cargo test -p klinx template`
- `dx serve --package klinx --platform desktop`

## Documentation Updates

Update `doc/ai/10_ARCHITECTURE.md`, `20_PROJECT_MAP.md`, `30_DESIGN_RULES.md`, and `AI_CHANGELOG.md` when this crate's architecture or invariants change.

## Unclear / Ask Human

Ask before changing Clinker/Dioxus pins, YAML preservation semantics, workspace state format, or desktop runtime behavior.

## Evidence

`main.rs`, `app.rs`, `state.rs`, `workspace.rs`, `sync.rs`, `pipeline_view.rs`, `yaml_patch.rs`, `docs/perf.md`, `.github/workflows/ci.yml`, and `doc/ai`.
