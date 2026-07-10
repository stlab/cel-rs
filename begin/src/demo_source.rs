//! Loads the demo pm-lang source from `begin/assets/demo.pm` and builds a
//! [`Sheet`]/[`Labels`] pair from it.
//!
//! Two independent bidirectional constraint systems (`a × b = c` and `d × e = f`)
//! are linked by two conditionals on `p`:
//!
//! - `p = 0`: the relationship `c = f` (bidirectional) becomes active.
//! - `p = 1`: the relationship `c = f × 2` (bidirectional) becomes active, and a
//!   single-method relationship `g = c × 10` also becomes active — `g` is *forced*
//!   while this branch is active (see [`property_model::Sheet::is_forced`]), so its
//!   Inspector field is disabled and it is highlighted in the graph.
//! - Any other `p`: the two systems are independent and `g` is not forced.
//!
//! `g`'s relationship is declared in its own `conditional p { .. }` block rather than
//! folded into the first: pm-lang groups every method in one branch into a single
//! relationship, and a relationship's forced outputs are the *intersection* of its
//! methods' pure outputs — mixing `[c] -> [g]` in with the `c`/`f` methods would make
//! that intersection empty, forcing nothing. Two conditionals sharing the same match
//! cell compose independently, so this is a distinct relationship gated on the same
//! `p == 1` condition. This also means the graph renders two diamond nodes for `p`.

use annotate_snippets::Renderer;
use dioxus::prelude::*;
use dioxus_devtools::HotReloadMsg;
use pm_lang::{PmParser, TypeRegistry};
use property_model::Sheet;

use crate::bridge::{
    Labels, SOURCE_FILE_NAME, format_property_model_error, labels_from_cell_names,
};

/// The demo pm-lang source file, referenced individually (not via a folder) so
/// `dx serve`'s file watcher reports changes to it in hot-reload messages.
///
/// Only resolved on desktop (see [`load_demo_source`]); unused when the `desktop`
/// feature is disabled.
#[cfg_attr(not(feature = "desktop"), allow(dead_code))]
static DEMO_ASSET: Asset = asset!("/assets/demo.pm");

/// Compile-time snapshot of the demo source.
///
/// Used as the non-desktop [`load_demo_source`] fallback and as the fixture for
/// unit tests, both of which need a value that doesn't depend on desktop asset
/// bundling being available. Compiled out entirely in an ordinary desktop build
/// (not just lint-suppressed): `include_str!` registers `demo.pm` as a compile-time
/// dependency in cargo's dep-info, and `dx serve` treats any changed file that
/// appears there as requiring a full rebuild — which would defeat this file's own
/// asset-based hot reload (see [`spawn_hot_reload`]) every time `demo.pm` is edited.
#[cfg(any(not(feature = "desktop"), test))]
pub(crate) const DEMO_SOURCE_TEXT: &str = include_str!("../assets/demo.pm");

/// The result of parsing and building a sheet from pm-lang source.
///
/// `sheet_labels` is `None` only on parse failure. A successful parse that
/// then fails to propagate still returns the built sheet and labels alongside
/// the formatted error, matching how the Inspector already tolerates
/// propagate failures during cell edits.
pub struct BuildOutcome {
    /// The built sheet and its UI labels, if parsing succeeded.
    pub sheet_labels: Option<(Sheet, Labels)>,
    /// A formatted rustc-style diagnostic, if parsing or propagation failed.
    pub error: Option<String>,
}

/// Parses `source` as pm-lang, builds a `Sheet` and `Labels`, and propagates
/// once so initial derived values are populated.
///
/// - Complexity: O(n) in the length of `source` plus the cost of one `propagate()`.
pub fn build_sheet(source: &str) -> BuildOutcome {
    let mut parser = PmParser::new(TypeRegistry::new(), cel_parser::OpLookup::new());
    let mut parsed = match parser.parse_str(source) {
        Ok(p) => p,
        Err(e) => {
            let msg = e.format_rustc_style(source, SOURCE_FILE_NAME, 1, &Renderer::styled());
            return BuildOutcome {
                sheet_labels: None,
                error: Some(msg),
            };
        }
    };
    let labels = labels_from_cell_names(&parsed.cell_names);
    match parsed.propagate() {
        Ok(()) => {
            parsed.clear_changed();
            BuildOutcome {
                sheet_labels: Some((parsed.sheet, labels)),
                error: None,
            }
        }
        Err(e) => {
            let msg = format_property_model_error(&e, source);
            BuildOutcome {
                sheet_labels: Some((parsed.sheet, labels)),
                error: Some(msg),
            }
        }
    }
}

/// Reads the demo source, resolving [`DEMO_ASSET`] to a filesystem path on desktop.
///
/// # Errors
///
/// Returns `Err` if `DEMO_ASSET` cannot be resolved to a filesystem path, or if the
/// resolved file cannot be read (e.g. a transient race with an editor's save).
#[cfg(feature = "desktop")]
pub fn load_demo_source() -> Result<String, String> {
    let path = dioxus::asset_resolver::asset_path(DEMO_ASSET)
        .map_err(|e| format!("failed to resolve demo.pm asset path: {e}"))?;
    std::fs::read_to_string(&path).map_err(|e| format!("failed to read {}: {e}", path.display()))
}

/// Non-desktop fallback: the compile-time snapshot, with no live reload.
#[cfg(not(feature = "desktop"))]
pub fn load_demo_source() -> Result<String, String> {
    Ok(DEMO_SOURCE_TEXT.to_string())
}

/// True if `msg` reports a change to the file at `demo_path`.
///
/// Only called (outside of tests) from [`spawn_hot_reload`], which is desktop-only;
/// unused when the `desktop` feature is disabled, same as [`DEMO_ASSET`].
///
/// - Complexity: O(n) in the number of changed assets in `msg`.
#[cfg_attr(not(feature = "desktop"), allow(dead_code))]
fn hot_reload_targets_demo(msg: &HotReloadMsg, demo_path: &std::path::Path) -> bool {
    msg.assets.iter().any(|p| p == demo_path)
}

/// Connects to the `dx serve` devserver and calls `on_change` whenever `demo.pm`
/// changes on disk. A no-op if not running under `dx serve` (`dioxus_devtools::connect`
/// itself returns immediately in that case) or if `DEMO_ASSET` can't be resolved to a
/// filesystem path.
///
/// - Complexity: spawns one background OS thread for the life of the process.
#[cfg(feature = "desktop")]
pub fn spawn_hot_reload(mut on_change: impl FnMut() + Send + 'static) {
    let Ok(demo_path) = dioxus::asset_resolver::asset_path(DEMO_ASSET) else {
        return;
    };
    dioxus_devtools::connect(move |msg| {
        if let dioxus_devtools::DevserverMsg::HotReload(hot_reload) = msg
            && hot_reload_targets_demo(&hot_reload, &demo_path)
        {
            on_change();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_SOURCE: &str = r#"
        sheet s {
            cell a: f64 = 2.0;
            cell b: f64 = 3.0;
            cell c: f64;
            relationship {
                method [a, b] -> [c] { a * b }
                method [b, c] -> [a] { c / b }
                method [a, c] -> [b] { c / a }
            }
        }
    "#;

    #[test]
    fn build_sheet_valid_source_succeeds_with_no_error() {
        let outcome = build_sheet(VALID_SOURCE);
        assert!(outcome.sheet_labels.is_some());
        assert!(outcome.error.is_none());
    }

    #[test]
    fn build_sheet_parse_error_has_no_sheet_and_formatted_message() {
        let outcome = build_sheet("sheet s { cell x }");
        assert!(outcome.sheet_labels.is_none());
        let msg = outcome.error.expect("expected a parse error message");
        assert!(msg.contains("error"), "{msg}");
    }

    #[test]
    fn build_sheet_runtime_error_still_returns_sheet_and_message() {
        let source = "sheet s { cell x: i32 = 0; cell y: i32; relationship { method [x] -> [y] { 10i32 / x } } }";
        let outcome = build_sheet(source);
        assert!(
            outcome.sheet_labels.is_some(),
            "sheet should still be built after a propagate error"
        );
        assert!(outcome.error.is_some());
    }

    #[test]
    fn demo_source_text_parses_successfully() {
        let outcome = build_sheet(DEMO_SOURCE_TEXT);
        assert!(outcome.sheet_labels.is_some());
    }
}

#[cfg(test)]
mod hot_reload_tests {
    use super::*;
    use dioxus_devtools::HotReloadMsg;
    use std::path::PathBuf;

    #[test]
    fn hot_reload_targets_demo_true_when_assets_contains_path() {
        let demo_path = PathBuf::from("/x/demo.pm");
        let msg = HotReloadMsg {
            assets: vec![demo_path.clone()],
            ..Default::default()
        };
        assert!(hot_reload_targets_demo(&msg, &demo_path));
    }

    #[test]
    fn hot_reload_targets_demo_false_when_assets_empty() {
        let demo_path = PathBuf::from("/x/demo.pm");
        let msg = HotReloadMsg::default();
        assert!(!hot_reload_targets_demo(&msg, &demo_path));
    }

    #[test]
    fn hot_reload_targets_demo_false_for_unrelated_asset() {
        let demo_path = PathBuf::from("/x/demo.pm");
        let msg = HotReloadMsg {
            assets: vec![PathBuf::from("/x/graph.css")],
            ..Default::default()
        };
        assert!(!hot_reload_targets_demo(&msg, &demo_path));
    }
}
