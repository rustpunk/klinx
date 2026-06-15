# Testing And Commands

## Required Tools

| Tool | Status | Evidence |
| --- | --- | --- |
| Rust 1.91 with `clippy` and `rustfmt` | Inferred | `rust-toolchain.toml`, CI |
| Dioxus CLI `dx` 0.7.4 | Inferred | CI installs `dioxus-cli@0.7.4`; README run command |
| Linux desktop deps: WebKitGTK 4.1, GTK3, libxdo | Inferred | `.github/workflows/ci.yml` |
| `git` CLI | Inferred | `klinx-git` implementation shells out |
| `gh` CLI for PR creation | Inferred | `provider.rs` PR helper |
| `cargo-deny` | Inferred | `deny.toml`, CI |
| Xvfb and ImageMagick for screenshot script | Inferred | `CLAUDE.md`, `scripts/shot.sh` |

## Metadata Commands

| Command | Status | Notes |
| --- | --- | --- |
| `cargo metadata --no-deps --format-version 1` | Verified on 2026-06-15 | Confirmed workspace members and current dependency pins. |
| `git status --short` | Verified on 2026-06-15 | Initial untracked `.claude/`, `.squad/`, and `notes/`; documentation files added later. |

## Fast Check Command

| Command | Status | Notes |
| --- | --- | --- |
| `cargo test -p klinx-git` | Verified on 2026-06-15 | Passed 12 unit tests; doc-tests had zero tests. |
| `cargo test -p klinx <module-filter>` | Inferred | Use filters such as `pipeline_view`, `field_lineage`, `sync`, `yaml_patch`, `template`, `search`, `tokenizer`, `file_explorer`, `inspector`. |

## Full Test Command

| Command | Status | Notes |
| --- | --- | --- |
| `cargo test --workspace` | Inferred | CI test command. First build may need network if git deps are not cached. |

## Formatting Command

| Command | Status | Notes |
| --- | --- | --- |
| `cargo fmt --all --check` | Verified on 2026-06-15 | Passed after documentation-only changes. CI runs on Linux only to avoid Windows CRLF false positives. |

## Linting Commands

| Command | Status | Notes |
| --- | --- | --- |
| `cargo clippy --workspace -- -D warnings` | Inferred | CI dead-code gate; intentionally omits `--all-targets`. |
| `cargo clippy --workspace --all-targets -- -D warnings` | Inferred | Lints tests/examples too; does not replace the first pass. |
| `cargo deny check` | Inferred | CI dependency/license/advisory gate. |

## Docs Command

| Command | Status | Notes |
| --- | --- | --- |
| `cargo doc --workspace --no-deps` | Inferred | Not found in CI, but standard Rust docs command. Use when public docs are touched. |

## Example And Demo Commands

| Command | Status | Notes |
| --- | --- | --- |
| `dx serve --package klinx --platform desktop` | Inferred | Main desktop run command. README notes desktop-only native webview. |
| `dx serve --package klinx --features perf-trace` | Inferred | Runs with parse/tokenize timing traces. |
| `cargo build --package klinx` then `scripts/shot.sh shot.png ./examples/pipelines` | Inferred | Headless screenshot workflow from existing docs. Requires Xvfb/ImageMagick/Linux desktop deps. |

## Benchmark And Performance Commands

| Command | Status | Notes |
| --- | --- | --- |
| `dx serve --package klinx --features perf-trace` | Inferred | Use with `crates/klinx/tests/fixtures/large_pipeline.yaml` and `docs/perf.md`. |
| Manual DevTools/profile workflow | Inferred | Documented in `docs/perf.md`; no Criterion benches found. |

## CI And Deploy Commands

| Command | Status | Notes |
| --- | --- | --- |
| `dx build --package klinx --platform desktop` | Inferred | CI builds desktop bundle on Linux/macOS/Windows. |
| `cargo deny check` | Inferred | CI has a separate deny job. |

## Commands Agents Should Run Before Claiming Success

- Documentation-only changes: `git diff --stat`, `git diff -- AGENTS.md doc/ai crates/klinx/AGENTS.md crates/klinx/src/components/AGENTS.md crates/klinx-git/AGENTS.md`, and markdown/path sanity searches.
- Rust source changes: focused module tests plus `cargo fmt --all --check`; for broader changes also run both clippy passes and `cargo test --workspace`.
- UI/layout changes: cargo checks plus manual desktop run or headless screenshot when available.
- Dependency changes: ask first, then run `cargo deny check` and the full CI command set.

## Expensive, Flaky, Or Environment-Dependent Commands

- `dx serve` starts a desktop app and may need native system packages.
- `dx build --package klinx --platform desktop` can be slower and platform-specific.
- First cargo build/test may need network for Clinker git dependencies if not cached.
- `scripts/shot.sh` depends on Xvfb, ImageMagick, and software rendering setup.
- `gh` PR creation requires authentication and should not be run without explicit user intent.

## Troubleshooting Notes

- If Dioxus desktop fails on Linux, check WebKitGTK/GTK/libxdo packages first.
- If Clinker types fail to resolve, verify root `Cargo.toml` pins the split crates at `997ea7d` and source imports use `clinker_plan`, `clinker_exec`, `clinker_core_types`, `clinker_record`, `clinker_schema`, `clinker_channel`, and `cxl`.
- If UI text highlighting drifts, check YAML sidebar CSS line height and `LINE_HEIGHT`.
- If git UI status seems stale, inspect `hooks/git_state.rs` and `fs_watcher.rs`; background watcher refresh is an open uncertainty.
