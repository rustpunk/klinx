pub mod breadcrumbs;
mod connector;
pub mod extract_modal;
mod node;
mod panel;

pub use panel::CanvasPanel;

use dioxus::prelude::*;

/// Canvas-scoped context carrying the currently hovered field anchor.
///
/// `Some((stage_idx, field_name))` while the pointer is over a field row;
/// `None` otherwise. The panel provides one of these per canvas and uses it to
/// compute a single field's lineage closure to reveal on hover; `CanvasNode`
/// sets/clears it from each field row's `onmouseenter`/`onmouseleave`.
///
/// A newtype (rather than a bare `Signal<Option<…>>`) keeps the context lookup
/// unambiguous and self-documenting at the `use_context` call sites.
#[derive(Clone, Copy)]
pub struct HoveredField(pub Signal<Option<(usize, String)>>);
