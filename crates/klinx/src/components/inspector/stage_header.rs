use dioxus::prelude::*;

use crate::state::use_app_state;

/// Inspector header row: accent border-top + type badge + stage label + close button.
#[component]
pub fn StageHeader(
    stage_id: String,
    kind_label: &'static str,
    kind_attr: &'static str,
    label: String,
) -> Element {
    let state = use_app_state();

    rsx! {
        div {
            class: "klinx-inspector-header",
            "data-stage-kind": kind_attr,
            style: "border-top: 3px solid var(--klinx-stage-accent);",

            span {
                class: "klinx-inspector-badge",
                style: "color: var(--klinx-stage-accent); border-color: var(--klinx-stage-accent);",
                "{kind_label}"
            }

            span { class: "klinx-inspector-label", "{label}" }

            span { style: "flex: 1;" }

            button {
                class: "klinx-inspector-close",
                onclick: move |_| {
                    let mut sel = state.selected_stages;
                    sel.set(std::collections::HashSet::new());
                },
                "\u{00D7}"
            }
        }
    }
}
