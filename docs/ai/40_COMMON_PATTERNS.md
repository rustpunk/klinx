# Common Patterns

## AppShell-Owned Signals

- **Where:** `crates/klinx/src/app.rs`, `crates/klinx/src/state.rs`, `crates/klinx/src/tab.rs`.
- **Why it seems to exist:** Avoids Dioxus signal scope ownership issues while allowing tab switching.
- **Copy it correctly:** Put long-lived reactive state in `AppShell`, expose it through `AppState`/`TabManagerState`, and store inactive tab data in `TabEntry::snapshot`.
- **Common mistakes:** Creating per-tab signals in child components, moving hooks into conditionals, or failing to flush snapshots before save/close.
- **Evidence:** `app.rs` module comment and explorer evidence around `AppShell`, `TabEntry`, `flush_snapshot_if_active`.

## EditSource-Gated Synchronization

- **Where:** `hooks/pipeline_sync.rs`, `sync.rs`, `app.rs`, `keyboard.rs`.
- **Why it seems to exist:** Separates user typing, inspector edits, file opens, and tab switches so effects do not loop.
- **Copy it correctly:** When updating YAML or parsed state, understand and set the relevant `EditSource`.
- **Common mistakes:** Updating `yaml_text` directly from UI actions without considering parse effects.
- **Evidence:** `EditSource` enum in `sync.rs`; AppShell comments on parse debounce and visible error settle.

## Text-First YAML Preservation

- **Where:** `yaml_patch.rs`, `keyboard.rs`, `tab.rs`, `file_ops.rs`.
- **Why it seems to exist:** Engine serialization can lose `nodes:` or comments; user-authored YAML must remain intact.
- **Copy it correctly:** Save `yaml_text` as the document and use `patch_yaml_preserving_nodes` for model-driven inspector changes.
- **Common mistakes:** Serializing `PipelineConfig` for normal saves or inspector edits.
- **Evidence:** `yaml_patch.rs` preservation tests, comments about issue #29, app-shell save rules.

## View Model Before UI

- **Where:** `pipeline_view.rs`, `pipeline_view/field_lineage.rs`, `components/canvas/**`, `components/inspector/**`.
- **Why it seems to exist:** Keeps engine config parsing and graph/lineage derivation out of Dioxus rendering code.
- **Copy it correctly:** Add new node or layout semantics to `pipeline_view` first, with tests, then render the resulting fields in components.
- **Common mistakes:** Deriving graph edges in components or adding wildcard engine variant matches.
- **Evidence:** `derive_pipeline_view`, `StageView`, `Connection`, field lineage helper tests.

## Component Module With Local Helpers

- **Where:** `components/file_explorer/model.rs`, `components/yaml_sidebar/tokenizer.rs`, `components/inspector/panel.rs`, `components/version_mode/pr_pane.rs`.
- **Why it seems to exist:** Pure helper logic near UI has targeted unit tests without needing desktop integration tests.
- **Copy it correctly:** Put testable data transforms in helper functions/modules beside the component.
- **Common mistakes:** Burying parsing or formatting logic inside RSX blocks.
- **Evidence:** unit tests in file explorer model, YAML tokenizer, inspector panel, PR pane.

## CLI-Backed Runtime Boundaries

- **Where:** `crates/klinx-git/src/gix_backend.rs`, `provider.rs`, `scripts/shot.sh`.
- **Why it seems to exist:** Uses existing system tools for git/PR workflows and headless screenshot capture.
- **Copy it correctly:** Keep shelling behavior behind a small API and document runtime tool requirements.
- **Common mistakes:** Assuming `gix_backend.rs` uses gitoxide, or invoking CLI tools directly from unrelated UI code.
- **Evidence:** `GitCliOps`, `GitOps`, provider helpers, script comments.

## CI Command Parity

- **Where:** `.github/workflows/ci.yml`, `README.md`, `CLAUDE.md`.
- **Why it seems to exist:** CI intentionally uses separate lint passes and builds desktop bundles on three OSes.
- **Copy it correctly:** Preserve command names exactly when documenting or running pre-commit checks.
- **Common mistakes:** Dropping the non-`--all-targets` clippy pass.
- **Evidence:** CI comments explaining the two clippy passes.
