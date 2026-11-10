//! Styled upstream button implementation; see `../UPSTREAM.md`.
use dioxus::prelude::*;
use dioxus_primitives::dioxus_attributes::attributes;
use dioxus_primitives::merge_attributes;

include!(concat!(env!("OUT_DIR"), "/button-styles.rs"));

/// Selects the button's visual intent without changing native button semantics.
///
/// # Examples
/// ```
/// use adam_web_ui::components::button::ButtonVariant;
/// let variant = ButtonVariant::Primary;
/// ```
#[derive(Copy, Clone, PartialEq, Default)]
#[non_exhaustive]
pub enum ButtonVariant {
    #[default]
    /// Marks the primary action.
    Primary,
    /// Marks a secondary action.
    Secondary,
    /// Marks an irreversible action.
    Destructive,
    /// Draws an outlined action.
    Outline,
    /// Omits the background until interaction.
    Ghost,
    /// Uses link-like styling without changing the element.
    Link,
}

impl ButtonVariant {
    /// Returns the upstream visual intent token.
    ///
    /// # Examples
    /// ```
    /// use adam_web_ui::components::button::ButtonVariant;
    /// assert_eq!(ButtonVariant::Primary.class(), "primary");
    /// ```
    pub fn class(&self) -> &'static str {
        match self {
            ButtonVariant::Primary => "primary",
            ButtonVariant::Secondary => "secondary",
            ButtonVariant::Destructive => "destructive",
            ButtonVariant::Outline => "outline",
            ButtonVariant::Ghost => "ghost",
            ButtonVariant::Link => "link",
        }
    }
}

/// Selects the button's layout size.
///
/// # Examples
/// ```
/// use adam_web_ui::components::button::ButtonSize;
/// let size = ButtonSize::Default;
/// ```
#[derive(Copy, Clone, PartialEq, Default)]
#[non_exhaustive]
pub enum ButtonSize {
    /// Uses an extra-small text button.
    Xs,
    /// Uses a small text button.
    Sm,
    #[default]
    /// Uses the standard text button.
    Default,
    /// Uses a large text button.
    Lg,
    /// Uses a standard square icon button.
    Icon,
    /// Uses an extra-small square icon button.
    IconXs,
    /// Uses a small square icon button.
    IconSm,
    /// Uses a large square icon button.
    IconLg,
}

impl ButtonSize {
    /// Returns the upstream layout size token.
    ///
    /// # Examples
    /// ```
    /// use adam_web_ui::components::button::ButtonSize;
    /// assert_eq!(ButtonSize::Default.class(), "default");
    /// ```
    pub fn class(&self) -> &'static str {
        match self {
            ButtonSize::Xs => "xs",
            ButtonSize::Sm => "sm",
            ButtonSize::Default => "default",
            ButtonSize::Lg => "lg",
            ButtonSize::Icon => "icon",
            ButtonSize::IconXs => "icon-xs",
            ButtonSize::IconSm => "icon-sm",
            ButtonSize::IconLg => "icon-lg",
        }
    }
}

/// Renders a styled native button with caller-supplied attributes and events.
///
/// - Complexity: O(attributes + children) render work.
///
/// # Examples
/// ```
/// use adam_web_ui::components::button::Button;
/// use dioxus::prelude::*;
/// fn app() -> Element { rsx! { Button { "Save" } } }
/// ```
#[component]
pub fn Button(
    #[props(default)] variant: ButtonVariant,
    #[props(default)] size: ButtonSize,
    #[props(extends=GlobalAttributes)]
    #[props(extends=button)]
    attributes: Vec<Attribute>,
    onclick: Option<EventHandler<MouseEvent>>,
    onmousedown: Option<EventHandler<MouseEvent>>,
    onmouseup: Option<EventHandler<MouseEvent>>,
    onkeydown: Option<EventHandler<KeyboardEvent>>,
    children: Element,
) -> Element {
    crate::components::styles::use_control_styles();
    let base = attributes!(button {
        class: Styles::dx_button,
        "data-style": variant.class(),
        "data-size": size.class(),
    });
    let merged = merge_attributes(vec![base, attributes]);

    rsx! {
        button {
            onclick: move |event| {
                if let Some(f) = &onclick {
                    f.call(event);
                }
            },
            onmousedown: move |event| {
                if let Some(f) = &onmousedown {
                    f.call(event);
                }
            },
            onmouseup: move |event| {
                if let Some(f) = &onmouseup {
                    f.call(event);
                }
            },
            onkeydown: move |event| {
                if let Some(f) = &onkeydown {
                    f.call(event);
                }
            },
            ..merged,
            {children}
        }
    }
}
