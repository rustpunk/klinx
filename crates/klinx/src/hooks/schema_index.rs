//! Workspace schema index: rebuild the schema catalog whenever the workspace
//! changes.
//!
//! Owns one side effect, behavior-identical to its former inline form in
//! `AppShell`.

use dioxus::prelude::*;

use clinker_schema::SchemaIndex;

use crate::workspace::Workspace;

/// Rebuild `schema_index` from the active workspace on every workspace change.
///
/// Reads `workspace` reactively; writes the rebuilt index into `schema_index`
/// (cleared to the default when no workspace is loaded). Both signals are passed
/// by value (`Signal<T>` is `Copy`).
pub fn use_schema_index(
    workspace: Signal<Option<Workspace>>,
    mut schema_index: Signal<SchemaIndex>,
) {
    // ── Schema index: rebuild when workspace changes ─────────────────────
    use_effect(move || {
        let ws = (workspace)();
        if let Some(ref ws) = ws {
            let (index, _errors) = ws.build_schema_index();
            schema_index.set(index);
        } else {
            schema_index.set(SchemaIndex::default());
        }
    });
}
