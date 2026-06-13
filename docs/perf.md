# Klinx desktop performance — measurement guide

This is the baseline/regression procedure for the editor input-latency and
startup work (epic #12). The goal is repeatable before/after numbers on the
**same fixture** across Linux, macOS, and Windows.

## Fixture

`crates/klinx/tests/fixtures/large_pipeline.yaml` — a ~4.7k-line linear pipeline
(a source, ~520 chained `transform` stages, an output). It is large enough that
the per-keystroke tokenize + DOM rebuild and the parse/canvas fan-out dominate,
which is exactly what we are optimizing.

Open it in klinx with:

```bash
dx serve --package klinx
# then File → Open and pick crates/klinx/tests/fixtures/large_pipeline.yaml
```

### Regenerating the fixture

The fixture is generated, not hand-written. To change its size, rerun the
generator (bumping `N`, the number of transform stages):

```bash
N=520; OUT=crates/klinx/tests/fixtures/large_pipeline.yaml
# (see the generator in the PR that added this file / git history)
```

## What to measure

| Metric | How |
| --- | --- |
| **Typing latency** | In the YAML editor, open DevTools → Performance, record while holding a key / typing a burst in the middle of the file. Read input-event → next paint. |
| **Parse frequency** | Build with `--features perf-trace` (below) and watch stderr: count `try_parse_yaml` / `tokenize` lines emitted per typing burst. |
| **First paint / cold start** | Time from process launch to the window appearing with content. The window is created hidden and revealed on first frame, so this measures real time-to-content. |
| **Idle memory** | OS task manager / `ps` RSS of the app process at idle with the fixture open. |

### `perf-trace` feature

A zero-cost-when-disabled timing feature prints tokenize/parse durations to
stderr:

```bash
dx serve --package klinx --features perf-trace
# stderr:
# [perf] tokenize: 4709 lines from 98123 bytes in 1.84ms
# [perf] try_parse_yaml: 98123 bytes in 5.21ms
```

Use it to confirm, e.g., that after the debounce work (#7) `try_parse_yaml`
fires **once per typing pause** rather than once per keystroke, and after the
memoization work (#8) that `tokenize` is not called on unrelated re-renders.

## Recording results

For each change, capture a row: platform, fixture line count, typing latency
(p50/p95), parse count per burst, cold-start ms, idle RSS — before and after.
Keep the numbers in the relevant PR description so the win is auditable.

### Baseline (fill in)

| Platform | Typing latency p50 | Parse / burst | Cold start | Idle RSS |
| --- | --- | --- | --- | --- |
| Windows | _tbd_ | _tbd_ | _tbd_ | _tbd_ |
| Linux | _tbd_ | _tbd_ | _tbd_ | _tbd_ |
| macOS | _tbd_ | _tbd_ | _tbd_ | _tbd_ |
