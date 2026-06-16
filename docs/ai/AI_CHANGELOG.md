# AI Changelog

## Purpose

This file is lightweight architecture/change memory for future agents. It should record durable facts, major changes, and resolved uncertainty. Do not invent past decisions.

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

## 2026-06-16: GitHub Agent Snapshot And Batch Update Helper

- Added `scripts/gh-agent-snapshot.sh` as the repo-local entry point for compact GitHub agent workflow reads and structured bulk updates.
- The repo wrapper delegates to `/home/glitch/.agents/skills/_shared/scripts/gh-agent-snapshot.sh` and defaults to repo `rustpunk/klinx`, Project owner `rustpunk`, and Project number `3`.
- GitHub workflow agents should prefer helper commands for queue, issue, Project status, closeout, and batch label/Project-field updates before falling back to repeated raw `gh` calls.
- Snapshot reads return visible ProjectV2 fields as both `projectItems[].fields` and typed `projectItems[].fieldValues[]` so agents should not perform follow-up GraphQL calls just to inspect Project metadata.
- Multi-issue workflows should use the bulk `issues --issues <n,n,n>` or `issues --file <file>` command instead of looping single-issue snapshots.
- Queue snapshots use issue-number discovery plus bounded bulk hydration to avoid GitHub GraphQL node-limit failures on milestone searches.
- Bulk updates preflight all Project fields/options before applying anything and support ProjectV2 single-select, text, date, and number fields.
- Readiness findings scan fetched comment text; compact mode still truncates emitted comment bodies unless `--full-comments` is passed.
- Verification: shell syntax check and helper dry-run commands were run during implementation.

## 2026-06-16: Milestone Orchestration Workflow

- Added repo-local skill source `.agents/skills/gh-milestone-orchestration` for coordinating a GitHub milestone through planning, queue curation, one-issue implementation agents, review, closeout, and final milestone verification.
- Added `docs/ai/github-workflow/ORCHESTRATION.md` as the durable runbook for coordinator ownership, state model, claim protocol, dispatch prompt shape, stop conditions, and milestone exit gate.
- Added `.github/ISSUE_TEMPLATE/milestone-orchestration.yml` so a milestone coordinator issue can persist active slots, queue, blockers, and closeout state.
- Root guidance now points agents to the orchestration skill/runbook and keeps the maintainer merge gate explicit.
