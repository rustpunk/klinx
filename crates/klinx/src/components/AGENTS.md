# AGENTS.md

## Purpose

This directory is the Dioxus component layer for the Klinx desktop IDE. It renders app chrome, canvas, YAML editor, inspector, explorer, search, schema, schematics, git/version mode, overlays, toasts, and placeholder pages.

## Responsibilities

- Render UI from `AppState` and `TabManagerState`.
- Keep layout, CSS class names, and data attributes aligned.
- Keep pure UI helper logic testable near components.
- Avoid introducing render-path work that belongs in models or hooks.

## Important Public APIs Or Entry Points

- `components::mod.rs` module exports.
- `CanvasPanel`, `YamlSidebar`, `SelectedInspector`, `FileExplorer`, `SearchPanel`, `SchemaPanel`, `SchematicsPanel`, `VersionMode`.
- Utility overlays: `CommandPalette`, `SettingsOverlay`, `TemplateGallery`, `ToastOverlay`, `ConfirmDialog`.

## Internal Module Map

- Chrome: `activity_bar.rs`, `title_bar.rs`, `status_bar.rs`, `tab_bar.rs`.
- Pipeline editing: `canvas/**`, `yaml_sidebar/**`, `inspector/**`, `schematics/**`.
- Side panels: `file_explorer/**`, `search_panel/**`, `schema_panel/**`.
- Git: `version_mode/**`.
- Overlays/utilities: command palette, settings, templates, run log, toast, confirm dialog.

## Dependency Rules

Components should consume app contexts and existing model helpers. Do not parse pipeline YAML, shell out to git, or rederive pipeline graph structure inside RSX when a model/helper boundary exists.

## Important Invariants

- `AppState` is active-tab state; `TabManagerState` is global shell/workspace state.
- Navigation is two-level: `NavigationContext` plus `PipelineLayoutMode`.
- CSS depends on `data-theme`, `data-layout`, `data-context`, and `klinx-*` class names.
- YAML highlight overlay must stay byte-aligned with textarea text.
- `LINE_HEIGHT` in YAML highlight code must match CSS.
- Canvas geometry must match `pipeline_view` node sizes and connector anchors.

## Common Mistakes To Avoid

- Moving Dioxus hooks into conditionals or event handlers.
- Changing CSS line height without updating code constants.
- Adding recursive file explorer rendering that rewalks disk on expand/collapse.
- Inline-styling field connector strokes instead of using CSS classes.
- Treating placeholder pages or partially wired actions as complete workflows.

## Local Commands

- `cargo test -p klinx file_explorer`
- `cargo test -p klinx tokenizer`
- `cargo test -p klinx inspector`
- `cargo test -p klinx humanize_branch`
- `dx serve --package klinx --platform desktop`
- `cargo build --package klinx` then `scripts/shot.sh shot.png ./examples/pipelines` when headless visual review is available.

## Documentation Updates

Update `docs/ai/30_DESIGN_RULES.md`, `40_COMMON_PATTERNS.md`, and `60_PERFORMANCE_NOTES.md` when component or CSS contracts change.

## Approval Gates

Ask before large visual redesigns, changing canvas geometry semantics, formalizing placeholder features, or introducing a new UI testing strategy.

## Evidence

`components/mod.rs`, `app.rs`, `state.rs`, `components/canvas/**`, `components/yaml_sidebar/**`, `components/file_explorer/model.rs`, `components/inspector/panel.rs`, `assets/klinx.css`, and `docs/ai`.
