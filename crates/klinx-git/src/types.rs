//! Core types for git operations.
//!
//! These types are the shared interface between the gix backend,
//! CLI fallback, and Kiln UI.

use std::path::{Path, PathBuf};

/// Git file status relative to HEAD.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileStatus {
    /// Path relative to repo root.
    pub path: PathBuf,
    /// Status kind.
    pub status: StatusKind,
}

/// File status categories.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StatusKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    Untracked,
}

impl StatusKind {
    /// Single-letter indicator for tab badges.
    pub fn letter(self) -> &'static str {
        match self {
            Self::Added => "A",
            Self::Modified => "M",
            Self::Deleted => "D",
            Self::Renamed => "R",
            Self::Untracked => "U",
        }
    }

    /// CSS modifier suffix shared by UI surfaces that render git statuses.
    pub fn css_modifier(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Modified => "modified",
            Self::Deleted => "deleted",
            Self::Renamed => "renamed",
            Self::Untracked => "untracked",
        }
    }
}

/// Find the git status for a file path in a changed-file list.
///
/// `git_files` contains paths relative to the git repository root. UI call
/// sites often have absolute workspace paths, and the workspace may be nested
/// below the repository root, so matching uses [`Path::ends_with`] to compare
/// whole path components without changing the git backend path semantics.
pub fn git_status_for_path(path: &Path, git_files: &[FileStatus]) -> Option<StatusKind> {
    git_files
        .iter()
        .find(|file| path.ends_with(&file.path))
        .map(|file| file.status)
}

/// Branch information.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchInfo {
    /// Branch name (e.g., "main", "feat/normalize").
    pub name: String,
    /// Whether this is the currently checked-out branch.
    pub is_current: bool,
    /// Commits ahead of upstream (0 if no upstream).
    pub ahead: usize,
    /// Commits behind upstream (0 if no upstream).
    pub behind: usize,
}

/// A single line of blame output.
#[derive(Clone, Debug)]
pub struct BlameLine {
    /// 1-based line number.
    pub line: usize,
    /// Author name.
    pub author: String,
    /// Author email.
    pub email: String,
    /// Commit hash (short).
    pub commit_id: String,
    /// Unix timestamp.
    pub timestamp: i64,
    /// Commit subject line.
    pub summary: String,
}

/// Commit metadata for log display.
#[derive(Clone, Debug)]
pub struct CommitInfo {
    /// Full commit hash.
    pub id: String,
    /// Author name.
    pub author: String,
    /// Author email.
    pub email: String,
    /// Unix timestamp.
    pub timestamp: i64,
    /// First line of commit message.
    pub subject: String,
    /// Remaining commit message body.
    pub body: Option<String>,
}

/// Summary of repository status for UI display.
#[derive(Clone, Debug, Default)]
pub struct RepoStatus {
    /// Current branch name.
    pub branch: String,
    /// Commits ahead of upstream.
    pub ahead: usize,
    /// Commits behind upstream.
    pub behind: usize,
    /// All file statuses.
    pub files: Vec<FileStatus>,
    /// Count of added files.
    pub added: usize,
    /// Count of modified files.
    pub modified: usize,
    /// Count of deleted files.
    pub deleted: usize,
    /// Count of untracked files.
    pub untracked: usize,
}

impl RepoStatus {
    /// Build summary counts from file list.
    pub fn from_files(branch: String, ahead: usize, behind: usize, files: Vec<FileStatus>) -> Self {
        let added = files
            .iter()
            .filter(|f| f.status == StatusKind::Added)
            .count();
        let modified = files
            .iter()
            .filter(|f| f.status == StatusKind::Modified)
            .count();
        let deleted = files
            .iter()
            .filter(|f| f.status == StatusKind::Deleted)
            .count();
        let untracked = files
            .iter()
            .filter(|f| f.status == StatusKind::Untracked)
            .count();

        Self {
            branch,
            ahead,
            behind,
            files,
            added,
            modified,
            deleted,
            untracked,
        }
    }

    /// Whether there are any changes (staged or unstaged).
    pub fn has_changes(&self) -> bool {
        !self.files.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_kind_css_modifier_matches_existing_selectors() {
        assert_eq!(StatusKind::Modified.css_modifier(), "modified");
        assert_eq!(StatusKind::Added.css_modifier(), "added");
        assert_eq!(StatusKind::Deleted.css_modifier(), "deleted");
        assert_eq!(StatusKind::Renamed.css_modifier(), "renamed");
        assert_eq!(StatusKind::Untracked.css_modifier(), "untracked");
    }

    #[test]
    fn git_status_for_path_matches_repo_relative_suffix() {
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

        assert_eq!(
            git_status_for_path(
                Path::new("/home/me/repo/examples/pipelines/customer_etl.yaml"),
                &files,
            ),
            Some(StatusKind::Modified)
        );
        assert_eq!(
            git_status_for_path(
                Path::new("/home/me/repo/examples/pipelines/new_pipe.yaml"),
                &files
            ),
            Some(StatusKind::Untracked)
        );
        assert_eq!(
            git_status_for_path(
                Path::new("/home/me/repo/examples/pipelines/audit_join.yaml"),
                &files
            ),
            None
        );
    }

    #[test]
    fn git_status_for_path_compares_path_components() {
        let files = vec![FileStatus {
            path: PathBuf::from("examples/pipelines/customer_etl.yaml"),
            status: StatusKind::Modified,
        }];

        assert_eq!(
            git_status_for_path(
                Path::new("/home/me/repo/examples/other/customer_etl.yaml"),
                &files
            ),
            None
        );
    }

    #[test]
    fn git_status_for_path_matches_workspace_at_repo_root() {
        let files = vec![FileStatus {
            path: PathBuf::from("customer_etl.yaml"),
            status: StatusKind::Added,
        }];

        assert_eq!(
            git_status_for_path(Path::new("/ws/customer_etl.yaml"), &files),
            Some(StatusKind::Added)
        );
    }
}
