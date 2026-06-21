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

use clinker_plan::config::{CompileContext, PipelineConfig};
use clinker_plan::plan::CompiledPlan;

use crate::workspace::Workspace;

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
/// All signals are passed by value (`Signal<T>` / `Memo<T>` are `Copy`).
pub fn use_compiled_plan(
    pipeline: Signal<Option<PipelineConfig>>,
    workspace: Signal<Option<Workspace>>,
    active_file: Memo<Option<PathBuf>>,
    mut compiled_plan: Signal<Option<Arc<CompiledPlan>>>,
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
            return;
        };

        let Some((root, pipeline_dir)) = compile_root(ws.as_ref(), af.as_deref()) else {
            // Neither a workspace nor a saved file to anchor compilation against
            // (an unsaved scratch tab) — nothing to compile.
            clear_plan(&mut compiled_plan);
            return;
        };

        // Run the heavy compile off the UI thread. Only the `spawn_blocking`
        // closure runs on the blocking pool; the async continuation (the
        // generation check and the `compiled_plan` write) resumes on the Dioxus
        // desktop executor that polls `spawn` — the same thread that owns the
        // signal — so writing `compiled_plan` here is sound.
        spawn(async move {
            let plan =
                tokio::task::spawn_blocking(move || compile_active(&config, &root, pipeline_dir))
                    .await
                    .ok()
                    .flatten();

            // A newer compile — or a clear — was dispatched while this one ran;
            // drop its result so it cannot clobber the fresher state.
            if *compile_gen.peek() != generation {
                return;
            }

            match plan {
                Some(p) => compiled_plan.set(Some(p)),
                // Compile diagnostics surface through the parse / schema-warning
                // paths already; here we only clear the plan so dependent
                // surfaces fall back to the no-plan path instead of a stale body.
                None => clear_plan(&mut compiled_plan),
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

/// Compile `config` against `workspace_root`, resolving relative `use:` paths
/// from `pipeline_dir`. Returns `None` on any compile error (the readers all
/// tolerate `None`).
///
/// This is the pure core of [`use_compiled_plan`], extracted so the resolution
/// path — the workspace `.comp.yaml` scan plus relative `use:` binding — is
/// exercisable against real workspace assets without a render harness.
fn compile_active(
    config: &PipelineConfig,
    workspace_root: &Path,
    pipeline_dir: PathBuf,
) -> Option<Arc<CompiledPlan>> {
    let ctx = CompileContext::with_pipeline_dir(workspace_root, pipeline_dir);
    config.compile(&ctx).ok().map(Arc::new)
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
        let plan = compile_active(&config, &ws_root, PathBuf::new())
            .expect("example pipeline compiles against the examples workspace");

        let frame = crate::state::resolve_composition_frame(&plan, "clean")
            .expect("composition node `clean` binds to a body in the compiled plan");
        assert!(
            plan.body_of(frame.body_id).is_some(),
            "the resolved body is present in the compiled plan",
        );
    }
}
