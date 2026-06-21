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
//! [`crate::hooks::pipeline_sync`]: it keys on `(pipeline, workspace)` and
//! re-derives whenever either changes. `pipeline` is already debounced upstream
//! (the ~150ms parse debounce in `AppShell`), so compile runs at most once per
//! typing pause, never per keystroke. A pipeline with no composition nodes skips
//! the workspace `.comp.yaml` scan entirely, so the common case is cheap.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use dioxus::prelude::*;

use clinker_plan::config::{CompileContext, PipelineConfig};
use clinker_plan::plan::CompiledPlan;

use crate::tab::{TabEntry, TabId};
use crate::workspace::Workspace;

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
/// All signals are passed by value (`Signal<T>` is `Copy`).
pub fn use_compiled_plan(
    pipeline: Signal<Option<PipelineConfig>>,
    workspace: Signal<Option<Workspace>>,
    tabs: Signal<Vec<TabEntry>>,
    active_tab_id: Signal<Option<TabId>>,
    mut compiled_plan: Signal<Option<Arc<CompiledPlan>>>,
) {
    use_effect(move || {
        // Subscribe to the two inputs that change the compiled output. A tab
        // switch also resets `pipeline` (from the arriving tab's snapshot), so
        // this fires on tab switches too — letting the active-file lookup below
        // read non-reactively without missing a switch.
        let pl = (pipeline)();
        let ws = (workspace)();

        // Clear the plan only when one is currently set, so a run with nothing
        // to compile never notifies readers redundantly. (The success path
        // below is deliberately unguarded — a fresh plan always carries new
        // information.)
        let mut clear = move || {
            if compiled_plan.peek().is_some() {
                compiled_plan.set(None);
            }
        };

        let Some(config) = pl.as_ref() else {
            // No pipeline (parse error / partial / empty tab): nothing to
            // compile. Clear so a stale plan never lingers under a freshly
            // broken pipeline.
            clear();
            return;
        };

        // Workspace-relative directory of the active pipeline file, for relative
        // `use:` path resolution during composition binding. Read `tabs` /
        // `active_tab_id` non-reactively: a tab switch resets `pipeline` (above),
        // so the peeked path is always current for the pipeline being compiled,
        // and we avoid recompiling on unrelated tab-list churn (snapshot syncs).
        let active_file = {
            let tabs_snapshot = tabs.peek();
            active_file_path(&tabs_snapshot, *active_tab_id.peek())
        };

        let Some((root, pipeline_dir)) = compile_root(ws.as_ref(), active_file.as_deref()) else {
            // Neither a workspace nor a saved file to anchor compilation against
            // (an unsaved scratch tab) — nothing to compile.
            clear();
            return;
        };

        match compile_active(config, &root, pipeline_dir) {
            Some(plan) => compiled_plan.set(Some(plan)),
            // Compile diagnostics surface through the parse / schema-warning
            // paths already; here we only clear the plan so dependent surfaces
            // fall back to the no-plan path instead of showing a stale body.
            None => clear(),
        }
    });
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
    if let Some(ws) = ws {
        return Some((ws.root.clone(), pipeline_dir_for(active_file, &ws.root)));
    }
    let dir = active_file.and_then(Path::parent)?;
    Some((dir.to_path_buf(), PathBuf::new()))
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

/// File path of the active tab, if any. Pulled out so the effect can compute the
/// pipeline directory from a borrowed snapshot of the tab list.
fn active_file_path(tabs: &[TabEntry], active_tab_id: Option<TabId>) -> Option<PathBuf> {
    let id = active_tab_id?;
    tabs.iter()
        .find(|t| t.id == id)
        .and_then(|t| t.file_path.clone())
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

    #[test]
    fn active_file_path_finds_the_active_tabs_path() {
        let path = PathBuf::from("/ws/flow.yaml");
        let file_tab = TabEntry::from_file(path.clone(), String::new());
        let active = file_tab.id;
        let tabs = vec![TabEntry::new_untitled(&[]), file_tab];
        assert_eq!(active_file_path(&tabs, Some(active)), Some(path));
    }

    #[test]
    fn active_file_path_is_none_when_no_tab_is_active() {
        let tabs = vec![TabEntry::new_untitled(&[])];
        assert_eq!(active_file_path(&tabs, None), None);
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
