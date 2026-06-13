use dioxus::prelude::*;

/// Run drawer stub — shows "no run data" placeholder.
/// Phosphor sub-aesthetic accent (#D4A017).
#[component]
pub fn DrawerRun() -> Element {
    rsx! {
        div {
            class: "klinx-drawer-content klinx-drawer-content--run",
            div {
                class: "klinx-drawer-placeholder",
                "No run data \u{2014} execute the pipeline to see results"
            }
        }
    }
}
