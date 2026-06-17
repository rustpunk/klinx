# Performance Notes

Do not optimize from this document alone. Use `docs/perf.md`, targeted profiling, and user-visible measurements.

## YAML Editing Path

- **Area/module:** `app.rs`, `sync.rs`, `components/yaml_sidebar/**`, `docs/perf.md`.
- **Why sensitive:** Typing latency depends on parse frequency, tokenization, and rendered line count.
- **Existing choices:** Parse debounce around 150ms; visible error settle around 500ms; YAML tokenization/rendering uses memoization and virtualization; `perf-trace` prints tokenize and parse timings.
- **Avoid:** Parsing on every re-render, removing debounce, changing textarea/highlight alignment casually.
- **Hooks:** `dx serve --package klinx --features perf-trace`; large fixture `crates/klinx/tests/fixtures/large_pipeline.yaml`.
- **Confidence:** High.
- **Evidence:** `docs/perf.md`, `app.rs` comments, YAML sidebar component/tokenizer reports.

## Canvas Layout And Field Lineage

- **Area/module:** `pipeline_view.rs`, `pipeline_view/field_lineage.rs`, `components/canvas/**`.
- **Why sensitive:** Large pipelines and wide schemas stress graph derivation, field edge computation, hover/pin filtering, and SVG connector rendering.
- **Existing choices:** Progressive field-lineage disclosure rather than drawing all field edges; hover is one-hop, click is transitive; canvas drag uses non-reactive state; wide-schema and adaptive display-mode nodes project their field rows through panel-owned mode/filter/load-more state before connector anchor resolution, so visible rows and field anchors stay derived from the same displayed row set.
- **Avoid:** Globally rendering all field edges, doing graph derivation inside components, adding pointer-move signal churn, or slicing/collapsing field rows only in `CanvasNode` after the panel has already resolved anchors.
- **Hooks:** `cargo test -p klinx pipeline_view`, `cargo test -p klinx field_lineage`, manual canvas review.
- **Confidence:** High.
- **Evidence:** `pipeline_view` tests, field lineage docs/research, component report.

## Startup And First Paint

- **Area/module:** `main.rs`, `assets/klinx.css`, `docs/perf.md`.
- **Why sensitive:** Desktop webview startup and CSS loading affect time-to-content.
- **Existing choices:** CSS is inlined into the head; window starts hidden and is revealed after first mounted frame; Linux DMABUF renderer is disabled unless explicitly set.
- **Avoid:** Reintroducing late CSS asset loading or visible unstyled window startup without measuring.
- **Hooks:** manual cold-start measurement in `docs/perf.md`; `scripts/shot.sh` for render capture.
- **Confidence:** High.
- **Evidence:** `main.rs` comments and `docs/perf.md`.

## File Explorer And Search

- **Area/module:** `components/file_explorer/**`, `search.rs`.
- **Why sensitive:** Filesystem walks and YAML scans can block UI for large workspaces.
- **Existing choices:** File explorer build/flatten is split and memoized; search has focused matching helpers.
- **Avoid:** Rewalking disk on expand/collapse; making search broader or recursive without checking UI responsiveness.
- **Hooks:** `cargo test -p klinx file_explorer`, `cargo test -p klinx search`.
- **Confidence:** Medium.
- **Evidence:** component explorer report, search tests.

## Git Operations

- **Area/module:** `crates/klinx-git`, `components/version_mode/**`, `hooks/git_state.rs`.
- **Why sensitive:** Current implementation shells out repeatedly to `git`; log, blame, status, and provider commands can be slow.
- **Existing choices:** Git operations are behind `GitOps`; current implementation favors CLI/credential-helper reliability.
- **Avoid:** Adding ad hoc git shellouts in UI components; running expensive git commands in render paths.
- **Hooks:** `cargo test -p klinx-git`; manual repo workflows.
- **Confidence:** Medium-High.
- **Evidence:** `klinx-git` explorer report, `GitOps`, git UI call sites.

## Debug/Run Data Display

- **Area/module:** `debug_state.rs`, `components/run_log/**`.
- **Why sensitive:** Large nested values and row sets can be expensive to display.
- **Existing choices:** Compact cell display truncates nested values by depth.
- **Avoid:** Rendering full nested data structures by default.
- **Hooks:** `cargo test -p klinx debug_state`.
- **Confidence:** Medium.
- **Evidence:** `debug_state.rs` tests and comments.
