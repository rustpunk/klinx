pub mod breadcrumbs;
mod connector;
pub mod extract_modal;
mod node;
mod panel;

pub use panel::CanvasPanel;

use dioxus::prelude::*;

/// What the pointer is currently over on the canvas, driving the field-lineage
/// reveal (#72).
///
/// The pointer's *scope* selects the reveal: a whole node (off any row) shows
/// that node's identity carries; a specific row shows that field's 1-hop
/// closure; nothing over a card shows the node-level DAG only.
#[derive(Clone, PartialEq, Default)]
pub enum HoverTarget {
    /// Pointer not over any node — or over a card with no rendered field rows.
    /// No field connectors are revealed (the node-level DAG stays as-is).
    #[default]
    None,
    /// Pointer over a node card but off any field row — reveal that node's
    /// identity carries (`Passthrough` + `Access`) in BOTH directions, never its
    /// derives (those are a per-field detail). Empty for a field-less card.
    Node(usize),
    /// Pointer over a specific field row — reveal that field's DIRECT (1-hop)
    /// lineage closure (today's behaviour, both edge directions, all kinds).
    Field(usize, String),
}

impl HoverTarget {
    /// The node index this target is anchored on, if any (`Node` and `Field`
    /// are both anchored on a card; `None` is not). The card/row leave handlers
    /// use this to clear or downgrade only their OWN node's hover, keeping the
    /// reset order-independent when the pointer jumps between cards.
    pub fn node(&self) -> Option<usize> {
        match self {
            HoverTarget::None => None,
            HoverTarget::Node(n) | HoverTarget::Field(n, _) => Some(*n),
        }
    }
}

/// Canvas-scoped context carrying the current pointer [`HoverTarget`].
///
/// The panel provides one per canvas and derives the field-lineage reveal from
/// it; `CanvasNode` (card) and `FieldRowView` (row) set it from their
/// `onmouseenter`/`onmouseleave` handlers. A newtype (rather than a bare
/// `Signal<HoverTarget>`) keeps the `use_context` lookup unambiguous and
/// self-documenting at the call sites.
#[derive(Clone, Copy)]
pub struct CanvasHover(pub Signal<HoverTarget>);

/// Canvas-scoped context carrying the PINNED field — the column a user clicked to
/// select (#75). `Some((stage_idx, field_name))` while a field is pinned; `None`
/// otherwise.
///
/// A pin is the *sticky* counterpart to the transient [`CanvasHover`]: a click
/// reveals the column's FULL transitive pipeline lineage and holds it across
/// pointer moves (so the user can trace the cables), where hover shows only the
/// 1-hop neighbourhood and follows the pointer. A pin, when set, takes precedence
/// over the hover in the panel's reveal computation. `FieldRowView` toggles it on
/// row click; a canvas-background click or a view swap clears it.
#[derive(Clone, Copy)]
pub struct PinnedField(pub Signal<Option<(usize, String)>>);
