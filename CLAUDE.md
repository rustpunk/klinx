# CLAUDE.md

Guidance for Claude Code (claude.ai/code) when working in the klinx repository.

## What klinx is

Klinx is the standalone Dioxus IDE for authoring Clinker pipeline YAML. It was
extracted from the `clinker-kiln` crate of the Clinker workspace. Klinx is a
`wry` desktop application (Linux/macOS/Windows), the default platform per
`Dioxus.toml`. It builds with `dioxus = { features = ["desktop"] }`, uses
`tokio` for async, and depends on the local `klinx-git` crate for VCS
operations. Launch: `dx serve --package klinx`.

The Dioxus version is pinned to `=0.7.4` to avoid silent breakage. The `dx` CLI
is required — install via `cargo install dioxus-cli`.

## Engine types come from Clinker via a git pin

Klinx does not vendor the engine. The five engine crates it imports — `cxl`,
`clinker-core`, `clinker-record`, `clinker-schema`, `clinker-channel` — are git
dependencies pinned to Clinker commit `c233a38`, declared in the workspace
`Cargo.toml` `[workspace.dependencies]` block:

```toml
cxl = { git = "https://github.com/rustpunk/clinker", rev = "c233a38" }
```

Keep these identifiers as `clinker_core` / `clinker_record` / `clinker_schema`
/ `clinker_channel` / `cxl` in source — they resolve to the git-pinned crates.
The first build fetches the commit over the network. To move to a newer engine
surface, bump the `rev` in `[workspace.dependencies]` (a single edit point) and
rebuild.

The git VCS layer is the local `klinx-git` crate (formerly `clinker-git`),
imported in source as `klinx_git`.

## Pre-commit checks

The CI gauntlet — exactly what GitHub CI runs (`.github/workflows/ci.yml`):

1. `cargo fmt --all` (CI runs `--check`; locally fix first)
2. `cargo clippy --workspace -- -D warnings`
3. `cargo clippy --workspace --all-targets -- -D warnings`
4. `cargo test --workspace`
5. `cargo deny check`

Steps 2 and 3 are both load-bearing. Step 2 omits `--all-targets` deliberately:
with test targets excluded, a `pub(crate)` item referenced only from
`#[cfg(test)]` code still trips the dead-code lint, so step 2 is the dead-code
gate. Step 3 adds `--all-targets` for lint coverage of test and example code
that step 2 never compiles — it does not replace step 2.

## Build & run

```bash
cargo build --workspace          # first build fetches the clinker git pin
cargo test --workspace
dx serve --package klinx         # desktop target (default per Dioxus.toml)
```

Klinx is desktop-only: there is no `wasm32`/web build and no Playwright web test
target. UI verification runs against the `wry` desktop app.

## Dioxus

Dioxus is pinned to `=0.7.4`. Apply Dioxus 0.7 patterns and anti-pattern
guidance to every component, signal, and RSX edit — not just new code. Prefer
signals and memos over manual interior mutability; key list items stably.

## Dependency policy

Prefer crates with a release in the last 12 months, a non-archived repo, and
zero open RustSec advisories. `cargo deny check` enforces `unmaintained` and
`yanked` advisories mechanically; the Dioxus desktop GTK/WebView transitive
graph is the only allowed exception (see `deny.toml`). Verify every new crate
before adding it. The pure-Rust policy (ban `cmake`) applies to klinx's own
crates; Dioxus/GTK transitive C deps are exempted via the `deny.toml` skips.

## Comment policy

Comments explain WHY the code is the way it is. A short WHAT is fine when it
adds precision the signature can't express (invariants, units, threading/UI
model). Every public item gets a `///` summary. Avoid deletion tombstones;
explanations for removed code belong in the commit message.

## Rust edition & toolchain

Edition 2024, Rust 1.91 (pinned in `rust-toolchain.toml`).

## Git

Commit or push only when the user asks. End commit messages with:

```
Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
```
