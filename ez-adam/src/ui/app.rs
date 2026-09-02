//! The top-level component: owns all UI state, composes the toolbar,
//! canvas, and side panel.

use dioxus::prelude::*;
use std::collections::HashSet;
use std::path::PathBuf;

use crate::model::document::Document;
use crate::ui::canvas::{Canvas, NodeId, ViewTransform};
use crate::ui::file_io::{
    pick_export_path, pick_open_path, pick_save_path, read_document_file, write_adm2_file,
    write_document_file,
};
use crate::ui::side_panel::SidePanel;
use crate::ui::toolbar::{Tool, Toolbar};

/// The application's root component: owns all top-level state and
/// composes the menu bar, toolbar, canvas, and side panel.
///
/// Open/Save/Export are dispatched as async tasks against the native file
/// dialogs (`crate::ui::file_io`); any failure (a dialog error, an
/// unreadable/unparsable file, a write failure, or an `.adm2` export
/// error) is surfaced via `error_message` rather than silently dropped,
/// and cleared on the next successful operation.
#[component]
pub fn App() -> Element {
    let document = use_signal(|| Document::new("untitled"));
    let document_path: Signal<Option<PathBuf>> = use_signal(|| None);
    let selection: Signal<HashSet<NodeId>> = use_signal(HashSet::new);
    let active_tool = use_signal(Tool::default);
    let view_transform = use_signal(ViewTransform::identity);
    let error_message: Signal<Option<String>> = use_signal(|| None);

    let mut document_for_open = document;
    let mut document_path_for_open = document_path;
    let mut error_message_for_open = error_message;
    let open = move |_| {
        spawn(async move {
            if let Some(path) = pick_open_path().await {
                match read_document_file(&path) {
                    Ok(doc) => {
                        document_for_open.set(doc);
                        document_path_for_open.set(Some(path));
                        error_message_for_open.set(None);
                    }
                    Err(e) => error_message_for_open.set(Some(e)),
                }
            }
        });
    };

    let document_for_save = document;
    let mut document_path_for_save = document_path;
    let mut error_message_for_save = error_message;
    let save = move |_| {
        let doc = document_for_save.read().clone();
        let existing_path = document_path_for_save.read().clone();
        spawn(async move {
            let path = match existing_path {
                Some(p) => Some(p),
                None => pick_save_path().await,
            };
            if let Some(path) = path {
                match write_document_file(&path, &doc) {
                    Ok(()) => {
                        document_path_for_save.set(Some(path));
                        error_message_for_save.set(None);
                    }
                    Err(e) => error_message_for_save.set(Some(e)),
                }
            }
        });
    };

    let document_for_export = document;
    let mut error_message_for_export = error_message;
    let export = move |_| {
        let doc = document_for_export.read().clone();
        spawn(async move {
            if let Some(path) = pick_export_path().await {
                match crate::codegen::generate_adm2(&doc) {
                    Ok(text) => match write_adm2_file(&path, &text) {
                        Ok(()) => error_message_for_export.set(None),
                        Err(e) => error_message_for_export.set(Some(e)),
                    },
                    Err(e) => error_message_for_export.set(Some(format!("{e:?}"))),
                }
            }
        });
    };

    rsx! {
        div {
            class: "app",
            div {
                class: "menu",
                button { onclick: open, "Open" }
                button { onclick: save, "Save" }
                button { onclick: export, "Export .adm2" }
            }
            if let Some(msg) = error_message.read().as_deref() {
                div { class: "error", "{msg}" }
            }
            Toolbar { active_tool, document, selection }
            div {
                class: "workspace",
                Canvas { document, view_transform, selection, active_tool }
                SidePanel { document, selection }
            }
        }
    }
}
