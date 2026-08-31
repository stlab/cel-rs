//! Cross-platform diagnostic reporting.
//!
//! `eprintln!` reaches a visible stderr on desktop, but `wasm32-unknown-unknown`'s stdio is
//! a no-op sink — nothing written to it is ever observable, in a devtools console or
//! otherwise. [`report_error`] is the single point every diagnostic raised through this
//! crate's UI should go through instead, so a failure is visible on every platform a
//! consumer of `adam-web-ui` ships on.
//!
//! Gated on `target_arch = "wasm32"` rather than just `feature = "desktop"`: the web-only
//! code paths this feeds are also built and unit-tested on the native host (`cargo test -p
//! adam-web-ui`), and calling the real `web_sys`/`wasm_bindgen` JS FFI outside an actual
//! wasm32 host crashes the process rather than erroring — `eprintln!` is the correct, safe
//! fallback for that native-host case too. A consumer embedding this crate's components into
//! its own desktop (webview) build should enable this crate's `desktop` feature so its
//! diagnostics land on that process's stderr instead of being silently dropped.

/// Reports `msg` on the platform's error channel: the process's stderr on desktop, or when
/// built for any non-wasm32 host (e.g. running this crate's own test suite natively).
#[cfg(any(feature = "desktop", not(target_arch = "wasm32")))]
pub fn report_error(msg: &str) {
    eprintln!("{msg}");
}

/// Reports `msg` on the platform's error channel: the browser's console, for a genuine
/// wasm32 web build.
#[cfg(all(not(feature = "desktop"), target_arch = "wasm32"))]
pub fn report_error(msg: &str) {
    web_sys::console::error_1(&wasm_bindgen::JsValue::from_str(msg));
}
