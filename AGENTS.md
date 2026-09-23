# AGENTS.md

## Project Summary

Klinx is a Rust 2024 workspace for a Dioxus 0.7 desktop IDE that authors Clinker YAML pipeline configurations. It contains the `klinx` desktop app crate and the `klinx-git` git abstraction crate.

Detailed architecture, commands, rules, and open questions live under [docs/ai/](docs/ai/). Treat `Cargo.toml`, source, and tests as authoritative when prose disagrees.

## Where To Look

Read what the task needs, when it needs it:

| When you… | Read |
|---|---|
| are new to the repository | [docs/ai/00_READ_THIS_FIRST.md](docs/ai/00_READ_THIS_FIRST.md) |
| work on workspace, sessions, tabs, keyboard, templates, or search | [10_ARCHITECTURE](docs/ai/10_ARCHITECTURE.md), [20_PROJECT_MAP](docs/ai/20_PROJECT_MAP.md), `crates/klinx/AGENTS.md` |
| work on pipeline parsing, the canvas model, field lineage, YAML patching, CXL diagnostics, or autodoc | [30_DESIGN_RULES](docs/ai/30_DESIGN_RULES.md), [40_COMMON_PATTERNS](docs/ai/40_COMMON_PATTERNS.md), `crates/klinx/AGENTS.md` |
| touch UI components, CSS, canvas, editor, inspector, or panels | `crates/klinx/src/components/AGENTS.md`, [60_PERFORMANCE_NOTES](docs/ai/60_PERFORMANCE_NOTES.md) |
| work on git or version mode | `crates/klinx-git/AGENTS.md` |
| design or review a change, or close out a unit of work | [35_SHORTCUT_SIGNATURES](docs/ai/35_SHORTCUT_SIGNATURES.md) |
| pick a command, or change CI, toolchain, or dependency policy | [50_TESTING_AND_COMMANDS](docs/ai/50_TESTING_AND_COMMANDS.md) |
| meet an unfamiliar term | [70_GLOSSARY](docs/ai/70_GLOSSARY.md) |
| hit something unclear | search [80_OPEN_QUESTIONS](docs/ai/80_OPEN_QUESTIONS.md), then record it there |

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
- Headless UI screenshot (no physical display needed): `cargo build --package klinx` then
  `scripts/shot.sh <out.png> <workspace-dir>` (e.g. `scripts/shot.sh /tmp/shot.png ./examples/pipelines`).
  Renders the real wry/WebKitGTK app under Xvfb with software GL and grabs the root window via
  ImageMagick `import`; drive interaction (hover/click/pan) with `xdotool` against the Xvfb display,
  then re-shot/crop. There is no Playwright/web target — this is the visual-verification path.
  Requires `xvfb-run`, ImageMagick, and mesa software GL. See `scripts/shot.sh`.

## Safety Rules For AI Agents

- Do not add dependencies, edit lockfiles, push, or commit unless explicitly asked.
- Do not modify application/source code during documentation-only tasks.
- Ask before bumping Dioxus, Clinker pins, dependency policy, or git backend strategy.
- Mark weak claims as Hypothesis or Open question in `docs/ai`.
- Preserve user changes in the worktree.

## Issues And Workflow Frameworks

- Implement only issues labelled `agent-ready`. Issues labelled `agent-plan-first` or `not-agent-ready` need a maintainer-approved plan first.
- When a planning or execution framework drives the work, this file overrides the framework's defaults; the framework owns only its planning-state layout and task sequencing. Planning state is local and stays out of git.
- A maintainer-approved plan authorizes commits on a non-`main` feature branch for the issues it names. Pushing, opening PRs, and merging still need an explicit request.
- The approval gates above (dependencies, pins, dependency policy, git backend strategy) stop for the maintainer, including in a framework's automatic modes.
- PR titles and descriptions become the squashed commit on `main`, so they use domain wording and reference the issue, not planning coordinates.

## Coding Conventions

- Follow Rust 2024 idioms and the repo's existing module style.
- Prefer focused helper functions with unit tests for pure UI-adjacent logic.
- Public items should have useful doc comments when introduced.
- Comments should explain why, not restate what the code says.

## Documentation Updates

Update `docs/ai` when architecture, commands, invariants, performance behavior, or open questions change. Update local `AGENTS.md` files when directory-specific rules change. Append durable architecture facts to [docs/ai/AI_CHANGELOG.md](docs/ai/AI_CHANGELOG.md).

## Definition Of Done

This section is the single owner of the close gate; other docs point here.

- While iterating, run the smallest meaningful check for the touched area (a focused `cargo test -p <crate> <filter>`).
- Before claiming a Rust change is done, the closing commit passes locally: `cargo fmt --all --check`, both clippy passes, `cargo test --workspace`, and `cargo deny check`. Add `dx build --package klinx --platform desktop` when bundle, asset, or Dioxus config changes.
- UI-affecting changes render through `scripts/shot.sh` and the screenshot is inspected before the change counts as verified.
- The closing state carries none of the signatures in [35_SHORTCUT_SIGNATURES](docs/ai/35_SHORTCUT_SIGNATURES.md).
- Documentation-only changes: `git diff --check` and path sanity.
- Document commands that were not run, keep docs and local agent guidance consistent, and leave no unsupported confident claims.
