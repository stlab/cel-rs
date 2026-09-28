//! Native macOS/Windows/Linux menu bar: File (open/save/export) and Edit
//! (undo/redo), plus a standard Window menu. Menu clicks are delivered as
//! `muda::MenuEvent`s and dispatched by `App` via
//! `dioxus::desktop::use_muda_event_handler`, matched against the id
//! constants below.

use dioxus::desktop::muda::accelerator::{Accelerator, Code, Modifiers};
use dioxus::desktop::muda::{Menu, MenuItem, PredefinedMenuItem, Submenu};

/// Menu item id for "File > Open...".
pub(crate) const MENU_OPEN: &str = "ez-adam-menu-open";
/// Menu item id for "File > Save".
pub(crate) const MENU_SAVE: &str = "ez-adam-menu-save";
/// Menu item id for "File > Save As...".
pub(crate) const MENU_SAVE_AS: &str = "ez-adam-menu-save-as";
/// Menu item id for "File > Export .adm2...".
pub(crate) const MENU_EXPORT: &str = "ez-adam-menu-export";
/// Menu item id for "Edit > Undo".
pub(crate) const MENU_UNDO: &str = "ez-adam-menu-undo";
/// Menu item id for "Edit > Redo".
pub(crate) const MENU_REDO: &str = "ez-adam-menu-redo";

/// Builds `ez-adam`'s native menu bar: File, Edit, and Window. Panics if
/// the underlying platform menu APIs fail (matches
/// `dioxus::desktop::menubar::default_menu_bar`'s own `.unwrap()`
/// convention — a menu-construction failure here is a programming error,
/// not a runtime condition to recover from).
///
/// - Precondition: called from the platform main thread — `muda`'s
///   underlying macOS menu items can only be constructed there, which
///   also means this function cannot be exercised by `cargo test`'s
///   worker-threaded harness; it has no dedicated unit test for that
///   reason, matching this crate's convention for framework glue that
///   genuinely cannot be tested in isolation.
#[must_use]
pub fn build_menu() -> Menu {
    let file_menu = Submenu::new("File", true);
    file_menu
        .append_items(&[
            &MenuItem::with_id(
                MENU_OPEN,
                "Open...",
                true,
                Some(Accelerator::new(Some(Modifiers::SUPER), Code::KeyO)),
            ),
            &MenuItem::with_id(
                MENU_SAVE,
                "Save",
                true,
                Some(Accelerator::new(Some(Modifiers::SUPER), Code::KeyS)),
            ),
            &MenuItem::with_id(
                MENU_SAVE_AS,
                "Save As...",
                true,
                Some(Accelerator::new(
                    Some(Modifiers::SUPER | Modifiers::SHIFT),
                    Code::KeyS,
                )),
            ),
            &PredefinedMenuItem::separator(),
            &MenuItem::with_id(MENU_EXPORT, "Export .adm2...", true, None),
        ])
        .unwrap();

    let edit_menu = Submenu::new("Edit", true);
    edit_menu
        .append_items(&[
            &MenuItem::with_id(
                MENU_UNDO,
                "Undo",
                true,
                Some(Accelerator::new(Some(Modifiers::SUPER), Code::KeyZ)),
            ),
            &MenuItem::with_id(
                MENU_REDO,
                "Redo",
                true,
                Some(Accelerator::new(
                    Some(Modifiers::SUPER | Modifiers::SHIFT),
                    Code::KeyZ,
                )),
            ),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::cut(None),
            &PredefinedMenuItem::copy(None),
            &PredefinedMenuItem::paste(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::select_all(None),
        ])
        .unwrap();

    // Matches `dioxus::desktop::menubar::default_menu_bar`'s own Window
    // menu exactly, since dropping it would lose standard
    // fullscreen/minimize/quit items this app still needs.
    let window_menu = Submenu::new("Window", true);
    window_menu
        .append_items(&[
            &PredefinedMenuItem::fullscreen(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::hide(None),
            &PredefinedMenuItem::hide_others(None),
            &PredefinedMenuItem::show_all(None),
            &PredefinedMenuItem::maximize(None),
            &PredefinedMenuItem::minimize(None),
            &PredefinedMenuItem::close_window(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::quit(None),
        ])
        .unwrap();

    let menu = Menu::new();
    menu.append_items(&[&file_menu, &edit_menu, &window_menu])
        .unwrap();

    #[cfg(target_os = "macos")]
    {
        window_menu.set_as_windows_menu_for_nsapp();
    }

    menu
}
