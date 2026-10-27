//! Structural module grammar, recovery, and legacy-boundary contracts.

use adam_lang::{AdamAstParser, ast};

/// Rejects reserved declaration, alias and reference names while recovering later sheets.
#[test]
fn reserved_module_identifiers_reject_consistently_and_recover() {
    for name in ["sheet", "use", "as"] {
        for broken in [
            format!("sheet {name}() {{}}"),
            format!("sheet {name} {{}}"),
            format!("use lib::helper as {name};"),
            format!("use {name}::helper;"),
            format!("use lib::{name};"),
            format!("sheet root {{ relationship {name}(); }}"),
            format!("sheet root {{ relationship lib::{name}(); }}"),
        ] {
            let source = format!("{broken}\nsheet next {{}}");
            let module = AdamAstParser::new().parse_module_str(&source).unwrap();
            assert!(!module.errors.is_empty(), "{source}");
            assert!(
                module.items.iter().any(|item| matches!(item,
                ast::ModuleItem::Sheet(sheet) if sheet.name == "next")),
                "{source}"
            );
            assert!(adam_lang::format_source(&source).is_err(), "{source}");
        }
    }
    let source = "sheet 尺寸() {} sheet root { relationship 尺寸(); }";
    assert!(
        AdamAstParser::new()
            .parse_module_str(source)
            .unwrap()
            .errors
            .is_empty()
    );
}

/// Retains ordered imports, forward references, typed signatures, and distinct roots.
#[test]
fn module_structural_shapes() {
    let source = "use constraints::ordered as ord;
        sheet first { relationship helper(left: a, right: b,); }
        sheet helper(left: i32, right: (i32, bool),) {}
        sheet empty() {}
        sheet second {}";
    let module = AdamAstParser::new().parse_module_str(source).unwrap();
    assert!(module.errors.is_empty(), "{:?}", module.errors);
    assert_eq!(module.items.len(), 5);
    let ast::ModuleItem::Use(import) = &module.items[0] else {
        panic!()
    };
    assert_eq!(import.path[0].0, "constraints");
    assert_eq!(import.path[1].0, "ordered");
    assert_eq!(import.alias.as_ref().unwrap().0, "ord");
    let ast::ModuleItem::Sheet(first) = &module.items[1] else {
        panic!()
    };
    assert!(first.parameters.is_none());
    let ast::SheetItem::RelationshipUse(call) = &first.items[0] else {
        panic!()
    };
    assert_eq!(call.path[0].0, "helper");
    assert_eq!(call.arguments[0].name, "left");
    assert_eq!(call.arguments[0].cell, "a");
    let ast::ModuleItem::Sheet(helper) = &module.items[2] else {
        panic!()
    };
    assert_eq!(helper.parameters.as_ref().unwrap().len(), 2);
    assert_eq!(helper.parameters.as_ref().unwrap()[0].name, "left");
    let ast::ModuleItem::Sheet(empty) = &module.items[3] else {
        panic!()
    };
    assert!(empty.parameters.as_ref().unwrap().is_empty());
}

/// Recovers malformed imports, signatures, and calls without dropping later definitions.
#[test]
fn module_recovery_preserves_following_definition() {
    for broken in [
        "use constraints:: as ord;",
        "sheet broken(a i32) {}",
        "sheet broken(a) {}",
        "sheet broken(source a: i32) {}",
        "sheet broken(out a: i32) {}",
        "sheet broken { relationship foo(a x); }",
        "sheet broken { relationship foo(x); }",
        "use constraints::*;",
        "sheet broken(a: i32 = 1) {}",
        "use broken sheet next_too {}",
        "sheet broken { relationship foo(a: x) cell y: i32; }",
        "sheet broken<T>() {}",
        "sheet broken { relationship foo(a: x + 1); }",
        "sheet broken { relationship foo(a: x.field); }",
        "sheet broken { relationship foo(a: (x, y)); }",
    ] {
        let source = format!("{broken}\nsheet next {{}}");
        let module = AdamAstParser::new().parse_module_str(&source).unwrap();
        assert!(!module.errors.is_empty(), "{source}");
        assert!(
            matches!(module.items.last(), Some(ast::ModuleItem::Sheet(s)) if s.name == "next"),
            "{source}"
        );
        assert!(adam_lang::format_source(&source).is_err());
    }
}

/// Retains the exact names and spans without resolving unknown types or executing initializers.
#[test]
fn module_names_types_and_spans_are_structural() {
    let source = "use lib::helper as local;\nsheet helper(a: [Unknown], b: (i32, bool),) { cell private = not_registered(); }\nsheet root { relationship local(a: x, b: y); }";
    let module = AdamAstParser::new().parse_module_str(source).unwrap();
    assert!(module.errors.is_empty(), "{:?}", module.errors);
    let ast::ModuleItem::Sheet(helper) = &module.items[1] else {
        panic!()
    };
    let parameters = helper.parameters.as_ref().unwrap();
    assert_eq!(parameters[0].name_span.start.start().line, 2);
    assert_eq!(
        parameters[0].name_span.start.source_text().as_deref(),
        Some("a")
    );
    assert_eq!(parameters[0].type_name.span().start.start().line, 2);
    assert!(matches!(
        parameters[0].type_name,
        ast::TypeExpr::Array { .. }
    ));
    assert!(matches!(
        parameters[1].type_name,
        ast::TypeExpr::Tuple { .. }
    ));
    let ast::ModuleItem::Sheet(root) = &module.items[2] else {
        panic!()
    };
    let ast::SheetItem::RelationshipUse(call) = &root.items[0] else {
        panic!()
    };
    assert_eq!(
        call.arguments[0].name_span.start.source_text().as_deref(),
        Some("a")
    );
    assert_eq!(
        call.arguments[0].cell_span.start.source_text().as_deref(),
        Some("x")
    );
}

/// Parses component uses alongside ordinary relationships inside conditional branches.
#[test]
fn conditional_component_use() {
    let module = AdamAstParser::new().parse_module_str(
        "sheet root { conditional mode { true => { relationship lib::helper(a: x); relationship { x := 1; } } _ => { relationship helper(); } } }",
    ).unwrap();
    assert!(module.errors.is_empty(), "{:?}", module.errors);
    let ast::ModuleItem::Sheet(sheet) = &module.items[0] else {
        panic!()
    };
    let ast::SheetItem::Conditional(cond) = &sheet.items[0] else {
        panic!()
    };
    assert!(matches!(
        cond.branches[0].relationships[0],
        ast::BranchItem::RelationshipUse(_)
    ));
    assert!(matches!(
        cond.branches[0].relationships[1],
        ast::BranchItem::Relationship(_)
    ));
}

/// Keeps a following definition when an import ends at a missing path segment or alias.
#[test]
fn import_recovery_does_not_consume_definition_keywords() {
    for broken in ["use", "use lib::", "use lib as"] {
        let source = format!("{broken}\nsheet following {{}}");
        let module = AdamAstParser::new().parse_module_str(&source).unwrap();
        assert!(!module.errors.is_empty(), "{source}");
        assert!(
            matches!(module.items.last(), Some(ast::ModuleItem::Sheet(sheet)) if sheet.name == "following"),
            "{source}: {:?}",
            module.items
        );
    }
}

/// Rejects ambiguous or component-only input through the single-root convenience.
#[test]
fn single_root_convenience_rejects_modules() {
    for source in [
        "sheet a {} sheet b {}",
        "sheet helper() {}",
        "use lib::helper; sheet root {}",
    ] {
        assert!(AdamAstParser::new().parse_str(source).is_err(), "{source}");
    }
    assert_eq!(
        AdamAstParser::new()
            .parse_str("sheet root {}")
            .unwrap()
            .name,
        "root"
    );
}

/// Reports the module-lowering boundary instead of silently ignoring structural uses.
#[test]
fn legacy_checker_reports_unlowered_components() {
    for source in [
        "sheet root { relationship helper(); }",
        "sheet helper() {}",
        "sheet root { conditional true { _ => { relationship helper(); } } }",
    ] {
        let module = AdamAstParser::new().parse_module_str(source).unwrap();
        let ast::ModuleItem::Sheet(sheet) = &module.items[0] else {
            panic!()
        };
        let diagnostics = adam_lang::check_sheet(
            sheet,
            &adam_lang::TypeRegistry::new(),
            &cel_parser::OpLookup::new(),
        );
        assert!(
            diagnostics
                .iter()
                .any(|d| d.message().contains("module lowering")),
            "{source}"
        );
    }
}
