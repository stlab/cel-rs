//! Runs the actual crop and pan/zoom sheets without any crop-specific operation.
//!
//! `cargo run -p cel-euclid --features adam --example adam_geometry`
//! validates input boundaries, checks all requirements, demonstrates source
//! spring-back and explicit edits, and reads the overlay from `Sheet::source`.

mod support;

use adam_lang::{AdamParser, ParsedSheet, TypeRegistry};
use anyhow::{Result, ensure};
use cel_euclid::{Angle, Point2D, Rect, Size2D, Transform2D};

/// Builds a parser with equivalent standard/Euclid operations and Adam types.
///
/// - Complexity: O(s²) in the number of installed signatures.
fn parser() -> AdamParser {
    let mut types = TypeRegistry::new();
    cel_euclid::register_adam_types(&mut types);
    let mut lookup = cel_parser::OpLookup::new();
    cel_std::install(&mut lookup);
    cel_euclid::install(&mut lookup);
    AdamParser::new(types, lookup)
}

/// Propagates and refuses values with violated requirements.
///
/// - Complexity: The cost of propagation plus O(c + r) for cells and requirements.
///
/// # Errors
/// Returns propagation errors or the name of the first invalid requirement.
fn propagate(parsed: &mut ParsedSheet) -> Result<()> {
    parsed.propagate()?;
    for (name, (id, _)) in &parsed.cell_names {
        if let Some(requirement) = parsed.violated_requirements(*id).next() {
            anyhow::bail!(
                "{name}: requirement {} failed",
                parsed
                    .requirement_names
                    .get(&requirement)
                    .map_or("unnamed", String::as_str)
            );
        }
    }
    Ok(())
}

/// Runs source-preserving straightening and the declarative graph-policy demonstration.
///
/// - Complexity: Library registration plus a constant number of sheet propagations.
///
/// # Errors
/// Returns invalid geometry, evaluation, requirement, or demonstration assertion errors.
fn main() -> Result<()> {
    let boundary = support::InputBoundary::new()?;
    let mut parsed = parser()
        .parse_str(include_str!("crop_straighten.adm2"))
        .map_err(|error| anyhow::anyhow!("{}", error.message()))?;
    let crop_id = parsed.cell_names["crop"].0;
    let transform_id = parsed.cell_names["transform"].0;
    let image_id = parsed.cell_names["image"].0;
    propagate(&mut parsed)?;
    let image = *parsed.read::<Rect>(image_id)?;
    for degrees in [45.0, 20.0, 0.0] {
        // Read the source, never the previously fitted crop, for a fixed overlay.
        let overlay = *parsed.sheet.source::<Rect>(crop_id)?;
        let anchor = overlay.center();
        let transform = Transform2D::translation(-anchor.x, -anchor.y)
            .then(&Transform2D::rotation(Angle::degrees(degrees)))
            .then(&Transform2D::translation(anchor.x, anchor.y));
        boundary.validate(image, overlay, transform)?;
        parsed.write(transform_id, transform)?;
        propagate(&mut parsed)?;
        let fitted = *parsed.read::<Rect>(crop_id)?;
        let presentation = *parsed.read::<Transform2D>(parsed.cell_names["presentation"].0)?;
        let displayed_anchor = presentation.transform_point(anchor);
        let tol = 64.0 * f64::EPSILON * image.width().max(image.height());
        ensure!(
            (displayed_anchor - overlay.center()).length() <= tol,
            "anchor moved"
        );
        if degrees == 45.0 {
            ensure!(
                (fitted.width() - 100.0 / 2.0_f64.sqrt()).abs() < 1e-12,
                "wrong fitting"
            );
        }

        if degrees == 0.0 {
            ensure!(fitted == overlay, "source crop did not spring back");
        }
        println!("{degrees:>4} degrees: overlay={overlay:?}, effective={fitted:?}");
    }
    let edited = Rect::new(Point2D::new(10.0, 10.0), Size2D::new(80.0, 80.0));
    boundary.validate(image, edited, Transform2D::identity())?;
    parsed.write(crop_id, edited)?;
    propagate(&mut parsed)?;
    ensure!(
        *parsed.sheet.source::<Rect>(crop_id)? == edited,
        "explicit edit was lost"
    );

    let mut graph = parser()
        .parse_str(include_str!("pan_zoom.adm2"))
        .map_err(|error| anyhow::anyhow!("{}", error.message()))?;
    propagate(&mut graph)?;
    println!(
        "pan/zoom fit scale: {} (demonstration, no live graph migration)",
        graph.read::<f64>(graph.cell_names["fit_scale"].0)?
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Requirement failures surface as errors, even when propagation itself succeeds.
    #[test]
    fn propagation_rejects_named_requirements() {
        let mut parsed = parser()
            .parse_str("sheet s { source x: f64 = 1.0 require { @positive x > 0.0; }; }")
            .unwrap();
        propagate(&mut parsed).unwrap();
        let x = parsed.cell_names["x"].0;
        parsed.write(x, -1.0).unwrap();
        let error = propagate(&mut parsed).unwrap_err().to_string();
        assert!(error.contains("positive"), "{error}");
    }

    /// Finite but inaccurate presentation returns the sheet's named precision error.
    #[test]
    fn propagation_rejects_insufficient_presentation_precision() {
        let mut parsed = parser()
            .parse_str(include_str!("crop_straighten.adm2"))
            .unwrap();
        let image = parsed.cell_names["image"].0;
        let crop = parsed.cell_names["crop"].0;
        let transform = parsed.cell_names["transform"].0;
        parsed
            .write(
                image,
                Rect::new(
                    Point2D::new(0.03333283332557231, 0.03333283332557231),
                    Size2D::new(1e-6, 1e-6),
                ),
            )
            .unwrap();
        let requested = Rect::new(Point2D::new(-499999.9, -499999.9), Size2D::new(1e6, 1e6));
        parsed.write(crop, requested).unwrap();
        parsed
            .write(transform, Transform2D::scale(3.0, 3.0))
            .unwrap();
        let error = propagate(&mut parsed).unwrap_err().to_string();
        assert!(
            error.contains("sufficient_fit_and_presentation_precision"),
            "{error}"
        );
        assert_eq!(*parsed.sheet.source::<Rect>(crop).unwrap(), requested);
    }

    /// Runtime arithmetic errors are returned rather than mistaken for valid output.
    #[test]
    fn propagation_returns_evaluation_errors() {
        let mut parsed = parser()
            .parse_str("sheet s { source x: i32 = 0; out y := 1 / x; }")
            .unwrap();
        assert!(propagate(&mut parsed).is_err());
    }
}
