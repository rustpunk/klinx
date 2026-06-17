# AI Changelog

## Purpose

This file is lightweight architecture/change memory for future agents. It should record durable facts, major changes, and resolved uncertainty. Do not invent past decisions.

## 2026-06-15: Initial AI Onboarding Documentation

### Major Architecture Facts Discovered

- Klinx is a Rust 2024 workspace with two members: `crates/klinx` and `crates/klinx-git`.
- `crates/klinx` is a Dioxus 0.7 desktop IDE binary for Clinker YAML pipeline authoring.
- `crates/klinx-git` is a CLI-backed git abstraction crate with a `GitOps` trait; `gix_backend.rs` does not currently use gitoxide.
- Current Clinker engine crates are git-pinned in root `Cargo.toml` to rev `997ea7d`.
- `AppShell` owns Dioxus signals; inactive tabs store plain snapshots.
- YAML text is authoritative; node-preserving patching exists to avoid losing user-authored YAML and `nodes:`.
- The CI gauntlet uses fmt, two clippy passes, workspace tests, cargo deny, and Dioxus desktop build.
- There is no documented automated browser UI test target for the desktop webview.

### Open Question Routing

Current unresolved questions are tracked in `docs/ai/80_OPEN_QUESTIONS.md`. Keep this changelog focused on dated evidence, resolved uncertainty, and factual documentation maintenance history.
### Update Instructions

When architecture changes, append a dated entry with:

- What changed.
- Files/modules affected.
- Verification commands run.
- Any rules in `30_DESIGN_RULES.md` or local `AGENTS.md` that changed.
- Open questions resolved or newly discovered.

## 2026-06-16: GitHub Agent Snapshot And Batch Update Helper

- Added `scripts/gh-agent-snapshot.sh` as the repo-local entry point for compact GitHub agent workflow reads and structured bulk updates.
- The repo wrapper delegates to `~/.agents/skills/_shared/scripts/gh-agent-snapshot.sh` and defaults to repo `rustpunk/klinx`, Project owner `rustpunk`, and Project number `3`.
- GitHub workflow agents should prefer helper commands for queue, issue, Project status, closeout, and batch label/Project-field updates before falling back to repeated raw `gh` calls.
- Snapshot reads return visible ProjectV2 fields as both `projectItems[].fields` and typed `projectItems[].fieldValues[]` so agents should not perform follow-up GraphQL calls just to inspect Project metadata.
- Multi-issue workflows should use the bulk `issues --issues <n,n,n>` or `issues --file <file>` command instead of looping single-issue snapshots.
- Queue snapshots use issue-number discovery plus bounded bulk hydration to avoid GitHub GraphQL node-limit failures on milestone searches.
- Bulk updates preflight all Project fields/options before applying anything and support ProjectV2 single-select, text, date, and number fields.
- Readiness findings scan fetched comment text; compact mode still truncates emitted comment bodies unless `--full-comments` is passed.
- Verification: shell syntax check and helper dry-run commands were run during implementation.

## 2026-06-17: Layout Migration Compatibility Boundary

- Added an explicit canvas layout selection wrapper around the existing pipeline view model.
- `CanvasLayoutEngine::CurrentBarycenter` remains the default for plain `derive_pipeline_view` callers.
- `CanvasLayoutEngine::PortAwareSugiyama` applies the port-aware layout model's deterministic node positions while preserving the existing `PipelineView` connection and field-edge data for renderer compatibility; the visible canvas now requests this layout path.
- Port-aware requests fall back to the current barycenter view when stage, branch, or field anchors cannot be validated, returning `CanvasLayoutFallback` metadata instead of panicking or partially applying new coordinates.
- The open migration question is now narrowed to visual QA and the eventual default-switch policy.
- Verification: `CARGO_TARGET_DIR=<scratch-target> cargo test -p klinx pipeline_view`, `CARGO_TARGET_DIR=<scratch-target> cargo clippy -p klinx -- -D warnings`, `CARGO_TARGET_DIR=<scratch-target> cargo clippy -p klinx --all-targets -- -D warnings`, `cargo fmt --all --check`, and `git diff --check`.

## 2026-06-17: Port-Aware Connector Path Rendering

- Added optional `PipelineView::connection_paths` and `PipelineView::field_edge_paths` vectors, parallel to `connections` and `field_edges`.
- Existing barycenter-derived views leave those path vectors empty, preserving endpoint-derived connector fallback for callers that do not request the port-aware layout.
- `apply_canvas_layout(..., CanvasLayoutEngine::PortAwareSugiyama)` now carries the layout model's orthogonal lane paths into world-space canvas paths, including route/cull branch and field endpoint semantics.
- `components/canvas/connector.rs` can render rounded orthogonal polylines from layout-provided point lists; `components/canvas/panel.rs` now requests the port-aware layout for the visible canvas, then repopulates visible connector lane positions dynamically so hidden edges do not reserve channels.
- Node-level connectors and active field-lineage connectors now repopulate their channel lanes from the currently visible connector set, centered in clean free corridors and fanning outward as visible connector count increases. Node-level pipes use one transform-orange stroke and run a second lane-reservation pass across overlapping connector groups so unrelated visible pipes do not stack on the same channel. Skip-rank paths score candidate lanes against rendered node rectangles so an intermediate card is not selected as the channel.
- Connector routing now validates the complete orthogonal polyline, not only the vertical lane X: blocked horizontal legs detour through a bounded free-space grid around rendered card rectangles, and unrelated independent connectors are blocked from reusing full pipe segments unless they share an endpoint/trunk.
- During a pinned field-lineage reveal, hovering another participating field spotlights that field's direct connector neighbourhood while keeping the full pinned lineage visible.
- Connector SVG overlays remain pointer-passive and below opaque node cards; dimmed cards recede with filter styling rather than alpha so connector strokes cannot bleed through field interiors.
- `docs/ai/30_DESIGN_RULES.md` now records the optional path-vector contract.
- Verification: `cargo fmt --all --check`, `cargo test -p klinx connector`, `cargo test -p klinx canvas`, `cargo test -p klinx`, `cargo clippy -p klinx -- -D warnings`, `cargo clippy -p klinx --all-targets -- -D warnings`, and `git diff --check`.

## 2026-06-17: Source First-Use Port-Aware Layout Ranking

- `pipeline_view/layout_model.rs` now ranks source nodes immediately before their earliest consumer, then repairs downstream ranks so every edge still satisfies `rank[to] > rank[from]`.
- The port-aware layout path adds bounded local rank relaxation and two-sided predecessor/successor node-order sweeps with weighted semantic port scores. Node-level dataflow carries the highest rank/order weight, route/cull branch ports stay fixed in authored/default order, aggregate role ports remain input ports, and field ports remain the only reorderable row ports.
- Added `LayoutMetrics` for pure Rust layout checks: node/edge/rank counts, rank spans, skip-rank source edges, structural crossing estimates, route length, and card-overlap risk.
- Added regression coverage for `order_fulfillment.yaml`, `layout_benchmark_source_reuse.yaml`, and `layout_benchmark_order_lifecycle.yaml`; benchmark max rank spans now target `<= 1`, `<= 2`, and `<= 1` respectively while reducing source skip-rank edges.
- Verification: `CARGO_TARGET_DIR=<scratch-target> cargo test -p klinx layout_model -- --nocapture`, `CARGO_TARGET_DIR=<scratch-target> cargo test -p klinx pipeline_view`, `cargo fmt --all --check`, `git diff --check`, and `CARGO_TARGET_DIR=<scratch-target> cargo build --package klinx`.

## 2026-06-17: Aggregate Raw Field-Lineage Rows

- Raw `pipeline_view` field-lineage derivation now treats Aggregate nodes as grouped output records: de-duplicated `group_by` keys first, then aggregate emit targets.
- Aggregate group-key rows derive from matching input producer fields, including qualified group keys that resolve to a bare producer field for lineage.
- Aggregate emit rows derive from CXL expression support fields, but unrelated input fields no longer appear as aggregate passthrough rows.
- Invalid Aggregate CXL degrades to config-derived group-key rows without inferred field edges.
- Added `docs/research/2026-06-17-aggregate-field-lineage-ui.md` to record the UI rationale: group keys are first-class output rows and full field-edge rendering remains demand-revealed.

## 2026-06-16: Milestone Orchestration Workflow

- Added repo-local skill source `.agents/skills/gh-milestone-orchestration` for coordinating a GitHub milestone through planning, queue curation, one-issue implementation agents, review, closeout, and final milestone verification.
- Added `docs/ai/github-workflow/ORCHESTRATION.md` as the durable runbook for coordinator ownership, state model, claim protocol, dispatch prompt shape, stop conditions, and milestone exit gate.
- Added `.github/ISSUE_TEMPLATE/milestone-orchestration.yml` so a milestone coordinator issue can persist active slots, queue, blockers, and closeout state.
- Root guidance now points agents to the orchestration skill/runbook and keeps the maintainer merge gate explicit.

## 2026-06-16: Compiled Body Drill-In Field Rows

- `pipeline_view::derive_body_view` now attaches field rows to compiled composition-body drill-in nodes from `BoundBody::body_rows`, keyed by compiled body node name.
- Body field edges are conservative same-name passthrough carries between rendered body predecessors when both endpoint rows are available; missing row data leaves the body node at node-level connectors only.
- `StageView.id` continues to use the compiled `PlanNode` body node name, while `NodeIndex` remains internal to the compiled body graph.
- Verification: `CARGO_TARGET_DIR=/home/glitch/.cargo/tmp/klinx-issue-95-target cargo test -p klinx pipeline_view`.

## 2026-06-16: Port-Aware Layout Model Scaffold

- Added `pipeline_view/layout_model.rs` as a pure Rust graph model for future port-aware layered canvas layout.
- The model represents stage nodes, node-level ports, field-row ports, route/cull branch ports, directed edges, ranks/layers, ordered ports, and placeholder orthogonal connector paths.
- The visible canvas still uses the existing `layout_positions` barycenter geometry; `layout_model` is a migration boundary, not a renderer switch.
- Prior-art summary: existing research notes point toward a Rust Sugiyama-style layered pass with port-aware crossing minimization and orthogonal routing, avoiding a JS/elkjs dependency.
- Open question added for when and how to migrate the visible canvas to this model.
- Verification: `CARGO_TARGET_DIR=/home/glitch/.cargo/tmp/klinx-issue-100-target cargo test -p klinx layout_model`.

## 2026-06-16: Resolved Top-Level Pipeline Field Rows

- Added `pipeline_view::derive_resolved_pipeline_view` for top-level canvas Resolved mode.
- Resolved mode now uses `CompiledPlan::typed_output_row` as the field row/type source, filters engine-internal `$ck.*` rows, and only draws lineage edges whose endpoints exist in the resolved row set.
- Raw mode still uses `derive_pipeline_view` and the existing klinx-side field-lineage approximation.
- `components/canvas/panel.rs` now dispatches to the resolved derivation path when `ChannelViewMode::Resolved` has a compiled plan.
- Verification: `CARGO_TARGET_DIR=/home/glitch/.cargo/tmp/klinx-issue-99-target cargo test -p klinx resolved_pipeline_fields_use_compiled_output_row_types`.

## 2026-06-16: Wide-Schema Canvas Field Projection

- Canvas nodes now cap wide field lists by default and expose per-node header filtering plus a footer load-more control.
- The cap/filter projection is owned by `components/canvas/panel.rs` and applied before connector anchor resolution; `CanvasNode` renders the projected `StageView` it receives, so card height, branch placement, and visible field anchors stay aligned.
- Changing a node's field filter or cap state clears stale hover/pin lineage state for that node.
- Correlation-key fields highlight the existing field ports on marked rows; unmarked rows reserve no leading gutter and short field names stay visually clean.
- Verification: `cargo test -p klinx wide_schema_projection`, `cargo test -p klinx pipeline_view`, `cargo fmt --all --check`, `cargo clippy -p klinx -- -D warnings`, `cargo clippy -p klinx --all-targets -- -D warnings`, `cargo build --package klinx`, and headless canvas screenshot capture.

## 2026-06-16: Delayed Field Hover Reveal

- Canvas field lineage hover now uses a short cold-entry dwell before applying the first row hover, with a pending target and generation token so quick pointer sweeps do not flash lineage cables for every row crossed.
- Once a field reveal is active or recently warm, row-to-row field movement applies immediately; leaving the field area schedules a short delayed close and then a brief warm skip window.
- Plain node chrome hover no longer reveals field-level carry edges; only actual field-row hover or a pinned field can show field connectors.
- Removed the old node-carry hover helper from `pipeline_view::field_lineage` because the UI no longer exposes a node-scope field reveal.
- Verification: `cargo fmt --all --check`, `cargo build --package klinx`, `cargo test -p klinx wide_schema_projection`, `cargo clippy -p klinx -- -D warnings`, `cargo test -p klinx pipeline_view`, `cargo clippy -p klinx --all-targets -- -D warnings`, and `git diff --check`.

## 2026-06-16: Temporary Hidden Field Reveal And Global Field Search

- Field projection now accepts transient field names from active lineage hover/pin and global field search, appending only currently hidden matching rows at the bottom of their node without changing the node's normal cap/filter state.
- Hidden lineage endpoints become real temporary rows before connector anchor resolution, so hover/pin cables can resolve to visible field ports; if load-more makes the field normally visible, the temporary marker naturally disappears.
- Added a canvas toolbar global field search that highlights matches and temporarily reveals hidden matches without filtering normal node field lists.
- Per-node field filter and global field search both support `*` and `?` wildcard matching against field names, types, and kind labels.
- Verification: `cargo build --package klinx`, `cargo test -p klinx wide_schema_projection`, `cargo test -p klinx field_search_accepts_wildcards`, `cargo fmt --all --check`, `cargo clippy -p klinx -- -D warnings`, `cargo test -p klinx pipeline_view`, and `git diff --check`.

## 2026-06-17: Aggregate Group-By Role Ports

- Aggregate `group_by` keys now render as semantic input role rows above the Aggregate output fields. The grouped output record still renders as normal field rows: de-duplicated group keys first, then aggregate emit targets.
- Added `RoleEdge` and `StagePortRow` plumbing so a producer field can feed `group_by:<field>` and the normal Aggregate output field without drawing duplicate cables into the same row.
- The canvas hover/pin selection model now supports field endpoints and role-port endpoints; role edges temporarily reveal hidden producer fields and tint the target role row.
- The port-aware layout model includes Aggregate group-key role ports and exports `PipelineView::role_edge_paths` parallel to `role_edges`.
- Verification: `cargo fmt --all --check`, `cargo test -p klinx pipeline_view`, `cargo test -p klinx canvas`, `cargo test --workspace`, `cargo clippy --workspace -- -D warnings`, `cargo clippy --workspace --all-targets -- -D warnings`, and `git diff --check`.

## 2026-06-17: Adaptive Canvas Node Field Disclosure

- Canvas nodes now support global display modes Auto, Compact, Preview, Schema, and Full, plus per-node overrides cycled from the node header.
- `components/canvas/panel.rs` owns display-mode resolution, preview ranking, filters, load-more state, and temporary hidden-row reveal before connector anchor resolution. This keeps rendered rows, field anchors, branch ports, card heights, fit-to-view bounds, and hover/pin lineage connectors synchronized.
- Auto mode uses graph size, maximum schema width, zoom level, and active lineage/search state to reduce row detail for wide schemas and large 30+ node graphs while keeping small graphs schema-readable by default.
- Preview ranking prioritizes correlation keys, emitted or derived fields, operator-relevant fields, declared fields, and passthrough filler. Active lineage/search/pinned rows keep their normal visible position when already projected; hidden active endpoints append as temporary reveal rows so anchors resolve without reordering the list being scanned.
- Verification: `cargo test -p klinx components::canvas::panel`, `cargo test -p klinx pipeline_view`, `cargo test -p klinx field_lineage`, `cargo clippy --workspace -- -D warnings`, `cargo clippy --workspace --all-targets -- -D warnings`, and `git diff --check`.
