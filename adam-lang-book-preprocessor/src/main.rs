//! `mdbook-live-examples`: an mdBook preprocessor that inserts a live-mount `<div>`
//! immediately after every `{{#include examples/<chapter>/<name>.adm2}}` directive in a
//! chapter, so the pairing between a shown example and its live widget can never drift as
//! examples move or new ones are added.
//!
//! Registered in `book.toml` as `[preprocessor.live-examples]`.

use mdbook::book::{Book, BookItem};
use mdbook::errors::Error;
use mdbook::preprocess::{CmdPreprocessor, Preprocessor, PreprocessorContext};
use regex::Regex;
use std::io;

/// Examples deliberately excluded from live mounting: sources whose whole point depends on a
/// parser configuration `adam_web_ui::build_sheet` doesn't use (here, the *absence* of
/// `cel-std`), so mounting them live would silently show different behavior than the
/// surrounding prose describes. See this plan's Global Constraints for why.
const NO_LIVE_MOUNT: &[&str] = &["expressions/no_standard_library"];

/// Matches an `.adm2` include directive, capturing `<chapter>/<name>` (without the
/// `.adm2` extension) for use as both the mount div's `data-example` value and the
/// [`NO_LIVE_MOUNT`] lookup key.
///
/// - Postcondition: only matches includes ending in `.adm2` — an ordinary
///   `{{#include ../tests/foo.rs:anchor}}` (if any remain elsewhere in the book) never
///   matches.
fn adm2_include_regex() -> Regex {
    Regex::new(r"\{\{#include\s+examples/([A-Za-z0-9_]+/[A-Za-z0-9_]+)\.adm2\s*\}\}").unwrap()
}

/// Inserts a live-mount `<div>` immediately after each `.adm2` include in `content`, except
/// for names in [`NO_LIVE_MOUNT`].
///
/// - Complexity: O(n + m) in the length of `content` plus the number of matches.
fn inject_mount_points(content: &str, re: &Regex) -> String {
    let mut out = String::with_capacity(content.len());
    let mut last_end = 0;
    for capture in re.captures_iter(content) {
        let whole = capture.get(0).unwrap();
        let name = &capture[1];
        out.push_str(&content[last_end..whole.end()]);
        if !NO_LIVE_MOUNT.contains(&name) {
            out.push_str(&format!(
                "\n<div class=\"adam-live\" data-example=\"{name}\"></div>\n"
            ));
        }
        last_end = whole.end();
    }
    out.push_str(&content[last_end..]);
    out
}

struct LiveExamples;

impl Preprocessor for LiveExamples {
    fn name(&self) -> &str {
        "live-examples"
    }

    fn run(&self, _ctx: &PreprocessorContext, mut book: Book) -> Result<Book, Error> {
        let re = adm2_include_regex();
        book.for_each_mut(|item| {
            if let BookItem::Chapter(chapter) = item {
                chapter.content = inject_mount_points(&chapter.content, &re);
            }
        });
        Ok(book)
    }

    fn supports_renderer(&self, renderer: &str) -> bool {
        renderer == "html"
    }
}

fn main() -> Result<(), Error> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("supports") {
        // mdBook calls `mdbook-live-examples supports <renderer>` to ask whether this
        // preprocessor applies; exit 0 to say yes, non-zero to say no.
        let renderer = args.get(2).map(String::as_str).unwrap_or_default();
        std::process::exit(if LiveExamples.supports_renderer(renderer) {
            0
        } else {
            1
        });
    }

    let (ctx, book) = CmdPreprocessor::parse_input(io::stdin())?;
    let processed = LiveExamples.run(&ctx, book)?;
    serde_json::to_writer(io::stdout(), &processed)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inject_mount_points_inserts_a_div_after_an_adm2_include() {
        let re = adm2_include_regex();
        let content =
            "some prose\n\n{{#include examples/cells/tuple_typed_cell.adm2}}\n\nmore prose";
        let result = inject_mount_points(content, &re);
        assert!(result.contains(
            "{{#include examples/cells/tuple_typed_cell.adm2}}\n<div class=\"adam-live\" data-example=\"cells/tuple_typed_cell\"></div>"
        ));
    }

    #[test]
    fn inject_mount_points_leaves_non_adm2_includes_untouched() {
        let re = adm2_include_regex();
        let content = "{{#include ../tests/tutorial.rs:first_sheet}}";
        let result = inject_mount_points(content, &re);
        assert_eq!(result, content);
    }

    #[test]
    fn inject_mount_points_skips_the_no_live_mount_list() {
        let re = adm2_include_regex();
        let content = "{{#include examples/expressions/no_standard_library.adm2}}";
        let result = inject_mount_points(content, &re);
        assert!(!result.contains("adam-live"));
    }

    #[test]
    fn inject_mount_points_handles_multiple_includes_in_one_chapter() {
        let re = adm2_include_regex();
        let content =
            "{{#include examples/cells/a.adm2}}\n\ntext\n\n{{#include examples/cells/b.adm2}}";
        let result = inject_mount_points(content, &re);
        assert_eq!(result.matches("adam-live").count(), 2);
        assert!(result.contains("data-example=\"cells/a\""));
        assert!(result.contains("data-example=\"cells/b\""));
    }

    #[test]
    fn live_examples_only_supports_html_renderer() {
        assert!(LiveExamples.supports_renderer("html"));
        assert!(!LiveExamples.supports_renderer("epub"));
    }
}
