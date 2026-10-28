//! Builds a source-free native component and explicitly selected imported roots.

use adam_rs::{CellId, Method, MethodRecipe, Sheet, TemplateBuilder};

/// Constructs a checked integer multiplication component without a parser or CEL source.
///
/// The caller owns the source factor and ordinary boundary cells. Construction
/// specializes away the factor-writing direction without propagating.
/// Selected divisions use Rust's truncating integer semantics; exact inverse
/// multiplication requires a divisible product. This is not a floating template.
/// - Complexity: Native template construction and instantiation costs.
///
/// # Errors
/// Returns native definition or construction failures.
fn native() -> Result<(Sheet, CellId, CellId), Box<dyn std::error::Error>> {
    let mut builder = TemplateBuilder::new();
    let a = builder.parameter::<i32>("a")?;
    let b = builder.parameter::<i32>("b")?;
    let product = builder.parameter::<i32>("product")?;
    builder.relationship(vec![
        MethodRecipe::new(
            &[a.clone(), b.clone()],
            std::slice::from_ref(&product),
            |i, o| {
                Ok(Method::from_fn_2_1(
                    [i[0], i[1]],
                    o[0],
                    |a: &i32, b: &i32| {
                        a.checked_mul(*b)
                            .ok_or_else(|| anyhow::anyhow!("integer multiplication overflow"))
                    },
                ))
            },
        )?,
        MethodRecipe::new(
            &[product.clone(), b.clone()],
            std::slice::from_ref(&a),
            |i, o| {
                Ok(Method::from_fn_2_1(
                    [i[0], i[1]],
                    o[0],
                    |product: &i32, b: &i32| {
                        product
                            .checked_div(*b)
                            .ok_or_else(|| anyhow::anyhow!("invalid integer division"))
                    },
                ))
            },
        )?,
        MethodRecipe::new(&[product, a], &[b], |i, o| {
            Ok(Method::from_fn_2_1(
                [i[0], i[1]],
                o[0],
                |product: &i32, a: &i32| {
                    product
                        .checked_div(*a)
                        .ok_or_else(|| anyhow::anyhow!("invalid integer division"))
                },
            ))
        })?,
    ])?;
    let template = builder.finish()?;
    let mut sheet = Sheet::new();
    let factor = sheet.add_source(3i32);
    let value = sheet.add_cell(2i32);
    let result = sheet.add_cell(0i32);
    let instance = template.instantiate(
        &mut sheet,
        &[("product", result), ("a", value), ("b", factor)],
    )?;
    // Live methods own their resources after all construction owners disappear.
    drop((instance, template));
    Ok((sheet, value, result))
}

/// Compiles explicitly registered files and propagates each explicitly selected root.
///
/// These scalar fixtures use only builtin CEL operations. Geometry clients must
/// register Euclid types and install cel-std/cel-euclid before creating a compiler.
/// - Complexity: Collection compilation plus three root constructions and propagations.
///
/// # Errors
/// Returns owning module diagnostics or native propagation errors.
fn parsed() -> Result<(), Box<dyn std::error::Error>> {
    let mut modules = adam_lang::ModuleSet::new();
    modules.insert_source(
        "relationships".parse()?,
        "relationships.adm2",
        include_str!("relationships.adm2"),
    )?;
    modules.insert_source(
        "composed".parse()?,
        "composed.adm2",
        include_str!("composed.adm2"),
    )?;
    let compiled =
        adam_lang::ModuleCompiler::new(adam_lang::TypeRegistry::new(), cel_parser::OpLookup::new())
            .compile(modules)
            .map_err(|mut diagnostics| diagnostics.remove(0))?;
    let path = "composed".parse()?;
    for (name, cell, expected) in [
        ("ordered_demo", "high", 10.0),
        ("source_specialized", "value", 2.0),
        ("repeated_products", "a", 2.0),
    ] {
        let mut parsed = compiled.instantiate_root(&path, name)?;
        parsed.propagate()?;
        assert_eq!(*parsed.read::<f64>(parsed.cell_names[cell].0)?, expected);
    }
    Ok(())
}

/// Runs the native construction independently, then the explicit module example.
///
/// # Errors
/// Returns the first construction, compilation or propagation failure.
/// - Complexity: Native construction/propagation plus [`parsed`].
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (mut sheet, value, product) = native()?;
    sheet.write(value, 7i32)?;
    sheet.propagate()?;
    assert_eq!(*sheet.read::<i32>(product)?, 21);
    println!("native: 7 * 3 = 21");
    parsed()?;
    println!("parsed: ordered bounds, source specialization, two distinct multiplication uses");
    Ok(())
}

#[cfg(test)]
mod tests {
    /// Detects a missing native reverse direction and unchecked signed overflow.
    #[test]
    fn native_component_has_checked_bidirectional_behavior() {
        let (mut sheet, value, product) = super::native().unwrap();
        sheet.write(value, 7i32).unwrap();
        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<i32>(product).unwrap(), 21);
        sheet.write(product, 12i32).unwrap();
        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<i32>(value).unwrap(), 4);
        sheet.write(value, i32::MAX).unwrap();
        assert!(sheet.propagate().is_err());
    }

    /// Executes every selected imported root from the registered module collection.
    #[test]
    fn imported_examples_construct_and_propagate() {
        super::parsed().unwrap();
    }
}
