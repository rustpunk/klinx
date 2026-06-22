//! Compile the active pipeline into a [`CompiledPlan`] so the composition
//! drill, the in-context body overlay (#171), and the inspector provenance
//! panel light up in the live app (#184).
//!
//! `compiled_plan` was initialized to `None` and never written outside test
//! fixtures, so every consumer that reads it (`resolve_composition_frame`,
//! `body_overlay`, `provenance`) silently took the no-plan path. This hook owns
//! the single side effect that populates it.
//!
//! The effect is a sibling of the schema-validation effect in
//! [`crate::hooks::pipeline_sync`]: it keys on `(pipeline, workspace,
//! active_file)` and re-derives whenever any of them changes. `pipeline` is
//! already debounced upstream (the ~150ms parse debounce in `AppShell`), so
//! compile runs at most once per typing pause, never per keystroke. A pipeline
//! with no composition nodes skips the workspace `.comp.yaml` scan entirely, so
//! the common case is cheap.
//!
//! The compile itself runs **off the render thread**. A composition pipeline
//! drives the engine's recursive workspace `.comp.yaml` scan, which is too heavy
//! to run synchronously inside `use_effect` on the desktop UI thread — doing so
//! blocks rendering on every debounced parse. Instead the owned
//! [`PipelineConfig`] is moved into [`tokio::task::spawn_blocking`] (Dioxus
//! desktop's `spawn` runs futures in a tokio runtime context), and only the
//! cheap `compiled_plan.set` happens back on the UI thread. This relies on
//! `CompiledPlan: Send + Sync` and `PipelineConfig: Send + Sync` (asserted by the
//! `const _` below) so the config can cross the thread boundary and the plan can
//! come back.
//!
//! Each compile carries a monotonic **generation**. The effect bumps the
//! generation at the start of EVERY run — including the no-pipeline / no-anchor
//! clears, not just dispatches — and the async continuation drops its result if a
//! newer run has since occurred, so a slow in-flight compile can never clobber a
//! newer pipeline's plan nor resurrect a stale plan over a clear. The generation
//! and `compiled_plan` signals are read only via `.peek()` inside the
//! effect/async block, so the effect does not subscribe to them and cannot loop.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use dioxus::prelude::*;

use clinker_core_types::Diagnostic;
use clinker_plan::config::{CompileContext, PipelineConfig, PipelineNode};
use clinker_plan::plan::CompiledPlan;

use crate::state::CompositionDiagnostic;
use crate::workspace::Workspace;

/// Engine diagnostic codes for composition binding failures that survive on the
/// SUCCESS path of [`PipelineConfig::compile_with_diagnostics`].
///
/// The engine's final non-fatal gate keeps any error whose code starts with
/// `"E10"` and drops the offending composition node from the DAG, returning the
/// rest as `Ok` (`clinker-plan` `config/pipeline.rs`: `!d.code.starts_with("E10")`
/// is the fatal predicate). So E101–E109 ride along on the Ok path — exactly the
/// silent no-op #187 surfaces — while a node-named diagnostic attributes to that
/// node. (E111 empty-body and E112 runtime-depth do NOT start with `"E10"`, so
/// they are fatal / runtime and never reach this filter; the fatal `Err`-path
/// diagnostics are surfaced separately by [`build_compile_error_diagnostics`] —
/// #189 — without going through this Ok-path set.)
///
/// Listed explicitly rather than prefix-matched so the set is a deliberate
/// decision. In practice the node-attributable plan-compile codes are E101–E104,
/// E107, and E108; E105/E106/E109 are kept for registry parity but are not emitted
/// by the plan composition-bind path, so they simply never match.
const COMPOSITION_DIAGNOSTIC_CODES: &[&str] = &[
    "E101", "E102", "E103", "E104", "E105", "E106", "E107", "E108", "E109",
];

const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<CompiledPlan>();
    assert_send_sync::<PipelineConfig>();
};

/// Wire the active pipeline → `compiled_plan` compile effect.
///
/// Compositions resolve against the workspace root (the recursive `.comp.yaml`
/// signature scan) and the active file's workspace-relative directory (relative
/// `use:` paths — see [`pipeline_dir_for`]). Without a workspace, a saved file
/// still compiles against its own directory (see [`compile_root`]) so provenance
/// lights up for single-file pipelines too. When there is no pipeline, nothing to
/// anchor compilation, or the pipeline fails to compile, the signal is cleared to
/// `None`; every reader already tolerates `None` by taking the no-plan path.
///
/// The compile runs off the render thread (`spawn_blocking`) under a generation
/// guard; see the module docs. `active_file` is a deduping [`Memo`] (built in
/// `AppShell`) over the active tab's path, so a Save-As — which writes the
/// `tabs` signal — re-fires this effect, while unrelated tab-snapshot churn does
/// not.
///
/// Composition / compile diagnostics ride through `composition_diagnostics` under
/// the same generation guard, so the canvas / inspector can flag the offending
/// node (or the YAML error bar a pipeline-level failure) instead of silently
/// no-opping. Two paths feed it:
/// - **Non-fatal (#187):** a successful compile returns the E101–E109 diagnostics
///   for any `composition` node whose `use:` failed to bind; the plan is still
///   set, the node flagged.
/// - **Hard failure (#189):** a failed compile (E111 empty-body, E200 type error,
///   …) carries no plan, but its error-severity diagnostics are still surfaced
///   (the plan is cleared, the diagnostics set). They are cleared only when there
///   is nothing to compile or the blocking compile panics.
///
/// All signals are passed by value (`Signal<T>` / `Memo<T>` are `Copy`).
pub fn use_compiled_plan(
    pipeline: Signal<Option<PipelineConfig>>,
    workspace: Signal<Option<Workspace>>,
    active_file: Memo<Option<PathBuf>>,
    mut compiled_plan: Signal<Option<Arc<CompiledPlan>>>,
    mut composition_diagnostics: Signal<Vec<CompositionDiagnostic>>,
) {
    // Monotonic compile generation. Declared once, before the effect, so it
    // survives across re-runs and lets a newer dispatch invalidate an older
    // in-flight compile. Read only via `.peek()` inside the effect/async block
    // so the effect never subscribes to it.
    let mut compile_gen = use_signal(|| 0u64);

    use_effect(move || {
        // Subscribe to the three inputs that change the compiled output. A tab
        // switch resets `pipeline` (from the arriving tab's snapshot); a Save-As
        // changes `active_file`. Both correctly re-fire the compile.
        let pl = (pipeline)();
        let ws = (workspace)();
        let af = (active_file)();

        // Bump the generation FIRST — before the early-return clears below — so
        // that EVERY effect run, including the no-pipeline / no-anchor clears,
        // supersedes any in-flight compile. Otherwise a slow compile dispatched
        // under a prior valid pipeline would, on resume, still match its
        // generation and resurrect a stale plan over a clear (e.g. the user
        // breaks the YAML while a compile is in flight → `pipeline` → None →
        // clear → the old compile lands a body for a pipeline that no longer
        // parses). Read via `.peek()` so the effect never subscribes to it.
        let generation = *compile_gen.peek() + 1;
        compile_gen.set(generation);

        // Own the config — it moves into `spawn_blocking` below, so it cannot be
        // a borrow into `pl`.
        let Some(config) = pl else {
            // No pipeline (parse error / partial / empty tab): nothing to
            // compile. Clear so a stale plan never lingers under a freshly
            // broken pipeline.
            clear_plan(&mut compiled_plan);
            clear_diagnostics(&mut composition_diagnostics);
            return;
        };

        let Some((root, pipeline_dir)) = compile_root(ws.as_ref(), af.as_deref()) else {
            // Neither a workspace nor a saved file to anchor compilation against
            // (an unsaved scratch tab) — nothing to compile.
            clear_plan(&mut compiled_plan);
            clear_diagnostics(&mut composition_diagnostics);
            return;
        };

        // Run the heavy compile off the UI thread. Only the `spawn_blocking`
        // closure runs on the blocking pool; the async continuation (the
        // generation check and the `compiled_plan` write) resumes on the Dioxus
        // desktop executor that polls `spawn` — the same thread that owns the
        // signal — so writing `compiled_plan` here is sound.
        spawn(async move {
            // `spawn_blocking(...).await` is `Err` only if the blocking compile
            // panicked (a `JoinError`); `compile_active` itself always returns a
            // `CompileOutcome`, never a sentinel `None`.
            let outcome =
                tokio::task::spawn_blocking(move || compile_active(&config, &root, pipeline_dir))
                    .await
                    .ok();

            // A newer compile — or a clear — was dispatched while this one ran;
            // drop its result so it cannot clobber the fresher state.
            if *compile_gen.peek() != generation {
                return;
            }

            match outcome {
                // Successful compile: a plan plus any non-fatal composition
                // diagnostics (#187). Set-if-changed so a clean compile (the
                // common case, empty diagnostics) does not notify the
                // canvas/inspector on every debounced re-compile.
                Some(CompileOutcome {
                    plan: Some(plan),
                    diagnostics,
                }) => {
                    compiled_plan.set(Some(plan));
                    set_diagnostics(&mut composition_diagnostics, diagnostics);
                }
                // Hard compile failure (#189): there is no plan, but the
                // error-severity diagnostics still ride through
                // `composition_diagnostics` so the canvas flags the offending
                // composition node and the YAML error bar explains a
                // pipeline-level failure — instead of a silent drop to the raw
                // fallback. Clear the plan so resolved-mode readers take the
                // no-plan path.
                Some(CompileOutcome {
                    plan: None,
                    diagnostics,
                }) => {
                    clear_plan(&mut compiled_plan);
                    set_diagnostics(&mut composition_diagnostics, diagnostics);
                }
                // The blocking compile panicked: nothing trustworthy to show.
                // Clear both so dependent surfaces take the no-plan path.
                None => {
                    clear_plan(&mut compiled_plan);
                    clear_diagnostics(&mut composition_diagnostics);
                }
            }
        });
    });
}

/// Clear `compiled_plan` only when one is currently set, so a run with nothing
/// to compile never notifies readers redundantly. Shared by the synchronous
/// no-pipeline / no-anchor early returns and the async compile-failure branch
/// (the success path is deliberately unguarded — a fresh plan always carries new
/// information).
fn clear_plan(compiled_plan: &mut Signal<Option<Arc<CompiledPlan>>>) {
    if compiled_plan.peek().is_some() {
        compiled_plan.set(None);
    }
}

/// Clear composition diagnostics only when some are currently set, so a run with
/// nothing to compile never notifies readers redundantly. Mirrors [`clear_plan`].
fn clear_diagnostics(diagnostics: &mut Signal<Vec<CompositionDiagnostic>>) {
    if !diagnostics.peek().is_empty() {
        diagnostics.set(Vec::new());
    }
}

/// Replace composition diagnostics only when they differ from the current set.
/// A clean compile (the common case) yields an empty `next`, so this no-ops when
/// the signal is already empty — keeping a debounced re-compile from re-rendering
/// the canvas/inspector on every keystroke pause.
fn set_diagnostics(
    diagnostics: &mut Signal<Vec<CompositionDiagnostic>>,
    next: Vec<CompositionDiagnostic>,
) {
    if *diagnostics.peek() != next {
        diagnostics.set(next);
    }
}

/// Resolve the workspace root and workspace-relative pipeline directory the
/// engine compiles against.
///
/// Prefers the loaded workspace — its recursive `.comp.yaml` scan is what lets
/// composition `use:` references resolve, so compositions only light up in the
/// workspace case. Falls back to the active file's own directory when no
/// workspace is loaded, so a saved single-file pipeline still compiles and lights
/// up inspector provenance even outside a workspace (the root is irrelevant to a
/// composition-free pipeline). Returns `None` only when there is neither a
/// workspace nor a saved file to anchor compilation (an unsaved scratch tab).
fn compile_root(ws: Option<&Workspace>, active_file: Option<&Path>) -> Option<(PathBuf, PathBuf)> {
    // Canonicalize the root and the active file used to derive `pipeline_dir`.
    // The engine documents `CompileContext.workspace_root` as canonical; a
    // symlinked or `..`-laden root would shift the `.comp.yaml` symbol-table keys
    // relative to `pipeline_dir`, forcing the weaker filename-match fallback.
    // Fall back to the path as-is when it does not yet exist on disk (e.g. tests
    // and unsaved-but-named files), so canonicalize failure is non-fatal.
    //
    // Deliberately localized here rather than canonicalizing `Workspace.root` at
    // load: the canonical form is needed only for the engine's symbol-table
    // keying, and keeping it out of `Workspace.root` avoids leaking a
    // canonicalized path (a `\\?\` verbatim path on Windows) into session
    // persistence, the title bar, and the last-workspace tracker.
    let canonical = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());

    if let Some(ws) = ws {
        let root = canonical(&ws.root);
        let af = active_file.map(canonical);
        return Some((root.clone(), pipeline_dir_for(af.as_deref(), &root)));
    }
    // No workspace: anchor on the active file's own directory. `Path::parent` of
    // a bare filename is `Some("")` (not `None`), which would wrongly compile
    // against an empty root — filter that out.
    let dir = active_file
        .and_then(Path::parent)
        .filter(|p| !p.as_os_str().is_empty())?;
    Some((canonical(dir), PathBuf::new()))
}

/// The result of one off-thread compile.
///
/// Both compile outcomes carry diagnostics, so neither is silent:
/// - **Success** — `plan: Some` plus any non-fatal composition-binding
///   diagnostics (#187): a `composition` node whose `use:` failed to bind is
///   dropped from the DAG but the rest of the pipeline still compiles.
/// - **Hard failure** — `plan: None` plus the error-severity diagnostics that
///   explain why the compiled tooling went dark (#189). The engine's fatal gate
///   fires on any error-severity diagnostic whose code does **not** start with
///   `"E10"` (e.g. E111 empty-body, E200 CXL type error, E153), discarding the
///   plan; surfacing those diagnostics is what keeps a hard failure from looking
///   like a node that simply has no composition body.
struct CompileOutcome {
    plan: Option<Arc<CompiledPlan>>,
    diagnostics: Vec<CompositionDiagnostic>,
}

/// Compile `config` against `workspace_root`, resolving relative `use:` paths
/// from `pipeline_dir`, into a [`CompileOutcome`].
///
/// This is the pure core of [`use_compiled_plan`], extracted so the resolution
/// path — the workspace `.comp.yaml` scan plus relative `use:` binding — is
/// exercisable against real workspace assets without a render harness.
fn compile_active(
    config: &PipelineConfig,
    workspace_root: &Path,
    pipeline_dir: PathBuf,
) -> CompileOutcome {
    let ctx = CompileContext::with_pipeline_dir(workspace_root, pipeline_dir);
    match config.compile_with_diagnostics(&ctx) {
        // Success path: `compile_with_diagnostics` returns the non-fatal
        // diagnostics (including the E101–E109 composition-binding errors that
        // the plain `compile` discarded); a dropped composition node still yields
        // `Ok` with the node omitted from the DAG.
        Ok((plan, engine_diagnostics)) => {
            let failed = failed_composition_nodes(config, &plan);
            let diagnostics = build_composition_diagnostics(&failed, &engine_diagnostics);
            CompileOutcome {
                plan: Some(Arc::new(plan)),
                diagnostics,
            }
        }
        // Hard failure path (#189): the whole `Err(Vec<Diagnostic>)` was
        // previously discarded, so a pipeline that parses but fails to compile
        // (E111 empty-body, E200 type error, …) dropped the resolved/compiled
        // tooling to the raw fallback with no surfaced reason. Capture the
        // error-severity diagnostics instead.
        Err(engine_diagnostics) => CompileOutcome {
            plan: None,
            diagnostics: build_compile_error_diagnostics(config, &engine_diagnostics),
        },
    }
}

/// Build user-facing diagnostics from a HARD compile failure — the `Err` path of
/// [`PipelineConfig::compile_with_diagnostics`] (#189).
///
/// Unlike the success path there is no plan, so the body-assignment diff that
/// [`failed_composition_nodes`] uses to identify a dropped composition is
/// unavailable; attribution falls back to the engine message. Each error-severity
/// diagnostic becomes a [`CompositionDiagnostic`] that is either:
/// - attributed to a `composition` node (`node: Some`) when its message carries
///   the engine's `composition node "X":` prefix (e.g. E111 `composition node
///   "clean": body file … has zero nodes`), so the canvas flags that node exactly
///   like the non-fatal #187 case; or
/// - kept pipeline-level (`node: None`) otherwise (e.g. an E200 type error in a
///   transform), which the YAML error bar surfaces independently of selection.
///
/// Attribution uses the full `composition node "X"` prefix
/// ([`diagnostic_attributes_to_composition`]), NOT a bare quoted-name match: on
/// this path there is no `is_composition_diagnostic` code filter nor
/// body-assignment diff to guard a stray match, so a hard error that merely quotes
/// a field or rule named like a composition (e.g. an E200 `order_by field
/// "clean"`) must not be mis-attributed to that composition.
///
/// Warnings are dropped: only the error-severity diagnostics explain the failure
/// (the fatal gate fires on an error-severity, non-`E10x` code), and the non-fatal
/// warnings have no plan to annotate here.
fn build_compile_error_diagnostics(
    config: &PipelineConfig,
    engine_diagnostics: &[Diagnostic],
) -> Vec<CompositionDiagnostic> {
    let composition_names: Vec<&str> = composition_node_names(config).collect();
    engine_diagnostics
        .iter()
        .filter(|diagnostic| matches!(diagnostic.severity, clinker_core_types::Severity::Error))
        .map(|diagnostic| CompositionDiagnostic {
            node: composition_names
                .iter()
                .copied()
                .find(|name| diagnostic_attributes_to_composition(&diagnostic.message, name))
                .map(str::to_string),
            code: diagnostic.code.clone(),
            message: diagnostic.message.clone(),
        })
        .collect()
}

/// True when a HARD compile error `message` attributes to composition `node` —
/// i.e. it carries the engine's `composition node "X":` prefix (the quoted
/// `{node:?}` form), as E111 empty-body does.
///
/// Stricter than [`diagnostic_names_node`], which matches the bare quoted name
/// anywhere. On the success path the bare match is safe because it only *enriches*
/// a node the body-assignment diff already flagged, gated by the E10x code filter.
/// The Err path has neither guard, so it must require the `composition node`
/// prefix or an unrelated hard error quoting a like-named field/rule would be
/// mis-attributed to a correctly-bound composition.
fn diagnostic_attributes_to_composition(message: &str, node: &str) -> bool {
    message.contains(&format!("composition node {node:?}"))
}

/// The names of every `composition` node in `config` that the engine dropped from
/// the compiled DAG — i.e. declared in the config but absent from
/// [`composition_body_assignments`](clinker_plan::plan::bind_schema::CompileArtifacts::composition_body_assignments).
///
/// This body-assignment diff is the authoritative source of "which compositions
/// failed to bind": it reads the engine's own record of what bound, so it is
/// immune to diagnostic message-format drift. The engine messages then only
/// *enrich* each failed node with a reason (see [`build_composition_diagnostics`]).
fn failed_composition_nodes(config: &PipelineConfig, plan: &CompiledPlan) -> Vec<String> {
    let bound = &plan.artifacts().composition_body_assignments;
    composition_node_names(config)
        .map(str::to_string)
        .filter(|name| !bound.contains_key(name))
        .collect()
}

/// The names of every `composition` node declared in `config`, in declaration
/// order. The shared projection behind both the success-path body-assignment diff
/// ([`failed_composition_nodes`]) and the Err-path attribution
/// ([`build_compile_error_diagnostics`]).
fn composition_node_names(config: &PipelineConfig) -> impl Iterator<Item = &str> {
    config
        .nodes
        .iter()
        .filter(|node| matches!(node.value, PipelineNode::Composition { .. }))
        .map(|node| node.value.name())
}

/// True when `code` is one of the engine's composition binding/expansion
/// diagnostics (the [`COMPOSITION_DIAGNOSTIC_CODES`] set).
fn is_composition_diagnostic(code: &str) -> bool {
    COMPOSITION_DIAGNOSTIC_CODES.contains(&code)
}

/// True when `message` names composition node `node`.
///
/// The engine formats node names with `{node_name:?}` — i.e. quoted —
/// (`composition node "clean": …`), so matching the quoted form avoids a
/// substring collision between e.g. `clean` and `clean_extra`. A failure of this
/// heuristic only costs a richer message (the node is still flagged from the
/// body-assignment diff and gets the generic fallback in
/// [`build_composition_diagnostics`]); a test pins the engine's current format.
fn diagnostic_names_node(message: &str, node: &str) -> bool {
    message.contains(&format!("{node:?}"))
}

/// Build the user-facing composition diagnostics for a successful compile.
///
/// `failed_nodes` (from [`failed_composition_nodes`]) is the authoritative set of
/// dropped composition nodes and drives the canvas flag. For each, the engine's
/// composition diagnostics that name it supply the *why* (one entry per matching
/// message, preserving the engine `code`). A dropped node that no engine message
/// named still gets a single generic entry, so a flagged node is never left
/// without a reason to show.
fn build_composition_diagnostics(
    failed_nodes: &[String],
    engine_diagnostics: &[Diagnostic],
) -> Vec<CompositionDiagnostic> {
    let mut out = Vec::new();
    for node in failed_nodes {
        let before = out.len();
        for diagnostic in engine_diagnostics
            .iter()
            .filter(|d| is_composition_diagnostic(&d.code))
            .filter(|d| diagnostic_names_node(&d.message, node))
        {
            out.push(CompositionDiagnostic {
                node: Some(node.clone()),
                code: diagnostic.code.clone(),
                message: diagnostic.message.clone(),
            });
        }
        if out.len() == before {
            out.push(CompositionDiagnostic {
                node: Some(node.clone()),
                code: String::new(),
                message: format!(
                    "composition `{node}` failed to bind — its `use:` body was \
                     dropped from the compiled pipeline"
                ),
            });
        }
    }
    out
}

/// Workspace-relative directory of the pipeline file, for resolving relative
/// `use:` paths during compile. An empty path means "workspace root" (the
/// pipeline file sits directly at the root).
///
/// Returns the empty path when the file is unknown (unsaved tab) or lies outside
/// the workspace — in those cases the engine's filename-match fallback still
/// resolves a `use:` reference as long as the `.comp.yaml` basename is unique in
/// the workspace; a correct `pipeline_dir` only matters to disambiguate
/// duplicate basenames in different directories.
fn pipeline_dir_for(file_path: Option<&Path>, ws_root: &Path) -> PathBuf {
    file_path
        .and_then(|p| p.strip_prefix(ws_root).ok())
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_dir_is_empty_for_file_at_workspace_root() {
        let dir = pipeline_dir_for(Some(Path::new("/ws/flow.yaml")), Path::new("/ws"));
        assert_eq!(dir, PathBuf::new());
    }

    #[test]
    fn pipeline_dir_is_the_relative_parent_for_a_nested_file() {
        let dir = pipeline_dir_for(
            Some(Path::new("/ws/pipelines/etl/flow.yaml")),
            Path::new("/ws"),
        );
        assert_eq!(dir, PathBuf::from("pipelines/etl"));
    }

    #[test]
    fn pipeline_dir_is_empty_when_file_is_outside_the_workspace() {
        // strip_prefix fails → fall back to the workspace root (empty dir). The
        // engine's filename-match fallback covers a unique-basename `use:` here.
        let dir = pipeline_dir_for(Some(Path::new("/other/flow.yaml")), Path::new("/ws"));
        assert_eq!(dir, PathBuf::new());
    }

    #[test]
    fn pipeline_dir_is_empty_for_an_unsaved_tab() {
        let dir = pipeline_dir_for(None, Path::new("/ws"));
        assert_eq!(dir, PathBuf::new());
    }

    fn workspace_at(root: &str) -> Workspace {
        Workspace {
            root: PathBuf::from(root),
            manifest: Default::default(),
            state: Default::default(),
        }
    }

    #[test]
    fn compile_root_prefers_the_workspace_and_its_relative_dir() {
        let ws = workspace_at("/ws");
        let got = compile_root(Some(&ws), Some(Path::new("/ws/pipelines/flow.yaml")));
        assert_eq!(
            got,
            Some((PathBuf::from("/ws"), PathBuf::from("pipelines"))),
        );
    }

    #[test]
    fn compile_root_falls_back_to_the_file_directory_without_a_workspace() {
        // A saved single-file pipeline still compiles (provenance) against its
        // own directory, with an empty pipeline_dir.
        let got = compile_root(None, Some(Path::new("/home/me/flow.yaml")));
        assert_eq!(got, Some((PathBuf::from("/home/me"), PathBuf::new())));
    }

    #[test]
    fn compile_root_is_none_with_no_workspace_and_no_file() {
        // An unsaved scratch tab has nothing to anchor compilation against.
        assert_eq!(compile_root(None, None), None);
    }

    #[test]
    fn compile_root_is_none_with_no_workspace_and_a_bare_filename() {
        // `Path::parent("flow.yaml")` is `Some("")`, not `None`; without the
        // empty-path filter this would wrongly compile against an empty root.
        assert_eq!(compile_root(None, Some(Path::new("flow.yaml"))), None);
    }

    /// End-to-end proof that the production compile core (`compile_active`)
    /// resolves a composition body against real workspace assets — the exact
    /// path `use_compiled_plan` drives in the live app, minus the Dioxus shell.
    /// Guards the wiring the issue (#184) lit up: a populated `compiled_plan`
    /// whose DAG carries a bound body the drill / overlay can open.
    #[test]
    fn compiles_example_composition_pipeline_and_resolves_its_body() {
        use clinker_plan::config::parse_config;

        let ws_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/pipelines")
            .canonicalize()
            .expect("examples workspace exists");
        let yaml = std::fs::read_to_string(ws_root.join("customer_clean.yaml"))
            .expect("read the example composition pipeline");

        let config = parse_config(&yaml).expect("example pipeline parses");
        // The pipeline file sits at the workspace root → empty pipeline_dir.
        let outcome = compile_active(&config, &ws_root, PathBuf::new());
        let plan = outcome
            .plan
            .expect("example pipeline compiles against the examples workspace");
        let diagnostics = outcome.diagnostics;

        let frame = crate::state::resolve_composition_frame(&plan, "clean")
            .expect("composition node `clean` binds to a body in the compiled plan");
        assert!(
            plan.body_of(frame.body_id).is_some(),
            "the resolved body is present in the compiled plan",
        );
        assert!(
            diagnostics.is_empty(),
            "a cleanly-binding example pipeline carries no composition diagnostics, got {diagnostics:?}",
        );
    }

    /// The #187 headline scenario: a `composition` node whose `use:` path does not
    /// resolve compiles non-fatally (the plan is still populated) but the node is
    /// dropped from the DAG — and `compile_active` now surfaces that as an E103
    /// diagnostic keyed to the node, instead of the old silent no-op.
    #[test]
    fn mispathed_use_surfaces_an_e103_diagnostic_for_the_node() {
        use clinker_plan::config::parse_config;

        let ws_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/pipelines")
            .canonicalize()
            .expect("examples workspace exists");

        // A composition node pointing at a non-existent `.comp.yaml`. The output
        // reads from the source (not the broken composition) so dropping `clean`
        // still leaves a valid, compilable DAG.
        let yaml = r#"
pipeline:
  name: broken_use
nodes:
  - type: source
    name: people
    config:
      name: people
      type: csv
      path: ./data/people.csv
      schema:
        - { name: first_name, type: string }
        - { name: last_name, type: string }
  - type: composition
    name: clean
    input: people
    use: ./compositions/does_not_exist.comp.yaml
    inputs:
      names: people
  - type: output
    name: out
    input: people
    config:
      name: out
      type: csv
      path: ./data/out.csv
"#;

        let config = parse_config(yaml).expect("pipeline parses");
        let outcome = compile_active(&config, &ws_root, PathBuf::new());
        let plan = outcome
            .plan
            .expect("a mis-pathed `use:` is non-fatal: the plan still compiles");
        let diagnostics = outcome.diagnostics;

        // The body is dropped → the drill would resolve to nothing …
        assert!(
            crate::state::resolve_composition_frame(&plan, "clean").is_none(),
            "the mis-pathed composition node has no bound body",
        );
        // … but the failure is no longer silent.
        let clean: Vec<_> = diagnostics
            .iter()
            .filter(|d| d.node.as_deref() == Some("clean"))
            .collect();
        assert!(
            !clean.is_empty(),
            "the dropped composition node is flagged, got {diagnostics:?}",
        );
        assert!(
            clean.iter().any(|d| d.code == "E103"),
            "the mis-pathed `use:` surfaces as E103, got {clean:?}",
        );
    }

    fn diag(code: &str, message: &str) -> Diagnostic {
        use clinker_core_types::span::{FileId, Span};
        use std::num::NonZeroU32;
        let file = FileId::new(NonZeroU32::new(1).expect("1 is non-zero"));
        Diagnostic::error(
            code,
            message,
            clinker_core_types::LabeledSpan::new(Span::point(file, 0), None),
        )
    }

    #[test]
    fn is_composition_diagnostic_matches_the_e10x_family_only() {
        assert!(is_composition_diagnostic("E101"));
        assert!(is_composition_diagnostic("E103"));
        assert!(is_composition_diagnostic("E108"));
        assert!(is_composition_diagnostic("E109"));
        // Not on the non-fatal Ok-path set: E111/E112 are fatal/runtime (do not
        // start with "E10"); E110 is an unused gap; E200/E004 are non-composition.
        assert!(!is_composition_diagnostic("E110"));
        assert!(!is_composition_diagnostic("E111"));
        assert!(!is_composition_diagnostic("E112"));
        assert!(!is_composition_diagnostic("E200"));
        assert!(!is_composition_diagnostic("E004"));
    }

    #[test]
    fn diagnostic_names_node_matches_the_quoted_engine_format() {
        // The engine emits `composition node "clean": …` (the `{:?}` quoted form).
        let msg = r#"composition node "clean": `use: x` does not match any .comp.yaml"#;
        assert!(diagnostic_names_node(msg, "clean"));
        // A prefix must not match a longer name and vice-versa (the quoting guards
        // against the `clean` / `clean_extra` substring collision).
        assert!(!diagnostic_names_node(msg, "clea"));
        let msg_extra = r#"composition node "clean_extra": broken"#;
        assert!(!diagnostic_names_node(msg_extra, "clean"));
        assert!(diagnostic_names_node(msg_extra, "clean_extra"));
    }

    #[test]
    fn build_composition_diagnostics_attributes_engine_messages_to_failed_nodes() {
        let failed = vec!["clean".to_string()];
        let engine = vec![
            diag(
                "E103",
                r#"composition node "clean": `use: x` does not match"#,
            ),
            // A non-composition diagnostic is ignored.
            diag("E200", r#"composition node "clean": type error"#),
            // A composition diagnostic for a different (bound) node is ignored.
            diag("E104", r#"composition node "other": missing input"#),
        ];
        let got = build_composition_diagnostics(&failed, &engine);
        assert_eq!(got.len(), 1, "only the matching E103 is surfaced: {got:?}");
        assert_eq!(got[0].node.as_deref(), Some("clean"));
        assert_eq!(got[0].code, "E103");
    }

    #[test]
    fn build_composition_diagnostics_synthesizes_a_fallback_for_unnamed_failures() {
        // A dropped node with no engine message naming it (e.g. a cycle whose
        // message lists paths, not the node) still gets a flagged entry.
        let failed = vec!["loop_node".to_string()];
        let engine = vec![diag(
            "E107",
            "cycle in composition `use:` graph: a.comp.yaml -> b.comp.yaml -> a.comp.yaml",
        )];
        let got = build_composition_diagnostics(&failed, &engine);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].node.as_deref(), Some("loop_node"));
        assert_eq!(got[0].code, "", "fallback entry carries no engine code");
        assert!(got[0].message.contains("loop_node"));
    }

    #[test]
    fn build_composition_diagnostics_keeps_every_message_for_a_node() {
        // A schema mismatch can emit several E102s for one node (one per column);
        // all are preserved so the inspector lists each.
        let failed = vec!["clean".to_string()];
        let engine = vec![
            diag("E102", r#"composition node "clean": column "a" missing"#),
            diag("E102", r#"composition node "clean": column "b" missing"#),
        ];
        let got = build_composition_diagnostics(&failed, &engine);
        assert_eq!(got.len(), 2, "both column errors surface: {got:?}");
        assert!(got.iter().all(|d| d.node.as_deref() == Some("clean")));
    }

    fn warn(code: &str, message: &str) -> Diagnostic {
        use clinker_core_types::span::{FileId, Span};
        use std::num::NonZeroU32;
        let file = FileId::new(NonZeroU32::new(1).expect("1 is non-zero"));
        Diagnostic::warning(
            code,
            message,
            clinker_core_types::LabeledSpan::new(Span::point(file, 0), None),
        )
    }

    /// A single composition pipeline config (a `clean` composition + an `out`
    /// output) for exercising the Err-path attribution against a real
    /// `PipelineConfig::nodes` taxonomy. The `use:` need not resolve — these tests
    /// feed synthetic diagnostics, not a live compile.
    fn config_with_composition_named(name: &str) -> PipelineConfig {
        use clinker_plan::config::parse_config;
        let yaml = format!(
            r#"
pipeline:
  name: hard_fail
nodes:
  - type: source
    name: people
    config:
      name: people
      type: csv
      path: ./data/people.csv
      schema:
        - {{ name: first_name, type: string }}
  - type: composition
    name: {name}
    input: people
    use: ./compositions/x.comp.yaml
    inputs:
      names: people
  - type: output
    name: out
    input: people
    config:
      name: out
      type: csv
      path: ./data/out.csv
"#
        );
        parse_config(&yaml).expect("pipeline parses")
    }

    #[test]
    fn build_compile_error_diagnostics_attributes_composition_and_keeps_rest_pipeline_level() {
        let config = config_with_composition_named("clean");
        let engine = vec![
            // Names the composition node (quoted) → attributed so the canvas flags it.
            diag(
                "E111",
                r#"composition node "clean": body file x.comp.yaml has zero nodes"#,
            ),
            // A hard error naming a non-composition node → pipeline-level (the YAML
            // error bar shows it; per-node CXL attribution is #161's job, not this).
            diag("E200", r#"transform node "out": type error in cxl"#),
            // A warning is dropped: only error-severity diagnostics explain a hard
            // failure, and there is no plan to annotate with non-fatal warnings.
            warn("W101", r#"composition node "clean": deprecated form"#),
        ];

        let got = build_compile_error_diagnostics(&config, &engine);

        assert_eq!(got.len(), 2, "the warning is dropped, got {got:?}");
        let e111 = got
            .iter()
            .find(|d| d.code == "E111")
            .expect("E111 is surfaced");
        assert_eq!(
            e111.node.as_deref(),
            Some("clean"),
            "an error naming the composition is attributed to it",
        );
        let e200 = got
            .iter()
            .find(|d| d.code == "E200")
            .expect("E200 is surfaced");
        assert_eq!(
            e200.node, None,
            "an error not naming a composition stays pipeline-level",
        );
    }

    #[test]
    fn build_compile_error_diagnostics_does_not_misattribute_a_field_named_like_a_composition() {
        // A composition node named `clean` binds fine, but an unrelated hard error
        // quotes a FIELD also named "clean". Attribution must require the engine's
        // `composition node "clean"` prefix — a bare quoted-name match would flag
        // the innocent composition and hide the real error from the bar.
        let config = config_with_composition_named("clean");
        let engine = vec![diag(
            "E200",
            r#"reshape node "shape": order_by field "clean" is not present in the upstream schema"#,
        )];

        let got = build_compile_error_diagnostics(&config, &engine);

        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(
            got[0].node, None,
            "a hard error merely quoting a field named like a composition stays pipeline-level",
        );
    }

    /// The #189 headline scenario, end-to-end through the production compile core.
    /// An empty-body composition is a HARD compile failure: E111 does not start
    /// `"E10"`, so the engine's fatal gate discards the plan. Previously the whole
    /// `Err` was dropped and the node silently vanished; `compile_active` now
    /// returns no plan but surfaces the E111 attributed to the node so the canvas
    /// flags it. Also guards against engine E111 message-format drift (the
    /// node-attribution relies on the quoted `{node:?}` form).
    #[test]
    fn empty_body_composition_is_a_hard_failure_surfaced_for_the_node() {
        use clinker_plan::config::parse_config;

        // An isolated temp workspace holding one empty-body `.comp.yaml`, so the
        // shared `examples/` workspace scan is untouched.
        let tmp = std::env::temp_dir().join("klinx_test_189_empty_body");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("compositions")).expect("create temp workspace");
        std::fs::write(
            tmp.join("compositions/empty.comp.yaml"),
            "_compose:\n  name: empty\n  inputs:\n    names:\n      schema:\n        \
             - { name: first_name, type: string }\n  outputs:\n    out: first_name\n  \
             config_schema: {}\nnodes: []\n",
        )
        .expect("write empty-body composition");

        let yaml = r#"
pipeline:
  name: empty_body
nodes:
  - type: source
    name: people
    config:
      name: people
      type: csv
      path: ./data/people.csv
      schema:
        - { name: first_name, type: string }
  - type: composition
    name: clean
    input: people
    use: ./compositions/empty.comp.yaml
    inputs:
      names: people
  - type: output
    name: out
    input: people
    config:
      name: out
      type: csv
      path: ./data/out.csv
"#;

        let config = parse_config(yaml).expect("pipeline parses");
        let outcome = compile_active(&config, &tmp, PathBuf::new());
        let _ = std::fs::remove_dir_all(&tmp);

        assert!(
            outcome.plan.is_none(),
            "an empty-body composition is a hard compile failure → no plan",
        );
        let e111: Vec<_> = outcome
            .diagnostics
            .iter()
            .filter(|d| d.code == "E111")
            .collect();
        assert_eq!(
            e111.len(),
            1,
            "exactly one E111 is surfaced, got {:?}",
            outcome.diagnostics,
        );
        assert_eq!(
            e111[0].node.as_deref(),
            Some("clean"),
            "the E111 is attributed to the offending composition node",
        );
    }
}
