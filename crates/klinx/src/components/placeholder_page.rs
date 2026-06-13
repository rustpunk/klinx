/// Placeholder page for contexts not yet implemented.
///
/// Styled "Coming soon" card in the rustpunk aesthetic.
/// Used for Channels and Runs contexts.
use dioxus::prelude::*;

#[component]
pub fn PlaceholderPage(name: &'static str, description: &'static str) -> Element {
    rsx! {
        div {
            class: "klinx-placeholder-page",

            div {
                class: "klinx-placeholder-card",

                div {
                    class: "klinx-placeholder-icon",
                    "◈"
                }

                h2 {
                    class: "klinx-placeholder-title",
                    "{name}"
                }

                p {
                    class: "klinx-placeholder-desc",
                    "{description}"
                }

                div {
                    class: "klinx-placeholder-status",
                    "Coming soon"
                }
            }
        }
    }
}
