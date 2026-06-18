# Glossary

| Term | Meaning | Where it appears | Related terms | Confidence |
| --- | --- | --- | --- | --- |
| Klinx | Desktop IDE for authoring Clinker YAML pipeline configurations | `README.md`, `main.rs` | Clinker, Dioxus | High |
| Clinker | External pipeline engine whose crates are git-pinned dependencies | `Cargo.toml`, examples | CXL, pipeline config | High |
| CXL | Expression language used in transforms/validation/lineage | `cxl_bridge.rs`, `pipeline_view/field_lineage.rs`, `components/inspector/model.rs`, `autodoc.rs` | field lineage, transform | High |
| Dioxus | Rust UI framework used for the desktop app | `crates/klinx/Cargo.toml`, components | signals, hooks, RSX | High |
| wry | Native webview runtime used by Dioxus desktop | `README.md`, `main.rs` | WebKitGTK, WebView2 | High |
| `AppShell` | Root Dioxus component that owns top-level signals | `app.rs` | `AppState`, `TabManagerState` | High |
| `AppState` | Active-tab state context used by components | `state.rs`, components | `TabEntry`, `yaml_text` | High |
| `TabManagerState` | Global shell/workspace/tab/git context | `state.rs`, components | workspace, tabs, overlays | High |
| `TabEntry` | Plain data model for open editor tabs and snapshots | `tab.rs`, `workspace.rs` | `TabSnapshot` | High |
| `EditSource` | Marker for origin of edits to prevent sync loops | `sync.rs`, hooks | YAML sync, inspector edits | High |
| `PipelineView` | Canvas-ready view model for stages and connections | `pipeline_view.rs`, canvas | `StageView`, `Connection` | High |
| `StageView` | Renderable node/card data for a pipeline stage | `pipeline_view.rs`, canvas node | `StageKind` | High |
| Field lineage | Inferred relationships between fields through pipeline stages | `pipeline_view/field_lineage.rs`, canvas | `FieldEdge`, CXL | High |
| `FieldEdgeKind` | Classification of field lineage edges | `pipeline_view/field_lineage.rs` | carry, derive, project | High |
| Composition | Reusable Clinker pipeline fragment detected by `_compose:` | `sync.rs`, examples/compositions | drill-in, ports | High |
| Channel | Workspace override layer for tenant/environment-specific config | `workspace.rs`, examples/channels | raw/resolved mode | Medium-High |
| Provenance | History of config override layers shown in inspector | `main.rs` docs, inspector provenance | channel override | Medium |
| Raw/Resolved | UI mode for showing raw pipeline config or channel-resolved config | `main.rs`, `state.rs` | channel view mode | High |
| Schematics | Generated documentation/blueprint-style view for stages | `components/schematics`, `autodoc.rs` | autodoc | Medium-High |
| Autodoc | Generated stage documentation model | `autodoc.rs`, components | schematics, inspector docs | High |
| `GitOps` | Trait boundary for git operations | `klinx-git/src/ops.rs` | `GitCliOps` | High |
| `GitCliOps` | Current CLI-backed git implementation | `klinx-git/src/gix_backend.rs` | git CLI, future gix | High |
| `gix_backend.rs` | Filename for current git CLI backend, not actual gitoxide use | `klinx-git/src/gix_backend.rs` | `GitCliOps` | High |
| `perf-trace` | Feature flag for parse/tokenize timing logs | `crates/klinx/Cargo.toml`, `docs/perf.md` | typing latency | High |
| `kiln.toml` | Workspace manifest file used by examples/app | `workspace.rs`, examples | workspace root | High |
| `.kiln-state.json` | Persisted UI/session state file | `workspace.rs`, examples | tabs, window, search | High |
| `dx` | Dioxus CLI command | README, CI | serve, build | High |
