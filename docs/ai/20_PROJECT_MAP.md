# Project Map

## Workspace

| Path | Purpose | Important files | Dependencies | Tests/examples | Confidence |
| --- | --- | --- | --- | --- | --- |
| `Cargo.toml` | Rust workspace root and shared dependency pins | workspace members, release profile, Clinker git deps | Rust 1.91 via `rust-toolchain.toml`; Dioxus `=0.7.4`; Clinker rev `997ea7d` | CI commands use workspace | High |
| `crates/klinx` | Dioxus desktop IDE binary | `src/main.rs`, `src/app.rs`, `src/state.rs`, `src/workspace.rs`, `Dioxus.toml`, `assets/klinx.css` | Dioxus desktop, Clinker crates, `klinx-git`, `tokio`, `notify`, `serde`, `petgraph`, `rfd` | many in-file unit tests; `tests/fixtures/large_pipeline.yaml` | High |
| `crates/klinx-git` | Git/VCS abstraction library | `src/lib.rs`, `src/ops.rs`, `src/gix_backend.rs`, `src/provider.rs`, `src/types.rs` | `tokio` declared; runtime `git` and `gh` CLI tools | unit tests in backend/provider modules | High |
| `examples/pipelines` | Sample Clinker workspace | `kiln.toml`, `*.yaml`, `compositions`, `channels`, `data`, `retract-demo` | parsed by app/engine | parse-checked from `template.rs` tests | High |
| `.github/workflows/ci.yml` | CI build/test matrix | Rust toolchain setup, Dioxus CLI install, fmt/clippy/test/build | Linux WebKitGTK/GTK/xdo packages | runs on Linux/macOS/Windows | High |
| `docs/perf.md` | Manual performance measurement guide | large fixture, `perf-trace` workflow | `perf-trace` feature | fixture path in `crates/klinx/tests/fixtures` | High |
| `docs/research` | Research/planning notes | field lineage and route visualization notes | source facts may need revalidation | planning evidence, not always current | Medium |

## `crates/klinx` Source Areas

| Area | Purpose | Important modules/files | Internal dependencies | Tests/examples | Confidence |
| --- | --- | --- | --- | --- | --- |
| App shell/state | Own Dioxus signals, tabs, navigation, layouts, overlays | `main.rs`, `app.rs`, `state.rs`, `tab.rs` | components, hooks, workspace, sync | app behavior mostly covered indirectly | High |
| Workspace/session | Workspace manifests, state persistence, restore, channel discovery | `workspace.rs` | `dirs`, `toml`, `serde_json`, tab/state types | examples workspace, session paths | High |
| Sync/parsing | YAML parse routing, composition detection, partial views, ranges | `sync.rs`, `parse_diagnostics.rs` | Clinker plan/yaml, pipeline view | composition and parse diagnostic tests | High |
| Pipeline view | Canvas model, current layout, future port-aware layout model, route ports, composition views, field lineage integration | `pipeline_view.rs`, `pipeline_view/field_lineage.rs`, `pipeline_view/layout_model.rs` | Clinker plan/exec/schema, CXL, petgraph | extensive in-file tests | High |
| YAML patching | Preserve document structure when inspector edits parsed model | `yaml_patch.rs` | Clinker YAML serializer/parser | preservation and fallback tests | High |
| Components | Dioxus rendering for all UI surfaces | `components/**`, `assets/klinx.css` | app/state contexts, pipeline models, git, schema/search/template helpers | component-adjacent helper tests | High |
| Hooks | Side effects for channels, git, pipeline sync, schema index, session persistence | `hooks/**` | state/workspace/git/schema/sync | mostly behavior comments and indirect coverage | Medium |
| Templates/examples | Bundled template metadata and example loading | `template.rs`, `src/templates/*.yaml` | Clinker parser, filesystem examples | template and vendored example parse tests | High |
| Search | Text and structural search over YAML files | `search.rs`, `components/search_panel/**` | regex, workspace paths, Clinker parse helpers | search unit tests | High |
| Autodoc/schematics | Generated stage docs and CXL statement summaries | `autodoc.rs`, `components/schematics/**`, `components/inspector/drawer_docs.rs` | Clinker config, heuristic CXL parsing | autodoc tests | High |
| Debug/run state | Data models for run/debug views | `debug_state.rs`, `components/run_log/**` | chrono dev tests, Dioxus contexts | extensive `debug_state.rs` tests | High |

## `crates/klinx-git` Modules

| Module | Purpose | Important APIs | Tests | Confidence |
| --- | --- | --- | --- | --- |
| `lib.rs` | Public exports and crate purpose | reexports `GitCliOps`, `GitOps`, types, provider helpers | doctest target exists | High |
| `ops.rs` | Stable git operation trait and error type | `GitOps`, `GitError` | consumed by backend/UI | High |
| `gix_backend.rs` | CLI-backed git implementation | `GitCliOps::{discover, discover_with_binary, root}` | parse/status/discover/log/blame tests | High |
| `provider.rs` | Remote/provider parsing and PR creation | `detect_provider`, `parse_remote_url`, `create_pr` | provider URL tests | High |
| `types.rs` | Status/branch/log/blame DTOs | `RepoStatus`, `FileStatus`, `StatusKind`, `CommitInfo` | used in backend/UI | High |

## Architecturally Important External Dependencies

- `dioxus =0.7.4`: desktop UI framework.
- `cxl`, `clinker-plan`, `clinker-exec`, `clinker-core-types`, `clinker-record`, `clinker-schema`, `clinker-channel`: engine surface from Clinker git rev `997ea7d`.
- `serde-saphyr` and `saphyr-parser-bw`: YAML parsing and span/token support.
- `petgraph`: compiled composition body views.
- `notify`: filesystem watching.
- `rfd`: native file dialogs.
- `klinx-git`: local git abstraction crate.
- Runtime tools: `git`, `gh`, Dioxus CLI `dx`.

## Evidence

Evidence sources include `Cargo.toml`, `crates/klinx/Cargo.toml`, `crates/klinx-git/Cargo.toml`, `README.md`, `CLAUDE.md`, `.github/workflows/ci.yml`, `docs/perf.md`, `examples/README.md`, module declarations in `main.rs`, component exports in `components/mod.rs`, hook exports in `hooks/mod.rs`, and in-file test names found under `crates/**/src`.
