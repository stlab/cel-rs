//! Pretty-prints a [`crate::Expr`] tree back to CEL source text: precedence-aware
//! parenthesization (added only where required, not exhaustively), single-space-around-operator
//! normalization, and no line-wrapping (every expression is emitted on one line regardless of
//! length — see the design doc's "Line wrapping" decision). Literal leaves are re-emitted via
//! [`proc_macro2::Span::source_text`] rather than synthesized from [`crate::Literal`], so exact
//! original notation (`1920.0` vs `1920.0f64`, a byte literal's spelling) round-trips.

use crate::ast::{Expr, LogicalOp};
use crate::trivia::{Comment, GapPiece, line_column_to_byte, line_start_byte_offsets, scan_gap};

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
                let trimmed_len = out.trim_end().len();
                out.truncate(trimmed_len);
                out.push_str(" // ");
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
///   `prev_end`/`next_start` must resolve within it (checked by `line_column_to_byte`'s own
///   `debug_assert!`).
///
/// - Complexity: O(n) in the gap's length.
fn gap_between(
    source: &str,
    prev_end: proc_macro2::Span,
    next_start: proc_macro2::Span,
    expected: &[&'static str],
) -> Vec<GapPiece> {
    if source.is_empty() {
        return scan_gap("", expected);
    }
    let line_starts = line_start_byte_offsets(source);
    let start = line_column_to_byte(source, &line_starts, prev_end.end());
    let end = line_column_to_byte(source, &line_starts, next_start.start());
    let gap = source.get(start..end).unwrap_or("");
    scan_gap(gap, expected)
}

/// Renders a comma-separated element list with per-gap comment recovery: the gap before the first
/// element (after `bounds.0`, the opening delimiter's position, e.g. a `(`), each inter-element
/// `,` gap, and the gap after the last element (before `bounds.1`, the position right past the
/// closing delimiter, e.g. a `)`). `delims` is `(open, close)`, the delimiter tokens. When
/// `trailing_comma` is set (a 1-tuple), a `,` is emitted after the sole element.
///
/// `bounds` holds raw [`proc_macro2::LineColumn`] positions (not `Span`s, unlike [`gap_between`]'s
/// `prev_end`/`next_start`) because `Expr::Apply`'s and `Expr::Tuple`'s own recorded delimiter
/// span covers the *whole* `(...)` group rather than a single delimiter character:
/// `AstContext::apply_op`'s `"()"` arm and `is_tuple_or_group` both capture `self.last_span` from
/// a `Token::CloseDelim`, and `lex_lexer`'s flattening gives every `OpenDelim`/`CloseDelim` the
/// *enclosing `Group`'s* span (see `LexLexer::next`), so a group span's `.start()` lands on `(`
/// and its `.end()` lands just past `)` regardless of which delimiter "produced" it. Resolving
/// which of `.start()`/`.end()` is the right boundary is the caller's job (it knows whether it's
/// holding a whole-group span or an ordinary leaf span, e.g. a callee's own end); this helper just
/// consumes the two positions it's given.
///
/// - Complexity: O(n) in the number of elements plus their gap lengths.
fn emit_list(
    source: &str,
    depth: usize,
    delims: (&'static str, &'static str),
    bounds: (proc_macro2::LineColumn, proc_macro2::LineColumn),
    elements: &[Expr],
    trailing_comma: bool,
) -> String {
    let (open, close) = delims;
    let (open_pos, close_pos) = bounds;
    // Scans the raw gap between two already-resolved positions, mirroring `gap_between` but
    // taking `LineColumn`s directly instead of deriving them from a fixed `prev_end.end()` /
    // `next_start.start()` pairing (which doesn't fit `open_pos`/`close_pos`; see above).
    let scan = |from: proc_macro2::LineColumn,
                to: proc_macro2::LineColumn,
                expected: &[&'static str]|
     -> Vec<GapPiece> {
        if source.is_empty() {
            return scan_gap("", expected);
        }
        let line_starts = line_start_byte_offsets(source);
        let start = line_column_to_byte(source, &line_starts, from);
        let end = line_column_to_byte(source, &line_starts, to);
        let gap = source.get(start..end).unwrap_or("");
        scan_gap(gap, expected)
    };

    if elements.is_empty() {
        // Whole gap between the delimiters: `open`, comments, `close`.
        let pieces = scan(open_pos, close_pos, &[open, close]);
        let whole = emit_gap(&pieces, Spacing::None, depth);
        return glue_before_close(&glue_after_open(&whole, open), close);
    }
    // Opening delimiter + any comment before the first element. `emit_gap`'s comment rendering
    // always pads a leading space before a comment that opens a gap, which is right when the gap
    // follows an already-rendered operand (`x /* c */ as i32`) but wrong right after a delimiter
    // -- `glue_after_open` corrects it back to `f(/* a */ 1i32)`.
    let pre = scan(open_pos, elements[0].span().start.start(), &[open]);
    let mut out = glue_after_open(&emit_gap(&pre, Spacing::None, depth), open);
    for (i, el) in elements.iter().enumerate() {
        out.push_str(&format_at(el, source, depth, Level::OR));
        if i + 1 < elements.len() {
            let pieces = gap_between(source, el.span().end, elements[i + 1].span().start, &[","]);
            out.push_str(&emit_gap(&pieces, Spacing::CommaAfter, depth));
        }
    }
    if trailing_comma {
        out.push(',');
    }
    // Any comment before the closing delimiter, then the delimiter; symmetric to the opening gap
    // above (`glue_before_close` drops the trailing space before `)` a comment would otherwise
    // leave: `f(1i32 /* end */)`, not `f(1i32 /* end */ )`).
    let post = scan(
        elements[elements.len() - 1].span().end.end(),
        close_pos,
        &[close],
    );
    out.push_str(&glue_before_close(
        &emit_gap(&post, Spacing::None, depth),
        close,
    ));
    out
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

/// Renders a closure parameter's unresolved type expression, e.g. `"i32"` or `"(i32, f64)"`.
///
/// - Complexity: O(n) in the number of (nested) tuple elements in the type expression.
fn render_closure_param_type(type_expr: &crate::ClosureParamTypeExpr) -> String {
    match type_expr {
        crate::ClosureParamTypeExpr::Named(name, _) => name.clone(),
        crate::ClosureParamTypeExpr::Tuple(elements, _) => {
            let inner = elements
                .iter()
                .map(render_closure_param_type)
                .collect::<Vec<_>>()
                .join(", ");
            format!("({inner})")
        }
    }
}

/// Returns the end position of a closure parameter's declared type expression — the boundary
/// right before the header's closing `|`.
fn closure_param_type_end(type_expr: &crate::ClosureParamTypeExpr) -> proc_macro2::Span {
    match type_expr {
        crate::ClosureParamTypeExpr::Named(_, span) => span.end,
        crate::ClosureParamTypeExpr::Tuple(_, span) => span.end,
    }
}

/// Renders `expr` on its own, returning its text alongside its binding-strength level, so the
/// caller ([`format_at`]) can decide whether the context it's being placed in requires parens.
///
/// `source` is the text `expr` was parsed from (or `""` for a hand-built `Expr`); `depth` is the
/// indent level for `//`-comment wrap continuations. Neither is used to change emission yet.
fn render(expr: &Expr, source: &str, depth: usize) -> (String, Level) {
    match expr {
        Expr::Literal { span, .. } => (render_literal(*span), Level::PRIMARY),
        // `AstContext::apply_op` records every arity-0 operator as a plain `Expr::Ident` (see
        // its own test `apply_op_with_arity_zero_records_an_ident_node`) — `"range_full"` (`..`)
        // is the only one this grammar has, so it's intercepted here rather than in the
        // `Expr::Op` arms below, which a real parse never produces it as.
        //
        // Known gap (tracked as issue #155): this is indistinguishable from a genuine bare
        // identifier reference literally named `range_full`, since `AstContext` deliberately
        // does no scope resolution during parsing (unlike the runtime path, where
        // `OpLookup::lookup` checks user scopes before builtins and so correctly lets a real
        // variable named `range_full` shadow this operator). Narrow in practice — the other
        // five range-family operators have no such collision, since a real function call is
        // never represented as `Expr::Op`.
        Expr::Ident { name, .. } if name == "range_full" => ("..".to_string(), Level::RANGE),
        Expr::Ident { name, .. } => (name.clone(), Level::PRIMARY),
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
            (
                format!(
                    "{lhs_s}{}{rhs_s}",
                    emit_gap(&pieces, Spacing::Around, depth)
                ),
                level,
            )
        }
        Expr::Op { operands, .. } if operands.is_empty() => {
            // Defensive only: a real parse never produces an arity-0 `Expr::Op` (see the
            // `Expr::Ident` arm above for how `"range_full"` actually arrives), but a hand-built
            // tree using this representation still round-trips correctly, matching this file's
            // existing convention of also handling non-parser-producible tree shapes (e.g.
            // `nested_comparison_needs_parens_on_both_sides`).
            ("..".to_string(), Level::RANGE)
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
                    format!("{operand_s}{}", emit_gap(&pieces, Spacing::None, depth))
                }
                "range_to" => {
                    // `span.start` is the `".."` token's own span; a `gap_between` call always
                    // scans strictly *after* its `prev_end` argument's end, so `".."` itself can
                    // never appear inside the scanned gap here (unlike the operand-then-operator
                    // arms below, where the operator genuinely sits inside the scanned range) —
                    // it's rendered directly instead, and only a comment is scanned for.
                    let pieces = gap_between(source, span.start, operands[0].span().start, &[]);
                    format!("..{}{operand_s}", emit_gap(&pieces, Spacing::None, depth))
                }
                "range_to_inclusive" => {
                    let pieces = gap_between(source, span.start, operands[0].span().start, &[]);
                    format!("..={}{operand_s}", emit_gap(&pieces, Spacing::None, depth))
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
            let default_sep = if operand_s.starts_with('-') || operand_s.starts_with('!') {
                " "
            } else {
                ""
            };
            let gap = if pieces.iter().any(|p| matches!(p, GapPiece::Comment(_))) {
                emit_gap(&pieces, Spacing::None, depth)
            } else {
                default_sep.to_string()
            };
            (format!("{op_static}{gap}{operand_s}"), Level::UNARY)
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
                format!("{lhs_s}{}{rhs_s}", emit_gap(&pieces, Spacing::None, depth)),
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
            (
                format!(
                    "{lhs_s}{}{rhs_s}",
                    emit_gap(&pieces, Spacing::Around, depth)
                ),
                level,
            )
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
            let gap = emit_gap(&pieces, Spacing::Around, depth);
            // A `//` comment in this gap made `emit_gap` end it with a line wrap (`\n` + the
            // continuation indent) rather than the usual trailing space; `type_name` must continue
            // right there; trimming it (as the comment-free/block-comment path below does) would
            // glue `type_name` onto the comment's own line and comment it out.
            let has_line_comment = pieces
                .iter()
                .any(|p| matches!(p, GapPiece::Comment(Comment::Line(_))));
            let text = if has_line_comment {
                format!("{expr_s}{gap}{type_name}")
            } else {
                format!("{expr_s}{} {type_name}", gap.trim_end())
            };
            (text, Level::CAST)
        }
        Expr::Apply { callee, args, span } => {
            let callee_s = format_at(callee, source, depth, Level::POSTFIX);
            // `callee.span().end` is a genuine leaf position (right after the callee, i.e. right
            // before `(`); `span.end` is the call's own recorded end, which -- per `emit_list`'s
            // doc comment -- is the whole `(...)` group's span, so its `.end()` (not `.start()`)
            // is the position right after `)`.
            let list = emit_list(
                source,
                depth,
                ("(", ")"),
                (callee.span().end.end(), span.end.end()),
                args,
                false,
            );
            (format!("{callee_s}{list}"), Level::POSTFIX)
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
        // Placeholder arm keeping `render` exhaustive: array literals are emitted as a plain
        // comma-separated list, without the comment/trivia handling the other list-shaped forms
        // get from `emit_list`. Replaced when array formatting proper is implemented.
        Expr::Array { elements, .. } => {
            let inner = elements
                .iter()
                .map(|element| format_at(element, source, depth, Level::RANGE))
                .collect::<Vec<_>>()
                .join(", ");
            (format!("[{inner}]"), Level::PRIMARY)
        }
        Expr::TupleIndex { base, index, .. } => (
            format!(
                "{}.{}",
                format_at(base, source, depth, Level::POSTFIX),
                index
            ),
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
            let cond_s = format_at(cond, source, depth, Level::OR);
            let then_s = format_at(then_branch, source, depth, Level::OR);
            let if_gap = gap_between(source, span.start, cond.span().start, &["if"]);
            let open_gap = gap_between(source, cond.span().end, then_branch.span().start, &["{"]);
            let mut text = String::new();
            // Only the very first gap is `trim_start`'d: it opens the whole node's text, so any
            // leading space `Spacing::Around` would otherwise add before `"if"` must be dropped.
            text.push_str(emit_gap(&if_gap, Spacing::Around, depth).trim_start());
            text.push_str(&cond_s);
            text.push_str(&emit_gap(&open_gap, Spacing::Around, depth));
            text.push_str(&then_s);
            match else_branch {
                None => {
                    // No `else`: the gap between the then-branch and the node's own end holds
                    // just the closing `}` (plus any comment before it).
                    let close_gap = gap_between(source, then_branch.span().end, span.end, &["}"]);
                    // `trim_end`'d because this is the node's last piece of text: a trailing
                    // space here would otherwise survive into the caller's output (or, wrapped in
                    // parens by `format_at`, land right before the close-paren).
                    text.push_str(emit_gap(&close_gap, Spacing::Around, depth).trim_end());
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
                    text.push_str(&emit_gap(&else_gap, Spacing::Around, depth));
                    if is_chain {
                        // The nested `if` recurses through `render`, which lays out its own
                        // `if`/`{`/`}`/`else` gaps (including its own trailing `trim_end`), so
                        // nothing further is appended here.
                        let (else_s, _) = render(else_branch, source, depth);
                        text.push_str(&else_s);
                    } else {
                        let else_s = format_at(else_branch, source, depth, Level::OR);
                        text.push_str(&else_s);
                        let final_close_gap =
                            gap_between(source, else_branch.span().end, span.end, &["}"]);
                        text.push_str(
                            emit_gap(&final_close_gap, Spacing::Around, depth).trim_end(),
                        );
                    }
                }
            }
            (text, Level::PRIMARY)
        }
        Expr::Closure { params, body, span } => {
            // `ClosureParam`s carry `name`/`name_span`/`type_expr`, not `Expr` nodes, and the
            // header's `|`/`|` delimiters have no span of their own, so the header text is
            // synthesized from `params` rather than re-scanned token-by-token. Only the gap
            // between the header's closing `|` and the body is recovered precisely (the
            // #201-style case, and the most common one): for a non-empty parameter list it's
            // scanned from the last parameter's declared type's end (expecting the closing `|`);
            // for an empty list, `span.start` already covers the merged `||` token, so the scan
            // starts right after it with no further token expected. A comment written *inside*
            // the parameter list, e.g. `|x: i32 /* c */, y: i32| body`, is a documented,
            // lossless-but-not-position-perfect limitation: it isn't scanned for at all here (only
            // the last parameter's post-type gap is), so it would need its own per-parameter gap
            // recovery to attach at its exact source spot.
            let body_s = format_at(body, source, depth, Level::OR);
            let header = if params.is_empty() {
                "||".to_string()
            } else {
                let params_s = params
                    .iter()
                    .map(|p| format!("{}: {}", p.name, render_closure_param_type(&p.type_expr)))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("|{params_s}|")
            };
            let (tail_start, expected): (proc_macro2::Span, &[&'static str]) = match params.last() {
                Some(last) => (closure_param_type_end(&last.type_expr), &["|"]),
                None => (span.start, &[]),
            };
            let pieces = gap_between(source, tail_start, body.span().start, expected);
            let comments: Vec<GapPiece> = pieces
                .into_iter()
                .filter(|p| matches!(p, GapPiece::Comment(_)))
                .collect();
            let gap = emit_gap(&comments, Spacing::None, depth);
            // A `//` comment among `comments` made `emit_gap` end `gap` with a line wrap (`\n` +
            // continuation indent); the body must continue right there, so `gap` is used verbatim
            // (already correctly spaced/wrapped) rather than trimmed — trimming would collapse the
            // wrap and glue the body onto the comment's own line, commenting it out.
            let has_line_comment = comments
                .iter()
                .any(|p| matches!(p, GapPiece::Comment(Comment::Line(_))));
            let sep = if has_line_comment {
                gap
            } else if gap.trim().is_empty() {
                " ".to_string()
            } else {
                format!(" {} ", gap.trim())
            };
            (format!("{header}{sep}{body_s}"), Level::PRIMARY)
        }
    }
}

/// Renders `expr`, wrapping it in parens if its own level is looser than `min_level` requires.
///
/// `source` and `depth` are forwarded to [`render`] unchanged; see its doc comment.
fn format_at(expr: &Expr, source: &str, depth: usize, min_level: Level) -> String {
    let (text, level) = render(expr, source, depth);
    if level < min_level {
        format!("({text})")
    } else {
        text
    }
}

/// Pretty-prints `expr` back to CEL source text — see the module doc for the printing rules.
///
/// `source` is the text `expr` was parsed from (or `""` for a hand-built `Expr`); `depth` is the
/// indent level for `//`-comment wrap continuations.
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
    format_at(expr, source, depth, Level::RANGE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AstContext, OpLookup, Parser};

    fn parse(source: &str) -> Expr {
        Parser::<AstContext>::new(OpLookup::new())
            .parse_str_ast(source)
            .unwrap()
    }

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
    fn comment_free_if_else_is_unchanged() {
        assert_eq!(
            fmt("if true { 1i32 } else { 2i32 }"),
            "if true { 1i32 } else { 2i32 }"
        );
        assert_eq!(fmt("if true { 1i32 }"), "if true { 1i32 }");
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
    fn comment_free_logical_is_unchanged() {
        assert_eq!(fmt("a || b && c"), "a || b && c");
    }

    #[test]
    fn format_is_idempotent_through_a_reparse() {
        let source = "(1i32 + 2i32) * 3i32 - -4i32";
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
    fn comment_free_closures_are_unchanged() {
        assert_eq!(fmt("|x: i32| x + 1i32"), "|x: i32| x + 1i32");
        assert_eq!(fmt("|| 1i32"), "|| 1i32");
        assert_eq!(fmt("|x: i32, y: i32| x + y"), "|x: i32, y: i32| x + y");
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
    fn a_line_comment_after_a_binary_operator_wraps_the_expression() {
        let source = "1i32 +\n    // why 2\n    2i32";
        assert_eq!(fmt(source), "1i32 + // why 2\n    2i32");
    }

    #[test]
    fn a_comment_free_binary_op_still_prints_on_one_line() {
        assert_eq!(fmt("1i32 + 2i32 * 3i32"), "1i32 + 2i32 * 3i32");
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
