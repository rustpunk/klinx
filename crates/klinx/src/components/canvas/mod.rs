pub mod breadcrumbs;
mod connector;
pub mod extract_modal;
mod node;
mod panel;

pub use panel::CanvasPanel;

use dioxus::prelude::*;

const FIELD_HOVER_ENTER_DELAY_MS: u64 = 180;
const FIELD_HOVER_EXIT_DELAY_MS: u64 = 150;
const FIELD_HOVER_SKIP_DELAY_MS: u64 = 300;

/// A selectable field-lineage endpoint on the canvas.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LineageTarget {
    Field(usize, String),
    RolePort(usize, String),
}

impl LineageTarget {
    pub fn node(&self) -> usize {
        match self {
            LineageTarget::Field(node, _) | LineageTarget::RolePort(node, _) => *node,
        }
    }
}

/// What the pointer is currently over on the canvas, driving the field-lineage
/// reveal (#72).
///
/// Only a specific field row reveals field-level cables. Plain node chrome does
/// not reveal a row set; it leaves the node-level DAG view alone.
#[derive(Clone, PartialEq, Default)]
pub enum HoverTarget {
    /// Pointer not over any node — or over a card with no rendered field rows.
    /// No field connectors are revealed (the node-level DAG stays as-is).
    #[default]
    None,
    /// Pointer over a specific field row — reveal that field's DIRECT (1-hop)
    /// lineage closure (today's behaviour, both edge directions, all kinds).
    Field(usize, String),
    /// Pointer over a semantic role port row, e.g. an Aggregate `group_by`
    /// input. Reveal the direct role-edge neighbourhood.
    RolePort(usize, String),
}

impl HoverTarget {
    /// The node index this target is anchored on, if any (`Node` and `Field`
    /// are both anchored on a card; `None` is not). The card/row leave handlers
    /// use this to clear or downgrade only their OWN node's hover, keeping the
    /// reset order-independent when the pointer jumps between cards.
    pub fn node(&self) -> Option<usize> {
        match self {
            HoverTarget::None => None,
            HoverTarget::Field(n, _) | HoverTarget::RolePort(n, _) => Some(*n),
        }
    }
}

/// Canvas-scoped context carrying the current pointer [`HoverTarget`].
///
/// The panel provides one per canvas and derives the field-lineage reveal from
/// it. The first field row after a cold entry uses a short intent delay; once a
/// field reveal is active or warm, row-to-row movement applies immediately. A
/// pending target plus generation counter makes stale delayed tasks harmless
/// after the pointer moves elsewhere.
#[derive(Clone, Copy)]
pub struct CanvasHover(
    pub Signal<HoverTarget>,
    Signal<HoverTarget>,
    Signal<u64>,
    Signal<bool>,
);

impl CanvasHover {
    pub fn request_field(&mut self, node: usize, field: String) {
        self.request_target(HoverTarget::Field(node, field));
    }

    pub fn request_role_port(&mut self, node: usize, port: String) {
        self.request_target(HoverTarget::RolePort(node, port));
    }

    fn request_target(&mut self, target: HoverTarget) {
        if !matches!(&*self.0.peek(), HoverTarget::None) || *self.3.peek() {
            self.next_generation();
            self.1.set(HoverTarget::None);
            self.3.set(true);
            self.0.set(target);
            return;
        }

        let generation = self.next_generation();
        self.1.set(target.clone());

        let mut active = self.0;
        let mut pending = self.1;
        let token = self.2;
        let mut warm = self.3;
        spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(FIELD_HOVER_ENTER_DELAY_MS)).await;
            if *token.peek() == generation && *pending.peek() == target {
                pending.set(HoverTarget::None);
                warm.set(true);
                active.set(target);
            }
        });
    }

    pub fn force_clear(&mut self) {
        self.next_generation();
        self.1.set(HoverTarget::None);
        self.0.set(HoverTarget::None);
        self.3.set(false);
    }

    pub fn force_clear_if_node(&mut self, node: usize) {
        let active_matches = self.0.peek().node() == Some(node);
        let pending_matches = self.1.peek().node() == Some(node);
        if active_matches || pending_matches {
            self.force_clear();
        }
    }

    pub fn close_if_node(&mut self, node: usize) {
        let active_matches = self.0.peek().node() == Some(node);
        let pending_matches = self.1.peek().node() == Some(node);
        if !(active_matches || pending_matches) {
            return;
        }

        let generation = self.next_generation();
        self.1.set(HoverTarget::None);

        if !active_matches {
            return;
        }

        let mut active = self.0;
        let token = self.2;
        let mut warm = self.3;
        spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(FIELD_HOVER_EXIT_DELAY_MS)).await;
            if *token.peek() == generation && active.peek().node() == Some(node) {
                active.set(HoverTarget::None);
                warm.set(true);

                tokio::time::sleep(std::time::Duration::from_millis(FIELD_HOVER_SKIP_DELAY_MS))
                    .await;
                if *token.peek() == generation && matches!(&*active.peek(), HoverTarget::None) {
                    warm.set(false);
                }
            }
        });
    }

    fn next_generation(&mut self) -> u64 {
        let generation = self.2.peek().wrapping_add(1);
        self.2.set(generation);
        generation
    }
}

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
pub struct PinnedField(pub Signal<Option<LineageTarget>>);
