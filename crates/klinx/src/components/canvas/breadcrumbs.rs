use dioxus::prelude::*;

use crate::state::use_app_state;

/// Which drill stack a [`BreadcrumbBar`] navigates (#171).
///
/// The same breadcrumb rendering drives two stacks: the full-swap drill stack
/// (the top-level canvas mount) and the in-context overlay stack (the in-overlay
/// mount). The variant selects which `AppState` signal the root/segment clicks
/// clear/truncate, so one component serves both without duplicating the markup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BreadcrumbTarget {
    /// Navigate the full-swap drill stack (`composition_drill_stack`).
    Drill,
    /// Navigate the in-context overlay stack (`composition_overlay_stack`).
    Overlay,
}

/// Breadcrumb navigation bar for composition drill-in.
///
/// Shows `<root> > alias1 > alias2` from the chosen stack. Clicking a segment
/// truncates that stack to the level; clicking the root clears it. `target`
/// selects which stack (#171): the top-level canvas drives the drill stack, the
/// in-context overlay drives the overlay stack. `root_label` lets the overlay
/// show a stack-appropriate root (e.g. the parent composition name) while the
/// top-level mount keeps "pipeline".
#[component]
pub fn BreadcrumbBar(
    frames: Vec<String>,
    #[props(default = BreadcrumbTarget::Drill)] target: BreadcrumbTarget,
    #[props(default = "pipeline".to_string())] root_label: String,
) -> Element {
    let state = use_app_state();

    // The signal this bar mutates — chosen once from `target` so the click
    // handlers below capture a single `Copy` signal handle regardless of stack.
    let stack_signal = match target {
        BreadcrumbTarget::Drill => state.composition_drill_stack,
        BreadcrumbTarget::Overlay => state.composition_overlay_stack,
    };

    rsx! {
        div {
            class: "klinx-breadcrumb-bar",

            // Root breadcrumb — clicking returns to the stack's top level.
            span {
                class: "klinx-breadcrumb klinx-breadcrumb--clickable",
                onclick: move |_| {
                    let mut stack = stack_signal;
                    stack.write().clear();
                },
                "{root_label}"
            }

            for (depth, alias) in frames.iter().enumerate() {
                span { class: "klinx-breadcrumb-sep", " > " }
                {
                    let is_last = depth == frames.len() - 1;
                    let target_depth = depth + 1;
                    rsx! {
                        span {
                            class: if is_last {
                                "klinx-breadcrumb klinx-breadcrumb--current"
                            } else {
                                "klinx-breadcrumb klinx-breadcrumb--clickable"
                            },
                            onclick: move |_| {
                                if !is_last {
                                    let mut stack = stack_signal;
                                    stack.write().truncate(target_depth);
                                }
                            },
                            "{alias}"
                        }
                    }
                }
            }
        }
    }
}
