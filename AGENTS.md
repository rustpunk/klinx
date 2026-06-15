# AGENTS.md

## Project Summary

Klinx is a Rust 2024 workspace for a Dioxus 0.7 desktop IDE that authors Clinker YAML pipeline configurations. It contains the `klinx` desktop app crate and the `klinx-git` git abstraction crate.

Read first: [doc/ai/00_READ_THIS_FIRST.md](doc/ai/00_READ_THIS_FIRST.md). Detailed architecture, commands, rules, and open questions live under [doc/ai/](doc/ai/).

## Repository Layout

- `crates/klinx`: Dioxus desktop IDE binary.
- `crates/klinx-git`: CLI-backed git/VCS abstraction.
- `examples/pipelines`: sample Clinker workspace and fixtures.
- `docs/perf.md`: performance measurement guide.
- `.github/workflows/ci.yml`: CI command source of truth.

## High-Level Design Rules

- Treat root `Cargo.toml` as the source of truth for dependency pins.
- Keep Dioxus hooks unconditional and preserve `AppShell` signal ownership.
- YAML text is authoritative; do not replace normal saves with full `PipelineConfig` serialization.
- Use `pipeline_view` for canvas/view-model derivation and `GitOps` for git operations.
- Keep both CI clippy passes; they check different target sets.

## Build, Test, Format, Lint

- Format check: `cargo fmt --all --check`
- Lint: `cargo clippy --workspace -- -D warnings`
- Lint all targets: `cargo clippy --workspace --all-targets -- -D warnings`
- Test: `cargo test --workspace`
- Dependency policy: `cargo deny check`
- Run desktop app: `dx serve --package klinx --platform desktop`
- Desktop bundle: `dx build --package klinx --platform desktop`

## Safety Rules For AI Agents

- Do not add dependencies, edit lockfiles, push, or commit unless explicitly asked.
- Do not modify application/source code during documentation-only tasks.
- Ask before bumping Dioxus, Clinker pins, dependency policy, or git backend strategy.
- Mark weak claims as Hypothesis or Open question in `doc/ai`.
- Preserve user changes in the worktree.

## Coding Conventions

- Follow Rust 2024 idioms and the repo's existing module style.
- Prefer focused helper functions with unit tests for pure UI-adjacent logic.
- Public items should have useful doc comments when introduced.
- Comments should explain why, not restate what the code says.

## Documentation Updates

Update `doc/ai` when architecture, commands, invariants, performance behavior, or open questions change. Update local `AGENTS.md` files when directory-specific rules change. Append durable architecture facts to [doc/ai/AI_CHANGELOG.md](doc/ai/AI_CHANGELOG.md).

## Definition Of Done

Run the smallest meaningful checks for the touched area, document commands that were not run, keep docs and local agent guidance consistent, and leave no unsupported confident claims.
