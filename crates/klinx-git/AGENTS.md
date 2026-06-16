# AGENTS.md

## Purpose

This crate is the git/VCS abstraction used by Klinx. It exposes a stable `GitOps` trait, CLI-backed implementation, git data types, and remote provider helpers.

## Responsibilities

- Discover repositories and repository roots.
- Read status, branches, log, blame, diffs, and ahead/behind information.
- Stage, unstage, commit, push, pull, fetch, and stage all.
- Detect remote providers and create GitHub PRs through `gh`.

## Important Public APIs Or Entry Points

- `GitOps`
- `GitCliOps::{discover, discover_with_binary, root}`
- `RepoStatus`, `FileStatus`, `StatusKind`, `BranchInfo`, `CommitInfo`, `BlameLine`
- `detect_provider`, `parse_remote_url`, `get_remote_url`, `get_default_branch`, `create_pr`

## Internal Module Map

- `lib.rs`: public exports and crate purpose.
- `ops.rs`: trait and `GitError`.
- `types.rs`: status/branch/log/blame data types.
- `gix_backend.rs`: current CLI-backed implementation despite filename.
- `provider.rs`: provider detection and PR helpers.

## Dependency Rules

Keep UI-independent git behavior here. Do not add ad hoc git shellouts in `crates/klinx` when this crate should own the behavior. Do not assume `gix_backend.rs` means gitoxide is currently used.

## Important Invariants

- `FileStatus.path` values are repo-root-relative.
- Detached HEAD is represented as `HEAD (detached)`.
- Missing upstream means ahead/behind `(0, 0)`.
- PR creation is implemented for GitHub; other providers return an operation error.
- CLI runtime tools are part of the behavior: `git`, and `gh` for PRs.

## Common Mistakes To Avoid

- Treating status paths as workspace-relative UI paths.
- Bypassing `GitOps` from UI code.
- Assuming staged versus unstaged status is fully modeled everywhere in the UI.
- Removing CLI behavior without checking credential-helper and platform implications.

## Local Commands

- `cargo test -p klinx-git`
- `cargo clippy -p klinx-git -- -D warnings`
- `cargo fmt --all --check`

## Documentation Updates

Update `docs/ai/20_PROJECT_MAP.md`, `30_DESIGN_RULES.md`, `60_PERFORMANCE_NOTES.md`, and `AI_CHANGELOG.md` if this crate's public surface or backend strategy changes.

## Unclear / Ask Human

Ask before replacing the CLI backend, adding non-GitHub PR behavior, changing path semantics, or changing runtime tool assumptions.

## Evidence

`src/lib.rs`, `src/ops.rs`, `src/gix_backend.rs`, `src/provider.rs`, `src/types.rs`, unit tests in this crate, and `docs/ai`.
