use dioxus::prelude::*;

use crate::state::use_app_state;

/// Breadcrumb navigation bar for composition drill-in.
///
/// Shows `pipeline > alias1 > alias2` from the drill stack.
/// Clicking a segment pops the stack to that level.
/// Clicking "pipeline" returns to the top-level view.
#[component]
pub fn BreadcrumbBar(frames: Vec<String>) -> Element {
    let state = use_app_state();

    rsx! {
        div {
            class: "klinx-breadcrumb-bar",

            // Root breadcrumb — clicking returns to top-level
            span {
                class: "klinx-breadcrumb klinx-breadcrumb--clickable",
                onclick: move |_| {
                    let mut stack = state.composition_drill_stack;
                    stack.write().clear();
                },
                "pipeline"
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
                                    let mut stack = state.composition_drill_stack;
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
