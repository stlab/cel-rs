//! Examples backing `book-src/expressions.md` (Chapter 4). See `src/lib.rs` for how these
//! `.adm2` files are wired into the book. `no_standard_library` is kept as a regression test
//! only — it needs a parser built without `cel-std`, so it is never `{{#include}}`d into the
//! chapter itself; see `NO_LIVE_MOUNT` in `adam-lang-book-live-config`.

#[test]
fn no_standard_library() {
    let mut parser =
        adam_lang::AdamParser::new(adam_lang::TypeRegistry::new(), cel_parser::OpLookup::new());
    let err = parser
        .parse_str(include_str!(
            "../book-src/examples/expressions/no_standard_library.adm2"
        ))
        .err()
        .unwrap();
    assert!(format!("{err}").to_lowercase().contains("min"));
}

// ANCHOR: registered-record
/// Exercises host record construction and member reads in a live Adam sheet.
#[test]
fn registered_record_and_member_example() {
    use adam_lang::{AdamParser, TypeRegistry};
    use cel_parser::{OpLookup, OperationSignature, RecordField, RecordSchema, ResolvedLeafType};
    use cel_runtime::ValueType;

    /// A host-defined type with a named field.
    #[derive(Clone, Debug, Default, PartialEq)]
    struct Sample {
        value: f64,
    }
    let sample = ResolvedLeafType::new("Sample", ValueType::leaf::<Sample>());
    let scalar = ResolvedLeafType::new("f64", ValueType::leaf::<f64>());
    let mut registry = TypeRegistry::new();
    registry.register::<Sample>("Sample");
    let mut lookup = OpLookup::new();
    lookup
        .register_record(RecordSchema {
            ty: sample.clone(),
            fields: vec![RecordField {
                name: "value",
                ty: scalar.clone().into(),
            }],
            emit: Box::new(|s, _, _, _| s.op1(|value: f64| Sample { value })),
        })
        .unwrap();
    lookup
        .register_operation(OperationSignature {
            name: "value",
            operands: vec![sample.into()],
            result: scalar.into(),
            emit: Box::new(|s, _| s.op1(|sample: Sample| sample.value)),
        })
        .unwrap();
    let source = "sheet samples {
        cell Sample: bool = true;
        cell value: bool = false;
        cell reading: Sample = Sample { value: 3.0 };
        out measured: f64 := reading.value();
    }";
    let ast = adam_lang::AdamAstParser::new().parse_str(source).unwrap();
    assert!(adam_lang::check_sheet(&ast, &registry, &lookup).is_empty());
    let mut sheet = AdamParser::new(registry, lookup).parse_str(source).unwrap();
    let reading = sheet.cell_names["reading"].0;
    let measured = sheet.output_names["measured"];
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<f64>(measured).unwrap(), 3.0);
    sheet.write(reading, Sample { value: 7.0 }).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<f64>(measured).unwrap(), 7.0);
}
// ANCHOR_END: registered-record
