//! Focused hooks extracted from the `AppShell` god-component.
//!
//! Each module owns one cohesive concern — its `use_*` hook registers the
//! effects/futures for that concern and takes the signals they read or write
//! as parameters (all `Signal<T>` are `Copy`, so they pass by value). This
//! mirrors the thin-hook convention of `use_app_state` in `state.rs`: `AppShell`
//! stays focused on signal ownership, routing, and layout while these hooks
//! carry the side-effect logic.
//!
//! Hook-order note: Dioxus assigns hook slots by call order, and the only
//! invariant is that the order is identical on every render. `AppShell` calls
//! each of these hooks unconditionally at the top level, so the order is stable
//! and these extractions are behavior-preserving.

pub mod channels;
pub mod git_state;
pub mod schema_index;
pub mod session_persistence;

pub use channels::use_channels;
pub use git_state::use_git_state;
pub use schema_index::use_schema_index;
pub use session_persistence::use_session_persistence;
