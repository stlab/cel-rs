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

/// Computes the byte ranges of fenced code blocks in `content`, one span per fence covering
/// from the start of its opening line through the end of its closing line (including the
/// closing line's trailing newline, if present).
///
/// A fence opens on a line whose content, after trimming leading whitespace, starts with a
/// run of 3 or more backticks or tildes; it closes on the next line consisting solely of that
/// same delimiter character, repeated at least as many times as the opening run — matching
/// CommonMark's actual close-fence rule. In particular the closing line's delimiter run is
/// compared only by character and length, never by matching the opening line's info string
/// (e.g. an opening `` ```rust `` line closes on a bare `` ``` `` line, not on another
/// `` ```rust `` line).
///
/// - Postcondition: an unterminated fence (opened but never closed before the end of
///   `content`) produces a span extending to the end of `content`.
/// - Postcondition: returned spans are non-overlapping and sorted by start byte offset.
/// - Complexity: O(n) in the length of `content`.
fn fence_spans(content: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut open: Option<(usize, char, usize)> = None;
    let mut pos = 0usize;
    for line in content.split_inclusive('\n') {
        let stripped = line.trim_end_matches('\n').trim_end_matches('\r').trim();
        match open {
            None => {
                if let Some(delim) = stripped.chars().next().filter(|&c| c == '`' || c == '~') {
                    let run_len = stripped.chars().take_while(|&c| c == delim).count();
                    if run_len >= 3 {
                        open = Some((pos, delim, run_len));
                    }
                }
            }
            Some((start, delim, len)) => {
                let is_close = !stripped.is_empty() && stripped.chars().all(|c| c == delim) && {
                    let close_len = stripped.chars().count();
                    close_len >= len
                };
                if is_close {
                    spans.push((start, pos + line.len()));
                    open = None;
                }
            }
        }
        pos += line.len();
    }
    if let Some((start, _, _)) = open {
        spans.push((start, content.len()));
    }
    spans
}

/// Inserts a live-mount `<div>` after each `.adm2` include in `content`, except for names in
/// [`NO_LIVE_MOUNT`].
///
/// An include that falls inside a fenced code block (see [`fence_spans`]) gets its mount div
/// placed after that fence's closing line instead of immediately after the include itself, so
/// the div renders as a real sibling element rather than as escaped text inside the rendered
/// `<pre><code>` block. Multiple includes sharing one fence each get their own div, all placed
/// together after that fence's close, in the order the includes appear.
///
/// - Complexity: O(n + m) in the length of `content` plus the number of matches.
fn inject_mount_points(content: &str, re: &Regex) -> String {
    let fences = fence_spans(content);
    let captures: Vec<_> = re.captures_iter(content).collect();
    let mut out = String::with_capacity(content.len());
    let mut last_end = 0;
    let mut fence_idx = 0;
    let mut i = 0;
    while i < captures.len() {
        let whole = captures[i].get(0).unwrap();

        // Advance to the first fence span that could still contain this match.
        while fence_idx < fences.len() && fences[fence_idx].1 <= whole.start() {
            fence_idx += 1;
        }
        let containing_fence = fences
            .get(fence_idx)
            .filter(|(start, end)| *start <= whole.start() && whole.end() <= *end)
            .copied();

        match containing_fence {
            Some((_, fence_end)) => {
                // Collect every include that shares this same fence so all their divs land
                // together after the one closing fence, rather than only the first.
                let mut names = Vec::new();
                while i < captures.len() {
                    let next_whole = captures[i].get(0).unwrap();
                    if next_whole.start() >= fence_end {
                        break;
                    }
                    let next_name = &captures[i][1];
                    if !NO_LIVE_MOUNT.contains(&next_name) {
                        names.push(next_name.to_string());
                    }
                    i += 1;
                }
                out.push_str(&content[last_end..fence_end]);
                for name in names {
                    out.push_str(&format!(
                        "\n<div class=\"adam-live\" data-example=\"{name}\"></div>\n"
                    ));
                }
                last_end = fence_end;
            }
            None => {
                let name = &captures[i][1];
                out.push_str(&content[last_end..whole.end()]);
                if !NO_LIVE_MOUNT.contains(&name) {
                    out.push_str(&format!(
                        "\n<div class=\"adam-live\" data-example=\"{name}\"></div>\n"
                    ));
                }
                last_end = whole.end();
                i += 1;
            }
        }
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
    fn inject_mount_points_inserts_the_div_after_the_closing_fence_not_inside_it() {
        // Matches how every real chapter in this book actually uses `{{#include}}`: wrapped
        // in a ```rust ... ``` fence with a language info string on the opening line.
        let re = adm2_include_regex();
        let content = "prose\n\n```rust\n{{#include examples/cells/tuple_typed_cell.adm2}}\n```\n\nmore prose";
        let result = inject_mount_points(content, &re);

        let include_pos = result.find("{{#include").unwrap();
        let fence_close_pos = result[include_pos..]
            .find("```\n")
            .map(|p| p + include_pos)
            .expect("closing fence line must still be present");
        let div_pos = result
            .find("adam-live")
            .expect("mount div must be inserted");

        assert!(
            div_pos > fence_close_pos,
            "mount div must land after the closing fence delimiter, not before or inside it"
        );
        assert!(
            !result[include_pos..fence_close_pos].contains("adam-live"),
            "mount div must not be injected inside the fenced block"
        );
    }

    #[test]
    fn inject_mount_points_handles_back_to_back_fenced_examples_without_merging_spans() {
        // Regression test for the bug where the first attempt at fence tracking matched a
        // closing line against the entire opening line (including its info string), so a bare
        // `` ``` `` close never matched a `` ```rust `` open — and the scanner instead treated
        // the *next* example's opening ```rust`` line as the first fence's close, merging two
        // examples' spans. Real chapters have exactly this shape: back-to-back fenced examples
        // with no unfenced prose between one's closing fence and the next's opening fence.
        let re = adm2_include_regex();
        let content = "```rust\n{{#include examples/cells/a.adm2}}\n```\n```rust\n{{#include examples/cells/b.adm2}}\n```\n";
        let result = inject_mount_points(content, &re);

        assert_eq!(result.matches("adam-live").count(), 2);

        let first_close = result.find("```\n").unwrap();
        let second_open = result[first_close..]
            .find("```rust")
            .map(|p| p + first_close)
            .unwrap();
        let div_a = result.find("data-example=\"cells/a\"").unwrap();
        let div_b = result.find("data-example=\"cells/b\"").unwrap();

        assert!(
            div_a > first_close && div_a < second_open,
            "div for `a` must land between the two fences, not merged into the second one"
        );

        let second_close = result[second_open..]
            .find("```\n")
            .map(|p| p + second_open)
            .unwrap();
        assert!(
            div_b > second_close,
            "div for `b` must land after its own closing fence, not before it"
        );
    }

    #[test]
    fn inject_mount_points_handles_multiple_includes_within_one_fence() {
        let re = adm2_include_regex();
        let content = "```rust\n{{#include examples/cells/a.adm2}}\n{{#include examples/cells/b.adm2}}\n```\n";
        let result = inject_mount_points(content, &re);

        assert_eq!(result.matches("adam-live").count(), 2);

        let fence_close = result.find("```\n").unwrap();
        let div_a = result.find("data-example=\"cells/a\"").unwrap();
        let div_b = result.find("data-example=\"cells/b\"").unwrap();
        assert!(
            div_a > fence_close && div_b > fence_close,
            "both divs must land after the shared closing fence, not inside it"
        );
    }

    #[test]
    fn inject_mount_points_treats_unterminated_fence_as_extending_to_end_of_content() {
        // Regression coverage for the fence_spans postcondition: an opening fence with no
        // matching closing line must produce a span that extends to the end of `content`, so
        // the mount div lands after everything that follows the include -- not immediately
        // after the include itself, which was the pre-fix (buggy) behavior.
        let re = adm2_include_regex();
        let content = "prose\n\n```rust\n{{#include examples/cells/tuple_typed_cell.adm2}}\nno closing fence here";
        let result = inject_mount_points(content, &re);

        let include_pos = result.find("{{#include").unwrap();
        let div_pos = result
            .find("adam-live")
            .expect("mount div must be inserted");
        let tail_pos = result.find("no closing fence here").unwrap();

        assert!(
            div_pos > include_pos,
            "mount div must land after the include itself"
        );
        assert!(
            div_pos > tail_pos,
            "mount div must land after all trailing content, since the unterminated fence's \
             span extends to the end of `content`"
        );
        assert!(
            result
                .trim_end()
                .ends_with("data-example=\"cells/tuple_typed_cell\"></div>"),
            "mount div must be the very last thing in the output"
        );
    }

    #[test]
    fn live_examples_only_supports_html_renderer() {
        assert!(LiveExamples.supports_renderer("html"));
        assert!(!LiveExamples.supports_renderer("epub"));
    }
}
