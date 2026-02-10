//! Parse error type with message and source span for CEL.

use owo_colors::OwoColorize;
use proc_macro2::Span;

/// A CEL parse error with a message and source location.
///
/// Carries the error message and a `proc_macro2::Span` so diagnostics can be
/// formatted with source context (e.g. rustc-style output).
#[derive(Clone, Debug)]
pub struct CELError {
    message: String,
    span: Span,
}

impl CELError {
    /// Creates a new error with the given message and span.
    pub fn new(message: impl Into<String>, span: Span) -> Self {
        CELError {
            message: message.into(),
            span,
        }
    }

    /// Returns the error message.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns the source span for this error.
    pub fn span(&self) -> Span {
        self.span
    }

    /// Formats this error in rustc diagnostic style with source context.
    ///
    /// Produces a multi-line string similar to Rust compiler diagnostics,
    /// including the source file location, error message, and a caret
    /// indicating the error position.
    ///
    /// # Arguments
    ///
    /// * `source_code` - The original source code being parsed
    /// * `filename` - The name of the file (for display)
    /// * `start_line` - The starting line number in the original file (1-based)
    ///
    /// # References
    ///
    /// See the [rustc diagnostic formatting guide](https://github.com/rust-lang/rustc-dev-guide/blob/master/src/diagnostics.md).
    pub fn format_rustc_style(
        &self,
        source_code: &str,
        filename: &str,
        start_line: u32,
    ) -> String {
        let start = self.span.start();
        let end = self.span.end();
        let lines: Vec<&str> = source_code.lines().collect();

        let mut output = String::new();
        let error_line = start_line + (start.line as u32) - 1;
        let error_column = start.column + 1;
        let max_line_num = start_line + (end.line as u32) - 1;
        let line_width = max_line_num.to_string().len();

        output.push_str(&format!("{}: {}\n", "error".red().bold(), self.message));
        output.push_str(&format!(
            " {} {}:{}:{}\n",
            "-->".blue().bold(),
            filename.blue(),
            error_line.to_string().blue(),
            error_column.to_string().blue()
        ));
        output.push_str(&format!(
            "{:width$} {}\n",
            "",
            "|".blue().bold(),
            width = line_width
        ));

        for line_num in start.line..=end.line {
            if let Some(line_content) = lines.get(line_num.saturating_sub(1)) {
                let display_line_num = start_line + (line_num as u32) - 1;
                output.push_str(&format!(
                    "{} {} {}\n",
                    display_line_num.to_string().blue().bold(),
                    "|".blue().bold(),
                    line_content
                ));

                if line_num == start.line {
                    output.push_str(&format!(
                        "{:width$} {} ",
                        "",
                        "|".blue().bold(),
                        width = line_width
                    ));
                    output.push_str(&" ".repeat(start.column));
                    let caret_len = if start.line == end.line {
                        end.column.saturating_sub(start.column).max(1)
                    } else {
                        line_content
                            .len()
                            .saturating_sub(start.column.saturating_sub(1))
                    };
                    output.push_str(&"^".repeat(caret_len).red().bold().to_string());
                    output.push('\n');
                }
            }
        }

        output
    }
}

impl std::fmt::Display for CELError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CELError {}
