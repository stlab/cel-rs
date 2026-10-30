//! Configuration shared between `adam-lang-book-preprocessor` (the `mdbook-live-examples`
//! mdBook preprocessor) and `xtask`'s live-book asset preparation
//! (`xtask::live_book_assets`). Both are independent `bin` crates that must agree on which
//! examples are excluded from live mounting; this tiny crate gives that agreement exactly one
//! source of truth instead of two hand-maintained copies that can silently drift apart.

/// Examples deliberately excluded from live mounting: sources whose whole point depends on a
/// parser configuration `adam_web_ui::build_sheet` doesn't use (here, the *absence* of
/// `cel-std`), so mounting them live would silently show different behavior than the
/// surrounding prose describes.
///
/// A name in this list is never given a live-mount `<div>` by the `mdbook-live-examples`
/// preprocessor, and never given a manifest entry by `xtask`'s live-book asset preparation —
/// both read this same list. Other missing example inputs still require validation
/// by the consuming build.
///
/// # Examples
///
/// ```
/// let examples = ["cells/example", "expressions/no_standard_library"];
/// for name in examples.into_iter().filter(|name|
///     !adam_lang_book_live_config::NO_LIVE_MOUNT.contains(name)
/// ) {
///     println!("Mount {name}");
/// }
/// ```
pub const NO_LIVE_MOUNT: &[&str] = &["expressions/no_standard_library"];
