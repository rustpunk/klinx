//! Pipeline edit-sync: the YAML <-> inspector <-> schema-validation loop.
//!
//! Owns three side effects, behavior-identical to their former inline form in
//! `AppShell`:
//! - YAML -> pipeline parse (gated on `EditSource::Yaml`),
//! - inspector -> YAML serialize (gated on `EditSource::Inspector`),
//! - schema validation (runs when the pipeline or schema index changes).
//!
//! The `EditSource` guards are load-bearing: they break the YAML<->inspector
//! feedback loop. A Yaml-sourced edit sets `pipeline`, which the serialize
//! effect subscribes to — but the serialize guard (`source != Inspector ->
//! return`) stops it from overwriting the YAML the user just typed. An
//! Inspector-sourced edit sets `yaml_text`, which the parse effect's debounce
//! upstream subscribes to — but the parse guard (`source != Yaml -> return`)
//! stops it from re-parsing inspector-authored YAML. The guards must stay
//! exactly as written; loosening either reopens the loop.

use dioxus::prelude::*;

use clinker_core::config::PipelineConfig;
use clinker_core::partial::PartialPipelineConfig;
use clinker_schema::{SchemaIndex, SchemaWarning};

use crate::perf::perf_trace;
use crate::pipeline_view::PipelineView;
use crate::sync::{
    EditSource, ParseResult, is_composition_yaml, parse_composition, try_parse_yaml,
};
use crate::workspace::Workspace;
use crate::yaml_patch::patch_yaml_preserving_nodes;

/// Wire up the YAML <-> pipeline <-> schema-validation edit-sync effects.
///
/// All signals are passed by value (`Signal<T>` is `Copy`). `parse_trigger` is
/// the debounced re-arm signal the parse effect keys on (bumped ~150ms after the
/// last keystroke by the debounce effect that remains in `AppShell`); without it
/// the parse effect would not re-run on debounced YAML edits.
#[allow(clippy::too_many_arguments)]
pub fn use_pipeline_sync(
    parse_trigger: Signal<u64>,
    edit_source: Signal<EditSource>,
    yaml_text: Signal<String>,
    workspace: Signal<Option<Workspace>>,
    mut pipeline: Signal<Option<PipelineConfig>>,
    mut partial_pipeline: Signal<Option<PartialPipelineConfig>>,
    mut composition_view: Signal<Option<PipelineView>>,
    mut parse_errors: Signal<Vec<String>>,
    schema_index: Signal<SchemaIndex>,
    mut schema_warnings: Signal<Vec<SchemaWarning>>,
) {
    // ── Sync effects: YAML ↔ pipeline model ──────────────────────────────
    {
        use_effect(move || {
            // Debounced trigger (keystrokes) + edit_source (immediate parse for
            // programmatic Yaml transitions: tab load, workspace re-resolve).
            let _ = (parse_trigger)();
            let source = (edit_source)();

            if source != EditSource::Yaml {
                return;
            }

            // Read text non-reactively: this effect fires only on the debounced
            // trigger / source change (never per keystroke) and always sees the
            // latest text, so there is no stale-text race.
            let text = yaml_text.peek().clone();

            // Composition documents (`_compose:`) take a separate parse path so
            // they validate as compositions — no spurious "missing required key:
            // pipeline" — and render their body DAG on the canvas.
            if is_composition_yaml(&text) {
                let (view, errors) = parse_composition(&text);
                composition_view.set(view);
                parse_errors.set(errors);
                pipeline.set(None);
                partial_pipeline.set(None);
                return;
            }
            // Not a composition: clear any stale composition view (e.g. switching
            // from a comp tab to a pipeline tab) and parse as a pipeline.
            composition_view.set(None);

            let ws_root = workspace.read().as_ref().map(|ws| ws.root.clone());

            let parse_result = perf_trace!(
                try_parse_yaml(&text, ws_root.as_deref()),
                "try_parse_yaml: {} bytes",
                text.len()
            );

            match parse_result {
                ParseResult::Complete(resolved) => {
                    pipeline.set(Some(resolved.resolved));
                    partial_pipeline.set(None);
                    parse_errors.set(Vec::new());
                }
                ParseResult::Partial(partial) => {
                    pipeline.set(None);
                    partial_pipeline.set(Some(partial.clone()));
                    parse_errors.set(partial.errors);
                }
                ParseResult::Failed(errors) => {
                    partial_pipeline.set(None);
                    parse_errors.set(errors);
                }
            }
        });
    }

    {
        let mut yaml_text = yaml_text;

        use_effect(move || {
            let source = (edit_source)();
            let pl_val = (pipeline)();

            if source != EditSource::Inspector {
                return;
            }

            if let Some(ref config) = pl_val {
                // `yaml_text` is the authoritative full document — it carries
                // the real `nodes:` block (which the engine serializer drops,
                // issue #29). Patch the edited node's region in place instead
                // of regenerating from `config`, so every other node's text and
                // the user's comments survive verbatim.
                //
                // Read the current text with `peek()` (non-reactive): this
                // effect must not subscribe to `yaml_text`, or writing it here
                // would re-fire the effect in a loop. The `EditSource` guards
                // (this `source != Inspector` early-return, plus the parse
                // effect's `source != Yaml` guard and the debounce's Yaml-only
                // re-arm) keep the YAML↔inspector loop broken; setting
                // `yaml_text` under `EditSource::Inspector` does not schedule a
                // parse, so the patched text is not fought by a re-parse.
                let current = yaml_text.peek().clone();
                let yaml = patch_yaml_preserving_nodes(&current, config);
                yaml_text.set(yaml);
                parse_errors.set(Vec::new());
            }
        });
    }

    // ── Schema validation: run when pipeline or schema index changes ──────
    use_effect(move || {
        let pl = (pipeline)();
        let idx = (schema_index)();
        let ws = (workspace)();

        if let (Some(config), Some(ws)) = (pl.as_ref(), ws.as_ref()) {
            let warnings = clinker_schema::validate_pipeline(config, &idx, &ws.root);
            schema_warnings.set(warnings);
        } else {
            schema_warnings.set(Vec::new());
        }
    });
}
