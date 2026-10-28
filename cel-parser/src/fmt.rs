//! Pretty-prints a [`crate::Expr`] tree back to CEL source text: precedence-aware
//! parenthesization (added only where required, not exhaustively), single-space-around-operator
//! normalization, and a 100-column layout target. Short expressions stay inline; longer lists,
//! records and closure bodies nest by four spaces, with leading binary continuation operators.
//! Unbreakable tokens and comments may exceed the target. Literal leaves are re-emitted via
//! [`proc_macro2::Span::source_text`] rather than synthesized from [`crate::Literal`], so exact
//! original notation (`1920.0` vs `1920.0f64`, a byte literal's spelling) round-trips.
//!
//! Typed arrays round-trip with their postfix `: Type` ascriptions, including recursive array
//! types such as `[]: [[i32]]` and reusable tuple syntax such as `[]: [(i32, f64)]`. The
//! formatter preserves complete tuple, array, and `Fn((A,), R)` callable annotations,
//! including the comma that distinguishes a singleton tuple from grouping.

use crate::ast::{Expr, LogicalOp};
use crate::trivia::{Comment, GapPiece, scan_gap, take_block_comment};

/// Owns one character-position index for a borrowed source.
///
/// Line starts index a shared character-to-byte boundary table.
struct Source<'a> {
    text: &'a str,
    lines: Vec<usize>,
    bytes: Vec<usize>,
}

impl<'a> Source<'a> {
    /// Indexes every source position once in O(n) time and space for n source bytes.
    fn new(text: &'a str) -> Self {
        let mut lines = vec![0];
        let mut bytes = Vec::new();
        // Record each character boundary in its owning line.
        for (character, (byte, ch)) in text.char_indices().enumerate() {
            bytes.push(byte);
            if ch == '\n' {
                lines.push(character + 1);
            }
        }
        bytes.push(text.len());
        Self { text, lines, bytes }
    }

    /// Returns the byte boundary for a position in this source.
    ///
    /// - Precondition: `position` belongs to this source.
    fn byte(&self, position: proc_macro2::LineColumn) -> usize {
        debug_assert!((1..=self.lines.len()).contains(&position.line));
        let character = self.lines[position.line - 1] + position.column;
        debug_assert!(
            character
                < self
                    .lines
                    .get(position.line)
                    .copied()
                    .unwrap_or(self.bytes.len())
        );
        self.bytes[character]
    }

    /// Borrows the source between two positions, or an empty slice for synthetic source.
    ///
    /// - Precondition: nonempty source owns both positions.
    fn between(&self, from: proc_macro2::LineColumn, to: proc_macro2::LineColumn) -> &'a str {
        if self.text.is_empty() {
            return "";
        }
        self.text.get(self.byte(from)..self.byte(to)).unwrap_or("")
    }
}

/// An immutable Wadler document; construction never lays out a child expression.
type Doc = pretty::RcDoc<'static, ()>;

/// Preserves a lexical token's internal newlines and indentation independently of layout nesting.
///
/// - Complexity: O(n) in the token's text length.
fn verbatim_doc(text: String) -> Doc {
    let mut lines = text.split('\n');
    let first = Doc::text(lines.next().unwrap_or_default().to_owned());
    lines.fold(first, |doc, line| {
        let line = Doc::text(line.to_owned());
        doc.append(Doc::nesting(move |indent| {
            let offset = isize::try_from(indent)
                .expect("document indentation fits isize")
                .checked_neg()
                .expect("nonnegative indentation can be negated");
            Doc::hardline().append(line.clone()).nest(offset)
        }))
    })
}

/// Lifts recovered trivia into a document with mandatory line-comment boundaries.
///
/// - Complexity: O(n) in the recovered text length.
fn trivia_doc(text: String) -> Doc {
    trivia_doc_at(text, 4)
}

/// Lifts recovered trivia with continuation nesting supplied by its grammar owner.
///
/// - Complexity: O(n) in the recovered text length.
fn trivia_doc_at(text: String, continuation: isize) -> Doc {
    let mut lines = text.split('\n');
    let first = Doc::text(lines.next().unwrap_or_default().to_owned());
    lines.fold(first, |doc, line| {
        doc.append(
            Doc::hardline()
                .append(Doc::text(
                    line.strip_prefix("    ").unwrap_or(line).to_owned(),
                ))
                .nest(continuation),
        )
    })
}

/// Builds a gap document, with a soft boundary before binary operators or after commas.
///
/// - Complexity: O(n) in the gap's text length.
fn gap_doc(pieces: &[GapPiece], spacing: Spacing) -> Doc {
    if pieces
        .iter()
        .any(|piece| matches!(piece, GapPiece::Comment(_)))
        && let Some(index) = pieces
            .iter()
            .position(|piece| matches!(piece, GapPiece::Punct(_)))
        && matches!(spacing, Spacing::Around | Spacing::CommaAfter)
    {
        let GapPiece::Punct(token) = pieces[index] else {
            unreachable!()
        };
        let before = emit_gap(&pieces[..index], Spacing::None, 0);
        let after = emit_gap(&pieces[index + 1..], Spacing::None, 0);
        let after_empty = after.is_empty();
        let before_line = matches!(
            pieces.get(index.wrapping_sub(1)),
            Some(GapPiece::Comment(Comment::Line(_)))
        );
        let after_line = matches!(pieces.last(), Some(GapPiece::Comment(Comment::Line(_))));
        let prefix = if before_line {
            trivia_doc_at(before, 0)
        } else {
            trivia_doc_at(before.trim_end().to_owned(), 0)
        };
        let suffix = if after_line {
            trivia_doc_at(after, 0)
        } else {
            trivia_doc_at(after.trim_end().to_owned(), 0)
        };
        return match spacing {
            Spacing::Around => prefix
                .append(if before_line {
                    Doc::nil()
                } else {
                    Doc::softline()
                })
                .append(token)
                .append(if after_empty {
                    Doc::space()
                } else {
                    suffix.append(if after_line { Doc::nil() } else { Doc::space() })
                }),
            Spacing::CommaAfter => prefix.append(token).append(if after_empty {
                Doc::line()
            } else {
                suffix.append(if after_line { Doc::nil() } else { Doc::line() })
            }),
            Spacing::None => unreachable!(),
        };
    }
    if pieces
        .iter()
        .all(|piece| matches!(piece, GapPiece::Punct(_)))
    {
        Doc::concat(pieces.iter().map(|piece| {
            let GapPiece::Punct(token) = piece else {
                unreachable!()
            };
            match spacing {
                Spacing::Around => Doc::line().append(*token).append(" "),
                Spacing::CommaAfter => Doc::text(*token).append(Doc::line()),
                Spacing::None => Doc::text(*token),
            }
        }))
    } else {
        trivia_doc(emit_gap(pieces, spacing, 0))
    }
}

/// Builds an indented leading-operator continuation without nesting the left operand.
///
/// - Complexity: O(n) in the gap's text length.
fn binary_doc(lhs: Doc, gap: &[GapPiece], rhs: Doc) -> Doc {
    lhs.append(gap_doc(gap, Spacing::Around).append(rhs).nest(4))
        .group()
}

/// Ends an opening-brace gap with a soft boundary, preserving mandatory comment boundaries.
///
/// `trim_start` removes spacing already supplied by the preceding closing-brace boundary.
///
/// - Complexity: O(n) in the recovered gap's text length.
fn body_gap_doc(pieces: &[GapPiece], trim_start: bool) -> Doc {
    let text = emit_gap(pieces, Spacing::Around, 0);
    let text = if trim_start { text.trim_start() } else { &text };
    if matches!(pieces.last(), Some(GapPiece::Comment(Comment::Line(_)))) {
        trivia_doc_at(text.to_owned(), 0)
    } else {
        trivia_doc_at(text.trim_end().to_owned(), 0).append(Doc::line())
    }
}

/// Binding-strength level, loosest first, mirroring `lib.rs`'s grammar chain from
/// `range_expression` (via `expression = range_expression`) through `primary_expression`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
struct Level(u8);

impl Level {
    const RANGE: Level = Level(0);
    const OR: Level = Level(1);
    const AND: Level = Level(2);
    const COMPARISON: Level = Level(3);
    const BIT_OR: Level = Level(4);
    const BIT_XOR: Level = Level(5);
    const BIT_AND: Level = Level(6);
    const SHIFT: Level = Level(7);
    const ADDITIVE: Level = Level(8);
    const MULTIPLICATIVE: Level = Level(9);
    const CAST: Level = Level(10);
    const UNARY: Level = Level(11);
    const POSTFIX: Level = Level(12);
    const PRIMARY: Level = Level(13);

    /// The next level up (strictly tighter-binding than `self`).
    fn tighter(self) -> Level {
        Level(self.0 + 1)
    }
}

/// 4 spaces per nesting level (mirrors `adam-lang::fmt::indent`).
///
/// - Complexity: O(depth).
fn indent(depth: usize) -> String {
    "    ".repeat(depth)
}

/// How a gap's expected `Punct` tokens are spaced against their operands.
#[derive(Clone, Copy)]
enum Spacing {
    /// A single space on each side of the token: `a + b`.
    Around,
    /// No surrounding spaces: `a..b`, or a bare delimiter.
    None,
    /// The token then one space: `a, b`.
    CommaAfter,
}

/// Renders the pieces of one scanned gap into the text that goes *between* two already-rendered
/// operands. A `Comment::Block` is inlined space-separated; a `Comment::Line` ends its line and
/// resumes at `indent(depth + 1)` (the wrap continuation), since a `//` comment runs to
/// end-of-line. A `Punct` is spaced per `spacing`.
///
/// - Postcondition: the returned string starts and ends exactly as `spacing` dictates for the
///   comment-free case (e.g. `Spacing::Around` with a lone `Punct` returns `" + "`), so a
///   comment-free gap reprints identically to the pre-`scan_gap` formatter.
///
/// - Complexity: O(n) in `pieces.len()` plus their text lengths.
fn emit_gap(pieces: &[GapPiece], spacing: Spacing, depth: usize) -> String {
    let cont = indent(depth + 1);
    let mut out = String::new();
    for piece in pieces {
        match piece {
            GapPiece::Punct(tok) => {
                match spacing {
                    Spacing::Around => {
                        // Skip the leading space only when the previous piece already left one
                        // (a block comment's trailing `" */ "`, or a line comment's continuation
                        // indent) — otherwise this token opens the gap and needs its own.
                        if !out.ends_with(' ') {
                            out.push(' ');
                        }
                        out.push_str(tok);
                        out.push(' ');
                    }
                    Spacing::None => out.push_str(tok),
                    Spacing::CommaAfter => {
                        // Unlike `Around`, a comma never gets a leading space of its own -- it
                        // glues directly to whatever precedes it, whether that's the element text
                        // itself (the comment-free case) or a comment's trailing `" */ "` (a block
                        // comment written between the element and its `,`, e.g.
                        // `1i32 /* c */, 2i32`). A line comment's trailing continuation *indent*
                        // is different: that space is meaningful (it's `indent(depth + 1)`, not
                        // filler), so it must be left alone -- only the incidental single-space
                        // filler a block comment or a prior `Around` punct leaves behind is
                        // trimmed.
                        if out.ends_with(' ') && !out.ends_with(&cont) {
                            out.pop();
                        }
                        out.push_str(tok);
                        out.push(' ');
                    }
                }
            }
            GapPiece::Comment(Comment::Block(text)) => {
                // Separate from whatever came before (or, when this is the first piece, from the
                // already-rendered operand this gap is appended to) with exactly one space,
                // unless the previous piece already left a trailing space.
                if !out.ends_with(' ') {
                    out.push(' ');
                }
                out.push_str("/* ");
                out.push_str(text);
                out.push_str(" */ ");
            }
            GapPiece::Comment(Comment::Line(text)) => {
                // A `//` comment runs to end of line, so anything after it must start on the next
                // line; drop any trailing space this gap had accumulated before appending it.
                let continuation = out
                    .rsplit_once('\n')
                    .is_some_and(|(_, tail)| tail.trim().is_empty());
                if continuation {
                    out.push_str("// ");
                } else {
                    let trimmed_len = out.trim_end().len();
                    out.truncate(trimmed_len);
                    out.push_str(" // ");
                }
                out.push_str(text);
                out.push('\n');
                out.push_str(&cont);
            }
        }
    }
    out
}

/// Returns the binding-strength level of a binary (two-operand) operator.
///
/// - Precondition: `name` is one of the binary operator tokens `lib.rs`'s grammar recognizes,
///   excluding `"range"`/`"range_inclusive"` — those, and every other range-family operator
///   (`"range_from"`/`"range_to"`/`"range_to_inclusive"`/`"range_full"`), are rendered by their
///   own dedicated [`render`] match arms before any operator ever reaches this function, since
///   their printed form (`x..y`, no surrounding spaces) doesn't fit this function's shared
///   `"{lhs} {name} {rhs}"` shape the way every other binary operator here does.
fn binary_op_level(name: &str) -> Level {
    match name {
        "|" => Level::BIT_OR,
        "^" => Level::BIT_XOR,
        "&" => Level::BIT_AND,
        "<<" | ">>" => Level::SHIFT,
        "+" | "-" => Level::ADDITIVE,
        "*" | "/" | "%" => Level::MULTIPLICATIVE,
        "==" | "!=" | "<" | ">" | "<=" | ">=" => Level::COMPARISON,
        other => unreachable!("binary_op_level called with unknown operator `{other}`"),
    }
}

/// Re-emits a literal's exact original text via its span, falling back to an empty string when
/// none is recoverable (spans built without a live source file — never a real parse; see the
/// module doc).
fn render_literal(span: crate::ExprSpan) -> String {
    span.start.source_text().unwrap_or_default()
}

/// Scans the source gap between two adjacent AST positions for comments interleaved with
/// `expected` tokens. Returns just the expected tokens (no comments) when `source` is empty (a
/// hand-built `Expr`), so a source-less format still works.
///
/// - Precondition: when `source` is non-empty, it is the exact text `expr` was parsed from —
///   `prev_end`/`next_start` must resolve within it.
///
/// - Complexity: O(g + e) time and space for g gap bytes and e expected-token bytes.
fn gap_between(
    source: &Source<'_>,
    prev_end: proc_macro2::Span,
    next_start: proc_macro2::Span,
    expected: &[&'static str],
) -> Vec<GapPiece> {
    scan_gap(source.between(prev_end.end(), next_start.start()), expected)
}

/// Recovers member-dot comments across closing receiver groups omitted from the AST.
///
/// - Precondition: non-empty `source` is the receiver's original parsed text, and both spans
///   resolve within it.
/// - Postcondition: grouping delimiters are omitted; comments and the dot retain source order.
/// - Complexity: O(g) time and space for g gap bytes.
fn member_gap(
    source: &Source<'_>,
    receiver_end: proc_macro2::Span,
    name_start: proc_macro2::Span,
) -> Vec<GapPiece> {
    grouping_gap(source, receiver_end.end(), name_start.start(), &["."])
}

/// Renders a postfix base, retaining leading group comments inside one owning group.
///
/// - Precondition: `start` is the postfix node's original start in `source`, or source is empty.
/// - Postcondition: the base's leading comments appear once, before its first rendered token.
/// - Complexity: O(n + g) document space for n base nodes and g recovered gap bytes.
fn format_postfix_base(
    base: &Expr,
    source: &Source<'_>,
    depth: usize,
    start: proc_macro2::LineColumn,
) -> Doc {
    let leading = grouping_gap(source, start, base.span().start.start(), &[]);
    if leading.is_empty() {
        format_at(base, source, depth, Level::POSTFIX)
    } else {
        let (text, _) = render(base, source, depth);
        let prefix = emit_gap(&leading, Spacing::None, 0);
        Doc::text("(")
            .append(trivia_doc(prefix.trim_start().to_owned()))
            .append(text)
            .append(")")
    }
}

/// Renders a comma-separated element list with per-gap comment recovery: the gap before the first
/// element (after `bounds.0`, the opening delimiter's position, e.g. a `(` or `[`), each
/// inter-element `,` gap, and the gap after the last element (before `bounds.1`, the position
/// right past the closing delimiter, e.g. a `)` or `]`). `delims` is `(open, close)`, the
/// delimiter tokens. When `trailing_comma` is set (a 1-tuple), a `,` is emitted after the sole
/// element.
///
/// `bounds` holds raw [`proc_macro2::LineColumn`] positions (not `Span`s, unlike [`gap_between`]'s
/// `prev_end`/`next_start`) because `Expr::Apply`'s, `Expr::Tuple`'s, and `Expr::Array`'s own
/// recorded delimiter span covers the *whole* `(...)`/`[...]` group rather than a single
/// delimiter character: `AstContext::apply_op`'s `"()"` arm, `is_tuple_or_group`, and
/// `is_array_expression` all capture `self.last_span` from a `Token::CloseDelim`, and
/// `lex_lexer`'s flattening gives every `OpenDelim`/`CloseDelim` the *enclosing `Group`'s* span
/// (see `LexLexer::next`), so a group span's `.start()` lands on the opening delimiter and its
/// `.end()` lands just past the closing delimiter regardless of which delimiter "produced" it.
/// Resolving which of `.start()`/`.end()` is the right boundary is the caller's job (it knows
/// whether it's holding a whole-group span or an ordinary leaf span, e.g. a callee's own end);
/// this helper just consumes the two positions it's given.
///
/// - Complexity: O(n) in the number of elements plus their gap lengths.
fn emit_list(
    source: &Source<'_>,
    depth: usize,
    delims: (&'static str, &'static str),
    bounds: (proc_macro2::LineColumn, proc_macro2::LineColumn),
    elements: &[Expr],
    trailing_comma: bool,
) -> Doc {
    let (open, close) = delims;
    let (open_pos, close_pos) = bounds;
    // Scans the raw gap between two already-resolved positions, mirroring `gap_between` but
    // taking `LineColumn`s directly instead of deriving them from a fixed `prev_end.end()` /
    // `next_start.start()` pairing (which doesn't fit `open_pos`/`close_pos`; see above).
    let scan = |from: proc_macro2::LineColumn,
                to: proc_macro2::LineColumn,
                expected: &[&'static str]|
     -> Vec<GapPiece> { scan_gap(source.between(from, to), expected) };

    if elements.is_empty() {
        // Whole gap between the delimiters: `open`, comments, `close`.
        let pieces = scan(open_pos, close_pos, &[open, close]);
        let whole = emit_gap(&pieces, Spacing::None, 0);
        return trivia_doc(glue_before_close(&glue_after_open(&whole, open), close));
    }
    // Opening delimiter + any comment before the first element. `emit_gap`'s comment rendering
    // always pads a leading space before a comment that opens a gap, which is right when the gap
    // follows an already-rendered operand (`x /* c */ as i32`) but wrong right after a delimiter
    // -- `glue_after_open` corrects it back to `f(/* a */ 1i32)`.
    let mut pre = grouping_gap(source, open_pos, elements[0].span().start.start(), &[open]);
    if open == "(" {
        pre.insert(0, GapPiece::Punct("("));
    }
    let mut out = trivia_doc(glue_after_open(&emit_gap(&pre, Spacing::None, 0), open));
    let mut body = Doc::line_();
    for (i, el) in elements.iter().enumerate() {
        body = body.append(format_at(el, source, depth, Level::OR));
        if i + 1 < elements.len() {
            let pieces = grouping_gap(
                source,
                el.span().end.end(),
                elements[i + 1].span().start.start(),
                &[","],
            );
            body = body.append(gap_doc(&pieces, Spacing::CommaAfter));
        }
    }
    if trailing_comma {
        body = body.append(",");
    }
    // Any comment before the closing delimiter, then the delimiter; symmetric to the opening gap
    // above (`glue_before_close` drops the trailing space before `)` a comment would otherwise
    // leave: `f(1i32 /* end */)`, not `f(1i32 /* end */ )`).
    let mut post = grouping_gap(
        source,
        elements[elements.len() - 1].span().end.end(),
        close_pos,
        &[close],
    );
    if close == ")" {
        post.push(GapPiece::Punct(")"));
    }
    out = out.append(body.nest(4));
    let tail = glue_before_close(&emit_gap(&post, Spacing::None, 0), close);
    out.append(Doc::line_()).append(trivia_doc(tail)).group()
}

/// Strips the one space [`emit_gap`] pads before a comment that immediately opens a gap, when
/// that gap begins right after `open` — appropriate between an operand and an operator, but not
/// right after an opening delimiter, which glues directly to a following comment.
fn glue_after_open(gap: &str, open: &'static str) -> String {
    match gap.strip_prefix(&format!("{open} ")) {
        Some(rest) => format!("{open}{rest}"),
        None => gap.to_string(),
    }
}

/// Symmetric to [`glue_after_open`]: strips the one space [`emit_gap`] pads after a comment that
/// immediately precedes `close`, so a trailing comment glues directly to the closing delimiter.
fn glue_before_close(gap: &str, close: &'static str) -> String {
    if let Some((_, last_line)) = gap.rsplit_once('\n') {
        // `gap` wrapped after a `//` comment; `last_line` is the continuation line `emit_gap`
        // started with its own indent. When that line holds nothing but indentation before
        // `close` (no block comment glued onto the same line), those spaces are the
        // continuation's own indent, not `emit_gap`'s one-space comment padding — leave them
        // alone instead of stripping one as if it were the padding space.
        if let Some(rest) = last_line.strip_suffix(close)
            && rest.chars().all(|c| c == ' ')
        {
            return gap.to_string();
        }
    }
    match gap.strip_suffix(&format!(" {close}")) {
        Some(rest) => format!("{rest}{close}"),
        None => gap.to_string(),
    }
}

/// Returns the `'static` source spelling of a binary operator name, for `scan_gap`'s expected-token
/// list (which needs `&'static str`). The spelling equals `name` for every binary operator this
/// formatter's `binary_op_level` accepts.
fn binary_op_token(name: &str) -> &'static str {
    match name {
        "|" => "|",
        "^" => "^",
        "&" => "&",
        "<<" => "<<",
        ">>" => ">>",
        "+" => "+",
        "-" => "-",
        "*" => "*",
        "/" => "/",
        "%" => "%",
        "==" => "==",
        "!=" => "!=",
        "<" => "<",
        ">" => ">",
        "<=" => "<=",
        ">=" => ">=",
        other => unreachable!("binary_op_token called with unknown operator `{other}`"),
    }
}

/// Returns the `'static` source spelling of a unary operator name (`"-"` or `"!"`).
fn unary_op_token(name: &str) -> &'static str {
    match name {
        "-" => "-",
        "!" => "!",
        other => unreachable!("unary_op_token called with unknown operator `{other}`"),
    }
}

/// Renders one unresolved array-annotation type expression.
///
/// Named leaves are emitted verbatim, so built-in names and host-registered custom names format
/// identically. Recursive arrays keep their bracket nesting, and tuple type expressions keep the
/// same reusable syntax the parser accepts for future typed CEL surfaces.
///
/// - Complexity: O(t + n + b) time for t inspected type-source bytes, n type nodes and
///   b emitted bytes; O(b) output and O(d) auxiliary stack space for type depth d.
fn render_type_expr(type_expr: &crate::TypeExpr, source: &Source<'_>) -> String {
    let span = type_expr.span();
    let original = source.between(span.start.start(), span.end.end());
    if original.contains("/*") || original.contains("//") {
        return original.to_owned();
    }
    let mut output = String::new();
    write_type_expr(type_expr, &mut output);
    output
}

/// Appends canonical type syntax without materializing intermediate child strings.
///
/// - Complexity: O(n + b) time and O(d) stack space for n nodes, b emitted bytes and depth d.
fn write_type_expr(type_expr: &crate::TypeExpr, output: &mut String) {
    match type_expr {
        crate::TypeExpr::Named { name, args, .. } => {
            output.push_str(name);
            if !args.is_empty() {
                write_type_list(args, output, false);
            }
        }
        crate::TypeExpr::Array { element, .. } => {
            output.push('[');
            write_type_expr(element, output);
            output.push(']');
        }
        crate::TypeExpr::Tuple { elements, .. } => {
            write_type_list(elements, output, true);
        }
    }
}

/// Appends parenthesized type arguments or tuple elements with committed separators.
///
/// - Complexity: O(n + b) time and O(d) stack space for n descendant nodes, b bytes and depth d.
fn write_type_list(elements: &[crate::TypeExpr], output: &mut String, tuple: bool) {
    output.push('(');
    // Emit each child directly into the shared output.
    for (index, element) in elements.iter().enumerate() {
        if index != 0 {
            output.push_str(", ");
        }
        write_type_expr(element, output);
    }
    if tuple && elements.len() == 1 {
        output.push(',');
    }
    output.push(')');
}

/// Builds `expr`'s document alongside its binding-strength level, so the
/// caller ([`format_at`]) can decide whether the context it's being placed in requires parens.
///
/// `source` is the text `expr` was parsed from (or `""` for a hand-built `Expr`).
/// Children contribute documents, never independently formatted strings.
///
/// - Complexity: O(v + g + b) time and space for v node visits, g inspected source-slice bytes
///   and b document-text bytes.
fn render(expr: &Expr, source: &Source<'_>, depth: usize) -> (Doc, Level) {
    match expr {
        Expr::Struct {
            name,
            name_span,
            fields,
            span,
        } => {
            let mut out = Doc::text(name.clone());
            let mut body = Doc::nil();
            if let Some(first) = fields.first() {
                let open = grouping_gap(
                    source,
                    name_span.end.end(),
                    first.name_span.start.start(),
                    &["{"],
                );
                body = body.append(body_gap_doc(&open, false));
                for (index, field) in fields.iter().enumerate() {
                    body = body.append(Doc::text(field.name.clone()));
                    let colon = grouping_gap(
                        source,
                        field.name_span.end.end(),
                        field.value.span().start.start(),
                        &[":"],
                    );
                    body = body.append(trivia_doc(emit_gap(&colon, Spacing::CommaAfter, 0)));
                    body = body.append(format_at(&field.value, source, depth, Level::RANGE));
                    if let Some(next) = fields.get(index + 1) {
                        let comma = grouping_gap(
                            source,
                            field.value.span().end.end(),
                            next.name_span.start.start(),
                            &[","],
                        );
                        body = body.append(gap_doc(&comma, Spacing::CommaAfter));
                    }
                }
                let last = fields.last().expect("non-empty fields");
                let mut tail = grouping_gap(
                    source,
                    last.value.span().end.end(),
                    span.end.end(),
                    &[",", "}"],
                );
                tail.retain(|piece| !matches!(piece, GapPiece::Punct(",")));
                let closing = emit_gap(&tail, Spacing::Around, 0);
                out = out
                    .append(body.nest(4))
                    .append(Doc::line())
                    .append(trivia_doc(closing.trim().to_owned()))
                    .group();
            } else {
                let tail = grouping_gap(source, name_span.end.end(), span.end.end(), &["{", "}"]);
                out = out
                    .append(" ")
                    .append(trivia_doc(emit_gap(&tail, Spacing::None, 0)));
            }
            (out, Level::PRIMARY)
        }
        Expr::Literal { span, .. } => (verbatim_doc(render_literal(*span)), Level::PRIMARY),
        // The parser records the full-range operator as an identifier; its source token
        // distinguishes syntax from a genuine value binding with the same lookup name.
        Expr::Ident { name, span }
            if name == "range_full"
                && span
                    .start
                    .source_text()
                    .is_some_and(|text| matches!(text.as_str(), "." | "..")) =>
        {
            (Doc::text(".."), Level::RANGE)
        }
        Expr::Ident { name, .. } => (Doc::text(name.clone()), Level::PRIMARY),
        Expr::Logical { op, lhs, rhs, .. } => {
            let level = match op {
                LogicalOp::Or => Level::OR,
                LogicalOp::And => Level::AND,
            };
            let op_static: &'static str = match op {
                LogicalOp::Or => "||",
                LogicalOp::And => "&&",
            };
            let lhs_s = format_at(lhs, source, depth, level);
            let rhs_s = format_at(rhs, source, depth, level.tighter());
            let pieces = gap_between(source, lhs.span().end, rhs.span().start, &[op_static]);
            (binary_doc(lhs_s, &pieces, rhs_s), level)
        }
        Expr::Op { operands, .. } if operands.is_empty() => {
            // Defensive only: a real parse never produces an arity-0 `Expr::Op` (see the
            // `Expr::Ident` arm above for how `"range_full"` actually arrives), but a hand-built
            // tree using this representation still round-trips correctly, matching this file's
            // existing convention of also handling non-parser-producible tree shapes (e.g.
            // `nested_comparison_needs_parens_on_both_sides`).
            (Doc::text(".."), Level::RANGE)
        }
        Expr::Op {
            name,
            operands,
            span,
        } if operands.len() == 1
            && matches!(
                name.as_str(),
                "range_from" | "range_to" | "range_to_inclusive"
            ) =>
        {
            // Range's own endpoints are always `or_expression`s (never chained further range
            // expressions — see `is_range_expression`'s doc comment), so an operand only needs
            // rendering strictly tighter than Range itself, the same non-chaining treatment
            // `Level::COMPARISON` gets below.
            let operand_s = format_at(&operands[0], source, depth, Level::RANGE.tighter());
            let text = match name.as_str() {
                "range_from" => {
                    let pieces = gap_between(source, operands[0].span().end, span.end, &[".."]);
                    operand_s.append(gap_doc(&pieces, Spacing::None))
                }
                "range_to" => {
                    // `span.start` is the `".."` token's own span; a `gap_between` call always
                    // scans strictly *after* its `prev_end` argument's end, so `".."` itself can
                    // never appear inside the scanned gap here (unlike the operand-then-operator
                    // arms below, where the operator genuinely sits inside the scanned range) —
                    // it's rendered directly instead, and only a comment is scanned for.
                    let pieces = gap_between(source, span.start, operands[0].span().start, &[]);
                    Doc::text("..")
                        .append(gap_doc(&pieces, Spacing::None))
                        .append(operand_s)
                }
                "range_to_inclusive" => {
                    let pieces = gap_between(source, span.start, operands[0].span().start, &[]);
                    Doc::text("..=")
                        .append(gap_doc(&pieces, Spacing::None))
                        .append(operand_s)
                }
                _ => unreachable!("guarded by the outer match arm"),
            };
            (text, Level::RANGE)
        }
        Expr::Op {
            name,
            operands,
            span,
        } if operands.len() == 1 => {
            let operand_s = format_at(&operands[0], source, depth, Level::UNARY);
            let op_static = unary_op_token(name);
            // Same reasoning as the `range_to`/`range_to_inclusive` arm above: the operator
            // precedes the operand, so it can never appear inside the scanned gap; only a
            // comment is scanned for, and the operator is rendered directly.
            let pieces = gap_between(source, span.start, operands[0].span().start, &[]);
            // Preserve the existing "- -1" disambiguating space when there are no comments.
            let default_sep = if matches!(&operands[0], Expr::Op { name, operands, .. }
                if operands.len() == 1 && matches!(name.as_str(), "-" | "!"))
            {
                " "
            } else {
                ""
            };
            let gap = if pieces.iter().any(|p| matches!(p, GapPiece::Comment(_))) {
                emit_gap(&pieces, Spacing::None, 0)
            } else {
                default_sep.to_string()
            };
            (
                Doc::text(op_static)
                    .append(trivia_doc(gap))
                    .append(operand_s),
                Level::UNARY,
            )
        }
        Expr::Op { name, operands, .. } if name == "range" || name == "range_inclusive" => {
            // Same non-chaining reasoning as the arity-1 range arm above: both endpoints render
            // strictly tighter than Range.
            let lhs_s = format_at(&operands[0], source, depth, Level::RANGE.tighter());
            let rhs_s = format_at(&operands[1], source, depth, Level::RANGE.tighter());
            let op_static: &'static str = if name == "range_inclusive" {
                "..="
            } else {
                ".."
            };
            let pieces = gap_between(
                source,
                operands[0].span().end,
                operands[1].span().start,
                &[op_static],
            );
            (
                lhs_s
                    .append(
                        Doc::softline_()
                            .append(gap_doc(&pieces, Spacing::None))
                            .append(rhs_s)
                            .nest(4),
                    )
                    .group(),
                Level::RANGE,
            )
        }
        Expr::Op { name, operands, .. } => {
            let level = binary_op_level(name);
            // Comparison can't chain — the grammar parses at most one per comparison_expression —
            // so both operands must be strictly tighter than Comparison itself, unlike the other,
            // left-associative (chaining) binary levels below, where only the right operand does.
            let (lhs_min, rhs_min) = if level == Level::COMPARISON {
                (level.tighter(), level.tighter())
            } else {
                (level, level.tighter())
            };
            let lhs_s = format_at(&operands[0], source, depth, lhs_min);
            let rhs_s = format_at(&operands[1], source, depth, rhs_min);
            // The operator token itself has no span; it lives in the gap between the two operands.
            let op_static = binary_op_token(name);
            let pieces = gap_between(
                source,
                operands[0].span().end,
                operands[1].span().start,
                &[op_static],
            );
            (binary_doc(lhs_s, &pieces, rhs_s), level)
        }
        Expr::Cast {
            expr,
            type_name,
            span,
        } => {
            // Left-associative, like multiplicative/additive: the operand only needs to be at
            // least as tight as Cast itself, so a chain like `x as i32 as f64` reprints without
            // extra parens.
            let expr_s = format_at(expr, source, depth, Level::CAST);
            // The type name has no span of its own, so the gap scanned here runs from the operand's
            // end through the whole cast node's end (which includes both `as` and the type name);
            // only `"as"` is expected inside it, and the type name is appended afterward.
            let pieces = gap_between(source, expr.span().end, span.end, &["as"]);
            let text = binary_doc(expr_s, &pieces, Doc::text(type_name.clone()));
            (text, Level::CAST)
        }
        Expr::Apply { callee, args, span } => {
            let mut callee_s = format_postfix_base(callee, source, depth, span.start.start());
            // Without parentheses a bare member followed by `()` becomes direct dispatch,
            // rather than an ordinary call of the member's result.
            if matches!(callee.as_ref(), Expr::Member { args: None, .. }) {
                callee_s = Doc::text("(").append(callee_s).append(")");
            }
            let tail = grouping_gap(source, callee.span().end.end(), span.end.start(), &[]);
            callee_s = callee_s.append(gap_doc(&tail, Spacing::None));
            // The call's end span covers the complete argument group, so neither boundary
            // includes closing callee groups omitted from the AST.
            let list = emit_list(
                source,
                depth,
                ("(", ")"),
                (span.end.start(), span.end.end()),
                args,
                false,
            );
            (callee_s.append(list), Level::POSTFIX)
        }
        Expr::Member {
            receiver,
            name,
            name_span,
            args,
            span,
        } => {
            let receiver_s = format_postfix_base(receiver, source, depth, span.start.start());
            let gap = member_gap(source, receiver.span().end, name_span.start);
            let member_gap = if gap.iter().all(|piece| matches!(piece, GapPiece::Punct(_))) {
                Doc::line_().append(".")
            } else if gap
                .iter()
                .any(|piece| matches!(piece, GapPiece::Comment(Comment::Line(_))))
            {
                trivia_doc_at(emit_gap(&gap, Spacing::None, 0), 0)
            } else {
                verbatim_doc(emit_gap(&gap, Spacing::None, 0))
            };
            let mut text = receiver_s
                .append(member_gap.append(Doc::text(name.clone())).nest(4))
                .group();
            if let Some(args) = args {
                text = text.append(emit_list(
                    source,
                    depth,
                    ("(", ")"),
                    (name_span.end.end(), span.end.end()),
                    args,
                    false,
                ));
            }
            (text, Level::POSTFIX)
        }
        Expr::Tuple { elements, span } => {
            // `span.start` and `span.end` are both the same whole-group span here (see
            // `emit_list`'s doc comment): `.start()` is the position of `(`, `.end()` the
            // position right after `)`.
            let list = emit_list(
                source,
                depth,
                ("(", ")"),
                (span.start.start(), span.end.end()),
                elements,
                elements.len() == 1,
            );
            (list, Level::PRIMARY)
        }
        Expr::Array {
            elements,
            annotation_span,
            type_annotation,
            span,
        } => {
            // `span.start` is the `[` token. The closing `]` sits either at `span.end` for an
            // unannotated array or immediately before `annotation_span` when a `: Type`
            // ascription follows. Array literals never take a trailing comma (the grammar is
            // LL(1) and rejects one outright), unlike the one-tuple special case above.
            let close_pos = annotation_span
                .map(|annotation_span| annotation_span.start.start())
                .unwrap_or_else(|| span.end.end());
            let mut list = emit_list(
                source,
                depth,
                ("[", "]"),
                (span.start.start(), close_pos),
                elements,
                false,
            );
            if let Some(type_annotation) = type_annotation {
                let comments = annotation_span
                    .map(|annotation| {
                        gap_between(source, annotation.start, type_annotation.span().start, &[])
                    })
                    .unwrap_or_default();
                let gap = emit_gap(&comments, Spacing::None, 0);
                list = list
                    .append(":")
                    .append(trivia_doc(if gap.is_empty() {
                        " ".to_owned()
                    } else {
                        gap
                    }))
                    .append(verbatim_doc(render_type_expr(type_annotation, source)));
            }
            (list, Level::PRIMARY)
        }
        Expr::TupleIndex { base, index, span } => (
            format_postfix_base(base, source, depth, span.start.start())
                .append(gap_doc(
                    &grouping_gap(source, base.span().end.end(), span.end.start(), &["."]),
                    Spacing::None,
                ))
                .append(Doc::text(index.to_string())),
            Level::POSTFIX,
        ),
        Expr::If {
            cond,
            then_branch,
            else_branch,
            span,
        } => {
            // `if` <cond> `{` <then_branch> `}` [ `else` ( `{` <else_branch> `}` | <else-if> ) ].
            // None of `if`/`{`/`}`/`else` carry their own span (each is a fixed keyword/delimiter
            // consumed structurally by the parser, never recorded as its own node), so every gap
            // between two real spans is scanned for its expected fixed token(s) alongside any
            // interleaved comment. `Spacing::Around` is used throughout (rather than `None`) so
            // that a gap holding more than one fixed token (`"}"`, `"else"`, and — for a braced
            // else — `"{"`) still gets a single space between each token and the next, matching
            // the pre-`scan_gap` literal `" } else { "` spacing exactly in the comment-free case.
            let cond_s = restricted_doc(
                cond,
                source,
                depth,
                crate::ExpressionRestriction::NoStructLiteral,
            );
            let then_s = format_at(then_branch, source, depth, Level::OR);
            let if_gap = grouping_gap(source, span.start.start(), header_start(cond), &["if"]);
            let open_gap = grouping_gap(
                source,
                header_end(cond),
                then_branch.span().start.start(),
                &["{"],
            );
            let mut text = Doc::nil();
            // Only the very first gap is `trim_start`'d: it opens the whole node's text, so any
            // leading space `Spacing::Around` would otherwise add before `"if"` must be dropped.
            text = text.append(trivia_doc(
                emit_gap(&if_gap, Spacing::Around, 0)
                    .trim_start()
                    .to_owned(),
            ));
            text = text.append(cond_s);
            text = text.append(body_gap_doc(&open_gap, false).append(then_s).nest(4));
            match else_branch {
                None => {
                    // No `else`: the gap between the then-branch and the node's own end holds
                    // just the closing `}` (plus any comment before it).
                    let close_gap = gap_between(source, then_branch.span().end, span.end, &["}"]);
                    // `trim_end`'d because this is the node's last piece of text: a trailing
                    // space here would otherwise survive into the caller's output (or, wrapped in
                    // parens by `format_at`, land right before the close-paren).
                    text = text.append(Doc::line()).append(trivia_doc(
                        emit_gap(&close_gap, Spacing::Around, 0).trim().to_owned(),
                    ));
                }
                Some(else_branch) => {
                    let is_chain = matches!(else_branch.as_ref(), Expr::If { .. });
                    // An `else if` chain has no braces of its own around the nested `if`; a plain
                    // `else` block does.
                    let expected: &[&'static str] = if is_chain {
                        &["}", "else"]
                    } else {
                        &["}", "else", "{"]
                    };
                    // One scan across the whole then-branch-to-else-branch gap (rather than one
                    // scan per token) so a comment sitting anywhere in it -- before `}`, between
                    // `}` and `else`, or between `else` and `{` -- is recovered in source order.
                    let else_gap = gap_between(
                        source,
                        then_branch.span().end,
                        else_branch.span().start,
                        expected,
                    );
                    text = text.append(Doc::line());
                    if is_chain {
                        // The nested `if` recurses through `render`, which lays out its own
                        // `if`/`{`/`}`/`else` gaps (including its own trailing `trim_end`), so
                        // nothing further is appended here.
                        let (else_s, _) = render(else_branch, source, depth);
                        text = text.append(body_gap_doc(&else_gap, true)).append(else_s);
                    } else {
                        let else_s = format_at(else_branch, source, depth, Level::OR);
                        text = text.append(body_gap_doc(&else_gap, true).append(else_s).nest(4));
                        let final_close_gap =
                            gap_between(source, else_branch.span().end, span.end, &["}"]);
                        text = text.append(Doc::line()).append(trivia_doc(
                            emit_gap(&final_close_gap, Spacing::Around, 0)
                                .trim()
                                .to_owned(),
                        ));
                    }
                }
            }
            (text.group(), Level::PRIMARY)
        }
        Expr::Closure { params, body, span } => {
            let body_s = format_at(body, source, depth, Level::OR);
            let header = if params.is_empty() {
                Doc::text("||")
            } else {
                let leading = gap_between(source, span.start, params[0].name_span.start, &[]);
                let mut parameters =
                    trivia_doc(emit_gap(&leading, Spacing::None, 0).trim_start().to_owned());
                for (index, param) in params.iter().enumerate() {
                    if index > 0 {
                        let comma = gap_between(
                            source,
                            params[index - 1].type_expr.span().end,
                            param.name_span.start,
                            &[","],
                        );
                        parameters = parameters.append(gap_doc(&comma, Spacing::CommaAfter));
                    }
                    let colon = gap_between(
                        source,
                        param.name_span.end,
                        param.type_expr.span().start,
                        &[":"],
                    );
                    parameters = parameters
                        .append(Doc::text(param.name.clone()))
                        .append(trivia_doc(emit_gap(&colon, Spacing::CommaAfter, 0)))
                        .append(verbatim_doc(render_type_expr(&param.type_expr, source)));
                }
                Doc::text("|")
                    .append(Doc::line_().append(parameters).nest(4))
                    .append(Doc::line_())
                    .append("|")
                    .group()
            };
            let (tail_start, expected): (proc_macro2::Span, &[&'static str]) = match params.last() {
                Some(last) => (last.type_expr.span().end, &["|"]),
                None => (span.start, &[]),
            };
            let pieces = gap_between(source, tail_start, body.span().start, expected);
            let comments: Vec<GapPiece> = pieces
                .into_iter()
                .filter(|p| matches!(p, GapPiece::Comment(_)))
                .collect();
            let gap = emit_gap(&comments, Spacing::None, 0);
            // A `//` comment among `comments` made `emit_gap` end `gap` with a line wrap (`\n` +
            // continuation indent); the body must continue right there, so `gap` is used verbatim
            // (already correctly spaced/wrapped) rather than trimmed — trimming would collapse the
            // wrap and glue the body onto the comment's own line, commenting it out.
            let has_line_comment = comments
                .iter()
                .any(|p| matches!(p, GapPiece::Comment(Comment::Line(_))));
            let sep = if has_line_comment {
                trivia_doc_at(gap, 0)
            } else if gap.trim().is_empty() {
                Doc::line()
            } else {
                trivia_doc(format!(" {} ", gap.trim()))
            };
            // A closure's body greedily consumes expressions, including following postfixes.
            // Its primary-expression grammar position does not make it a tight-binding value.
            (
                header.append(sep.append(body_s).nest(4)).group(),
                Level::RANGE,
            )
        }
    }
}

/// Returns the first rendered header token's position, excluding omitted operand groups.
///
/// Prefix/postfix operators and explicit expression delimiters retain their starting positions;
/// postfix rendering owns any leading receiver-group trivia.
///
/// - Complexity: O(h) in the visited leftmost AST path, with O(h) stack space.
fn header_start(expr: &Expr) -> proc_macro2::LineColumn {
    match expr {
        Expr::Op { name, operands, .. }
            if operands.len() == 2 || (name == "range_from" && operands.len() == 1) =>
        {
            header_start(&operands[0])
        }
        Expr::Logical { lhs, .. } => header_start(lhs),
        Expr::Cast { expr, .. } => header_start(expr),
        _ => expr.span().start.start(),
    }
}

/// Returns the last rendered header token's position, excluding omitted operand/body groups.
///
/// Postfix operators and explicit expression delimiters retain their own ending positions.
///
/// - Complexity: O(h) in the visited rightmost AST path, with O(h) stack space.
fn header_end(expr: &Expr) -> proc_macro2::LineColumn {
    match expr {
        Expr::Op { name, operands, .. } if name != "range_from" && !operands.is_empty() => {
            header_end(operands.last().expect("non-empty operands"))
        }
        Expr::Logical { rhs, .. } => header_end(rhs),
        Expr::Closure { body, .. } => header_end(body),
        _ => expr.span().end.end(),
    }
}

/// Recovers a field or header gap across grouping parentheses omitted from the AST.
///
/// - Precondition: positions refer to `source`, or `source` is empty.
/// - Postcondition: comments and expected tokens retain source order without grouping tokens.
/// - Complexity: O(g + e) time and space for g gap bytes and e expected-token bytes.
fn grouping_gap(
    source: &Source<'_>,
    from: proc_macro2::LineColumn,
    to: proc_macro2::LineColumn,
    expected: &[&'static str],
) -> Vec<GapPiece> {
    let gap = source.between(from, to);
    let mut rest = gap;
    let mut tokens = Vec::new();
    let mut next = 0;
    loop {
        rest = rest.trim_start();
        if let Some(after) = rest.strip_prefix("//") {
            rest = &after[after.find('\n').unwrap_or(after.len())..];
        } else if let Some((_, after)) = take_block_comment(rest) {
            rest = after;
        } else if let Some(after) = rest.strip_prefix('(') {
            tokens.push("(");
            rest = after;
        } else if let Some(after) = rest.strip_prefix(')') {
            tokens.push(")");
            rest = after;
        } else if next < expected.len() && rest.starts_with(expected[next]) {
            tokens.push(expected[next]);
            rest = &rest[expected[next].len()..];
            next += 1;
        } else {
            break;
        }
    }
    tokens.extend_from_slice(&expected[next..]);
    scan_gap(gap, &tokens)
        .into_iter()
        .filter(|piece| !matches!(piece, GapPiece::Punct("(" | ")")))
        .collect()
}

/// Detects record syntax reached without an explicit expression boundary in a header.
///
/// - Complexity: O(n) in the number of visited AST nodes.
fn header_contains_record(expr: &Expr) -> bool {
    match expr {
        Expr::Struct { .. } => true,
        Expr::Op { operands, .. } => operands.iter().any(header_contains_record),
        Expr::Logical { lhs, rhs, .. } => {
            header_contains_record(lhs) || header_contains_record(rhs)
        }
        Expr::Cast { expr, .. } | Expr::Closure { body: expr, .. } => header_contains_record(expr),
        Expr::Member { receiver, .. } => header_contains_record(receiver),
        Expr::TupleIndex { base, .. } => header_contains_record(base),
        Expr::Apply { callee, .. } => header_contains_record(callee),
        Expr::Literal { .. }
        | Expr::Ident { .. }
        | Expr::Tuple { .. }
        | Expr::Array { .. }
        | Expr::If { .. } => false,
    }
}

/// Formats an embedded expression while preserving a header's brace reservation.
///
/// Groups the complete header when an otherwise unparenthesized record would be ambiguous.
/// Ordinary delimiters and nested branch bodies retain their normal formatting.
///
/// - Precondition: `source` is the parsed source, or empty for a hand-built AST.
/// - Complexity: O(s + v + g + b) construction time and space for s source bytes,
///   v node visits, g inspected source-slice bytes and b document-text bytes, plus pretty layout.
///
/// # Examples
/// ```
/// use cel_parser::{AstContext, ExpressionRestriction, OpLookup, Parser, format_expr_restricted};
/// let source = "(Point { x: 1.0 }).x";
/// let ast = Parser::<AstContext>::new(OpLookup::new()).parse_str_ast(source).unwrap();
/// let text = format_expr_restricted(&ast, source, 0, ExpressionRestriction::NoStructLiteral);
/// assert_eq!(text, "(Point { x: 1.0 }.x)");
/// ```
pub fn format_expr_restricted(
    expr: &Expr,
    source: &str,
    depth: usize,
    restriction: crate::ExpressionRestriction,
) -> String {
    render_document(
        restricted_doc(expr, &Source::new(source), depth, restriction),
        depth,
        depth * 4,
        100,
        0,
    )
}

/// Builds a brace-reserving expression without independently laying out its children.
///
/// - Complexity: O(v + g + b) time and space for v node visits, g inspected source-slice bytes
///   and b document-text bytes.
fn restricted_doc(
    expr: &Expr,
    source: &Source<'_>,
    depth: usize,
    restriction: crate::ExpressionRestriction,
) -> Doc {
    let text = format_at(expr, source, depth, Level::RANGE);
    if restriction == crate::ExpressionRestriction::NoStructLiteral && header_contains_record(expr)
    {
        Doc::text("(").append(text).append(")")
    } else {
        text
    }
}

/// Formats a brace-reserving header and its surrounding comments within explicit boundaries.
///
/// `bounds` runs from just after the introducing keyword to just before the body brace.
/// Grouping parentheses omitted from the AST do not interrupt comment recovery.
///
/// - Precondition: `source` is the parsed source and `bounds` encloses `expr`, or source is empty.
/// - Postcondition: boundary comments appear exactly once in source order.
/// - Complexity: O(s + v + g + b) construction time and space for s source bytes,
///   v node visits, g inspected source-slice bytes and b document-text bytes, plus pretty layout.
///
/// # Examples
/// ```
/// use cel_parser::{AstContext, OpLookup, Parser, format_expr_header};
/// let source = "(Point { x: 1.0 }) /* keep */";
/// let expr = Parser::<AstContext>::new(OpLookup::new()).parse_str_ast(source).unwrap();
/// let text = format_expr_header(&expr, source, 0, (
///     proc_macro2::LineColumn { line: 1, column: 0 },
///     proc_macro2::LineColumn { line: 1, column: source.len() },
/// ));
/// assert_eq!(text, "(Point { x: 1.0 }) /* keep */");
/// ```
pub fn format_expr_header(
    expr: &Expr,
    source: &str,
    depth: usize,
    bounds: (proc_macro2::LineColumn, proc_macro2::LineColumn),
) -> String {
    format_expr_header_with_layout(expr, source, depth, bounds, depth * 4, 100, 0)
}

/// Formats a brace-reserving header at `column`, reserving `suffix_columns` on its last line.
///
/// The target width includes the already-written prefix and following punctuation. Continuations
/// nest four spaces from `depth`; unbreakable tokens and comments may exceed `width`.
///
/// - Precondition: `source` is the parsed source and `bounds` encloses `expr`, or source is empty.
/// - Complexity: O(s + v + g + b) construction time and space for s source bytes,
///   v node visits, g inspected source-slice bytes and b document-text bytes, plus pretty layout.
///
/// # Examples
/// ```
/// use cel_parser::{AstContext, OpLookup, Parser, format_expr_header_with_layout, LineColumn};
/// let source = "value";
/// let expr = Parser::<AstContext>::new(OpLookup::new()).parse_str_ast(source).unwrap();
/// assert_eq!(format_expr_header_with_layout(&expr, source, 1,
///     (LineColumn { line: 1, column: 0 }, LineColumn { line: 1, column: 5 }),
///     16, 100, 2), "value");
/// ```
pub fn format_expr_header_with_layout(
    expr: &Expr,
    source: &str,
    depth: usize,
    bounds: (proc_macro2::LineColumn, proc_macro2::LineColumn),
    column: usize,
    width: usize,
    suffix_columns: usize,
) -> String {
    let source = &Source::new(source);
    let leading = grouping_gap(source, bounds.0, header_start(expr), &[]);
    let trailing = grouping_gap(source, header_end(expr), bounds.1, &[]);
    let mut text = trivia_doc(emit_gap(&leading, Spacing::None, 0).trim_start().to_owned());
    text = text.append(restricted_doc(
        expr,
        source,
        depth,
        crate::ExpressionRestriction::NoStructLiteral,
    ));
    let tail = emit_gap(&trailing, Spacing::None, 0);
    if matches!(trailing.last(), Some(GapPiece::Comment(Comment::Line(_)))) {
        text = text.append(trivia_doc(tail));
    } else {
        text = text.append(trivia_doc(tail.trim_end().to_owned()));
    }
    render_document(text, depth, column, width, suffix_columns)
}

/// Renders `expr`, wrapping it in parens if its own level is looser than `min_level` requires.
///
/// `source` and `depth` are forwarded to [`render`] unchanged; see its doc comment.
///
/// - Complexity: O(v + g + b) time and space for v node visits, g inspected source-slice bytes
///   and b document-text bytes.
fn format_at(expr: &Expr, source: &Source<'_>, depth: usize, min_level: Level) -> Doc {
    let (text, level) = render(expr, source, depth);
    if level < min_level {
        Doc::text("(").append(text).append(")").group()
    } else {
        text
    }
}

/// Pretty-prints `expr` back to CEL source text — see the module doc for the printing rules.
///
/// `source` is the text `expr` was parsed from (or `""` for a hand-built `Expr`); `depth` supplies
/// the base indentation for wrapped continuations and the assumed initial output column.
/// Postfix bases retain leading group comments, and member-dot comments survive omitted groups.
/// Closure bases stay grouped so following postfixes cannot move into their bodies.
///
/// - Complexity: O(s + v + g + b) construction time and space for s source bytes,
///   v node visits, g inspected source-slice bytes and b document-text bytes, plus pretty layout.
///
/// # Examples
///
/// ```
/// use cel_parser::{AstContext, OpLookup, Parser, format_expr};
///
/// let mut parser = Parser::<AstContext>::new(OpLookup::new());
/// let expr = parser.parse_str_ast("(1i32 + 2i32) * 3i32").unwrap();
/// assert_eq!(format_expr(&expr, "(1i32 + 2i32) * 3i32", 0), "(1i32 + 2i32) * 3i32");
/// ```
pub fn format_expr(expr: &Expr, source: &str, depth: usize) -> String {
    format_expr_with_layout(expr, source, depth, depth * 4, 100, 0)
}

/// Formats an expression at the actual output `column` with a target `width`.
///
/// Reserves `suffix_columns` for following punctuation on the final line. Short expressions remain
/// inline. Lists, records and closure bodies nest four spaces; binary continuations lead with their
/// operator. Unbreakable tokens and comments may exceed the target. The result excludes the prefix,
/// suffix and initial indentation; continuation indentation starts at `depth * 4`.
/// When an indented continuation has more room and the first fragment cannot fit after the prefix,
/// the expression starts on a new line indented four spaces beyond `depth`.
///
/// - Precondition: `source` is the expression's parsed source, or empty for a hand-built AST.
/// - Complexity: O(s + v + g + b) construction time and space for s source bytes,
///   v node visits, g inspected source-slice bytes and b document-text bytes, plus pretty layout.
///
/// # Examples
/// ```
/// use cel_parser::{AstContext, OpLookup, Parser, format_expr_with_layout};
/// let source = "first + second";
/// let expr = Parser::<AstContext>::new(OpLookup::new()).parse_str_ast(source).unwrap();
/// assert_eq!(format_expr_with_layout(&expr, source, 1, 12, 26, 1),
///     "first\n        + second");
/// ```
pub fn format_expr_with_layout(
    expr: &Expr,
    source: &str,
    depth: usize,
    column: usize,
    width: usize,
    suffix_columns: usize,
) -> String {
    render_document(
        format_at(expr, &Source::new(source), depth, Level::RANGE),
        depth,
        column,
        width,
        suffix_columns,
    )
}

/// Lays out one complete expression document, accounting for both embedding boundaries.
///
/// - Complexity: O(n + column + suffix_columns) space; layout uses the document library's group
///   fit traversal and never renders subtrees to measure them.
fn render_document(
    doc: Doc,
    depth: usize,
    column: usize,
    width: usize,
    suffix_columns: usize,
) -> String {
    let prefix = " ".repeat(column);
    let suffix = " ".repeat(suffix_columns);
    let doc = if column > depth * 4 + 4 {
        // Group only the boundary: fitting inspects the following document in broken mode,
        // stopping at its first existing break instead of demanding a flat whole expression.
        Doc::hardline()
            .append("    ")
            .flat_alt(Doc::nil())
            .group()
            .append(Doc::column(move |current| {
                if current == column {
                    doc.clone()
                } else {
                    doc.clone().nest(4)
                }
            }))
    } else {
        doc
    };
    let document = Doc::text(prefix)
        .append(doc.nest((depth * 4) as isize))
        .append(Doc::text(suffix));
    let mut out = String::new();
    document
        .render_fmt(width, &mut out)
        .expect("writing to String is infallible");
    out.truncate(out.len() - suffix_columns);
    out.drain(..column);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AstContext, OpLookup, Parser};

    /// Long binary expressions wrap before their operators and reparse unchanged.
    #[test]
    fn wraps_long_expressions_with_leading_operators() {
        let source = std::iter::repeat_n("long_operand_name", 8)
            .collect::<Vec<_>>()
            .join(" + ");
        let text = fmt(&source);
        assert!(text.contains("\n    + "), "{text}");
        assert!(text.lines().all(|line| line.len() <= 100), "{text}");
        assert_eq!(fmt(&text), text);
    }

    /// Binary ranges retain compact spelling and expose a zero-width leading-operator boundary.
    #[test]
    fn wraps_binary_ranges_before_their_operators() {
        for operator in ["..", "..="] {
            let source = format!("{}{}{}", "a".repeat(45), operator, "b".repeat(45));
            let expr = parse(&source);
            let text = format_expr_with_layout(&expr, &source, 1, 13, 100, 1);
            assert_eq!(
                text,
                format!("{}\n        {operator}{}", "a".repeat(45), "b".repeat(45))
            );
            assert_eq!(
                format_expr_with_layout(&parse(&text), &text, 1, 13, 100, 1),
                text
            );
            assert_eq!(
                fmt(&format!("left{operator}right")),
                format!("left{operator}right")
            );
            let call = format!("call({source})");
            let formatted = fmt(&call);
            assert!(
                formatted.lines().all(|line| line.len() <= 100),
                "{formatted}"
            );
            assert_eq!(fmt(&formatted), formatted);
        }
    }

    /// An empty closure's header can move with its body when the embedding prefix fills the line.
    #[test]
    fn empty_closure_can_break_before_its_header() {
        let source = "|| predicate";
        let expr = parse(source);
        let text = format_expr_with_layout(&expr, source, 1, 100, 100, 1);
        assert_eq!(text, "\n        || predicate");
        assert_eq!(
            format_expr_with_layout(&parse(&text), &text, 1, 100, 100, 1),
            text
        );
    }

    /// Nested grammar-owned lists break without introducing unsupported trailing commas.
    #[test]
    fn wraps_nested_calls_records_arrays_and_closures() {
        let name = "a_long_argument_name_that_occupies_thirty_columns";
        for source in [
            format!("outer(inner({name}, {name}), [{name}, {name}])"),
            format!("Record {{ first: ({name}, {name}), second: [{name}, {name}] }}"),
            format!("map([{name}, {name}], |value: i32| value + {name} + {name})"),
        ] {
            let text = fmt(&source);
            assert!(text.contains('\n'), "{text}");
            assert!(text.lines().all(|line| line.len() <= 100), "{text}");
            assert_eq!(fmt(&text), text);
        }
    }

    /// Parameter lists wrap and retain comments on both sides of their separators.
    #[test]
    fn wraps_closure_parameters_without_losing_comments() {
        let source = "|/* first */ a_long_parameter: i32 /* second */, another_long_parameter: i32, third_long_parameter: i32, fourth_long_parameter: i32| a_long_parameter";
        let text = fmt(source);
        for label in ["first", "second"] {
            assert_eq!(text.matches(label).count(), 1, "{text}");
        }
        assert!(text.lines().all(|line| line.len() <= 100), "{text}");
        assert_eq!(fmt(&text), text);
    }

    /// Block comments do not disable leading-operator break opportunities.
    #[test]
    fn wraps_commented_binary_gaps() {
        let operand = "a".repeat(45);
        let source = format!("{operand} /* keep */ + {operand} + {operand}");
        let text = fmt(&source);
        assert!(text.lines().all(|line| line.len() <= 100), "{text}");
        assert_eq!(text.matches("keep").count(), 1);
        assert_eq!(fmt(&text), text);
    }

    /// Consecutive line comments keep separate mandatory line boundaries.
    #[test]
    fn consecutive_line_comments_remain_separate() {
        let text = fmt("1 + // first\n// second\n2");
        assert_eq!(text, "1 + // first\n    // second\n    2");
        assert_eq!(fmt(&text), text);
    }

    /// A comment after an opening brace cannot consume the first field or branch expression.
    #[test]
    fn brace_opening_line_comments_keep_their_mandatory_breaks() {
        for source in [
            "Record { // first\n value: 1 }",
            "if condition { // first\n 1 } else { // second\n 2 }",
        ] {
            let text = fmt(source);
            assert!(text.contains("// first\n"), "{text}");
            assert_eq!(fmt(&text), text);
        }
    }

    /// Width includes the actual prefix and suffix, not an independently reduced child budget.
    #[test]
    fn embedding_reserves_exact_prefix_and_suffix_columns() {
        let source = "first + second";
        let expr = parse(source);
        assert_eq!(format_expr_with_layout(&expr, source, 1, 12, 27, 1), source);
        assert_eq!(
            format_expr_with_layout(&expr, source, 1, 12, 26, 1),
            "first\n        + second"
        );
        let bounds = (
            proc_macro2::LineColumn { line: 1, column: 0 },
            proc_macro2::LineColumn {
                line: 1,
                column: source.len(),
            },
        );
        assert_eq!(
            format_expr_header_with_layout(&expr, source, 1, bounds, 12, 28, 2),
            source
        );
        assert_eq!(
            format_expr_header_with_layout(&expr, source, 1, bounds, 12, 27, 2),
            "first\n        + second"
        );
    }

    /// Literal-internal newlines retain their spelling rather than inheriting expression nesting.
    #[test]
    fn multiline_literal_spelling_is_independent_of_layout_nesting() {
        let source = "call(r#\"first\n  second\"#, 0xffi32)";
        let expr = parse(source);
        let text = format_expr_with_layout(&expr, source, 2, 20, 30, 1);
        assert!(text.contains("r#\"first\n  second\"#"), "{text}");
        assert!(text.contains("0xffi32"));
        assert_eq!(
            format_expr_with_layout(&parse(&text), &text, 2, 20, 30, 1),
            text
        );
    }

    /// Layout retains grammar, literal spelling, and precedence across every compound surface.
    #[test]
    fn narrow_layout_preserves_expression_semantics_and_reaches_a_fixed_point() {
        for source in [
            "-(first_long_name + second_long_name) as i32",
            "first_long_name - (second_long_name - third_long_name)",
            "(|value: i32| value + first_long_name)(second_long_name)",
            "(holder.long_member_name)(first_long_name, second_long_name)",
            "first_long_name..=second_long_name",
            "[first_long_name, second_long_name]: [i32]",
            "(first_long_name + second_long_name,)",
            "[(first_long_name, second_long_name), (third_long_name, fourth_long_name)]",
            "if (Record { value: first_long_name }).value { 0xffi32 } else { 1i32 }",
            "first_long_name /* keep\n block */ + second_long_name",
            "call(first_long_name, // keep line\n second_long_name)",
            "first_long_name + // first\n // second\n second_long_name",
        ] {
            let expr = parse(source);
            let text = format_expr_with_layout(&expr, source, 0, 0, 40, 0);
            let parsed = parse(&text);
            assert_eq!(
                format_expr_with_layout(&parsed, &text, 0, 0, 40, 0),
                text,
                "{source}"
            );
            assert_eq!(
                format_expr_with_layout(&parsed, &text, 0, 0, 10_000, 0),
                format_expr_with_layout(&expr, source, 0, 0, 10_000, 0),
                "{source} -> {text}"
            );
        }
        for (source, expected) in [
            ("-(10 + 3) as i32", -13),
            ("10 - (3 - 1)", 8),
            ("(|value: i32| value + 5)(7)", 12),
            ("(1 + 2,).0", 3),
            ("if true && false { 1 / 0 } else { 42 }", 42),
        ] {
            let text = format_expr_with_layout(&parse(source), source, 0, 0, 12, 0);
            let mut segment = crate::CELParser::new(OpLookup::new())
                .parse_str(&text)
                .unwrap();
            assert_eq!(
                segment.call0::<i32>().unwrap(),
                expected,
                "{source} -> {text}"
            );
        }
    }

    /// Maps empty, Unicode, newline, and final-line positions through one reusable index.
    #[test]
    fn source_index_preserves_character_boundaries() {
        let source = Source::new("é\nβ\n");
        assert_eq!(
            source.byte(proc_macro2::LineColumn { line: 1, column: 1 }),
            2
        );
        assert_eq!(
            source.byte(proc_macro2::LineColumn { line: 2, column: 1 }),
            5
        );
        assert_eq!(
            source.byte(proc_macro2::LineColumn { line: 3, column: 0 }),
            6
        );
        assert_eq!(
            Source::new("").byte(proc_macro2::LineColumn { line: 1, column: 0 }),
            0
        );
    }

    /// Parses one complete expression for formatter contracts.
    fn parse(source: &str) -> Expr {
        Parser::<AstContext>::new(OpLookup::new())
            .parse_str_ast(source)
            .unwrap()
    }

    /// Formats one parsed expression with the default layout.
    fn fmt(source: &str) -> String {
        format_expr(&parse(source), source, 0)
    }

    #[test]
    fn additive_and_multiplicative_reprint_without_extra_parens() {
        assert_eq!(fmt("1i32 + 2i32 * 3i32"), "1i32 + 2i32 * 3i32");
    }

    #[test]
    fn explicit_grouping_that_changes_precedence_keeps_its_parens() {
        assert_eq!(fmt("(1i32 + 2i32) * 3i32"), "(1i32 + 2i32) * 3i32");
    }

    #[test]
    fn left_associative_chain_at_the_same_precedence_has_no_parens() {
        assert_eq!(fmt("1i32 - 2i32 - 3i32"), "1i32 - 2i32 - 3i32");
    }

    #[test]
    fn a_right_leaning_tree_at_the_same_precedence_needs_parens() {
        // Not producible by real parsing (the grammar's additive_expression loop is always
        // left-associative) — built by hand to prove the printer round-trips a tree shape it
        // didn't itself produce. Uses Ident operands (rendered from `name`, not a span) so the
        // assertion reads as real text rather than the no-source-text fallback.
        fn ident(name: &str) -> Expr {
            Expr::Ident {
                name: name.to_string(),
                span: point(),
            }
        }
        fn point() -> crate::ExprSpan {
            crate::ExprSpan {
                start: proc_macro2::Span::call_site(),
                end: proc_macro2::Span::call_site(),
            }
        }
        let expr = Expr::Op {
            name: "-".to_string(),
            operands: vec![
                ident("a"),
                Expr::Op {
                    name: "-".to_string(),
                    operands: vec![ident("b"), ident("c")],
                    span: point(),
                },
            ],
            span: point(),
        };
        assert_eq!(format_expr(&expr, "", 0), "a - (b - c)");
    }

    #[test]
    fn nested_comparison_needs_parens_on_both_sides() {
        // Also not producible by real parsing (comparison_expression allows at most one
        // comparison, never a nested one) — proves format_expr stays reparseable even for a
        // hand-built tree shape the grammar itself can't emit.
        fn ident(name: &str) -> Expr {
            Expr::Ident {
                name: name.to_string(),
                span: crate::ExprSpan {
                    start: proc_macro2::Span::call_site(),
                    end: proc_macro2::Span::call_site(),
                },
            }
        }
        let inner = Expr::Op {
            name: "==".to_string(),
            operands: vec![ident("a"), ident("b")],
            span: crate::ExprSpan {
                start: proc_macro2::Span::call_site(),
                end: proc_macro2::Span::call_site(),
            },
        };
        let expr = Expr::Op {
            name: "==".to_string(),
            operands: vec![inner, ident("c")],
            span: crate::ExprSpan {
                start: proc_macro2::Span::call_site(),
                end: proc_macro2::Span::call_site(),
            },
        };
        assert_eq!(format_expr(&expr, "", 0), "(a == b) == c");
    }

    #[test]
    fn literal_notation_is_preserved_exactly() {
        assert_eq!(fmt("1920.0"), "1920.0");
        assert_eq!(fmt("1920.0f64"), "1920.0f64");
        assert_eq!(fmt("1i32"), "1i32");
    }

    #[test]
    fn unary_minus_of_a_binary_expression_needs_parens() {
        assert_eq!(fmt("-(1i32 + 2i32)"), "-(1i32 + 2i32)");
    }

    #[test]
    fn double_unary_minus_keeps_a_separating_space() {
        assert_eq!(fmt("- -1i32"), "- -1i32");
    }

    #[test]
    fn cast_chain_reprints_without_extra_parens() {
        assert_eq!(fmt("x as i32 as f64"), "x as i32 as f64");
    }

    #[test]
    fn unary_minus_before_a_cast_needs_no_parens() {
        // Matches Rust: `-x as f64` parses as `(-x) as f64` - unary already binds tighter than
        // Cast, so the printer doesn't need to add parens to preserve that grouping.
        assert_eq!(fmt("-x as f64"), "-x as f64");
    }

    #[test]
    fn explicit_grouping_before_a_cast_keeps_its_parens() {
        // `as` binds tighter than `+`, so without parens `(a + b) as i32` would reprint as
        // `a + b as i32` - a different expression (`a + (b as i32)`). The parens must survive.
        assert_eq!(fmt("(a + b) as i32"), "(a + b) as i32");
    }

    #[test]
    fn cast_operand_of_an_additive_expression_needs_no_parens() {
        // `a + b as i32` already parses as `a + (b as i32)` (`as` binds tighter than `+`), so no
        // parens are needed around the cast when reprinting.
        assert_eq!(fmt("a + b as i32"), "a + b as i32");
    }

    #[test]
    fn explicit_grouping_around_a_cast_before_multiplicative_is_redundant_and_dropped() {
        // `(x as i32) * y` parses to the exact same tree as `x as i32 * y` (cast already binds
        // tighter than `*`), so the now-redundant parens are dropped on reprint - matching the
        // module doc's "parens added only where required, not exhaustively".
        assert_eq!(fmt("(x as i32) * y"), "x as i32 * y");
    }

    #[test]
    fn one_tuple_keeps_its_trailing_comma() {
        assert_eq!(fmt("(1i32,)"), "(1i32,)");
    }

    #[test]
    fn multi_element_tuple_has_no_trailing_comma() {
        assert_eq!(fmt("(1i32, 2i32)"), "(1i32, 2i32)");
    }

    #[test]
    fn arrays_format_with_brackets_and_normalized_commas() {
        assert_eq!(fmt("[1i32,2i32]"), "[1i32, 2i32]");
        assert_eq!(fmt("[[1i32], [2i32]]"), "[[1i32], [2i32]]");
    }

    #[test]
    fn typed_arrays_format_with_a_type_ascription() {
        assert_eq!(fmt("[1i32,2i32]:[i32]"), "[1i32, 2i32]: [i32]");
        assert_eq!(fmt("[]:[[i32]]"), "[]: [[i32]]");
    }

    #[test]
    fn format_array_annotation_with_a_generic_builtin_element_type() {
        let source = "[]: [RangeInclusive(f64)]";
        let expr = parse(source);
        let formatted = format_expr(&expr, source, 0);
        assert_eq!(formatted, source);
    }

    #[test]
    fn array_comments_remain_in_source_order() {
        assert_eq!(
            fmt("[1i32 /* first */, /* second */ 2i32]"),
            "[1i32 /* first */, /* second */ 2i32]"
        );
    }

    #[test]
    fn comments_in_a_call_argument_list_are_preserved() {
        assert_eq!(
            fmt("f(/* a */ 1i32, /* b */ 2i32)"),
            "f(/* a */ 1i32, /* b */ 2i32)"
        );
    }

    #[test]
    fn a_comment_before_a_closing_call_paren_is_preserved() {
        assert_eq!(fmt("f(1i32 /* end */)"), "f(1i32 /* end */)");
    }

    #[test]
    fn comments_in_a_tuple_are_preserved() {
        assert_eq!(fmt("(1i32, /* mid */ 2i32)"), "(1i32, /* mid */ 2i32)");
    }

    #[test]
    fn comment_free_calls_and_tuples_are_unchanged() {
        assert_eq!(fmt("f(1i32, 2i32)"), "f(1i32, 2i32)");
        assert_eq!(fmt("(1i32, 2i32)"), "(1i32, 2i32)");
        assert_eq!(fmt("(1i32,)"), "(1i32,)");
        assert_eq!(fmt("(1i32, 2i32).1"), "(1i32, 2i32).1");
    }

    #[test]
    fn if_without_else_omits_the_else_clause() {
        assert_eq!(fmt("if true { 1i32 }"), "if true { 1i32 }");
    }

    #[test]
    fn if_else_reprints_both_branches() {
        assert_eq!(
            fmt("if true { 1i32 } else { 2i32 }"),
            "if true { 1i32 } else { 2i32 }"
        );
    }

    #[test]
    fn else_if_chain_has_no_braces_around_the_nested_if() {
        let source = "if true { 1i32 } else if false { 2i32 } else { 3i32 }";
        assert_eq!(fmt(source), source);
    }

    #[test]
    fn a_comment_after_if_condition_is_preserved() {
        assert_eq!(fmt("if a /* c */ { 1i32 }"), "if a /* c */ { 1i32 }");
    }

    #[test]
    fn a_comment_before_else_is_preserved() {
        assert_eq!(
            fmt("if a { 1i32 } /* e */ else { 2i32 }"),
            "if a { 1i32 } /* e */ else { 2i32 }"
        );
    }

    #[test]
    fn logical_or_and_and_are_not_desugared_and_need_no_extra_parens() {
        assert_eq!(fmt("a || b && c"), "a || b && c");
    }

    #[test]
    fn a_comment_around_a_logical_operator_is_preserved() {
        assert_eq!(fmt("a /* x */ || b"), "a /* x */ || b");
        assert_eq!(fmt("a && /* y */ b"), "a && /* y */ b");
    }

    #[test]
    fn format_is_idempotent_through_a_reparse() {
        let source = "(1i32 + 2i32) * 3i32 - -4i32";
        let once = fmt(source);
        let twice = format_expr(&parse(&once), &once, 0);
        assert_eq!(once, twice);
    }

    #[test]
    fn typed_array_annotation_round_trips_through_reparse() {
        let source = "[1.0, 42.5]: [f64]";
        let once = fmt(source);
        let twice = format_expr(&parse(&once), &once, 0);
        assert_eq!(once, twice);
    }

    #[test]
    fn closure_with_one_param_reprints_with_its_type() {
        assert_eq!(fmt("|x: i32| x + 1i32"), "|x: i32| x + 1i32");
    }

    #[test]
    fn closure_with_no_params_reprints_with_double_pipe() {
        assert_eq!(fmt("|| 1i32"), "|| 1i32");
    }

    #[test]
    fn closure_with_multiple_params_joins_them_with_commas() {
        assert_eq!(fmt("|x: i32, y: i32| x + y"), "|x: i32, y: i32| x + y");
    }

    #[test]
    fn closure_with_a_tuple_typed_param_reprints_the_tuple_type() {
        assert_eq!(fmt("|x: (i32, f64)| x.0"), "|x: (i32, f64)| x.0");
    }

    #[test]
    fn a_comment_before_a_closure_body_is_preserved() {
        assert_eq!(
            fmt("|x: i32| /* b */ x + 1i32"),
            "|x: i32| /* b */ x + 1i32"
        );
    }

    #[test]
    fn a_line_comment_before_a_closure_body_wraps_the_expression() {
        // A `//` comment forces a line wrap; the body must land on the continuation line, not get
        // glued onto the comment's own line (which would comment it out and make the output
        // non-reparseable).
        let source = "|x: i32| // c\n x + 1i32";
        let once = fmt(source);
        assert!(once.contains("// c"), "comment must survive: {once:?}");
        assert!(once.contains("x + 1i32"), "body must survive: {once:?}");
        let twice = format_expr(&parse(&once), &once, 0);
        assert_eq!(once, twice, "formatting must be idempotent");
    }

    #[test]
    fn range_inclusive_reprints_without_spaces() {
        assert_eq!(fmt("1i32..=5i32"), "1i32..=5i32");
    }

    #[test]
    fn range_reprints_without_spaces() {
        assert_eq!(fmt("1i32..5i32"), "1i32..5i32");
    }

    #[test]
    fn range_from_reprints_without_spaces() {
        assert_eq!(fmt("1i32.."), "1i32..");
    }

    #[test]
    fn range_to_reprints_without_spaces() {
        assert_eq!(fmt("..5i32"), "..5i32");
    }

    #[test]
    fn range_to_inclusive_reprints_without_spaces() {
        assert_eq!(fmt("..=5i32"), "..=5i32");
    }

    #[test]
    fn range_full_reprints_as_two_dots() {
        assert_eq!(fmt(".."), "..");
    }

    #[test]
    fn range_endpoints_that_are_comparisons_need_no_parens() {
        // From `is_range_expression`'s own doc comment: `a == b..c == d` parses as
        // `(a == b)..(c == d)` -- comparison already binds tighter than range, so the printer
        // doesn't need parens to preserve that grouping.
        assert_eq!(fmt("a == b..c == d"), "a == b..c == d");
    }

    #[test]
    fn range_endpoints_that_are_arithmetic_need_no_parens() {
        // From the same doc comment: `1 + 2..3 * 4` parses as `(1 + 2)..(3 * 4)` -- arithmetic
        // binds well inside range's own endpoints, so no parens are needed on reprint either.
        assert_eq!(fmt("1i32 + 2i32..3i32 * 4i32"), "1i32 + 2i32..3i32 * 4i32");
    }

    #[test]
    fn emit_gap_around_a_plain_operator_uses_single_spaces() {
        let pieces = vec![GapPiece::Punct("+")];
        assert_eq!(emit_gap(&pieces, Spacing::Around, 0), " + ");
    }

    #[test]
    fn emit_gap_none_spacing_glues_a_range_operator() {
        let pieces = vec![GapPiece::Punct("..")];
        assert_eq!(emit_gap(&pieces, Spacing::None, 0), "..");
    }

    #[test]
    fn emit_gap_comma_after_puts_one_trailing_space() {
        let pieces = vec![GapPiece::Punct(",")];
        assert_eq!(emit_gap(&pieces, Spacing::CommaAfter, 0), ", ");
    }

    #[test]
    fn emit_gap_inlines_a_block_comment_before_an_operator() {
        let pieces = vec![
            GapPiece::Comment(Comment::Block("a".to_string())),
            GapPiece::Punct("+"),
        ];
        assert_eq!(emit_gap(&pieces, Spacing::Around, 0), " /* a */ + ");
    }

    #[test]
    fn emit_gap_inlines_a_block_comment_after_an_operator() {
        let pieces = vec![
            GapPiece::Punct("+"),
            GapPiece::Comment(Comment::Block("b".to_string())),
        ];
        assert_eq!(emit_gap(&pieces, Spacing::Around, 0), " + /* b */ ");
    }

    #[test]
    fn emit_gap_wraps_after_a_line_comment_to_the_continuation_indent() {
        let pieces = vec![
            GapPiece::Punct("+"),
            GapPiece::Comment(Comment::Line("why".to_string())),
        ];
        // depth 0 => continuation at depth 1 (4 spaces). The operator keeps its leading space; the
        // line comment ends the line, and the next operand resumes at the continuation indent.
        assert_eq!(emit_gap(&pieces, Spacing::Around, 0), " + // why\n    ");
    }

    #[test]
    fn emit_gap_comma_after_a_line_comment_keeps_the_full_continuation_indent() {
        // A comma following a line comment must NOT lose one space of the continuation indent --
        // that indent is meaningful (it's `indent(depth + 1)`), unlike the single incidental
        // space a block comment or an `Around` punct leaves before a comma.
        let pieces = vec![
            GapPiece::Comment(Comment::Line("why".to_string())),
            GapPiece::Punct(","),
        ];
        assert_eq!(emit_gap(&pieces, Spacing::CommaAfter, 0), " // why\n    , ");
    }

    #[test]
    fn a_block_comment_before_a_binary_operator_is_preserved() {
        let source = "1i32 /* a */ + 2i32";
        assert_eq!(fmt(source), "1i32 /* a */ + 2i32");
    }

    #[test]
    fn a_block_comment_after_a_binary_operator_is_preserved() {
        let source = "1i32 + /* b */ 2i32";
        assert_eq!(fmt(source), "1i32 + /* b */ 2i32");
    }

    #[test]
    fn comments_on_both_sides_of_a_binary_operator_are_preserved() {
        let source = "1i32 /* a */ + /* b */ 2i32";
        assert_eq!(fmt(source), "1i32 /* a */ + /* b */ 2i32");
    }

    #[test]
    fn a_comment_after_unary_minus_is_preserved() {
        assert_eq!(fmt("- /* neg */ 1i32"), "- /* neg */ 1i32");
    }

    #[test]
    fn a_comment_around_a_cast_operator_is_preserved() {
        assert_eq!(fmt("x /* c */ as i32"), "x /* c */ as i32");
    }

    #[test]
    fn a_comment_inside_a_range_is_preserved() {
        assert_eq!(fmt("1i32 /* r */ ..5i32"), "1i32 /* r */ ..5i32");
    }

    #[test]
    fn comment_free_unary_cast_and_range_are_unchanged() {
        assert_eq!(fmt("-1i32"), "-1i32");
        assert_eq!(fmt("x as i32"), "x as i32");
        assert_eq!(fmt("1i32..5i32"), "1i32..5i32");
    }

    #[test]
    fn a_line_comment_after_a_cast_operator_wraps_the_expression() {
        // A `//` comment forces a line wrap; `i32` must land on the continuation line, not get
        // glued onto the comment's own line (which would comment it out and make the output
        // non-reparseable).
        let source = "x as // c\n i32";
        let once = fmt(source);
        assert!(once.contains("// c"), "comment must survive: {once:?}");
        assert!(once.contains("i32"), "type name must survive: {once:?}");
        let twice = format_expr(&parse(&once), &once, 0);
        assert_eq!(once, twice, "formatting must be idempotent");
    }

    #[test]
    fn issue_201_line_comment_inside_an_expression_round_trips() {
        // The exact repro from stlab/cel-rs#201.
        let source = "1i32 +\n    // why 2\n    2i32";
        let once = fmt(source);
        assert_eq!(once, "1i32 + // why 2\n    2i32");
        assert!(once.contains("// why 2"), "comment must survive: {once:?}");
        let twice = format_expr(&parse(&once), &once, 0);
        assert_eq!(once, twice, "formatting must be idempotent");
    }

    #[test]
    fn the_full_inline_comment_example_round_trips() {
        // From the design discussion: block comments on both operands and around the operator,
        // plus a trailing line comment. The leading `/* p */` and trailing `/* r */` sit outside
        // the expression's own span (before its first token, after its last), which the design
        // doc explicitly carves out of `format_expr`'s scope: "`Expr`'s own outer boundary ... is
        // not `format_expr`'s to own ... that boundary belongs to whatever embeds the expression"
        // (adam-lang's Part A2, not yet implemented) -- so only the two in-scope, inter-operand
        // comments are expected to survive here.
        let source = "/* p */ 1i32 /* q */ + // hello\n2i32 /* r */";
        let once = fmt(source);
        assert!(
            once.contains("/* q */") && once.contains("// hello"),
            "the in-scope operand/operator comments survive: {once:?}"
        );
        let twice = format_expr(&parse(&once), &once, 0);
        assert_eq!(once, twice);
    }
}
