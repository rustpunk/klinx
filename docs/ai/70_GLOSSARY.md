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
| `FieldEdgeKind` | Classification of a field lineage edge: DIRECT (`Passthrough`/`Access`/`Derive`) or INDIRECT (`Filter`/`GroupBy`/`JoinKey`/`Conditional`) | `pipeline_view/field_lineage.rs` | DIRECT, INDIRECT, `EdgeNature` | High |
| DIRECT edge | A field edge whose output VALUE is carried from or (re)derived from the input — `Passthrough`/`Access`/`Derive`. Rendered solid. | `pipeline_view/field_lineage.rs`, canvas | `EdgeNature::Direct` | High |
| INDIRECT edge | A field edge where the input only *influenced* which output rows/values exist (a filter, group-by, join key, or branch condition) without contributing a value — `Filter`/`GroupBy`/`JoinKey`/`Conditional` (#147). Rendered ghosted/dashed, collapsed until a field is selected. Mirrors OpenLineage INDIRECT subtypes. | `pipeline_view/field_lineage.rs`, canvas | `EdgeNature::Indirect`, OpenLineage | High |
| `EdgeNature` | DIRECT vs INDIRECT axis, derived purely from `FieldEdgeKind::nature()` (never stored, so illegal states like a Direct join key are unrepresentable) | `pipeline_view/field_lineage.rs` | `FieldEdgeKind` | High |
| `Precision` | Graded lineage-faithfulness tier (#148) carried on every `FieldEdge` and `FieldRow`, orthogonal to `FieldEdgeKind`: `Exact` / `Approximate` / `Unknown`. Surfaced as an Inspector per-field + per-hop badge and a selection/hover-gated hatched canvas node-corner. Replaced the binary `lineage_unavailable_reason`. | `pipeline_view/field_lineage.rs`, `components/inspector/model.rs`, canvas | `Exact`, `Approximate`, `Unknown` | High |
| Exact (precision) | Lineage is faithful: an identity `Passthrough`/`Access` carry, or a clean-CXL `Derive` whose support fully resolved with no `emit each` fan-out. | `pipeline_view/field_lineage.rs` | `Precision` | High |
| Approximate (precision) | Lineage is a sound over-approximation: any INDIRECT influence edge, a conservative CXL-less Merge/Combine fan-in carry, or an `emit each`-fanned derive (per-element provenance lost). | `pipeline_view/field_lineage.rs` | `Precision`, INDIRECT | High |
| Unknown (precision) | Lineage could not be computed: the node's CXL failed `parse_clean`, so its edges were suppressed. Lives on the `FieldRow` (no edge to annotate). | `pipeline_view/field_lineage.rs` | `Precision`, `parse_clean` | High |
| Aggregate grain | The post-Aggregate grouped-record correlation grain. Represented exactly once as the `GroupBy` INDIRECT edge from each group-key driver column (#147 retired the former `FieldRow::is_aggregate_grain` flag). | `pipeline_view.rs`, `components/inspector/model.rs` | `GroupBy`, INDIRECT | High |
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
