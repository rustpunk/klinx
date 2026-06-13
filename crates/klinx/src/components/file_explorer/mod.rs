//! Workspace file explorer — left-panel tree of the discovered workspace.
//!
//! `model` is the pure, headless tree model (sectioned + filesystem views);
//! `panel` is the Dioxus component that renders it.

pub mod model;
pub mod panel;

pub use panel::FileExplorer;
