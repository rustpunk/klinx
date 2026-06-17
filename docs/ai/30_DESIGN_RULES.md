# Design Rules

## Core Philosophy

- **Verified:** Keep Klinx as a desktop-first Dioxus app for Clinker YAML authoring. Evidence: `README.md`, `crates/klinx/src/main.rs`, `Dioxus.toml`.
- **Verified:** Treat current manifests as dependency source of truth. Evidence: root `Cargo.toml` pins Clinker crates to `997ea7d`; older README prose names stale crates/rev.
- **Strong inference:** Prefer preserving user-authored YAML text and comments over regenerating documents from parsed models. Evidence: `yaml_patch.rs` tests and AppShell/tab comments.

## Dependency Direction

- **Verified:** `crates/klinx` may depend on `klinx-git`; `klinx-git` must remain independent of the Dioxus app.
- **Verified:** Engine behavior comes from git-pinned Clinker crates; do not vendor engine code.
- **Verified:** UI components should consume shared app state and view models, not independently parse or derive pipeline graphs when existing helpers exist.

## Public API Rules

- **Verified:** Use `GitOps` as the git operation boundary.
- **Verified:** Use `pipeline_view` APIs to derive canvas-ready data.
- **Strong inference:** Keep public app APIs small; most app internals are crate-local modules in the binary crate.
- Future git backend direction is tracked in `docs/ai/80_OPEN_QUESTIONS.md`; keep the current `GitOps` trait surface stable unless that decision changes.

## Error Handling Rules

- **Verified:** `klinx-git` returns `Result<T, GitError>`.
- **Verified:** File operations and CXL/YAML UI adapters return user-facing diagnostics or string errors.
- **Strong inference:** Expected user input failures should be displayed, not panicked.
- **Verified:** Test fixtures may use `unwrap`, `expect`, and `panic!` to assert invariants.

## State, Ownership, And Concurrency

- **Verified:** `AppShell` owns Dioxus signals; child components consume contexts.
- **Verified:** Do not move hooks into conditionals or event handlers.
- **Verified:** Per-tab state lives as plain snapshots in `TabEntry`; active tab signals are swapped through `AppShell`.
- **Verified:** `EditSource` is load-bearing for avoiding parse/sync loops.
- **Verified:** Canvas drag uses non-reactive state to avoid pointer-move re-render churn.

## YAML And Pipeline Rules

- **Verified:** YAML text is authoritative for saving.
- **Verified:** Use `patch_yaml_preserving_nodes` for normal inspector edits; full serialization is fallback behavior.
- **Verified:** Composition YAML is detected by root `_compose:`.
- **Verified:** `PipelineNode` variant matches should stay exhaustive; do not add broad wildcard arms around engine variants.
- **Verified:** Field lineage should use clean CXL parses; parse errors render diagnostics/rows but should not drive inferred edges.
- **Verified:** Raw Aggregate field-lineage rows are the grouped output shape: de-duplicated `group_by` keys first, then aggregate emit targets. Aggregate `group_by` keys also render as semantic input role ports above the output fields. With clean CXL, Aggregate group keys and emits derive from their source/support fields, while unrelated input fields do not pass through. Evidence: `pipeline_view` aggregate field-lineage tests and `docs/research/2026-06-17-aggregate-field-lineage-ui.md`.
- **Verified:** Route nodes expose branch ports, including default branches, rather than generic edge labels.

## UI And CSS Rules

- **Verified:** CSS class names and data attributes are part of layout behavior: `data-theme`, `data-layout`, `data-context`, and `klinx-*`.
- **Verified:** YAML overlay text must stay byte-aligned with the textarea; update `LINE_HEIGHT` in code if CSS line height changes.
- **Verified:** Canvas geometry and CSS must stay aligned with `pipeline_view` constants, node card heights, field anchors, and SVG connectors.
- **Verified:** Field connector colors are CSS-class driven; do not inline strokes casually.
- **Verified:** The visible canvas requests `CanvasLayoutEngine::PortAwareSugiyama` and falls back to the current barycenter view when stage, branch, or field anchors cannot be validated.
- **Verified:** `PipelineView::connection_paths`, `PipelineView::field_edge_paths`, and `PipelineView::role_edge_paths` are optional, renderer-ready routes parallel to the edge vectors. They are empty for barycenter fallback views and populated by the port-aware layout path; individual empty entries fall back to endpoint-derived routing. The visible canvas intentionally repopulates lane X positions from the currently visible connector set so filtered/active subsets stay centered and do not reserve lanes for hidden edges.
- **Verified:** Visible connector channels are populated from the connector set currently being rendered, not every possible field edge. A one-connector set occupies the clean channel center; larger visible sets fan outward from that center. Node-level connector pipes use the uniform transform-orange stroke, then reserve independent visible lanes across overlapping connector groups so unrelated pipes do not visually stack. Skip-rank connectors must route every segment around rendered node rectangles; when the preferred lane would send a horizontal leg through an intermediate card, the connector adds a bounded orthogonal detour above or below that card. Unrelated independent connectors must not reuse full polyline segments; shared segments are reserved for connectors with a real shared endpoint/trunk. Connector overlays stay pointer-passive and below opaque node cards so nodes remain readable and clickable.
- **Strong inference:** UI changes need manual/headless desktop visual review when layout or interaction changes.

## Testing Rules

- **Verified:** Run both clippy passes when claiming CI parity. The first omits `--all-targets` intentionally for dead-code coverage.
- **Verified:** There is no automated Playwright/web UI test target documented.
- **Verified:** Use focused cargo tests for changed modules when possible, then broader workspace tests for higher-risk changes.
- **Strong inference:** Example pipelines are regression fixtures for engine compatibility.

## Performance Rules

- **Verified:** YAML parse/tokenize paths are typing-latency sensitive.
- **Verified:** `perf-trace` is the opt-in timing feature for parse/tokenize tracing.
- **Verified:** Do not remove parse debounce or visible error settle behavior without measuring.
- **Strong inference:** Avoid adding filesystem or git CLI work to hot render paths.

## Documentation Rules

- **Verified:** Update `docs/ai` when architecture, commands, invariants, or open questions change.
- **Strong inference:** Keep root and local `AGENTS.md` concise; move detailed reasoning to `docs/ai`.
- **Verified:** Do not invent past decisions. Mark weak claims as Hypothesis or Open question.

## Never Do This Unless Explicitly Approved

- Add dependencies or edit lockfiles.
- Bump Dioxus or Clinker git pins.
- Vendor Clinker engine code.
- Replace node-preserving YAML patching with full serialization for normal editor saves.
- Remove either CI clippy pass.
- Assume a browser automation UI test exists.
- Push or commit without explicit user approval.

## Ask The Human Before Changing These Areas

- Workspace dependency pins, especially Clinker and Dioxus.
- Dependency policy in `deny.toml`.
- Git backend strategy or replacement of CLI behavior.
- YAML document preservation semantics.
- Desktop platform support or Dioxus runtime configuration.
- Large visual redesigns that change CSS/layout contracts.
