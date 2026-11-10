//! Inspects accepted model publications without owning a mutable parsed sheet.
//!
//! Initialize a [`SheetHandle`] with [`crate::binding::use_sheet`] and pass it
//! to [`SheetInspector`]. The host receives every edit outcome and chooses recovery.
//!
//! [`InspectorPresentation`] selects the surrounding chrome: [`InspectorPresentation::Panel`]
//! keeps the 260px side panel with a "Cells" header for the `begin` application,
//! while [`InspectorPresentation::Card`] drops the header and centers the rows in a
//! card up to 24rem wide for the book and slides. In both, the rows share one flex
//! column with a 1.5rem gap. The layout classes (`panel`, `card`, `cells`) live in
//! `style.css` and are compiled into scoped `adam_inspector_<local>` names by the
//! crate's build script.
use crate::components::Label;
use crate::{
    binding::*,
    components::{Card, CardContent, Separator, Theme},
};
use dioxus::prelude::*;
use std::rc::Rc;

include!(concat!(env!("OUT_DIR"), "/inspector-styles.rs"));

#[cfg(test)]
mod tests;

/// Selects the chrome wrapping the inspector's cell rows.
///
/// [`Panel`](InspectorPresentation::Panel) keeps the fixed-width side panel with a
/// "Cells" header; [`Card`](InspectorPresentation::Card) centers the rows in a
/// headerless card. The default is [`Panel`](InspectorPresentation::Panel).
///
/// # Examples
/// ```
/// use adam_web_ui::InspectorPresentation;
/// assert_eq!(InspectorPresentation::default(), InspectorPresentation::Panel);
/// ```
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum InspectorPresentation {
    /// Fixed-width side panel with a "Cells" header, used by the `begin` application.
    #[default]
    Panel,
    /// Headerless card centered up to 24rem wide, used by the book and slides.
    Card,
}

/// Renders named accepted cells with bound controls and visible capability diagnostics.
///
/// Numeric values retain their exact primitive codecs; derived cells remain
/// editable. Output, relevance, forced and invalid states come from publication,
/// not a second mutable model. Failed-model inspection is never attempted. The
/// `presentation` prop chooses the surrounding chrome; see [`InspectorPresentation`].
///
/// - Complexity: O(named cells + displayed bytes) plus bound-control lookups.
///
/// # Examples
/// ```
/// use adam_web_ui::{SheetInspector, InspectorPresentation, binding::*, build_sheet};
/// use dioxus::prelude::*;
/// use std::rc::Rc;
/// fn app() -> Element {
///     let factory: ModelFactory = Rc::new(||
///         build_sheet("sheet s { cell count: i32 = 1; }", "s.adm2"));
///     match use_sheet(factory, "s/v1".into(), EventHandler::new(|_| {})) {
///         Ok(sheet) => rsx! {
///             SheetInspector { sheet, presentation: InspectorPresentation::Card }
///         },
///         Err(error) => rsx! { pre { "{error:?}" } },
///     }
/// }
/// ```
#[component]
pub fn SheetInspector(
    sheet: SheetHandle,
    #[props(default)] presentation: InspectorPresentation,
) -> Element {
    let views = match sheet.views() {
        Ok(views) => views,
        Err(error) => return rsx! { div { role: "alert", "{error}" } },
    };
    let instance = sheet.instance();
    let rows = rsx! {
        div { class: Styles::cells,
            for view in views {
                div { key: "{instance}:{view.key}", class: "adam-cell-row",
                    "data-source": view.is_source.to_string(),
                    "data-relevant": view.flags.relevant.to_string(),
                    "data-forced": view.flags.forced.to_string(),
                    if !view.codec_supported {
                        DisplayOnly { id: "adam-display-{instance}:{view.key}", view: view.clone() }
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
    };
    rsx! { Theme {
        match presentation {
            InspectorPresentation::Panel => rsx! {
                div { class: "{Styles::panel} adam-sheet-inspector",
                    h2 { "Cells" }
                    Separator {}
                    {rows}
                }
            },
            InspectorPresentation::Card => rsx! {
                Card { class: "{Styles::card} adam-sheet-inspector",
                    CardContent { {rows} }
                }
            },
        }
    } }
}

/// Renders a cell without an editing codec as a labeled, read-only value.
///
/// Explicit outputs show only the value; other cells add a "Display only" note
/// because a user might otherwise expect to edit them.
///
/// - Complexity: O(label + value bytes).
#[component]
fn DisplayOnly(id: String, view: Rc<CellView>) -> Element {
    rsx! {
        div { class: Styles::display,
            Label { html_for: id.clone(), "{view.key}" }
            output { id, class: Styles::value, "{view.text}" }
            if !view.flags.readonly {
                small { class: Styles::note, "Display only" }
            }
        }
    }
}
