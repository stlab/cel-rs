//! Exercises recovery, registration, checking, and formatting at their public boundaries.

use adam_lang::{AdamAstParser, TypeRegistry};
use std::any::TypeId;

/// Preserves nested block payloads and valid fixed-point output at leading, inline, and closing gaps.
#[test]
fn nested_block_comments_survive_formatting_and_reparsing() {
    for source in [
        "sheet s {\n/* outer\n/* inner */\ntail */\ncell x: i32;\n}",
        "sheet s {\ncell a: i32;\n/* outer\n/* inner */\ntail */\ncell x: i32;\n}",
        "sheet s {\ncell a: i32; /* outer\n/* inner */\ntail */\ncell x: i32;\n}",
        "/* outer\n/* inner */\ntail */\nsheet s { cell x: i32; }",
        "sheet s {\ncell x: i32;\n/* outer\n/* inner */\ntail */\n}",
    ] {
        let input = AdamAstParser::new().parse_module_str(source).unwrap();
        assert!(input.errors.is_empty(), "{source}");
        let formatted = adam_lang::format_source(source).unwrap();
        for marker in ["outer", "inner", "tail", "/* inner */"] {
            assert_eq!(formatted.matches(marker).count(), 1, "{formatted}");
        }
        assert!(formatted.find("outer").unwrap() < formatted.find("inner").unwrap());
        assert!(formatted.find("inner").unwrap() < formatted.find("tail").unwrap());
        let reparsed = AdamAstParser::new().parse_module_str(&formatted).unwrap();
        assert!(reparsed.errors.is_empty(), "{formatted}");
        for item in &reparsed.items {
            if let adam_lang::ast::ModuleItem::Sheet(sheet) = item {
                assert!(sheet.errors.is_empty(), "{formatted}");
            }
        }
        assert_eq!(adam_lang::format_source(&formatted).unwrap(), formatted);
    }
}

/// Distinguishes blank lines outside a single block comment from blank payload lines.
#[test]
fn single_block_comment_spacing_ignores_blank_payload_lines() {
    for (gap, expected_blank) in [
        ("\n/* outer\n\n/* inner */\ntail */\n", false),
        ("\n\n/* outer\n/* inner */\ntail */\n", true),
        ("\n/* outer\n/* inner */\ntail */\n \t\n", true),
    ] {
        let source = format!("sheet s {{ cell a: i32;{gap}    cell x: i32; }}");
        let mut sheet = AdamAstParser::new().parse_str(&source).unwrap();
        assert!(sheet.errors.is_empty());
        adam_lang::attach_trivia(&source, &mut sheet);
        let adam_lang::ast::SheetItem::Cell(cell) = &sheet.items[1] else {
            panic!("expected the following cell");
        };
        assert_eq!(cell.blank_line_before, expected_blank, "{gap}");
    }
}

/// Never equates unrelated erased values, including two mismatches.
#[test]
fn erased_equality_rejects_mismatched_operands() {
    let registry = TypeRegistry::new();
    let eq = registry.get("i32").unwrap().eq_dyn_fn;
    assert!(!eq(&"wrong", &"wrong"));
    assert!(!eq(&1i32, &"wrong"));
    assert!(!eq(&"wrong", &1i32));
    assert!(eq(&1i32, &1i32));
    assert!(!eq(&1i32, &2i32));
}

/// Preserves each following declaration when an unterminated item fails.
#[test]
fn recovery_preserves_every_sheet_item_starter() {
    for following in [
        "cell kept: i32;",
        "source kept: i32;",
        "out kept := 1;",
        "relationship { kept := 1; }",
        "conditional true { true => { relationship { kept := 1; } } }",
    ] {
        let source = format!("sheet s {{ cell bad unknown {following} }}");
        let sheet = AdamAstParser::new().parse_str(&source).unwrap();
        assert_eq!(sheet.errors.len(), 1, "{following}");
        assert_eq!(sheet.items.len(), 2, "{following}");
        assert!(!matches!(
            sheet.items[1],
            adam_lang::ast::SheetItem::Error { .. }
        ));
        assert!(sheet.items[0].span().end.end() <= sheet.items[1].span().start.start());
    }
}

/// Keeps reverse lookup and sheet construction available through surviving aliases.
#[test]
fn replacing_an_alias_preserves_reverse_reachability() {
    for (with_default, replaced, surviving) in [
        (false, "Integer", "i32"),
        (true, "Integer", "i32"),
        (false, "i32", "Integer"),
        (true, "i32", "Integer"),
    ] {
        let mut registry = TypeRegistry::new();
        registry.register::<i32>("Integer");
        let earlier_snapshot = registry.cel_type_resolver();
        if with_default {
            registry.register::<u32>(replaced);
        } else {
            registry.register_no_default::<u32>(replaced);
        }
        assert_eq!(
            registry.get(surviving).unwrap().type_id,
            TypeId::of::<i32>()
        );
        assert_eq!(
            registry
                .entry_by_type_id(TypeId::of::<i32>())
                .unwrap()
                .type_id,
            TypeId::of::<i32>()
        );
        assert!(registry.registered_name(TypeId::of::<i32>()).is_some());
        let snapshot = registry.cel_type_resolver();
        assert!(cel_parser::TypeResolver::resolve_named_type(&registry, surviving, &[]).is_some());
        assert!(cel_parser::TypeResolver::resolve_named_type(&snapshot, surviving, &[]).is_some());
        assert_eq!(
            cel_parser::TypeResolver::resolve_named_type(&earlier_snapshot, replaced, &[])
                .unwrap()
                .type_id(),
            TypeId::of::<i32>()
        );
        assert_eq!(
            cel_parser::TypeResolver::resolve_named_type(&snapshot, replaced, &[])
                .unwrap()
                .type_id(),
            TypeId::of::<u32>()
        );
        let mut parser = adam_lang::AdamParser::new(registry, cel_parser::OpLookup::new());
        let mut sheet = parser
            .parse_str(&format!(
                "sheet s {{ cell n: {surviving} = 3; out doubled := n + n; }}"
            ))
            .unwrap();
        sheet.propagate().unwrap();
        assert_eq!(
            *sheet.read::<i32>(sheet.output_names["doubled"]).unwrap(),
            6
        );
    }
}

/// Rejects non-Boolean CEL conditions at every tuple-checking entry point.
#[test]
fn tuple_conditions_require_bool() {
    for declaration in [
        "cell pair: (i32, i32) = if 1 { (1, 2) } else { (3, 4) };",
        "source pair: (i32, i32) = if 1 { (1, 2) } else { (3, 4) };",
        "out pair: (i32, i32) := if 1 { (1, 2) } else { (3, 4) };",
        "cell pair: (i32, i32); relationship { pair := if 1 { (1, 2) } else { (3, 4) }; }",
        "cell a: i32; cell b: i32; relationship { (a, b) := if 1 { (1, 2) } else { (3, 4) }; }",
        "cell pair: (i32, i32) = if true { (1, 2) } else if 1 { (3, 4) } else { (5, 6) };",
        "cell a: i32; cell b: i32; relationship { (a, b) := if true { (1, 2) } else if 1 { (3, 4) } else { (5, 6) }; }",
    ] {
        let source = format!("sheet s {{ {declaration} }}");
        let sheet = AdamAstParser::new().parse_str(&source).unwrap();
        assert!(sheet.errors.is_empty(), "{:?}", sheet.errors);
        let diagnostics =
            adam_lang::check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert_eq!(diagnostics.len(), 1, "{declaration}: {diagnostics:?}");
        assert_eq!(
            diagnostics[0].span().start().column,
            source.find("if 1").unwrap() + 3
        );
    }
}

/// Reports an invalid condition child once, without a duplicate condition-type diagnostic.
#[test]
fn tuple_conditions_retain_child_diagnostics_without_duplicates() {
    for declaration in [
        "cell pair: (i32, i32) = if 1 + true { (1, 2) } else { (3, 4) };",
        "cell a: i32; cell b: i32; relationship { (a, b) := if 1 + true { (1, 2) } else { (3, 4) }; }",
    ] {
        let source = format!("sheet s {{ {declaration} }}");
        let sheet = AdamAstParser::new().parse_str(&source).unwrap();
        assert!(sheet.errors.is_empty(), "{:?}", sheet.errors);
        let diagnostics =
            adam_lang::check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert_eq!(diagnostics.len(), 1, "{declaration}: {diagnostics:?}");
        let condition_start = source.find("1 + true").unwrap();
        assert!(
            (condition_start..condition_start + "1 + true".len())
                .contains(&diagnostics[0].span().start().column)
        );
    }
}

/// Accepts an inferred tuple destination without weakening annotated scalar checks.
#[test]
fn inferred_tuple_binding_agrees_with_runtime() {
    let source =
        "sheet s { cell pair = (1, 2); cell n: i32 = 3; relationship { pair := (n, n); } }";
    let sheet = AdamAstParser::new().parse_str(source).unwrap();
    assert!(sheet.errors.is_empty());
    assert!(
        adam_lang::check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new(),)
            .is_empty()
    );
    assert!(
        adam_lang::AdamParser::new(TypeRegistry::new(), cel_parser::OpLookup::new())
            .parse_str(source)
            .is_ok()
    );
    for (annotation, errors) in [("(i32, i32)", 0), ("(i32, bool)", 1), ("i32", 1)] {
        let source = format!(
            "sheet s {{ cell pair: {annotation}; cell n: i32 = 3; relationship {{ pair := (n, n); }} }}"
        );
        let sheet = AdamAstParser::new().parse_str(&source).unwrap();
        assert!(sheet.errors.is_empty());
        assert_eq!(
            adam_lang::check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new())
                .len(),
            errors
        );
    }
}

/// Preserves ordered detached and mixed comment runs through formatting and reparsing.
#[test]
fn formatting_preserves_complete_comment_gaps() {
    for item in ["cell x: i32;", "source x: i32;", "out x := 1;"] {
        let source = format!("sheet s {{\n// first\n\n/* second */\n{item}\n}}");
        let formatted = adam_lang::format_source(&source).unwrap();
        assert_eq!(formatted.matches("first").count(), 1, "{formatted}");
        assert_eq!(formatted.matches("second").count(), 1, "{formatted}");
        assert!(formatted.find("first") < formatted.find("second"));
        assert_eq!(adam_lang::format_source(&formatted).unwrap(), formatted);
    }

    for body in [
        "relationship {\n// first\n\n/* second */\nx := 1;\n}",
        "cell x: i32 require {\n// first\n\n/* second */\ntrue;\n};",
        "conditional true {\n// first\n\n/* second */\ntrue => { relationship {} }\n}",
        "relationship { x := 1;\n// first\n\n/* second */\n}",
        "conditional true { true => {\n// first\n\n/* second */\nrelationship {}\n} }",
        "conditional true {\n// first\n\n/* second */\n_ => { relationship {} }\n}",
        "conditional true { true => { relationship {} }\n// first\n\n/* second */\n_ => { relationship {} }\n}",
    ] {
        let source = format!("sheet s {{ {body} }}");
        let formatted = adam_lang::format_source(&source).unwrap();
        assert_eq!(formatted.matches("first").count(), 1, "{formatted}");
        assert_eq!(formatted.matches("second").count(), 1, "{formatted}");
        assert!(formatted.find("first") < formatted.find("second"));
        assert_eq!(adam_lang::format_source(&formatted).unwrap(), formatted);
    }
}

/// Preserves Unicode source regions in later sheets while reusing one module source index.
#[test]
fn formatting_shared_source_index_preserves_unicode_regions() {
    let source = r#"sheet first { cell greeting: String = "héllo"; }
// 中文 header
sheet second {
    cell label: String /* 型 */ = /* значение */ "世界";
    cell next: i32 = 1 /* końcowy */;
}"#;
    let formatted = adam_lang::format_source(source).unwrap();
    for marker in ["héllo", "中文 header", "型", "значение", "世界", "końcowy"] {
        assert_eq!(formatted.matches(marker).count(), 1, "{formatted}");
    }
    let parsed = AdamAstParser::new().parse_module_str(&formatted).unwrap();
    assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
    assert_eq!(adam_lang::format_source(&formatted).unwrap(), formatted);
}
