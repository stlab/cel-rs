//! Styled upstream card implementation; see `../UPSTREAM.md`.
use dioxus::prelude::*;
use dioxus_primitives::dioxus_attributes::attributes;
use dioxus_primitives::merge_attributes;

include!(concat!(env!("OUT_DIR"), "/card-styles.rs"));

/// Renders the card surface, grouping its children inside a bordered, rounded container.
///
/// - Complexity: O(attributes + children) render work.
///
/// # Examples
/// ```
/// use adam_web_ui::components::card::{Card, CardContent};
/// use dioxus::prelude::*;
/// fn app() -> Element {
///     rsx! { Card { CardContent { "Body" } } }
/// }
/// ```
#[component]
pub fn Card(
    #[props(extends=GlobalAttributes)] attributes: Vec<Attribute>,
    children: Element,
) -> Element {
    crate::components::styles::use_control_styles();
    let base = attributes!(div {
        class: Styles::dx_card,
        "data-slot": "card",
    });
    let merged = merge_attributes(vec![base, attributes]);

    rsx! {
        div { ..merged, {children} }
    }
}

/// Renders the card's header region, placing an optional [`CardAction`] in its own column.
///
/// - Complexity: O(attributes + children) render work.
///
/// # Examples
/// ```
/// use adam_web_ui::components::card::{CardHeader, CardTitle};
/// use dioxus::prelude::*;
/// fn app() -> Element {
///     rsx! { CardHeader { CardTitle { "Title" } } }
/// }
/// ```
#[component]
pub fn CardHeader(
    #[props(extends=GlobalAttributes)] attributes: Vec<Attribute>,
    children: Element,
) -> Element {
    crate::components::styles::use_control_styles();
    let base = attributes!(div {
        class: Styles::dx_card_header,
        "data-slot": "card-header",
    });
    let merged = merge_attributes(vec![base, attributes]);

    rsx! {
        div { ..merged, {children} }
    }
}

/// Renders the card's title text with the official heading weight and line height.
///
/// - Complexity: O(attributes + children) render work.
///
/// # Examples
/// ```
/// use adam_web_ui::components::card::CardTitle;
/// use dioxus::prelude::*;
/// fn app() -> Element {
///     rsx! { CardTitle { "Title" } }
/// }
/// ```
#[component]
pub fn CardTitle(
    #[props(extends=GlobalAttributes)] attributes: Vec<Attribute>,
    children: Element,
) -> Element {
    crate::components::styles::use_control_styles();
    let base = attributes!(div {
        class: Styles::dx_card_title,
        "data-slot": "card-title",
    });
    let merged = merge_attributes(vec![base, attributes]);

    rsx! {
        div { ..merged, {children} }
    }
}

/// Renders the card's secondary description text beneath its title.
///
/// - Complexity: O(attributes + children) render work.
///
/// # Examples
/// ```
/// use adam_web_ui::components::card::CardDescription;
/// use dioxus::prelude::*;
/// fn app() -> Element {
///     rsx! { CardDescription { "Details" } }
/// }
/// ```
#[component]
pub fn CardDescription(
    #[props(extends=GlobalAttributes)] attributes: Vec<Attribute>,
    children: Element,
) -> Element {
    crate::components::styles::use_control_styles();
    let base = attributes!(div {
        class: Styles::dx_card_description,
        "data-slot": "card-description",
    });
    let merged = merge_attributes(vec![base, attributes]);

    rsx! {
        div { ..merged, {children} }
    }
}

/// Renders the card header's action slot, placed in the header's trailing column.
///
/// - Complexity: O(attributes + children) render work.
///
/// # Examples
/// ```
/// use adam_web_ui::components::card::{CardAction, CardHeader};
/// use dioxus::prelude::*;
/// fn app() -> Element {
///     rsx! { CardHeader { CardAction { "Action" } } }
/// }
/// ```
#[component]
pub fn CardAction(
    #[props(extends=GlobalAttributes)] attributes: Vec<Attribute>,
    children: Element,
) -> Element {
    crate::components::styles::use_control_styles();
    let base = attributes!(div {
        class: Styles::dx_card_action,
        "data-slot": "card-action",
    });
    let merged = merge_attributes(vec![base, attributes]);

    rsx! {
        div { ..merged, {children} }
    }
}

/// Renders the card's main content region beneath its optional header.
///
/// - Complexity: O(attributes + children) render work.
///
/// # Examples
/// ```
/// use adam_web_ui::components::card::{Card, CardContent};
/// use dioxus::prelude::*;
/// fn app() -> Element {
///     rsx! { Card { CardContent { "Body" } } }
/// }
/// ```
#[component]
pub fn CardContent(
    #[props(extends=GlobalAttributes)] attributes: Vec<Attribute>,
    children: Element,
) -> Element {
    crate::components::styles::use_control_styles();
    let base = attributes!(div {
        class: Styles::dx_card_content,
        "data-slot": "card-content",
    });
    let merged = merge_attributes(vec![base, attributes]);

    rsx! {
        div { ..merged, {children} }
    }
}

/// Renders the card's footer region beneath its content.
///
/// - Complexity: O(attributes + children) render work.
///
/// # Examples
/// ```
/// use adam_web_ui::components::card::{Card, CardFooter};
/// use dioxus::prelude::*;
/// fn app() -> Element {
///     rsx! { Card { CardFooter { "Footer" } } }
/// }
/// ```
#[component]
pub fn CardFooter(
    #[props(extends=GlobalAttributes)] attributes: Vec<Attribute>,
    children: Element,
) -> Element {
    crate::components::styles::use_control_styles();
    let base = attributes!(div {
        class: Styles::dx_card_footer,
        "data-slot": "card-footer",
    });
    let merged = merge_attributes(vec![base, attributes]);

    rsx! {
        div { ..merged, {children} }
    }
}
