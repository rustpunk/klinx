//! Pure, headless tree model for the workspace file explorer.
//!
//! This module has **no Dioxus dependency** so it is unit-testable in plain
//! `cargo test`. It builds a tree from the workspace discovery already present
//! in [`crate::workspace`] (sectioned view) or from a raw directory walk
//! (filesystem view), and flattens it into a `Vec<FlatNode>` that the component
//! renders one row per entry — the flattened shape keeps rendering
//! virtualization-friendly and avoids recursive-component re-render storms.
//!
//! The split between `build_*` (filesystem I/O, run on discovery change) and
//! [`flatten`] (cheap, run on every expand/collapse) is deliberate: toggling a
//! node never re-walks the disk.
//!
//! A future klinx-H overlay-aware provider (#23) augments [`build_sectioned`]
//! without touching the rendering layer.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use clinker_schema::SchemaIndex;
use klinx_git::{FileStatus, StatusKind};

use crate::state::ChannelState;
use crate::workspace::Workspace;

/// Which view the explorer presents — the header toggle.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ExplorerView {
    /// Discovery-grouped: Pipelines / Compositions / Channels / Schemas.
    #[default]
    Sections,
    /// Raw filesystem tree rooted at the workspace directory.
    Files,
}

/// The four discovery sections in the sectioned view.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SectionKind {
    Pipelines,
    Compositions,
    Channels,
    Schemas,
}

impl SectionKind {
    /// Section header label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Pipelines => "Pipelines",
            Self::Compositions => "Compositions",
            Self::Channels => "Channels",
            Self::Schemas => "Schemas",
        }
    }
}

/// Stable identity for a tree node — used both as the expand-set key and the
/// RSX `key:` (via [`fmt::Display`]). File nodes key on their absolute path so
/// the key survives a tree rebuild; groups key on a path/name string.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum NodeId {
    /// A discovery section (sectioned view only).
    Section(SectionKind),
    /// A grouping node — a channel tenant dir (sectioned) or a directory (files).
    Group(String),
    /// A file leaf (absolute path).
    File(PathBuf),
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Section(kind) => write!(f, "section:{}", kind.label()),
            Self::Group(key) => write!(f, "group:{key}"),
            Self::File(path) => write!(f, "file:{}", path.display()),
        }
    }
}

/// What a node *is* — drives the row icon and click behaviour.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum NodeKind {
    /// Section header.
    Section(SectionKind),
    /// Expandable grouping (directory / tenant).
    Group,
    /// File leaf. `openable` is true only for the YAML family — clicking a
    /// non-YAML file (e.g. a `.csv` or `.toml`) is a no-op rather than opening
    /// a tab that fails to parse.
    File { openable: bool },
}

/// A node in the built (not-yet-flattened) tree.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TreeNode {
    pub id: NodeId,
    pub kind: NodeKind,
    pub label: String,
    /// File path (File) or directory path (Group in the files view); None for
    /// sections and synthetic groups.
    pub path: Option<PathBuf>,
    pub children: Vec<TreeNode>,
}

/// The built tree: a list of root nodes (4 sections, or the workspace root's
/// top-level entries in the files view).
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct ExplorerTree {
    pub roots: Vec<TreeNode>,
}

/// One rendered row: a node projected to a fixed depth, ready to emit.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FlatNode {
    pub id: NodeId,
    pub depth: u16,
    pub kind: NodeKind,
    pub label: String,
    /// `Some` for files (and files-view dirs); a `File` with an openable path is
    /// what a click opens as a tab.
    pub path: Option<PathBuf>,
    /// True when the node has children (a chevron is shown).
    pub expandable: bool,
}

/// YAML-family files are openable as pipeline/overlay/schema tabs.
pub fn is_openable(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("yaml") | Some("yml")
    )
}

// ── Tree construction: sectioned view ────────────────────────────────────

/// Build the discovery-sectioned tree (the default view).
///
/// Reuses today's kiln.toml-based discovery — pipeline include/exclude globs,
/// the compositions directory, [`crate::workspace::discover_channels`] output,
/// and the [`SchemaIndex`]. No dependency on the unfinished clinker.toml
/// semantic layout (klinx-B..H).
pub fn build_sectioned(
    ws: &Workspace,
    idx: &SchemaIndex,
    chans: Option<&ChannelState>,
) -> ExplorerTree {
    let roots = vec![
        section_node(SectionKind::Pipelines, file_nodes(resolve_pipelines(ws))),
        section_node(
            SectionKind::Compositions,
            file_nodes(resolve_compositions(ws)),
        ),
        section_node(SectionKind::Channels, channel_groups(ws, chans)),
        section_node(SectionKind::Schemas, file_nodes(resolve_schemas(idx))),
    ];
    ExplorerTree { roots }
}

fn section_node(kind: SectionKind, children: Vec<TreeNode>) -> TreeNode {
    TreeNode {
        id: NodeId::Section(kind),
        kind: NodeKind::Section(kind),
        label: kind.label().to_string(),
        path: None,
        children,
    }
}

fn file_node(path: PathBuf) -> TreeNode {
    let label = file_label(&path);
    let openable = is_openable(&path);
    TreeNode {
        id: NodeId::File(path.clone()),
        kind: NodeKind::File { openable },
        label,
        path: Some(path),
        children: Vec::new(),
    }
}

fn file_nodes(paths: Vec<PathBuf>) -> Vec<TreeNode> {
    let mut nodes: Vec<TreeNode> = paths.into_iter().map(file_node).collect();
    nodes.sort_by(|a, b| a.label.to_lowercase().cmp(&b.label.to_lowercase()));
    nodes
}

fn file_label(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Group discovered channel bindings by their parent directory (the tenant
/// folder), producing depth-2 `Group → File` nodes.
fn channel_groups(ws: &Workspace, chans: Option<&ChannelState>) -> Vec<TreeNode> {
    let Some(chans) = chans else {
        return Vec::new();
    };
    // One pass: bucket each binding under its parent (tenant) directory. The
    // BTreeMap keeps tenants in stable sorted order; `file_nodes` sorts within.
    let mut by_dir: BTreeMap<PathBuf, Vec<PathBuf>> = BTreeMap::new();
    for b in &chans.channels {
        let parent = b
            .source_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| ws.root.clone());
        by_dir
            .entry(parent)
            .or_default()
            .push(b.source_path.clone());
    }

    by_dir
        .into_iter()
        .map(|(dir, files)| {
            let label = dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| dir.display().to_string());
            TreeNode {
                id: NodeId::Group(dir.to_string_lossy().into_owned()),
                kind: NodeKind::Group,
                label,
                path: Some(dir),
                children: file_nodes(files),
            }
        })
        .collect()
}

// ── Discovery resolvers (filesystem I/O) ─────────────────────────────────

/// Resolve the workspace's pipeline files from the manifest include/exclude
/// globs against the workspace root. Returns sorted, de-duplicated paths.
pub fn resolve_pipelines(ws: &Workspace) -> Vec<PathBuf> {
    let mut set = BTreeSet::new();
    for pat in ws.pipeline_include_globs() {
        glob_walk(&ws.root, &pat, &mut set);
    }
    let excludes = ws.pipeline_exclude_globs();
    if !excludes.is_empty() {
        let mut ex = BTreeSet::new();
        for pat in excludes {
            glob_walk(&ws.root, &pat, &mut ex);
        }
        set.retain(|p| !ex.contains(p));
    }
    set.into_iter().collect()
}

fn compositions_dir(ws: &Workspace) -> String {
    ws.manifest
        .compositions
        .as_ref()
        .map(|c| c.directory.clone())
        .unwrap_or_else(|| "compositions".to_string())
}

/// Resolve `*.comp.yaml` files under the workspace compositions directory.
pub fn resolve_compositions(ws: &Workspace) -> Vec<PathBuf> {
    let dir = ws.root.join(compositions_dir(ws));
    let mut out = BTreeSet::new();
    if let Ok(rd) = fs::read_dir(&dir) {
        for entry in rd.flatten() {
            if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                let name = entry.file_name();
                if name.to_string_lossy().ends_with(".comp.yaml") {
                    out.insert(entry.path());
                }
            }
        }
    }
    out.into_iter().collect()
}

/// Schema file paths from the already-built [`SchemaIndex`] (cross-link, no
/// re-walk).
pub fn resolve_schemas(idx: &SchemaIndex) -> Vec<PathBuf> {
    idx.schemas
        .values()
        .map(|s| s.path.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

// ── Tree construction: filesystem view ───────────────────────────────────

/// Names skipped in the raw filesystem walk (in addition to all dotfiles).
const FS_SKIP: &[&str] = &[".kiln-state.json"];

/// Build a raw recursive filesystem tree rooted at `root`.
///
/// Eager (workspaces are small); directories become `Group` nodes, files become
/// `File` nodes. Dotfiles and machine-managed state are skipped. Symlinks are
/// not followed (treated as leaves) so the walk cannot loop.
pub fn build_filesystem(root: &Path) -> ExplorerTree {
    ExplorerTree {
        roots: fs_children(root),
    }
}

fn fs_children(dir: &Path) -> Vec<TreeNode> {
    let mut dirs: Vec<TreeNode> = Vec::new();
    let mut files: Vec<TreeNode> = Vec::new();

    if let Ok(rd) = fs::read_dir(dir) {
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || FS_SKIP.contains(&name.as_str()) {
                continue;
            }
            let Ok(ft) = entry.file_type() else { continue };
            let path = entry.path();
            if ft.is_dir() {
                dirs.push(TreeNode {
                    id: NodeId::Group(path.to_string_lossy().into_owned()),
                    kind: NodeKind::Group,
                    label: name,
                    children: fs_children(&path),
                    path: Some(path),
                });
            } else if ft.is_file() {
                files.push(file_node(path));
            }
        }
    }

    let by_label = |a: &TreeNode, b: &TreeNode| a.label.to_lowercase().cmp(&b.label.to_lowercase());
    dirs.sort_by(by_label);
    files.sort_by(by_label);
    dirs.into_iter().chain(files).collect()
}

// ── Flatten ──────────────────────────────────────────────────────────────

/// Flatten the tree to a render list, emitting a node's children only when the
/// node is in `expanded`. Cheap — safe to run on every expand/collapse.
pub fn flatten(tree: &ExplorerTree, expanded: &HashSet<NodeId>) -> Vec<FlatNode> {
    let mut out = Vec::new();
    for node in &tree.roots {
        push_flat(node, 0, expanded, &mut out);
    }
    out
}

fn push_flat(node: &TreeNode, depth: u16, expanded: &HashSet<NodeId>, out: &mut Vec<FlatNode>) {
    let expandable = !node.children.is_empty();
    out.push(FlatNode {
        id: node.id.clone(),
        depth,
        kind: node.kind.clone(),
        label: node.label.clone(),
        path: node.path.clone(),
        expandable,
    });
    if expandable && expanded.contains(&node.id) {
        for child in &node.children {
            push_flat(child, depth + 1, expanded, out);
        }
    }
}

/// Default expansion for a freshly-built tree: expand all section roots so the
/// sectioned view shows its contents immediately (a good first impression).
/// Files-view roots (directories) start collapsed.
pub fn expand_sections(tree: &ExplorerTree) -> HashSet<NodeId> {
    tree.roots
        .iter()
        .filter(|n| matches!(n.kind, NodeKind::Section(_)))
        .map(|n| n.id.clone())
        .collect()
}

// ── Row decorations ────────────────────────────────────────────────────────

/// Git status for a file row, matched against the repo's changed-file list.
///
/// `git_files` holds only the changed files (added/modified/deleted/untracked),
/// so it is short and a linear scan is cheap. Its paths are relative to the
/// **git repo root**, which may be an ancestor of the workspace root when the
/// workspace is nested inside a larger repo (e.g. `examples/pipelines/` within
/// this repo). A suffix match (`Path::ends_with`, which compares whole
/// components) therefore matches correctly regardless of that offset — the same
/// rule [`crate::components::tab_bar`] uses for tab badges. `strip_prefix(ws.root)`
/// would *not* work here: it yields workspace-relative paths, a different base.
///
/// Returns `None` for non-file rows (sections, groups/directories).
///
/// Known limitations (all shared with the tab bar, and all rooted in the git
/// layer rather than here — precise matching is deferred to a follow-up that
/// would unify both call sites):
/// - A *short* repo-relative path (e.g. a single component when the workspace
///   **is** the repo root) can suffix-match a same-named file in a deeper
///   directory, tinting it spuriously. An exact match would need the repo root
///   *and* a canonicalized workspace root (the workspace root is stored
///   un-canonicalized), so suffix matching is the robust choice here.
/// - Renamed files do not match: `klinx-git` parses `git status --porcelain`
///   rename lines as the single string `"old -> new"`, which is no row's path.
/// - Files inside a freshly-created directory do not match: porcelain reports
///   the directory (`?? dir/`), not its individual files (no `-uall`).
///
/// In practice the explorer lists files that exist on disk, so the reachable
/// states are Modified / Added / Untracked.
pub fn row_git_status(row: &FlatNode, git_files: &[FileStatus]) -> Option<StatusKind> {
    if !matches!(row.kind, NodeKind::File { .. }) {
        return None;
    }
    let path = row.path.as_deref()?;
    git_files
        .iter()
        .find(|f| path.ends_with(&f.path))
        .map(|f| f.status)
}

// ── Glob matcher ─────────────────────────────────────────────────────────
//
// A tiny `read_dir`-based matcher for the include/exclude patterns the manifest
// actually uses (`*.yaml`, `dir/*.yaml`, `**/*.yaml`). klinx has no glob
// dependency and clinker's resolver is an external git dep, so this avoids both
// a new crate and coupling to engine internals.

/// Greedy wildcard match over a single path segment: `*` matches any run
/// (including empty), `?` matches one char; everything else is literal.
fn wildcard_match(pat: &[u8], name: &[u8]) -> bool {
    let (mut pi, mut ni) = (0usize, 0usize);
    let (mut star, mut mark) = (None, 0usize);
    while ni < name.len() {
        if pi < pat.len() && (pat[pi] == b'?' || pat[pi] == name[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < pat.len() && pat[pi] == b'*' {
            star = Some(pi);
            mark = ni;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    while pi < pat.len() && pat[pi] == b'*' {
        pi += 1;
    }
    pi == pat.len()
}

fn glob_walk(root: &Path, pattern: &str, out: &mut BTreeSet<PathBuf>) {
    let segs: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
    if !segs.is_empty() {
        walk_segments(root, &segs, out);
    }
}

fn walk_segments(dir: &Path, segs: &[&str], out: &mut BTreeSet<PathBuf>) {
    let Some((seg, rest)) = segs.split_first() else {
        return;
    };

    if *seg == "**" {
        if rest.is_empty() {
            collect_files_recursive(dir, out);
        } else {
            // `**` matches zero dirs (try `rest` here) ...
            walk_segments(dir, rest, out);
            // ... or one-or-more dirs (descend, keeping `**`).
            if let Ok(rd) = fs::read_dir(dir) {
                for entry in rd.flatten() {
                    if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                        walk_segments(&entry.path(), segs, out);
                    }
                }
            }
        }
        return;
    }

    let is_last = rest.is_empty();
    if let Ok(rd) = fs::read_dir(dir) {
        for entry in rd.flatten() {
            let name = entry.file_name();
            if !wildcard_match(seg.as_bytes(), name.to_string_lossy().as_bytes()) {
                continue;
            }
            let Ok(ft) = entry.file_type() else { continue };
            if is_last {
                if ft.is_file() {
                    out.insert(entry.path());
                }
            } else if ft.is_dir() {
                walk_segments(&entry.path(), rest, out);
            }
        }
    }
}

fn collect_files_recursive(dir: &Path, out: &mut BTreeSet<PathBuf>) {
    if let Ok(rd) = fs::read_dir(dir) {
        for entry in rd.flatten() {
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_file() {
                out.insert(entry.path());
            } else if ft.is_dir() {
                collect_files_recursive(&entry.path(), out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::ChannelBindingSummary;
    use crate::workspace;

    /// The example workspace shipped at `examples/pipelines/` (PR #40 fixture).
    fn fixture() -> Workspace {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/pipelines")
            .canonicalize()
            .expect("examples/pipelines fixture exists");
        workspace::load_workspace(&path).expect("load fixture workspace")
    }

    fn labels(nodes: &[TreeNode]) -> Vec<String> {
        nodes.iter().map(|n| n.label.clone()).collect()
    }

    #[test]
    fn wildcard_matches() {
        assert!(wildcard_match(b"*.yaml", b"customer_etl.yaml"));
        assert!(wildcard_match(b"*.yaml", b"order_classification.comp.yaml"));
        assert!(!wildcard_match(b"*.yaml", b"clinker.toml"));
        assert!(!wildcard_match(b"*.yaml", b".gitignore"));
        assert!(wildcard_match(b"*.comp.yaml", b"fiscal_date.comp.yaml"));
        assert!(!wildcard_match(b"*.comp.yaml", b"plain.yaml"));
        assert!(wildcard_match(b"pipeline.yaml", b"pipeline.yaml"));
        assert!(wildcard_match(b"a?c", b"abc"));
        assert!(!wildcard_match(b"a?c", b"ac"));
    }

    #[test]
    fn openable_only_for_yaml_family() {
        assert!(is_openable(Path::new("x/customer_etl.yaml")));
        assert!(is_openable(Path::new("x/a.channel.yaml")));
        assert!(is_openable(Path::new("x/a.comp.yaml")));
        assert!(is_openable(Path::new("x/a.schema.yaml")));
        assert!(is_openable(Path::new("x/a.yml")));
        assert!(!is_openable(Path::new("x/clinker.toml")));
        assert!(!is_openable(Path::new("x/data.csv")));
    }

    #[test]
    fn pipelines_are_root_yaml_only() {
        let ws = fixture();
        let pipes = resolve_pipelines(&ws);
        let names: Vec<String> = pipes.iter().map(|p| file_label(p)).collect();

        let expected = [
            "audit_join.yaml",
            "customer_etl.yaml",
            "hopping_sliding_5m_1h.yaml",
            "invoices.yaml",
            "long_field_support.yaml",
            "multi_source_session.yaml",
            "order_fulfillment.yaml",
            "tumbling_clicks.yaml",
        ];
        assert_eq!(names, expected, "root *.yaml pipelines (sorted)");

        // Excludes non-yaml, nested, and sibling-category files.
        assert!(
            !names
                .iter()
                .any(|n| n == "clinker.toml" || n == "kiln.toml")
        );
        assert!(
            !pipes.iter().any(|p| p.components().any(|c| {
                matches!(c, std::path::Component::Normal(s) if s == "data" || s == "compositions" || s == "channels")
            })),
            "no files from data/ compositions/ channels/"
        );
    }

    #[test]
    fn compositions_are_comp_yaml() {
        let ws = fixture();
        let names: Vec<String> = resolve_compositions(&ws)
            .iter()
            .map(|p| file_label(p))
            .collect();
        assert_eq!(
            names,
            [
                "clean_names.comp.yaml",
                "fiscal_date.comp.yaml",
                "order_classification.comp.yaml",
                "shipping_cost.comp.yaml",
                "validate_email.comp.yaml",
            ]
        );
    }

    #[test]
    fn sectioned_has_four_sections() {
        let ws = fixture();
        let (idx, _) = ws.build_schema_index();
        let tree = build_sectioned(&ws, &idx, None);
        assert_eq!(tree.roots.len(), 4);
        assert_eq!(
            tree.roots
                .iter()
                .map(|n| n.label.clone())
                .collect::<Vec<_>>(),
            ["Pipelines", "Compositions", "Channels", "Schemas"]
        );
        // Channels empty when no ChannelState supplied.
        let channels = &tree.roots[2];
        assert!(channels.children.is_empty());
    }

    #[test]
    fn channels_group_by_tenant() {
        let ws = fixture();
        let (idx, _) = ws.build_schema_index();
        let cs = ChannelState {
            channels: vec![
                ChannelBindingSummary {
                    name: "acme-etl".into(),
                    source_path: ws.root.join("channels/acme-corp/customer_etl.channel.yaml"),
                    target: "pipeline: customer_etl.yaml".into(),
                },
                ChannelBindingSummary {
                    name: "acme-ful".into(),
                    source_path: ws
                        .root
                        .join("channels/acme-corp/order_fulfillment.channel.yaml"),
                    target: "pipeline: order_fulfillment.yaml".into(),
                },
                ChannelBindingSummary {
                    name: "west-ful".into(),
                    source_path: ws
                        .root
                        .join("channels/warehouse-west/order_fulfillment.channel.yaml"),
                    target: "pipeline: order_fulfillment.yaml".into(),
                },
            ],
            active_channel: None,
            recent_channels: Vec::new(),
        };
        let tree = build_sectioned(&ws, &idx, Some(&cs));
        let channels = &tree.roots[2];
        assert_eq!(labels(&channels.children), ["acme-corp", "warehouse-west"]);
        assert_eq!(channels.children[0].children.len(), 2);
        assert_eq!(channels.children[1].children.len(), 1);
    }

    #[test]
    fn flatten_respects_expansion() {
        let ws = fixture();
        let (idx, _) = ws.build_schema_index();
        let tree = build_sectioned(&ws, &idx, None);

        // Collapsed: only the 4 section rows.
        let collapsed = flatten(&tree, &HashSet::new());
        assert_eq!(collapsed.len(), 4);
        assert!(collapsed.iter().all(|r| r.depth == 0));

        // Expand Pipelines: 4 sections + 8 pipeline files.
        let mut expanded = HashSet::new();
        expanded.insert(NodeId::Section(SectionKind::Pipelines));
        let rows = flatten(&tree, &expanded);
        assert_eq!(rows.len(), 12);
        assert_eq!(rows.iter().filter(|r| r.depth == 1).count(), 8);

        // expand_sections expands all four.
        let all = expand_sections(&tree);
        assert_eq!(all.len(), 4);
    }

    #[test]
    fn filesystem_view_skips_dotfiles_and_state() {
        let ws = fixture();
        let tree = build_filesystem(&ws.root);
        let top: Vec<String> = labels(&tree.roots);
        assert!(top.iter().any(|n| n == "compositions"));
        assert!(top.iter().any(|n| n == "channels"));
        assert!(top.iter().any(|n| n == "customer_etl.yaml"));
        // Dotfiles excluded.
        assert!(!top.iter().any(|n| n.starts_with('.')));
        // Directories sort before files.
        let first_file = top.iter().position(|n| n.contains('.')).unwrap();
        let last_dir = top
            .iter()
            .rposition(|n| !n.contains('.'))
            .unwrap_or(first_file);
        assert!(last_dir < first_file, "dirs precede files");
    }

    #[test]
    fn node_id_display_is_stable() {
        assert_eq!(
            NodeId::Section(SectionKind::Pipelines).to_string(),
            "section:Pipelines"
        );
        assert_eq!(NodeId::Group("t".into()).to_string(), "group:t");
        assert_eq!(
            NodeId::File(PathBuf::from("/a/b.yaml")).to_string(),
            "file:/a/b.yaml"
        );
    }

    fn file_row(abs: &str) -> FlatNode {
        FlatNode {
            id: NodeId::File(PathBuf::from(abs)),
            depth: 1,
            kind: NodeKind::File { openable: true },
            label: file_label(Path::new(abs)),
            path: Some(PathBuf::from(abs)),
            expandable: false,
        }
    }

    #[test]
    fn git_status_matches_repo_relative_suffix() {
        // Workspace nested under the repo root: `git status` paths are relative
        // to the repo root, not the workspace, so matching is suffix-based.
        let files = vec![
            FileStatus {
                path: PathBuf::from("examples/pipelines/customer_etl.yaml"),
                status: StatusKind::Modified,
            },
            FileStatus {
                path: PathBuf::from("examples/pipelines/new_pipe.yaml"),
                status: StatusKind::Untracked,
            },
        ];

        let row = file_row("/home/me/repo/examples/pipelines/customer_etl.yaml");
        assert_eq!(row_git_status(&row, &files), Some(StatusKind::Modified));

        let row = file_row("/home/me/repo/examples/pipelines/new_pipe.yaml");
        assert_eq!(row_git_status(&row, &files), Some(StatusKind::Untracked));

        // Unchanged file → no decoration.
        let row = file_row("/home/me/repo/examples/pipelines/audit_join.yaml");
        assert_eq!(row_git_status(&row, &files), None);

        // Same filename under a different directory must NOT false-match —
        // `ends_with` compares whole components, not raw string suffixes.
        let row = file_row("/home/me/repo/examples/other/customer_etl.yaml");
        assert_eq!(row_git_status(&row, &files), None);
    }

    #[test]
    fn git_status_workspace_is_repo_root() {
        // Workspace == repo root: a single-component repo-relative path matches.
        let files = vec![FileStatus {
            path: PathBuf::from("customer_etl.yaml"),
            status: StatusKind::Added,
        }];
        let row = file_row("/ws/customer_etl.yaml");
        assert_eq!(row_git_status(&row, &files), Some(StatusKind::Added));
    }

    #[test]
    fn git_status_none_for_non_file_rows() {
        let files = vec![FileStatus {
            path: PathBuf::from("Pipelines"),
            status: StatusKind::Modified,
        }];
        let section = FlatNode {
            id: NodeId::Section(SectionKind::Pipelines),
            depth: 0,
            kind: NodeKind::Section(SectionKind::Pipelines),
            label: "Pipelines".into(),
            path: None,
            expandable: true,
        };
        assert_eq!(row_git_status(&section, &files), None);
    }
}
