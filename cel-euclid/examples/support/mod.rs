//! Validation-only input boundary shared by the runner and its contract tests.
//!
//! This module never computes an effective crop or a shrink factor. The actual
//! Adam relationship owns fitting and the visible numerical policy.

use anyhow::{Result, ensure};
use cel_euclid::{Rect, Transform2D};
use cel_runtime::DynClosure;

/// Compiles general checked geometry operations once for candidate validation.
pub struct InputBoundary {
    /// Tests source-anchor feasibility and representability of inverse bounds.
    check: DynClosure,
}

impl InputBoundary {
    /// Builds a validation closure in an explicitly opted-in type/operation environment.
    ///
    /// - Complexity: O(s²) for library registration, plus constant expression compilation.
    ///
    /// # Errors
    /// Returns a parser/evaluator error if the validation expression cannot compile.
    pub fn new() -> Result<Self> {
        let mut types = adam_lang::TypeRegistry::new();
        cel_euclid::register_adam_types(&mut types);
        let mut lookup = cel_parser::OpLookup::new();
        cel_std::install(&mut lookup);
        cel_euclid::install(&mut lookup);
        let mut parser =
            cel_parser::CELParser::with_type_resolver(lookup, types.cel_type_resolver());
        let mut segment = parser
            .parse_str(
                "|i: Rect, r: Rect, t: Transform2D|
                (|i: Rect, a: Point2D, b: Rect|
                    a.is_finite()
                    && a.x > i.min_x && a.x < i.max_x
                    && a.y > i.min_y && a.y < i.max_y
                    && b.width > 0.0 && b.height > 0.0
                    && (Size2D {
                        width: max(a.x - b.min_x, b.max_x - a.x),
                        height: max(a.y - b.min_y, b.max_y - a.y)
                    }).is_finite()
                )(i, t.inverse().transform_point(r.center()),
                    t.inverse().outer_transformed_rect(r))",
            )
            .map_err(|error| anyhow::anyhow!("{}", error.message()))?;
        Ok(Self {
            check: segment.call0()?,
        })
    }

    /// Checks source geometry and its inverse mapping before the runner writes any input.
    ///
    /// This accepts crops that need shrinking; it does not fit them. After propagation,
    /// callers must also inspect every Adam requirement before using effective values.
    ///
    /// # Errors
    /// Returns an error for non-finite/nonpositive/unrepresentable rectangles,
    /// invalid checked inverses or bounds, or a source anchor not strictly inside the image.
    pub fn validate(&self, image: Rect, crop: Rect, transform: Transform2D) -> Result<()> {
        ensure!(valid_rect(image), "invalid or unrepresentable source image");
        ensure!(valid_rect(crop), "invalid or unrepresentable source crop");
        ensure!(
            self.check.call::<bool>(&[&image, &crop, &transform])?,
            "infeasible source anchor or unrepresentable inverse crop bounds"
        );
        Ok(())
    }
}

/// Tests positive finite extents, advancing finite endpoints, and a finite center.
fn valid_rect(rect: Rect) -> bool {
    rect.origin.is_finite()
        && rect.size.is_finite()
        && rect.width() > 0.0
        && rect.height() > 0.0
        && rect.max_x().is_finite()
        && rect.max_y().is_finite()
        && rect.max_x() > rect.min_x()
        && rect.max_y() > rect.min_y()
        && rect.center().is_finite()
}
