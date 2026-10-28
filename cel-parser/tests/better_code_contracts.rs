//! Cross-surface contracts for malformed names, deferred bounds, and source preservation.

use cel_parser::{
    AstContext, CELParser, Expr, FormatRustcStyle, OpLookup, Parser, SourceSpan, SpanContext,
};

/// Heterogeneous shifts accept the last valid bit and reject a count equal to the width.
#[test]
fn shifts_report_width_boundaries_without_wrapping() {
    for (source, expected) in [("1u8 << 7u128", 128u8), ("128u8 >> 7i128", 1u8)] {
        assert_eq!(
            CELParser::new(OpLookup::new())
                .parse_str(source)
                .unwrap()
                .call0::<u8>()
                .unwrap(),
            expected
        );
    }
    for source in ["1u8 << 8u128", "128u8 >> 8i128"] {
        assert!(
            CELParser::new(OpLookup::new())
                .parse_str(source)
                .unwrap()
                .call0::<u8>()
                .is_err(),
            "{source}"
        );
    }
}

/// Signed literal tokens must fit before unary negation; checked arithmetic can express minima.
#[test]
fn signed_literal_boundaries_agree_across_parser_surfaces() {
    macro_rules! check {
        ($($ty:ty),* $(,)?) => {$(
            let source = format!("-{}{} - 1{}", <$ty>::MAX, stringify!($ty), stringify!($ty));
            let expr = Parser::<AstContext>::new(OpLookup::new()).parse_str_ast(&source).unwrap();
            let mut retained = cel_parser::compile_expr(&expr, &OpLookup::new(), cel_parser::builtin_type_resolver().as_ref(), &mut |_, _, _| Ok(false)).unwrap();
            assert_eq!(retained.call0::<$ty>().unwrap(), <$ty>::MIN);
            assert_eq!(CELParser::new(OpLookup::new()).parse_str(&source).unwrap().call0::<$ty>().unwrap(), <$ty>::MIN);
            assert!(cel_parser::ty::check_expr(&expr, &|_| cel_parser::Ty::Any).1.is_empty());
            for source in [
                format!("{}{}", <$ty>::MIN.unsigned_abs(), stringify!($ty)),
                format!("-{}{}", <$ty>::MIN.unsigned_abs(), stringify!($ty)),
            ] {
                assert!(CELParser::new(OpLookup::new()).parse_str(&source).is_err());
                assert!(Parser::<AstContext>::new(OpLookup::new()).parse_str_ast(&source).is_err());
                let tokens: proc_macro2::TokenStream = source.parse().unwrap();
                let mut parser = CELParser::new(OpLookup::new());
                parser.set_lex_tokens(cel_parser::lex_lexer::LexLexer::new(tokens.into_iter()).peekable());
                assert!(parser.parse_literal_pattern().is_err());
            }
        )*};
    }
    check!(i8, i16, i32, i64, i128, isize);
}

/// Duplicate parameters are malformed syntax, independently of parameter types or use.
#[test]
fn duplicate_parameters_are_rejected_by_both_parsers() {
    for source in ["|x: i32, x: bool| x", "|x: i32, x: i32| 1"] {
        assert!(
            CELParser::new(OpLookup::new()).parse_str(source).is_err(),
            "{source}"
        );
        assert!(
            Parser::<AstContext>::new(OpLookup::new())
                .parse_str_ast(source)
                .is_err(),
            "{source}"
        );
    }
}

/// Reserved value names cannot become parameters or ordinary references.
#[test]
fn reserved_value_names_are_rejected() {
    for source in [
        "|else: i32| else",
        "|as: i32| as",
        "|if: i32| 1",
        "else",
        "as",
    ] {
        assert!(
            Parser::<AstContext>::new(OpLookup::new())
                .parse_str_ast(source)
                .is_err(),
            "{source}"
        );
    }
}

/// Syntax retains every representable tuple index while execution validates bounds.
#[test]
fn maximum_tuple_index_is_deferred_without_a_sentinel_collision() {
    let source = format!("value.{}", usize::MAX);
    let expr = Parser::<AstContext>::new(OpLookup::new())
        .parse_str_ast(&source)
        .unwrap();
    assert!(matches!(
        expr,
        Expr::TupleIndex {
            index: usize::MAX,
            ..
        }
    ));
    let (_, diagnostics) =
        cel_parser::ty::check_expr(&expr, &|_| cel_parser::Ty::Tuple(vec![cel_parser::Ty::I32]));
    assert!(!diagnostics.is_empty());
    assert!(
        CELParser::new(OpLookup::new())
            .parse_str(&format!("(1,).{}", usize::MAX))
            .is_err()
    );
}

/// Formatting preserves annotation and tuple-suffix comments exactly once and stays stable.
#[test]
fn comments_in_types_and_tuple_suffixes_survive_formatting() {
    for source in [
        "[1]: /* keep */ [i32]",
        "[1]: [/* keep */ i32]",
        "|x: [/* keep */ i32]| x",
        "(1,). /* keep */ 0",
        "|x: [// keep\n i32]| x",
        "[1]: [/* 外 /* keep */ */ i32]",
        "[1]: (/* keep */ [i32])",
        "[1]: ([i32] /* keep */)",
        "|x: (/* keep */ i32)| x",
        "|x: (i32 /* keep */), y: i32| x",
    ] {
        let parse = |text: &str| {
            Parser::<AstContext>::new(OpLookup::new())
                .parse_str_ast(text)
                .unwrap()
        };
        let formatted = cel_parser::format_expr(&parse(source), source, 0);
        assert_eq!(
            formatted.matches("keep").count(),
            1,
            "{source} -> {formatted}"
        );
        assert_eq!(
            cel_parser::format_expr(&parse(&formatted), &formatted, 0),
            formatted
        );
    }
}

/// Formatting preserves multiple type and suffix comments in source order without changing values.
#[test]
fn comment_order_and_execution_survive_formatting() {
    let source = "(|x: (/* first */ i32 /* second */)| (x,). /* third */ 0)(11)";
    let parse = |text: &str| {
        Parser::<AstContext>::new(OpLookup::new())
            .parse_str_ast(text)
            .unwrap()
    };
    let formatted = cel_parser::format_expr(&parse(source), source, 0);
    let mut previous = 0;
    for marker in ["first", "second", "third"] {
        assert_eq!(formatted.matches(marker).count(), 1);
        let next = formatted.find(marker).unwrap();
        assert!(next > previous);
        previous = next;
    }
    assert_eq!(
        CELParser::new(OpLookup::new())
            .parse_str(&formatted)
            .unwrap()
            .call0::<i32>()
            .unwrap(),
        11
    );
    assert_eq!(
        cel_parser::format_expr(&parse(&formatted), &formatted, 0),
        formatted
    );
}

/// Textual context outside location wrappers cannot hide the operation failure.
#[test]
fn nested_textual_error_context_preserves_the_failure() {
    let error = anyhow::anyhow!("arithmetic overflow")
        .context(SpanContext::new(SourceSpan::new(1, 0, 1, 1)))
        .context("inner operation")
        .context(SpanContext::new(SourceSpan::new(1, 0, 1, 1)))
        .context("outer operation");
    let rendered =
        error.format_rustc_style("1", "test.cel", 1, &annotate_snippets::Renderer::plain());
    assert!(
        rendered.contains("error: arithmetic overflow"),
        "{rendered}"
    );
    assert!(rendered.contains("test.cel"), "{rendered}");
}

/// Hand-built closures receive the same duplicate-name diagnostic as parsed closures.
#[test]
fn hand_built_duplicate_parameters_are_rejected() {
    let mut expr = Parser::<AstContext>::new(OpLookup::new())
        .parse_str_ast("|x: i32, y: bool| x")
        .unwrap();
    let Expr::Closure { params, .. } = &mut expr else {
        unreachable!()
    };
    params[1].name = "x".to_owned();
    let (_, diagnostics) = cel_parser::ty::check_expr(&expr, &|_| cel_parser::Ty::Any);
    assert!(
        diagnostics
            .iter()
            .any(|d| d.to_string().contains("duplicate"))
    );
    assert!(
        cel_parser::compile_expr(
            &expr,
            &OpLookup::new(),
            cel_parser::builtin_type_resolver().as_ref(),
            &mut |_, _, _| Ok(false)
        )
        .is_err()
    );
}

/// A real identifier named `range_full` does not become range syntax.
#[test]
fn range_full_identifier_and_operator_keep_distinct_spelling() {
    for source in ["range_full", ".."] {
        let expr = Parser::<AstContext>::new(OpLookup::new())
            .parse_str_ast(source)
            .unwrap();
        assert_eq!(cel_parser::format_expr(&expr, source, 0), source);
    }
}

/// Nested noncapturing closures cannot infer captured formals as their own result.
#[test]
fn nested_closure_checking_does_not_capture_outer_formals() {
    let source = "|x: i32| || x";
    let expr = Parser::<AstContext>::new(OpLookup::new())
        .parse_str_ast(source)
        .unwrap();
    let (ty, _) = cel_parser::ty::check_expr(&expr, &|_| cel_parser::Ty::F64);
    assert_eq!(
        ty,
        cel_parser::Ty::Callable(
            vec![cel_parser::Ty::I32],
            Box::new(cel_parser::Ty::Callable(
                vec![],
                Box::new(cel_parser::Ty::Any)
            ))
        )
    );
    assert!(CELParser::new(OpLookup::new()).parse_str(source).is_err());
    assert!(
        cel_parser::compile_expr(
            &expr,
            &OpLookup::new(),
            cel_parser::builtin_type_resolver().as_ref(),
            &mut |_, _, _| Ok(false)
        )
        .is_err()
    );
}
