//! Full-module comment preservation and formatter rejection contracts.

use adam_lang::{AdamAstParser, attach_module_trivia, format_module, format_source};

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
