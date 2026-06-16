# Local Agent Plan

## Recommended Local `AGENTS.md` Locations

| Location | Priority | Why local guidance is needed | Rules to include | Local commands | Risks without guidance | Evidence | Confidence |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `crates/klinx/AGENTS.md` | High | Main IDE crate mixes Dioxus state, workspace persistence, YAML parsing, view derivation, and tests. | AppShell owns signals; YAML text authoritative; preserve `EditSource`; use focused tests. | `cargo test -p klinx <filter>`, `dx serve --package klinx --platform desktop` | Agents may break hook order, tab snapshots, or YAML preservation. | `app.rs`, `sync.rs`, `yaml_patch.rs`, explorer reports | High |
| `crates/klinx/src/components/AGENTS.md` | High | Component and CSS layer has layout contracts, Dioxus hook constraints, and no browser test target. | Context usage, CSS/data attributes, YAML line-height alignment, canvas geometry, manual visual validation. | component-focused tests; `dx serve`; screenshot script when available | Layout drift, hook misuse, slow render paths, CSS/component mismatch. | components report, `klinx.css`, README | High |
| `crates/klinx-git/AGENTS.md` | High | Git abstraction boundary can be confused with gitoxide because of filename; runtime CLI behavior matters. | Use `GitOps`; paths repo-relative; do not ad hoc shell out; GitHub-only PR creation. | `cargo test -p klinx-git`, `cargo clippy -p klinx-git -- -D warnings` | Agents may bypass abstraction, mis-handle paths, or assume `gix` is in use. | git explorer report, `lib.rs`, `ops.rs`, `gix_backend.rs` | High |

## Medium-Priority Candidates To Add Later

| Location | Priority | Why | Suggested trigger |
| --- | --- | --- | --- |
| `examples/AGENTS.md` | Medium | Examples are regression fixtures and sample workspaces. | Add if agents frequently edit examples or engine fixture YAML. |
| `docs/AGENTS.md` | Medium | Research docs may be stale relative to source. | Add if planning/research docs are actively maintained. |
| `crates/klinx/src/hooks/AGENTS.md` | Medium | Effects and side effects have Dioxus constraints. | Add if hooks become a frequent edit area. |
| `crates/klinx/src/pipeline_view/AGENTS.md` | Medium | Field lineage helper is important, but `pipeline_view.rs` is a sibling file, so this local guide would not cover the main module. | Add only if the subdirectory grows. |

## Probably Unnecessary

- `target/`: generated build artifacts.
- `.github/`: CI is small and already documented in root and `50_TESTING_AND_COMMANDS.md`.
- `scripts/`: currently only screenshot helper; root docs are enough.
- `notes/`: untracked handoff/planning notes, not durable onboarding docs.
- `.claude/` and `.squad/`: untracked tool/worktree state; do not treat as source of durable truth.

## Suggested Creation Batches

1. Batch 1: Create high-priority local guides for `crates/klinx`, `crates/klinx/src/components`, and `crates/klinx-git`.
2. Batch 2: After future source work, decide whether hooks or examples need specialized guidance.
3. Batch 3: Revisit docs/research once stale README/research discrepancies are resolved.
