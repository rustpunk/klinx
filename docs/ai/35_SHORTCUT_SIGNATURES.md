# Shortcut Signatures

Single source of truth for shortcut signatures and forbidden patterns in this
repo, read when designing, reviewing, or closing a unit of work. Signatures
describe the **forbidden final state of a unit of work** (sprint, PR);
intermediate commits may carry transitional violations provided the closing
commit eliminates them.

## Shortcut signatures — block at close

### 1. Suppression attributes

- New `#[allow(dead_code)]`, `#[allow(unused_*)]`, `#[allow(clippy::*)]`
  introduced in the diff

**Correct alternative**: delete the dead item or fix the lint; surface
multi-step rip plans explicitly.

### 2. Test suppression and weakening

- `#[ignore]` on any test; `#[should_panic]` silencing an equality
- Weakened assertions: `assert_eq!` → `assert!(contains)`; structured →
  substring; deleted assertions with no replacement

**Correct alternative**: fix the code under test, or surface the semantic
change as a blocking decision before touching the assertion.

### 3. Tombstone / removal-history comments

- `// removed X`, `// was:`, `// old:`, `// previously:`, `// kept for compat`

**Correct alternative**: removal history belongs in the commit message.

### 4. Placeholder fixtures

- `TODO: real data`, `FIXME: placeholder`, `unimplemented!()` / `todo!()` on
  reachable paths

**Correct alternative**: populate real data or fail loudly; if the data isn't
ready, the work isn't done.

### 5. Speculative API surface

- `pub` items with no non-test caller in the closing state; visibility widened
  without a downstream consumer

**Correct alternative**: minimal surface; every public item needs a real
consumer at close.

### 6. Klinx-specific gotchas

- **Conditional Dioxus hooks**: any `use_*` hook call behind `if` / `match` /
  early-return, or inside a loop — hooks must run unconditionally, in the same
  order, on every render.
- **`AppShell` signal-ownership violations**: moving signal ownership out of
  `AppShell`, or child components writing its signals directly instead of
  receiving handlers.
- **Full-config serialization replacing text-preserving saves**: swapping a
  normal save path for full `PipelineConfig` serialization — the YAML text is
  authoritative; round-tripping through the struct loses comments, ordering,
  and unrecognized fields.
- **Bypassing `pipeline_view` / `GitOps`**: deriving canvas or view-model
  state outside `pipeline_view`, or shelling out to `git` directly instead of
  going through `GitOps`.
- **Dropping a CI clippy pass**: removing or weakening either clippy
  invocation — the default and `--all-targets` passes check different target
  sets and both are load-bearing.
- **Dependency pin drift**: adding or bumping dependencies outside the root
  `Cargo.toml` pins without approval.

### 7. Schema / type loosening

- Named struct → `serde_yaml::Value` / `serde_json::Value` to silence a schema
  error; enum variants collapsed to a catch-all

**Correct alternative**: the schema fix is the work, not its avoidance.

## Forbidden rationalization phrases

Treat any of these in a commit message or PR description as a stop signal and
re-examine the underlying decision: "simplest fix", "minimum-churn",
"pragmatic shortcut", "deferred for now", "good enough", "probably" /
"likely" without code-read evidence, "vestigial" without proof, "other tests
cover this" without naming the specific tests.

## Verification gates at close

The close gate is owned by the Definition Of Done in the root
[AGENTS.md](../../AGENTS.md#definition-of-done).
