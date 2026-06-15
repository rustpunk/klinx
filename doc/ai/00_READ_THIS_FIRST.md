# Read This First

## Purpose

This is the entry point for agents and contributors. It explains how to use the repository memory, what has been verified, and which files to update when the system changes.

## Status

Created on 2026-06-15 from repository inspection, Cargo metadata, CI config, existing docs, and read-only subsystem explorer reports. No source code, manifests, lockfiles, or dependencies were changed to create this documentation.

## Evidence Labels

- **Verified**: Directly observed in source, tests, config, examples, or command output.
- **Strong inference**: Supported by several files or comments, but not explicitly stated as a design decision.
- **Hypothesis**: Plausible but weakly supported. Treat as a prompt to inspect code before acting.
- **Open question**: Known uncertainty that should be resolved before broad or risky changes.

## Reading Order By Task

- Workspace, sessions, tabs, keyboard, templates, search: read `10_ARCHITECTURE.md`, `20_PROJECT_MAP.md`, `30_DESIGN_RULES.md`, then `crates/klinx/AGENTS.md`.
- Pipeline parsing, canvas model, field lineage, YAML patching, CXL diagnostics, autodoc: read `10_ARCHITECTURE.md`, `30_DESIGN_RULES.md`, `40_COMMON_PATTERNS.md`, then `crates/klinx/AGENTS.md`.
- UI components, CSS, canvas, YAML editor, inspector, panels: read `crates/klinx/src/components/AGENTS.md`, `60_PERFORMANCE_NOTES.md`, and `50_TESTING_AND_COMMANDS.md`.
- Git/version mode: read `crates/klinx-git/AGENTS.md` and the git sections in `20_PROJECT_MAP.md`.
- Tooling, CI, dependency policy: read `50_TESTING_AND_COMMANDS.md` and root `AGENTS.md`.

## Minimum Reading Checklist Before Editing Code

1. Root `AGENTS.md`.
2. This file.
3. The relevant local `AGENTS.md`, if one exists.
4. `30_DESIGN_RULES.md` entries for the subsystem.
5. `50_TESTING_AND_COMMANDS.md` for the smallest meaningful verification command.

## Repository Memory Model

Root `AGENTS.md` is the compact, always-loaded guide. `doc/ai/*.md` is durable detailed memory. Local `AGENTS.md` files specialize guidance for high-risk directories. Existing `CLAUDE.md`, `README.md`, `docs/perf.md`, and `examples/README.md` remain useful, but current `Cargo.toml` is the source of truth for dependency pins.

## Rules For Future AI Agents

- Do not modify application/source code when asked for documentation-only work.
- Do not edit `Cargo.lock` or add dependencies without explicit approval.
- Do not push or commit unless explicitly asked.
- Verify current dependency pins from `Cargo.toml`, not older prose.
- Preserve Dioxus hook ordering and signal ownership rules.
- Preserve YAML text as the authoritative editor content.
- Prefer focused tests for the touched subsystem, then workspace CI commands when practical.
- Mark uncertainty as Hypothesis or Open question instead of making confident claims.

## Definition Of Done

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
- `40_COMMON_PATTERNS.md`: repeated implementation patterns.
- `50_TESTING_AND_COMMANDS.md`: command guide and verification status.
- `60_PERFORMANCE_NOTES.md`: known hot paths and profiling hooks.
- `70_GLOSSARY.md`: project terms and symbols.
- `80_OPEN_QUESTIONS.md`: central uncertainty list.
- `90_LOCAL_AGENT_PLAN.md`: local `AGENTS.md` placement rationale.
- `AI_CHANGELOG.md`: durable architecture/change memory.

## When To Update Which Doc

- Architecture or subsystem boundaries changed: update `10_ARCHITECTURE.md`, `20_PROJECT_MAP.md`, and `AI_CHANGELOG.md`.
- New invariant or failure mode found: update `30_DESIGN_RULES.md` and the relevant local `AGENTS.md`.
- Repeated implementation style emerges: update `40_COMMON_PATTERNS.md`.
- Commands, CI, toolchain, or dependencies change: update `50_TESTING_AND_COMMANDS.md` and root `AGENTS.md`.
- Performance behavior changes: update `60_PERFORMANCE_NOTES.md` and `docs/perf.md` if user-facing measurement guidance changes.
- Term or domain meaning changes: update `70_GLOSSARY.md`.
- An uncertainty is resolved or discovered: update `80_OPEN_QUESTIONS.md`.

## Known Limitations

- No automated UI integration target was found for the desktop webview; UI validation remains cargo checks plus manual or headless screenshot review.
- Existing prose in `README.md` appears stale about older Clinker crate names/rev; current manifests pin the split Clinker crates to `997ea7d`.
- Some UI pages and actions are partially implemented or placeholder-like. Inspect source before claiming a workflow is complete.

## First Prompt For A New Codex Session

Read `AGENTS.md`, `doc/ai/00_READ_THIS_FIRST.md`, and the local `AGENTS.md` for the area I ask you to change. Treat `Cargo.toml` and source/tests as authoritative, preserve YAML text semantics and Dioxus hook order, and update `doc/ai` if your change alters architecture, commands, invariants, or open questions.
