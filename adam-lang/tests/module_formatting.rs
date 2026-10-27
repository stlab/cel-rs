//! Full-module comment preservation and formatter rejection contracts.

use adam_lang::{AdamAstParser, attach_module_trivia, format_module, format_source};

/// Preserves all documentation-to-keyword comments and documentation order on imports and components.
#[test]
fn documentation_regions_preserve_interspersed_comments() {
    for declaration in ["use lib::helper;", "sheet helper() {}"] {
        let source = format!(
            "//! doc_first\n/* keep_between_doc_and_header */\n//! doc_second\n// keep_before_keyword\n{declaration}\nsheet root {{}}"
        );
        let formatted = format_source(&source).unwrap();
        for marker in [
            "doc_first",
            "keep_between_doc_and_header",
            "doc_second",
            "keep_before_keyword",
        ] {
            assert_eq!(
                formatted.matches(marker).count(),
                1,
                "{marker}:\n{formatted}"
            );
        }
        assert!(formatted.find("doc_first") < formatted.find("keep_between_doc_and_header"));
        assert!(formatted.find("keep_between_doc_and_header") < formatted.find("doc_second"));
        assert!(formatted.find("doc_second") < formatted.find("keep_before_keyword"));
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }
}

/// Preserves every detached and mixed-style opening comment before sheet and branch uses.
#[test]
fn first_relationship_uses_preserve_complete_opening_comments() {
    let gap = "\n// keep_first\n\n/* keep_block */\n\n// keep_second\n";
    for source in [
        format!("sheet root {{{gap}relationship helper();\n}}"),
        format!(
            "sheet root {{ conditional true {{ true => {{{gap}relationship helper();\n}} }} }}"
        ),
        format!("sheet root {{ conditional true {{ _ => {{{gap}relationship helper();\n}} }} }}"),
    ] {
        let formatted = format_source(&source).unwrap();
        for marker in ["keep_first", "keep_block", "keep_second"] {
            assert_eq!(
                formatted.matches(marker).count(),
                1,
                "{marker}:\n{formatted}"
            );
        }
        assert!(formatted.find("keep_first") < formatted.find("keep_block"));
        assert!(formatted.find("keep_block") < formatted.find("keep_second"));
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }
}

/// Returns a span-independent representation of a complete public type-expression shape.
///
/// - Complexity: O(n * d) time and space for n type/name bytes and nesting depth d.
fn type_shape(ty: &adam_lang::ast::TypeExpr) -> String {
    use adam_lang::ast::TypeExpr;
    match ty {
        TypeExpr::Named { name, args, .. } => format!(
            "{name}({})",
            args.iter().map(type_shape).collect::<Vec<_>>().join(",")
        ),
        TypeExpr::Array { element, .. } => format!("[{}]", type_shape(element)),
        TypeExpr::Tuple { elements, .. } => format!(
            "({})",
            elements
                .iter()
                .map(type_shape)
                .collect::<Vec<_>>()
                .join(",")
        ),
    }
}

/// Preserves applied concrete type structure and all internal comments across formatting.
#[test]
fn parameter_types_preserve_complete_shapes_and_comments() {
    for type_text in [
        "Wrapper(i32)",
        "Wrapper /* keep_name_to_args */ (/* keep_inside_args */ i32)",
        "Wrapper // keep_name_line\n(/* keep_nested */ Other((i32, bool)), [Nested(bool)])",
    ] {
        let source = format!("sheet helper(a: {type_text}) {{}}");
        let before = AdamAstParser::new().parse_module_str(&source).unwrap();
        assert!(before.errors.is_empty(), "{:?}", before.errors);
        let formatted = format_source(&source).unwrap();
        let after = AdamAstParser::new().parse_module_str(&formatted).unwrap();
        assert!(after.errors.is_empty(), "{formatted}: {:?}", after.errors);
        let adam_lang::ast::ModuleItem::Sheet(before_sheet) = &before.items[0] else {
            panic!()
        };
        let adam_lang::ast::ModuleItem::Sheet(after_sheet) = &after.items[0] else {
            panic!()
        };
        assert_eq!(
            type_shape(&before_sheet.parameters.as_ref().unwrap()[0].type_name),
            type_shape(&after_sheet.parameters.as_ref().unwrap()[0].type_name),
            "{formatted}"
        );
        for marker in [
            "keep_name_to_args",
            "keep_inside_args",
            "keep_name_line",
            "keep_nested",
        ] {
            if source.contains(marker) {
                assert_eq!(
                    formatted.matches(marker).count(),
                    1,
                    "{marker}:\n{formatted}"
                );
            }
        }
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }
}

/// Uses compact path/list punctuation when comments require no intervening layout.
#[test]
fn module_canonical_punctuation() {
    let source = "use lib :: helper as h; sheet helper(a:i32,){} sheet empty(){} sheet root{relationship h(a:x,);}";
    assert_eq!(
        format_source(source).unwrap(),
        "use lib::helper as h;\nsheet helper(a: i32) {\n}\nsheet empty() {\n}\nsheet root {\n    relationship h(a: x);\n}\n"
    );
}

/// Preserves each module/header/argument comment once and reaches a formatting fixed point.
#[test]
fn module_comments_round_trip() {
    let source = "// leading\nuse /* import */ constraints::ordered as ord;
        // between
        sheet helper(/* signature */ left: i32, right: i32, /* signature end */) {}
        sheet empty(/* empty signature */) {}
        sheet first {
            relationship /* callee */ ord(/* argument */ left: a, right: /* actual */ b, /* arguments end */);
        }
        // inter-definition
        sheet second {}
        // trailing\n";
    let formatted = format_source(source).unwrap();
    assert_eq!(format_source(&formatted).unwrap(), formatted);
    for comment in [
        "leading",
        "import",
        "between",
        "/* signature */",
        "signature end",
        "empty signature",
        "callee",
        "/* argument */",
        "actual",
        "arguments end",
        "inter-definition",
        "trailing",
    ] {
        assert_eq!(
            formatted.matches(comment).count(),
            1,
            "{comment}: {formatted}"
        );
    }
    let mut module = AdamAstParser::new().parse_module_str(source).unwrap();
    attach_module_trivia(source, &mut module);
    assert_eq!(format_module(&module, source).unwrap(), formatted);
}

/// Refuses a recovered erroneous module rather than formatting its remaining definitions.
#[test]
fn module_formatter_rejects_recovered_errors() {
    let source = "use a::; sheet next {}";
    let mut module = AdamAstParser::new().parse_module_str(source).unwrap();
    attach_module_trivia(source, &mut module);
    assert!(format_module(&module, source).is_err());
    module.errors.clear();
    assert!(format_module(&module, source).is_err());
}

/// Rejects a sheet recovery placeholder even if diagnostic vectors have been removed.
#[test]
fn module_formatter_checks_nested_recovery_placeholders() {
    let source = "sheet root { cell broken unknown; }";
    let mut module = AdamAstParser::new().parse_module_str(source).unwrap();
    module.errors.clear();
    let adam_lang::ast::ModuleItem::Sheet(sheet) = &mut module.items[0] else {
        panic!()
    };
    sheet.errors.clear();
    assert!(format_module(&module, source).is_err());
}

/// Preserves documentation preceding an import rather than consuming it as a sheet header.
#[test]
fn import_documentation_round_trip() {
    let source = "//! file docs\nuse /* import keyword gap */ lib::helper;\nsheet root {}";
    let formatted = format_source(source).unwrap();
    assert_eq!(formatted.matches("file docs").count(), 1, "{formatted}");
    assert_eq!(
        formatted.matches("import keyword gap").count(),
        1,
        "{formatted}"
    );
    assert_eq!(format_source(&formatted).unwrap(), formatted);
}

/// Preserves line/block comments at every new declaration boundary, including detached runs.
#[test]
fn module_boundary_comments_are_unique() {
    let source = "/* file header */

        // detached

        use /* before path */ lib /* before separator */ :: /* after separator */ helper
        /* before alias */ as /* after alias */ h /* before semicolon */;
        //! component docs
        sheet /* before name */ helper // before signature
        ( // first formal
            a /* before colon */ : /* after colon */ i32 // after type
            , /* before close */ ) /* before body */ {
        }
        /* between definitions */
        sheet root {
            /// use docs
            relationship /* before callee */ h // before arguments
            ( // first argument
                a /* argument colon */ : /* before actual */ x // after actual
                , /* before argument close */ ) /* before use semicolon */ ;
            conditional true {
                true => {
                    // before branch use
                    relationship h(a: x); // after branch use
                }
                _ => { relationship h(); /* default trailing */ }
            }
        } /* after root */
        // file end
        ";
    let formatted = format_source(source).unwrap();
    assert_eq!(format_source(&formatted).unwrap(), formatted);
    for comment in [
        "file header",
        "detached",
        "before path",
        "before separator",
        "after separator",
        "before alias",
        "after alias",
        "before semicolon",
        "component docs",
        "before name",
        "before signature",
        "first formal",
        "before colon",
        "after colon",
        "after type",
        "before close",
        "before body",
        "between definitions",
        "use docs",
        "before callee",
        "before arguments",
        "first argument",
        "argument colon",
        "before actual",
        "after actual",
        "before argument close",
        "before use semicolon",
        "before branch use",
        "after branch use",
        "default trailing",
        "after root",
        "file end",
    ] {
        assert_eq!(
            formatted.matches(comment).count(),
            1,
            "{comment}:\n{formatted}"
        );
    }
}
