//! Inspects accepted model publications without owning a mutable parsed sheet.
//!
//! Initialize a [`SheetHandle`] with [`crate::binding::use_sheet`] and pass it
//! to [`SheetInspector`]. The host receives every edit outcome and chooses recovery.
use crate::{
    binding::*,
    components::{Separator, Theme},
};
use dioxus::prelude::*;

#[cfg(test)]
#[path = "inspector_tests.rs"]
mod inspector_tests;

/// Renders named accepted cells with bound controls and visible capability diagnostics.
///
/// Numeric values retain their exact primitive codecs; derived cells remain
/// editable. Output, relevance, forced and invalid states come from publication,
/// not a second mutable model. Failed-model inspection is never attempted.
///
/// - Complexity: O(named cells + displayed bytes) plus bound-control lookups.
///
/// # Examples
/// ```
/// use adam_web_ui::{SheetInspector, binding::*, build_sheet};
/// use dioxus::prelude::*;
/// use std::rc::Rc;
/// fn app() -> Element {
///     let factory: ModelFactory = Rc::new(||
///         build_sheet("sheet s { cell count: i32 = 1; }", "s.adm2"));
///     match use_sheet(factory, "s/v1".into(), EventHandler::new(|_| {})) {
///         Ok(sheet) => rsx! { SheetInspector { sheet } },
///         Err(error) => rsx! { pre { "{error:?}" } },
///     }
/// }
/// ```
#[component]
pub fn SheetInspector(sheet: SheetHandle) -> Element {
    let views = match sheet.views() {
        Ok(views) => views,
        Err(error) => return rsx! { div { role: "alert", "{error}" } },
    };
    let instance = sheet.instance();
    rsx! { Theme {
        div {
            class: "adam-sheet-inspector",
            style: "width: 260px; min-width: 260px; height: 100%; overflow-y: auto; padding: 12px; box-sizing: border-box;",
            h2 { "Cells" }
            Separator {}
            for view in views {
                div { key: "{instance}:{view.key}", class: "adam-cell-row",
                    "data-source": view.is_source.to_string(),
                    "data-relevant": view.flags.relevant.to_string(),
                    "data-forced": view.flags.forced.to_string(),
                    if !view.codec_supported {
                        p { "{view.key}: {view.text}" }
                    } else if view.is_bool {
                        CheckBox { bind: sheet.cell(&view.key), label: view.key.clone() }
                    } else if view.number_kind.is_some() {
                        if view.range.is_some() {
                            EditSlider { bind: sheet.cell(&view.key), label: view.key.clone() }
                        } else {
                            EditNumber { bind: sheet.cell(&view.key), label: view.key.clone() }
                        }
                    } else {
                        EditText { bind: sheet.cell(&view.key), label: view.key.clone() }
                    }
                    for issue in &view.issues {
                        p { role: "alert", "{issue.message}" }
                    }
                }
            }
        }
    } }
}
