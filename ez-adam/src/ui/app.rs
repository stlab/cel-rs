//! The top-level component: owns all UI state, composes the toolbar,
//! canvas, and side panel.

use adam_web_ui::spectrum::SpTheme;
use dioxus::desktop::use_muda_event_handler;
use dioxus::prelude::*;
use std::collections::HashSet;
use std::path::PathBuf;

use crate::model::document::Document;
use crate::ui::canvas::{Canvas, NodeId, ViewTransform};
use crate::ui::file_io::{
    pick_export_path, pick_open_path, pick_save_path, read_document_file, write_adm2_file,
    write_document_file,
};
use crate::ui::history::{record_history, redo_target, undo_target};
use crate::ui::menu::{MENU_EXPORT, MENU_OPEN, MENU_REDO, MENU_SAVE, MENU_SAVE_AS, MENU_UNDO};
use crate::ui::side_panel::{SidePanel, panel_target};
use crate::ui::toolbar::{Tool, Toolbar};

/// The application's root component: owns all top-level state and
/// composes the native menu bar's command dispatch, the toolbar, canvas,
/// and side panel.
///
/// Open/Save/Export/Undo/Redo are triggered from the native menu bar
/// (`crate::ui::menu::build_menu`, set on the window in `main`) via
/// `dioxus::desktop::use_muda_event_handler`, not in-window buttons.
/// Open/Save/Export are dispatched as async tasks against the native file
/// dialogs (`crate::ui::file_io`); any failure (a dialog error, an
/// unreadable/unparsable file, a write failure, or an `.adm2` export
/// error) is surfaced via `error_message` rather than silently dropped,
/// and cleared on the next successful operation.
///
/// Undo/redo is a linear history of `Document` snapshots
/// (`crate::ui::history`): a `use_effect` watching `document` records a
/// new entry whenever it changes to something other than what's already
/// at the current history position (so restoring a prior entry via
/// undo/redo — which sets `document` to a value already equal to that
/// entry — is never re-recorded as a fresh edit, with no separate
/// "am I currently restoring" flag needed).
#[component]
pub fn App() -> Element {
    let document = use_signal(|| Document::new("untitled"));
    let document_path: Signal<Option<PathBuf>> = use_signal(|| None);
    let selection: Signal<HashSet<NodeId>> = use_signal(HashSet::new);
    let active_tool = use_signal(Tool::default);
    let view_transform = use_signal(ViewTransform::identity);
    let error_message: Signal<Option<String>> = use_signal(|| None);

    let history: Signal<Vec<Document>> = use_signal(|| vec![Document::new("untitled")]);
    let history_index: Signal<usize> = use_signal(|| 0);

    {
        let mut history = history;
        let mut history_index = history_index;
        use_effect(move || {
            let current = document.read().clone();
            let idx = *history_index.read();
            if history.read().get(idx) == Some(&current) {
                return;
            }
            let new_idx = record_history(&mut history.write(), idx, current);
            history_index.set(new_idx);
        });
    }

    let mut document_for_open = document;
    let mut document_path_for_open = document_path;
    let mut error_message_for_open = error_message;
    let mut history_for_open = history;
    let mut history_index_for_open = history_index;
    let open = move || {
        spawn(async move {
            if let Some(path) = pick_open_path().await {
                match read_document_file(&path) {
                    Ok(doc) => {
                        // Opening a different file starts a fresh undo
                        // history rather than appending to the previous
                        // file's — undoing past the point of opening a
                        // new document back into unrelated content would
                        // be confusing.
                        history_for_open.set(vec![doc.clone()]);
                        history_index_for_open.set(0);
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
    let save = move |force_new_path: bool| {
        let doc = document_for_save.read().clone();
        let existing_path = document_path_for_save.read().clone();
        spawn(async move {
            let path = if force_new_path {
                pick_save_path().await
            } else {
                match existing_path {
                    Some(p) => Some(p),
                    None => pick_save_path().await,
                }
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
    let export = move || {
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

    let mut document_for_undo = document;
    let mut history_index_for_undo = history_index;
    let mut undo = move || {
        let idx = *history_index_for_undo.read();
        if let Some(target) = undo_target(idx) {
            let snapshot = history.read()[target].clone();
            history_index_for_undo.set(target);
            document_for_undo.set(snapshot);
        }
    };

    let mut document_for_redo = document;
    let mut history_index_for_redo = history_index;
    let mut redo = move || {
        let idx = *history_index_for_redo.read();
        let len = history.read().len();
        if let Some(target) = redo_target(idx, len) {
            let snapshot = history.read()[target].clone();
            history_index_for_redo.set(target);
            document_for_redo.set(snapshot);
        }
    };

    use_muda_event_handler(move |event| {
        let id = event.id().as_ref();
        if id == MENU_OPEN {
            open();
        } else if id == MENU_SAVE {
            save(false);
        } else if id == MENU_SAVE_AS {
            save(true);
        } else if id == MENU_EXPORT {
            export();
        } else if id == MENU_UNDO {
            undo();
        } else if id == MENU_REDO {
            redo();
        }
    });

    rsx! {
        // Bundled via `cargo xtask build-js` (ez-adam/package.json +
        // ez-adam/js/spectrum-entry.js) into one esbuild module, mirroring
        // `begin`'s own Spectrum Web Components integration — see
        // docs/superpowers/specs/2026-07-11-begin-spectrum2-theme-tokens-design.md
        // for why this must be a single compiled bundle rather than
        // separate vendored/live files.
        document::Script { r#type: "module", src: asset!("/assets/swc.js") }
        // This neutralizes the browser's default `<body>` margin, which
        // would otherwise offset the canvas's `<svg>` a few pixels from
        // the viewport's true top-left and reintroduce the coordinate
        // mismatch `Canvas`'s `<svg>` doc comment describes fixing.
        style { "html, body {{ margin: 0; padding: 0; overflow: hidden; }}" }
        SpTheme {
            color: "light".to_string(),
            scale: "medium".to_string(),
            system: "spectrum-two".to_string(),
            div {
                class: "app",
                // Fills the viewport and clips anything that would otherwise
                // scroll it (native wheel-scroll is also prevented in
                // `Canvas`'s `onwheel`, but this is cheap defensive insurance).
                style: "position: relative; width: 100vw; height: 100vh; overflow: hidden;",
                // Bottom-left rather than sharing the toolbar's top-left
                // corner: the two would otherwise overlap whenever both are
                // showing at once, since neither reserves space for the
                // other in this absolute-overlay layout.
                if let Some(msg) = error_message.read().as_deref() {
                    div {
                        class: "error",
                        style: "position: absolute; bottom: 12px; left: 12px; z-index: 10; background: #fee; color: #900; border: 1px solid #f5c2c2; border-radius: 6px; padding: 8px 12px; max-width: 400px;",
                        "{msg}"
                    }
                }
                div {
                    style: "position: absolute; top: 12px; left: 12px; z-index: 10; background: white; border-radius: 8px; padding: 4px; box-shadow: 0 1px 4px rgba(0, 0, 0, 0.15);",
                    Toolbar { active_tool, document, selection }
                }
                div {
                    class: "workspace",
                    style: "position: absolute; top: 0; left: 0; width: 100%; height: 100%;",
                    Canvas { document, view_transform, selection, active_tool }
                    // Only occupies (and intercepts clicks/drags over) screen
                    // space when there's actually something to show — an
                    // always-present strip here, even showing nothing but "No
                    // selection" text, would permanently block canvas
                    // interaction underneath it (e.g. dragging a relationship
                    // group onto a conditional that happens to fall under it).
                    if panel_target(&selection.read()).is_some() {
                        div {
                            style: "position: absolute; top: 0; right: 0; bottom: 0; z-index: 10; background: white; border-left: 1px solid #ccc; width: 420px; max-width: 40vw; overflow: auto; padding: 12px; box-sizing: border-box;",
                            SidePanel { document, selection }
                        }
                    }
                }
            }
        }
    }
}
