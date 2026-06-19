# AI Changelog

## Purpose

This file is lightweight architecture/change memory for future agents. It should record durable facts, major changes, and resolved uncertainty. Do not invent past decisions.

## 2026-06-19: Derive-Aware Composition Body Lineage (#174)

- **`body_field_edges` (`pipeline_view.rs`) is no longer carry-only.** It now mirrors the top-level resolved path (`resolved_pipeline_field_lineage`) in the body's slot space: a body node with a resolved CXL program runs the same `emit_supports` / `emit_copy_targets` / `emit_each_fanned_targets` analysis and emits `Derive`/carry edges, so a COMPUTED body column (`emit c = a + 1`) draws a `Derive` cable to the producer column it is computed from, and a carried-and-accessed column draws an `Access` carry rather than a pure `Passthrough`. The per-consumer `producers_of`/`input_cols` fold is the same one `compute_field_lineage` uses, built from predecessor stages' fields in the body's existing ordered-predecessor resolution (`body.node_input_refs`, slot space). Edges go through `EdgeAccumulator`.
- **New helper `body_node_program(body, node_name) -> Option<&cxl::ast::Program>`.** Resolves `body.graph[body.name_to_idx[node_name]]` and returns `&payload.typed.program` for a `PlanNode::Transform { resolved: Some(_), .. }`. Reuses the in-process-compiled `TypedProgram.program` (`cxl::ast::Program`) the top-level path already consumes — NO `.comp.yaml` re-parse, NO TypedProgram→Program adapter. Returns `None` for every other node: in clinker-plan `997ea7d` only `Transform` carries a CXL `TypedProgram` on the plan node (`Aggregation` carries a `CompiledAggregate`; `Combine` keeps its typed `where`/`body` programs in `CompileArtifacts`, not on the node), and Source/Route/Output/Merge/Cull/Reshape/Envelope/nested-Composition carry no program. The outer `resolved` field is `#[serde(skip)]` + `Option`, so a deserialized plan degrades to the carry fallback rather than panicking; an in-process compile always populates it.
- **No-program fallback preserves nested-composition correctness.** A node whose helper returns `None` keeps the original best-effort same-name passthrough carry. A nested `PlanNode::Composition`'s output columns surface from its body rows, so the same-name carries into its consumers still draw — exactly as the top-level resolved path skips a Composition's own emit analysis.
- **The Inspector descent and drill-in canvas needed no change.** The in-body BFS rides `scope.view.field_edges` (built via `derive_body_scope` → `build_body_view` → `body_field_edges`), so it now follows derive edges automatically — a computed body column traces to its true in-body origin and resurfaces to the outer source, including across NESTED boundaries (depth 2 proven by test). `derive_body_view` shares `build_body_view`, so the drill-in canvas gets derive cables for free. Body derive lineage stays Resolved-mode-only (the raw/boundary path is unchanged). Coverage is Transform-only: a column computed by a body `Aggregation`/`Combine` still dead-ends (those carry no CXL program on the plan node — only `Transform` does), and indirect (Filter/GroupBy/JoinKey/Conditional) body influence edges remain out of scope. Both are follow-ups, not regressions. Multi-producer body fan-in also stays `Exact`/`Passthrough` rather than the top-level `conservative_fan_in`/`Approximate` (no current fixture exercises it; tracked separately).
- Verification: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test -p klinx pipeline_view` (136), `cargo test -p klinx inspector` (40), `cargo test -p klinx field_lineage` (40), `cargo test --workspace` (378, green).

## 2026-06-18: Whole-path Lineage Ribbon + Focus Mode + Dual Value/Influence (#152, lineage-v2 PR5)

- **Ribbon overlay (render-only, no reflow).** Selecting a field lights its full transitive lineage as a canvas overlay built from the existing reveal closure (`field_lineage_full`) and the precomputed `field_edge_paths` — it reads laid-out outputs (`canvas_x/canvas_y`, `*_paths`) by reference and writes NONE of them, so the port-aware Sugiyama layout never reflows on selection. `resolve_edge_anchors` takes the layout by `&`. Guarded by `selection_is_render_only_and_never_reflows_layout` (snapshots positions + all `*_paths`, runs the closure walk + anchor resolution, asserts byte-for-byte equality, with a non-vacuity guard that real cables were produced).
- **Dual value/influence ribbons.** Two independent canvas-local toggles `show_value_ribbon` / `show_influence_ribbon` (default ON) gate which ribbon the overlay DRAWS, via the pure predicate `ribbon_edge_visible(nature, value_on, influence_on)` keyed on `FieldEdgeKind::nature()` (#147). They gate only the drawn cables — the dim/focus closure (which cards dim) is computed from the UNfiltered closure, so hiding a ribbon never un-dims an off-path card. Unlike `hop_cap` (a per-graph bound the view-swap effect resets), these are view PREFERENCES and intentionally PERSIST across tab swaps (like the reveal mode). The two overlay loops consume `active_*_edges` by value (`into_iter().filter()`), so a cable's `kind_attr`/`path` MOVE into the connector rather than cloning every render.
- **Ribbon styling = nature + precision (the cascade rule).** `field_edge_classes(kind, precision, spotlight)`: DIRECT → `--value` (solid, continuous), INDIRECT → `--indirect` (ghosted halo, dash `3 5`, opacity 0.45). The precision hatch `--approximate` (tight dash `2 3`) is emitted **only on DIRECT (value) edges** — every INDIRECT edge is `Approximate` by construction (`FieldEdge::influence`), so `--indirect` already conveys it, and emitting `--approximate` there would (being declared later with equal CSS specificity) OVERRIDE the influence-halo dash/opacity. So the approximate hatch is a value-ribbon-only marker distinguishing an Approximate DIRECT carry/derive from an Exact one. Guarded by a connector test asserting every INDIRECT kind carries `--indirect` and NOT `--approximate`.
- **Focus mode = #123 Highlight, persistent.** A field click sets both `pinned_field` and `selected_field`, so a selection populates the same closure a hover would → off-path cards dim persistently in Highlight mode without an active hover. No parallel dim mechanism; Filter mode (hide off-path) stays distinct. Role-port cables are graded `Approximate` (an influence input; `RoleEdge` carries no precision tier) and, being `GroupBy` (INDIRECT), render as the influence halo.

## 2026-06-18: Highlight/Filter Lineage Reveal Modes + Hop Caps (#123, lineage-v2 PR4)

- **Mode state.** New `LineageRevealMode { Highlight (default), Filter }` (`state.rs`) with `as_data_attr`/`label`/`toggled` helpers, stored as `AppState.lineage_reveal_mode: Signal<LineageRevealMode>`. It lives on `AppState` (constructed once in `app.rs`) so the canvas reveal logic AND a toolbar toggle share one source of truth; it is a UI PREFERENCE that persists across tab switches (like `channel_view_mode`), NOT per-tab state cleared on switch. PR5 (#152) reuses this exact signal for its persistent focus toggle — design intent, not incidental.
- **Highlight = today's behavior, now gated.** When `mode == Highlight` the reveal is the pre-#123 effect for every pipeline: the FULL uncapped closure, off-path cards get `--dimmed` (filter-recede, opaque), node cables `--recede`. The hop cap never applies in Highlight, so deep pipelines are not clipped in the default mode. Locked by the existing `dimmed_node_css_keeps_card_opaque` test plus a new `filter_hidden_node_css_removes_card_from_layout` asserting the dim rule never uses `display:none`.
- **Filter mode.** When `mode == Filter` and a reveal is active, off-path cards are HIDDEN (`.klinx-node--hidden { display:none }`, a new `hidden` prop on `CanvasNode`) instead of dimmed, and a node-level connector is drawn ONLY when BOTH endpoints survive the keep-set — so no half-edge dangles to a gone card. The keep-set is the tested helper `lineage_keep_nodes(participating_edges, anchor)` = anchor ∪ endpoints-of-resolved-edges; because the closure walks full up+down paths, participating nodes already form connected paths, so endpoints-of-participating-edges suffices to keep every connecting-path midpoint (asserted by `filter_keep_set_retains_connecting_path_midpoint`). Computed only when Filter is actually suppressing a reveal, so Highlight pays nothing. To carry node indices to the connector draw, the render `connections` tuple became a named `CanvasConnection` struct (also resolves a clippy `type_complexity`).
- **Hop caps (deterministic, helper-level) — FILTER-mode only.** `field_lineage_full_capped(edges, node, field, hop_cap: Option<usize>)` in `field_lineage.rs`; `field_lineage_full` delegates with `None` (callers unchanged + it stays the public uncapped entry). The walk is BREADTH-FIRST PER DEPTH LAYER (a `next_frontier` per layer) so `hop_cap = Some(n)` stops at an exact edge depth deterministically; `Some(0)` = empty, larger caps are strict supersets (monotonic "expand further"). The walk stays KIND-AGNOSTIC (the canvas value/influence toggle is still deferred to PR5) — `indirect_edge_endpoints_are_revealed_by_closure_and_full_walk` unchanged and green. The cap is applied to the selected/pinned reveal ONLY in Filter mode (`closure_cap = if is_filter_mode { Some(active_hop_cap) } else { None }`); Highlight uses the FULL uncapped closure so it preserves the pre-#123 selected-field reveal for EVERY pipeline, not just ones shallower than the cap (the "#123: Highlight preserves current behavior" acceptance criterion, honored at any depth). Hover stays 1-hop and is never capped.
- **Filter-mode default guard.** `DEFAULT_HOP_CAP = 16` sits ABOVE the deepest closure in any bundled example (measured: 7), so a default-cap Filter reveal equals the full closure on every bundled pipeline (no example silently hidden; EXPAND+ does not spuriously appear). `default_hop_cap_does_not_clip_example_pipelines` (panel.rs) iterates every field of every example asserting `capped == full` — a deeper future example fails loudly. Highlight is unconditionally uncapped, so it is depth-independent and needs no such guard.
- **Expand-further affordance.** A canvas-local `use_signal(DEFAULT_HOP_CAP)` hop cap; an `EXPAND +` toolbar button raises it by `HOP_CAP_STEP = 8`. It is shown ONLY in Filter mode when a pinned Field OR RolePort closure is clipped (its uncapped walk strictly exceeds the already-computed capped `closure`) — so a clipped role-port reveal is recoverable, not a silent dead-end, and the uncapped walk runs only in that Filter+pinned case (Highlight and idle hovers pay nothing). The view-swap effect resets the cap to `DEFAULT_HOP_CAP` so a raised cap never carries into a different graph; the reveal MODE is intentionally NOT reset (it is a persistent preference).
- **State hygiene.** The existing view-swap `use_effect` (panel.rs) already clears hover + pinned + `selected_field` on graph/tab/drill/channel-mode swap; the hop-cap reset was added alongside. The `app.rs` tab-switch path independently clears `selected_field`. Both confirmed, so a mode/reveal never points at a different graph's node.
- Verification: `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test -p klinx field_lineage`/`pipeline_view`/`-p klinx` (308 pass), `cargo build --package klinx`.

## 2026-06-18: Hop-by-hop Expandable Lineage Trace Tree (#153, lineage-v2 PR3)

- The Inspector's field LINEAGE now renders an **expandable hop-by-hop TREE** instead of two flat, hop-sorted lists. `inspector/model.rs` replaced `trace_endpoints` (which flattened the BFS into a sorted `Vec<TraceEndpointView>`) with `trace_tree`, which records each discovered endpoint's parent and assembles a `Vec<TraceNode>` forest (`TraceNode { endpoint, cxl_mentions, children }`). The selected field is the implicit root (hop 0); the top-level `Vec` is its direct (hop-1) children. The global `(node, field)` BFS dedup means every endpoint is discovered once, so the discovery relation is a clean spanning tree. Sibling order is the former tie-break (stage label, then field name). `FieldInspectorModel::upstream`/`downstream` changed from `Vec<TraceEndpointView>` to `Vec<TraceNode>`.
- **Per-hop transform attribution (plan decision A):** the baseline is the edge-kind label + precision badge already carried per node — always present. ENRICHMENT: `build_field_detail` walks the assembled trees and attaches each hop's responsible CXL statement(s) via `generate_stage_doc(config, hop_stage_id)` → `cxl_mentions_for_field`, cached per `stage_id` (`StageDocCache`) so a stage is parsed once across all its hops. Stages with no CXL analysis (Route/Aggregate/Merge/Source) attach nothing — the edge kind/precision is the attribution there. No new edge-level statement plumbing.
- **INDIRECT include/exclude toggle (the deferred PR3 marker), SCOPED to the Inspector tree.** `trace_tree` takes `include_indirect: bool`. `build_field_detail` builds BOTH a full pair (`upstream`/`downstream`, `include_indirect = true`) and a direct-only pair (`upstream_direct`/`downstream_direct`, `include_indirect = false`); the panel's `INDIRECT` header toggle SELECTS between the two precomputed trees, keeping the model free of UI state. Building the direct-only tree with the BFS flag (rather than pruning the full tree by each hop's `EdgeNature`) is load-bearing for correctness: a dual-role column — reached by BOTH a DIRECT carry and an INDIRECT influence, e.g. a Combine join key — is tagged INDIRECT in the full tree by the worst-precision dedup (#148), so pruning it would drop a real value hop; the `include_indirect = false` walk instead reaches it via the surviving carry edge and keeps it tagged DIRECT. The **canvas** walk toggle remains deferred to PR5.
- **Render (panel.rs):** the flat `TraceList` became `TraceTree` + `TraceTreeRow`, reusing the file-explorer expand pattern — a `use_signal(HashSet<TraceKey>)` of expanded keys, a `flatten_trace` depth-walk emitting visible rows, and a `▾`/`▸` caret per expandable row. Expansion is keyed by the STABLE `(stage_id, field_name, hop)` tuple, not vec index, so a branch's state survives re-renders. Default: hop-1 expanded, deeper collapsed. Click-to-select (`to_selected_field`), `data-stage-kind`, edge-kind badge, precision badge, and the empty-states are preserved; the `klinx-field-lineage-summary` counts every tree node (`count_trace_nodes`). New CSS under `klinx-field-trace-*` (caret column, depth indent, per-hop CXL line, lineage header + INDIRECT toggle).
- The canvas closure/full-walk toggle stays deferred to PR5; `lineage_closure`/`field_lineage_full` remain KIND-AGNOSTIC and the `indirect_edge_endpoints_are_revealed_by_closure_and_full_walk` guard test is unchanged and green.
- Verification: `cargo fmt --all`, `cargo clippy --workspace -- -D warnings`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test -p klinx inspector`/`field_lineage`/`pipeline_view`, `cargo build --package klinx`.

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

## 2026-06-18: Precision Tiers On Rows & Edges (#148, lineage-v2 PR2)

- Added `pub enum Precision { Exact, Approximate, Unknown }` (`pipeline_view/field_lineage.rs`) — a graded lineage-faithfulness tier carried ALONGSIDE `FieldEdgeKind`, never folded into it (a `Derive` edge can be Exact or Approximate). `Default` is `Exact`. `precision_label`/`precision_attr` mirror the `edge_kind_*` helpers (slug-safe, guarded by `precision_attr_is_always_slug_safe`). `Precision::worst` folds two tiers keeping the less-precise (`Unknown` > `Approximate` > `Exact`).
- `FieldEdge` gained `precision: Precision` + `precision_reason: String`; `FieldRow` gained `lineage_precision: Precision` + `precision_reason: String`. **Precision is EXCLUDED from edge identity**: `FieldEdge` has a hand-written `PartialEq` over only the 5-tuple `(from_node, from_field, to_node, to_field, kind)`, matching the `EdgeAccumulator` dedup key, so `==`/`.contains` and dedup stay in lock-step and two otherwise-identical edges never split on a reason string. `FieldEdgeKind` got a `#[default] Passthrough` so `FieldEdge` derives `Default`, letting test literals elide the new fields via `..Default::default()`.
- Classification funnels through four `FieldEdge` constructors (the single classifier both lineage paths use): `carry` (Passthrough/Access → Exact), `derive(.., fanned)` (Exact, or Approximate when fanned by `emit each`), `influence` (any INDIRECT kind → Approximate), `conservative_fan_in` (CXL-less Merge join-key fan-in → Approximate). `emit_each_fanned_targets` reports which emit targets sit inside an `emit each`/`explode outer` body (independent of the source's column support — the fan-out itself is the imprecision). A node whose CXL fails `parse_clean` has its rows marked `Unknown` via `mark_rows_unknown` (no edge to annotate). `derive_row_precision` is a post-pass folding each row's precision from the edges PRODUCING it (the `to` endpoint), defaulting Exact — so a clean producer that merely feeds a downstream filter stays Exact while the downstream consumer row degrades.
- Two PR1-deferred cleanups folded in: (a) the duplicated Aggregate GroupBy emission loop extracted into the shared `emit_group_by_edges` helper used by both `compute_field_lineage` and `resolved_pipeline_field_lineage`; (b) `EdgeAccumulator` gained a `push_direct` method for can't-collide DIRECT edges, so the dedup-bypass contract is expressed by the type rather than direct `edges` field pokes — every production edge now flows through `push_direct`/`push_deduped`.
- Inspector (`components/inspector/model.rs` + `panel.rs`): **replaced** `FieldInspectorModel::lineage_unavailable_reason: Option<String>` with `lineage_precision` + `precision_reason` + `lineage_empty`. The original empty-state message ("No field-level lineage edges mention this field in the current view.") is PRESERVED verbatim, folded into the edgeless presentation (`lineage_empty`). The field precision is the worst of the row's own precision and its incident trace-hop precisions. `TraceEndpointView` carries `precision_label`/`precision_attr` per hop (alongside `edge_kind_*`). Per-field badge in the FIELD summary, per-hop badge in the trace rows, a quieter degraded-precision note in the LINEAGE section.
- Canvas (`components/canvas/node.rs` + `assets/klinx.css`): a subtle hatched node-corner (`.klinx-node-precision-corner`, `data-precision` hue) conveys the node's WORST field precision, hidden by default and revealed ONLY on selection/hover via CSS (`.klinx-node:hover`/`.klinx-node--selected`) — no always-on overlay, avoiding badge fatigue. Computed once from `stage.fields`, so it adds no signal subscription.
- Verification: `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo deny check` — all pass.

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

## 2026-06-17: Aggregate Failure-Grain Field Markers

- Aggregate `group_by` output fields now carry a separate `FieldRow::is_aggregate_grain` marker. This lets the canvas style fields like `invoice_date` as part of the post-Aggregate failure grain without labeling them as source-declared `correlation_key` fields.
- Aggregate group-key role ports now render under one shared `GROUP_BY` section label while preserving one hidden/semantic role-port row per group key for connector anchors.
- The marker propagates through unchanged passthrough rows after the Aggregate, and preview projection prioritizes aggregate-grain rows with source CK rows so the grouped failure grain remains visible on dense nodes.
- Normal canvas rows still filter hidden `$ck.*` engine fields; `$ck.aggregate.<name>` remains an internal bridge represented by the visible group-key fields and tooltips rather than a normal business column.
- Verification: `cargo test -p klinx field_lineage`, `cargo test -p klinx pipeline_view`, `cargo test -p klinx components::canvas::panel`, `cargo fmt --all --check`, `git diff --check`, both workspace clippy passes, `cargo build --package klinx`, and a headless screenshot smoke check against the example workspace.

## 2026-06-17: Selected-Item Inspector And Scoped YAML Editing

- `SelectedInspector` replaces the separate node and field inspector surfaces. Field-row clicks now select a field at app-state level, keep its transitive lineage pinned on the canvas, and render field details through the same inspector shell used for selected nodes.
- `components/inspector/model.rs` derives selected-node and selected-field view models outside RSX from the current pipeline view, parsed config, autodoc, notes, visible parse errors, schema warnings, channel mode, and compiled-plan availability.
- Node details now focus on explanation/debugging: overview/status, topology, logic, output fields, branch/default ports, Aggregate role ports, CXL reads/writes when autodoc can infer them, contract/channel facts, diagnostics, and explicit unavailable reasons.
- Field details keep kind/type/badges/transitive upstream/downstream/role-use information and add stage context, annotations, emitted/passthrough/declared explanations, CXL statement mentions when available, and lineage-unavailable reasons.
- `ScopedYamlEditor` replaces the read-only scoped YAML panel. It derives node ranges from every `PipelineNode` in `config.nodes`, splices node-scoped drafts back into `yaml_text` under `EditSource::Yaml`, and keeps a last-known range so temporary invalid scoped YAML does not erase the inspector.
- Verification: `cargo test -p klinx inspector`, `cargo test -p klinx sync`, `cargo test -p klinx pipeline_view`, `cargo fmt --all --check`, and `cargo clippy -p klinx -- -D warnings`.

## 2026-06-17: Re-established Live CXL Syntax Validation In The Inspector

- Restored `crates/klinx/src/cxl_bridge.rs` (the parser-only `validate_expr` adapter over `cxl::parser::Parser::parse`) that #139 deleted, and re-added `mod cxl_bridge;` to `main.rs`. `CxlDiagnostic` was trimmed to the fields the inspector consumes (`message`, `how_to_fix`); the pre-#139 byte-span (`start`/`end`) and single-variant `DiagSeverity` fields were dropped because no caller reads them (they served the deleted `cxl_input` inline span highlighting). The pinned cxl `997ea7d` `ParseError` fields (`message`, `how_to_fix`) match, so no field-access adaptation was needed.
- `components/inspector/model.rs` now validates a node's `cxl:` block at edit time: `node_diagnostics` emits a `"cxl"` Error diagnostic per parse error (flipping the node status chip off `ok`), and `cxl_section` prepends CXL-section error rows. Messages append ` → {how_to_fix}` when the parser supplies a fix.
- This re-establishes the invariant regressed by #139: a structurally-valid pipeline whose CXL is malformed (e.g. `emit x =`) is flagged at edit time instead of rendering as `ok` (#141). See the new rule under "YAML And Pipeline Rules" in `30_DESIGN_RULES.md`.
- Verification: `cargo fmt --all`, `cargo build --package klinx`, `cargo test --package klinx`.

## 2026-06-18: INDIRECT Influence Field-Edge Class — review fixes (#147, lineage-v2 PR1)

- **Merge no longer fabricates a join key.** A Merge is a streamwise row UNION (`MergeMode::Concat`/`Interleave`, verified in clinker-plan `997ea7d` `config/pipeline_node.rs`) — it stacks rows and never aligns them — so it now emits NO `JoinKey` edge. Removed `emit_fan_in_join_key_edges` and `node_is_fan_in` (and the emit-set recompute they needed) from `pipeline_view.rs`.
- **Combine join key now comes from `where_expr`, not a name-collision carry heuristic.** `node_influence_predicates` gained a `Combine` arm returning `(config.where_expr.as_ref(), FieldEdgeKind::JoinKey)`, so the join key flows through the SAME `predicate_support` path as Cull `Filter` / Route `Conditional`. This captures a join whose two sides differ in name (`left.k1 == right.k2`) — which the old shared-name heuristic missed — and stops false-tagging an unrelated column present under the same name on both inputs. The Combine's own `emit` body keeps flowing through the DIRECT path unchanged.
- `emitted_field_names` (`field_lineage.rs`) reverted to non-`pub` (it was only public for the deleted fan-in block; nothing outside the module uses it now).
- **`push_field_edge_deduped` dedup is now O(1)** via a `HashSet<EdgeKey>` (`EdgeKey = (from_node, from_field, to_node, to_field, kind)`) maintained alongside each builder's `field_edges`, replacing the `Vec::contains` O(n) rescan inside the predicate fan triple-loop. Emitted edge order and dedup semantics are unchanged. Both builders (`compute_field_lineage`, `resolved_pipeline_field_lineage`) thread the `seen` set through.
- `edge_kind_attr` (`inspector/model.rs`) collapsed from a 7-arm duplicate of `edge_kind_label` to a 2-arm hyphen override (`group by`→`group-by`, `join key`→`join-key`) delegating to the label for the rest, so the two cannot drift.
- Behavioral tests added for the central mechanisms: connector `--indirect` class is gated on `nature()` (`field_edge_classes` helper extracted and asserted); layout maps Filter/JoinKey/Conditional → zero-weight `IndirectField` and GroupBy → weighted `Field` (with a code comment that GroupBy keeps `Field` weight intentionally despite `nature()==Indirect`); the closure walks (`lineage_closure`/`field_lineage_full`) reveal INDIRECT-edge endpoints so they are never permanently invisible; and a raw/resolved parity test locks a Cull's `Filter` edge into BOTH lineage paths. Merge/Combine lineage tests reworked: Merge yields zero JoinKey, Combine drives JoinKey from `where_expr` (same-name and different-name fixtures).
- INDIRECT include/exclude toggle: the **Inspector trace tree** half landed in PR3 (#153) — `trace_tree` takes an `include_indirect` flag, the model precomputes a full and a direct-only tree pair, and the LINEAGE panel toggle selects between them. The **canvas** closure/full-walk toggle (`field_lineage_full`, `lineage_closure`) is deferred to PR5 (the dual value/influence ribbon); those walks stay KIND-AGNOSTIC so an INDIRECT edge is never permanently invisible on the canvas.
- The `predicate_support` doc and its test dropped a false "let-chain resolution" claim — a predicate is structurally a single expression and cannot carry a statement-level `let`; the test was renamed to `predicate_support_single_column_resolves_to_that_column`.

## 2026-06-17: INDIRECT Influence Field-Edge Class (#147, lineage-v2 PR1)

- `FieldEdgeKind` (`pipeline_view/field_lineage.rs`) widened from the three DIRECT kinds (`Passthrough | Access | Derive`) with four INDIRECT *influence* kinds: `Filter` (Cull removal predicate), `GroupBy` (Aggregate group-by key), `JoinKey` (Combine `where_expr` join predicate), `Conditional` (Route branch condition). These model OpenLineage `ColumnLineageDatasetFacet` INDIRECT subtypes — a column that influenced *which* output rows exist (or how they group/join/branch) without contributing any output value.
- Added `pub enum EdgeNature { Direct, Indirect }` and `FieldEdgeKind::nature(&self) -> EdgeNature` as a pure, total match. **Nature is derived from kind, never stored** — a separate field would permit illegal states (e.g. a Direct join key). The renderer's `--indirect` ghosting and the layout's zero-weight overlay treatment both read `nature()`, so it is the single value/influence source of truth.
- INDIRECT edges are emitted in BOTH lineage paths — the conservative `compute_field_lineage` and the schema-resolved `resolved_pipeline_field_lineage` — via shared helpers (`emit_indirect_influence_edges`, `predicate_support`). Each node still emits its DIRECT rows/carries, then ADDS INDIRECT edges. Edge-target policy: a predicate's support producer connects to EVERY surviving output row of the node (deduped on the full `(from,from_field,to,to_field,kind)` tuple), since a predicate influences which rows survive. JoinKey is derived from a Combine's `where_expr` through the SAME `predicate_support` path as Filter/Conditional, so a join key whose two sides differ in name (`left.k1 == right.k2`) is captured and an unrelated same-named column is not. Cull predicates live under `CullBody.rules[].drop_group_when` (a list, OR-combined), Route conditions under `RouteBody.conditions` (`IndexMap<name, CxlSource>`), and a Combine's join condition is `CombineBody.where_expr`. A Merge is a streamwise row UNION (`MergeMode::Concat`/`Interleave`) — it stacks rows and performs no join, so it emits NO `JoinKey` edge. Per-input value provenance for other Combine columns stays out of scope (#67).
- `predicate_support(expr) -> Option<HashSet<String>>` wraps a predicate string as a synthetic `emit __pred = {expr}` and reuses `parse_clean` + `emit_supports`; returns `None` on parse failure (never infer lineage from unparseable CXL — the module-wide degrade-gracefully rule), an empty set for a constant predicate.
- **Retired `FieldRow::is_aggregate_grain`.** The post-Aggregate grain is now represented exactly once — as the `GroupBy` edge — instead of also as a row flag (#147 acceptance). All readers were migrated: the canvas row marker was removed (the grain shows via its revealed GroupBy cable), the inspector "aggregate grain" badge derives from an incident GroupBy edge (`is_group_by_grain`), and canvas preview-rank/search read a `GroupBy`-edge-derived `FieldRankSignals::aggregate_grain_by_node` set. The dead `.klinx-node-field--aggregate-grain` CSS and its anchor variables were removed.
- Render (`components/canvas/connector.rs` + `assets/klinx.css`): DIRECT edges stay solid; INDIRECT edges read ghosted + finer-dashed via a `--indirect` modifier plus a per-kind hue class (`--filter`/`--groupby`/`--joinkey`/`--conditional`). INDIRECT edges stay collapsed-by-default — they only draw when a field is selected, reusing the existing hover/pin reveal gating (the closure walks include all kinds). The layout model maps Filter/JoinKey/Conditional to a zero-weight `LayoutEdgeKind::IndirectField` so influence overlays route a cable without distorting node ranks; `GroupBy` stays an ordinary weighted `Field` edge (it replaced the group-key `Derive` one-for-one, preserving tuned layouts).
- Verification: `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo deny check` — all pass.
