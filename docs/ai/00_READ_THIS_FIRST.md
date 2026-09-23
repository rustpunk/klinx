# Read This First

## Purpose

This is the entry point for agents and contributors. It explains how to use the repository memory, what has been verified, and which files to update when the system changes.

## Status

Created on 2026-06-15 from repository inspection, Cargo metadata, CI config, existing docs, and read-only subsystem explorer reports. No source code, manifests, lockfiles, or dependencies were changed to create this documentation.

## Evidence Labels

- **Verified**: Directly observed in source, tests, config, examples, or command output.
- **Strong inference**: Supported by several files or comments, but not explicitly stated as a design decision.
- **Needs grounding**: Plausible but weakly supported. Move it to open questions or validate it against code before acting.
- **Open question**: Known uncertainty that should be resolved before broad or risky changes.

## What To Read

Root `AGENTS.md` routes each task to the docs it needs (its "Where To Look" table). Read those, plus the nearest local `AGENTS.md` for the directory being edited, rather than the whole set.

## Repository Memory Model

Root `AGENTS.md` is the compact, always-loaded guide. `docs/ai/*.md` is durable detailed memory. Local `AGENTS.md` files specialize guidance for high-risk directories. Existing `CLAUDE.md`, `README.md`, `docs/perf.md`, and `examples/README.md` remain useful, but current `Cargo.toml` is the source of truth for dependency pins.

## Rules For Future AI Agents

The rules live in root `AGENTS.md` (Safety Rules, Issues And Workflow Frameworks, Definition Of Done). Record uncertainty in `80_OPEN_QUESTIONS.md` instead of making confident claims.

## Documentation Quality Bar

- Documentation reflects current repository evidence.
- Commands are labeled Verified only if run successfully in this session.
- Weak claims are labeled or removed.
- Links and paths point to real repository files.
- Root and local `AGENTS.md` files remain concise.
- `AI_CHANGELOG.md` records major architecture facts and unresolved questions.

## Documentation Map

- `10_ARCHITECTURE.md`: high-level system shape and control/data flow.
- `20_PROJECT_MAP.md`: factual module/package map.
- `30_DESIGN_RULES.md`: practical rules with evidence strength.
- `35_SHORTCUT_SIGNATURES.md`: forbidden final states checked when closing a unit of work.
- `40_COMMON_PATTERNS.md`: repeated implementation patterns.
- `50_TESTING_AND_COMMANDS.md`: command guide and verification status.
- `60_PERFORMANCE_NOTES.md`: known hot paths and profiling hooks.
- `70_GLOSSARY.md`: project terms and symbols.
- `80_OPEN_QUESTIONS.md`: central uncertainty list.
- `90_LOCAL_AGENT_PLAN.md`: local `AGENTS.md` placement rationale.
- `AI_CHANGELOG.md`: durable architecture/change memory.
- `github-workflow/`: git submodule with the GitHub agent workflow (operations and orchestration notes). Run `git submodule update --init` to populate it.

## When To Update Which Doc

- Architecture or subsystem boundaries changed: update `10_ARCHITECTURE.md`, `20_PROJECT_MAP.md`, and `AI_CHANGELOG.md`.
- New invariant or failure mode found: update `30_DESIGN_RULES.md` and the relevant local `AGENTS.md`.
- Repeated implementation style emerges: update `40_COMMON_PATTERNS.md`.
- Commands, CI, toolchain, or dependencies change: update `50_TESTING_AND_COMMANDS.md` and root `AGENTS.md`.
- GitHub agent workflow, milestone coordination, helper behavior, or Project-state rules change: update the `github-workflow` submodule, root `AGENTS.md`, and `AI_CHANGELOG.md`.
- Performance behavior changes: update `60_PERFORMANCE_NOTES.md` and `docs/perf.md` if user-facing measurement guidance changes.
- Term or domain meaning changes: update `70_GLOSSARY.md`.
- An uncertainty is resolved or discovered: update `80_OPEN_QUESTIONS.md`.

## Known Limitations

- No automated UI integration target was found for the desktop webview; UI validation remains cargo checks plus manual or headless screenshot review.
- Existing prose in `README.md` conflicts with current manifests about older Clinker crate names/rev; current manifests pin the split Clinker crates to `f7a1509`.
- Some UI pages and actions are partially implemented or placeholder-like. Inspect source before claiming a workflow is complete.

## First Prompt For A New Session

Start from `AGENTS.md` and follow its routing table for the area I ask you to change. Treat `Cargo.toml` and source/tests as authoritative, preserve YAML text semantics and Dioxus hook order, and update `docs/ai` if your change alters architecture, commands, invariants, or open questions.
