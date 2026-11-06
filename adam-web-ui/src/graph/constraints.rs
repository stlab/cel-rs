//! Pure owner of the embedded Adam graph-view policy.
//!
//! Construct one [`ViewConstraints`] per graph, submit absolute proposals, and
//! use [`ViewAction::Refresh`] for geometry-only updates. Only an explicit
//! [`ViewAction::Fit`] replaces the user's pan/zoom aspiration with the fit.
//!
//! ```
//! use adam_web_ui::graph::{ViewAction, ViewConstraints, ViewInput};
//! use cel_euclid::{Point2D, Rect, Size2D};
//! let mut model = ViewConstraints::new()?;
//! let output = model.update(ViewInput {
//!     bounds: Rect::new(Point2D::zero(), Size2D::new(200.0, 100.0)),
//!     viewport: Size2D::new(400.0, 300.0),
//!     action: ViewAction::Fit,
//! })?;
//! assert_eq!((output.view.x, output.view.y, output.view.k), (0.0, 50.0, 2.0));
//! # Ok::<(), adam_web_ui::graph::GraphFailure>(())
//! ```

use adam_lang::{AdamParser, ParsedSheet, SheetDiagnostics};
use adam_rs::CellId;
use anyhow::ensure;
use cel_euclid::{Rect, Size2D, Transform2D, Vector2D};
use serde::{Deserialize, Serialize};

use super::GraphFailure;
use crate::{RuntimeFailure, build, labels::Renderer};
/// Keeps thread-confined diagnostic owners in their original typed error.
type Result<T> = std::result::Result<T, GraphFailure>;

/// The embedded policy evaluated by every model instance.
const SOURCE: &str = include_str!("view_constraints.adm2");
/// The diagnostic source name for the embedded policy.
const FILE_NAME: &str = "view_constraints.adm2";

/// A uniform scale followed by a viewport-space translation.
///
/// Approved values are finite and `k` is positive.
///
/// # Examples
/// ```
/// use adam_web_ui::graph::ViewTransform;
/// let view = ViewTransform { x: 0.0, y: 50.0, k: 2.0 };
/// assert_eq!(view.k, 2.0);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ViewTransform {
    /// The horizontal viewport-space translation.
    pub x: f64,
    /// The vertical viewport-space translation.
    pub y: f64,
    /// The positive uniform zoom factor.
    pub k: f64,
}

/// An explicit source edit, geometry refresh, or replacement with the sheet's fit.
///
/// # Examples
/// ```
/// use adam_web_ui::graph::ViewAction;
/// assert_eq!(serde_json::to_string(&ViewAction::Fit).unwrap(), r#"{"kind":"fit"}"#);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ViewAction {
    /// Replaces the original source aspiration with an absolute proposal.
    Propose {
        /// The finite proposed translation and positive finite zoom.
        view: ViewTransform,
    },
    /// Updates only bounds and viewport, preserving source aspirations.
    Refresh,
    /// Replaces pan and zoom sources with the sheet's current derived fit.
    Fit,
}

/// Geometry and an action for a single model update.
///
/// Bounds have finite ordered endpoints and representable nonnegative extents.
/// Viewport extents are positive and finite.
///
/// # Examples
/// ```
/// use adam_web_ui::graph::{ViewAction, ViewInput};
/// use cel_euclid::{Rect, Size2D};
/// let input = ViewInput {
///     bounds: Rect::default(),
///     viewport: Size2D::new(400.0, 300.0),
///     action: ViewAction::Refresh,
/// };
/// assert!(input.viewport.width > 0.0);
/// ```
#[derive(Clone, Copy, Debug)]
pub struct ViewInput {
    /// The graph's content bounds, including any renderer-provided margin.
    pub bounds: Rect,
    /// The measured viewport size.
    pub viewport: Size2D,
    /// The requested source operation.
    pub action: ViewAction,
}

/// Complete finite approved values and gesture hints derived by Adam.
///
/// `min_zoom <= view.k <= max_zoom`; the maximum can exceed eight when fit does.
///
/// # Examples
/// ```
/// # use adam_web_ui::graph::{ViewOutput, ViewTransform};
/// let view = ViewTransform { x: 0.0, y: 50.0, k: 2.0 };
/// let output = ViewOutput { view, fit: view, min_zoom: 2.0, max_zoom: 8.0 };
/// assert!(output.min_zoom <= output.view.k && output.view.k <= output.max_zoom);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ViewOutput {
    /// The conformed view to render.
    pub view: ViewTransform,
    /// The fit target for the current geometry.
    pub fit: ViewTransform,
    /// The fit-derived minimum scale.
    pub min_zoom: f64,
    /// The sheet's effective maximum scale.
    pub max_zoom: f64,
}

/// Cached identities for the fixed embedded sheet.
struct Cells {
    /// The bounds source.
    bounds: CellId,
    /// The viewport source.
    viewport: CellId,
    /// The ordinary pan cell retaining its source aspiration.
    pan: CellId,
    /// The ordinary zoom cell retaining its source aspiration.
    zoom: CellId,
    /// The derived fit scale.
    fit_zoom: CellId,
    /// The derived fit translation.
    fit_pan: CellId,
    /// The derived minimum scale.
    min_zoom: CellId,
    /// The derived maximum scale.
    max_zoom: CellId,
    /// The derived affine view.
    view: CellId,
}

/// Owns one parsed sheet and cached identities without per-request parsing.
///
/// All fit, centering, clamping, and zoom-limit arithmetic lives in the sheet.
/// Invalid input leaves the model available. Any error after mutation discards
/// its candidate permanently; subsequent updates report unavailable.
///
/// # Examples
/// ```
/// let model = adam_web_ui::graph::ViewConstraints::new();
/// assert!(model.is_ok());
/// ```
pub struct ViewConstraints {
    /// The sole candidate, absent after a mutating error.
    candidate: Option<ViewCandidate>,
}

impl ViewConstraints {
    /// Constructs the embedded policy once.
    ///
    /// # Errors
    /// Returns construction, initialization, or output diagnostics.
    ///
    /// - Complexity: O(s² + n + p) time and O(s + n + p) space for installed
    ///   signatures `s`, source bytes `n`, and propagation work `p`.
    ///
    /// # Examples
    /// ```
    /// let model = adam_web_ui::graph::ViewConstraints::new()?;
    /// # Ok::<(), adam_web_ui::graph::GraphFailure>(())
    /// ```
    pub fn new() -> Result<Self> {
        Ok(Self {
            candidate: Some(ViewCandidate::new()?),
        })
    }

    /// Applies valid input only to an available candidate and retires it on error.
    ///
    /// # Errors
    /// Returns invalid input before mutation, typed runtime errors, invalid outputs,
    /// or unavailable after any previous mutating failure.
    ///
    /// - Complexity: O(p) time and space in propagation work; no parsing.
    ///
    /// # Examples
    /// See the module example for a fit request.
    pub fn update(&mut self, input: ViewInput) -> Result<ViewOutput> {
        if self.candidate.is_none() {
            return Err(GraphFailure::Unavailable);
        }
        validate_input(&input).map_err(|error| GraphFailure::InvalidInput(error.to_string()))?;
        let mut candidate = self.candidate.take().expect("availability checked");
        let output = candidate.update(input)?;
        self.candidate = Some(candidate);
        Ok(output)
    }
}

/// Contains the live model only while its owner admits mutation.
struct ViewCandidate {
    /// The persistent source and derived values.
    parsed: ParsedSheet,
    /// The identities resolved once during construction.
    cells: Cells,
}

impl ViewCandidate {
    /// Parses and propagates the embedded sheet using the configured library host.
    ///
    /// # Errors
    /// Returns a formatted diagnostic for invalid sheet syntax, missing cells,
    /// evaluation failure, or incomplete/non-finite initial outputs.
    ///
    /// - Complexity: O(s² + n + p) time and O(s + n + p) space for installed
    ///   signatures `s`, source length `n`, and the sheet's propagation work `p`.
    ///
    /// # Examples
    /// ```
    /// let mut model = adam_web_ui::graph::ViewConstraints::new()?;
    /// # Ok::<(), adam_web_ui::graph::GraphFailure>(())
    /// ```
    pub fn new() -> Result<Self> {
        let mut parser = AdamParser::new(build::type_registry(), build::op_lookup());
        let parsed = parser.parse_str(SOURCE).map_err(|error| {
            GraphFailure::InvalidInput(error.format_rustc_style(
                SOURCE,
                FILE_NAME,
                1,
                &Renderer::plain(),
            ))
        })?;
        let id = |name: &str| {
            parsed
                .cell_names
                .get(name)
                .map(|&(id, _)| id)
                .ok_or_else(|| {
                    GraphFailure::InvalidInput(format!("{FILE_NAME}: missing cell `{name}`"))
                })
        };
        let cells = Cells {
            bounds: id("bounds")?,
            viewport: id("viewport")?,
            pan: id("pan")?,
            zoom: id("zoom")?,
            fit_zoom: id("fit_zoom")?,
            fit_pan: id("fit_pan")?,
            min_zoom: id("min_zoom")?,
            max_zoom: id("effective_max_zoom")?,
            view: id("view")?,
        };
        let mut model = Self { parsed, cells };
        model.propagate()?;
        model.output()?;
        Ok(model)
    }

    /// Validates inputs, writes geometry and requested sources, and propagates.
    ///
    /// Refresh preserves the original pan/zoom aspiration; Fit copies only the
    /// sheet's derived fit into those sources and propagates a second time.
    ///
    /// # Errors
    /// Returns an error for non-finite/unordered/unrepresentable bounds,
    /// nonpositive/non-finite viewport or proposed zoom, non-finite proposed pan,
    /// sheet evaluation failure, or incomplete/non-finite/nonpositive outputs.
    /// A failed update never returns a fallback view.
    ///
    /// - Complexity: O(p) time and space in sheet propagation work; no parsing.
    ///
    /// # Examples
    /// ```
    /// # use adam_web_ui::graph::{ViewAction, ViewConstraints, ViewInput};
    /// # use cel_euclid::{Rect, Size2D};
    /// let mut model = ViewConstraints::new()?;
    /// let output = model.update(ViewInput {
    ///     bounds: Rect::default(),
    ///     viewport: Size2D::new(400.0, 300.0),
    ///     action: ViewAction::Fit,
    /// })?;
    /// assert_eq!(output.view.k, 300.0);
    /// # Ok::<(), adam_web_ui::graph::GraphFailure>(())
    /// ```
    pub fn update(&mut self, input: ViewInput) -> Result<ViewOutput> {
        self.write(self.cells.bounds, input.bounds)?;
        self.write(self.cells.viewport, input.viewport)?;
        if let ViewAction::Propose { view } = input.action {
            self.write(self.cells.pan, Vector2D::new(view.x, view.y))?;
            self.write(self.cells.zoom, view.k)?;
        }
        self.propagate()?;
        let output = self.output()?;
        if input.action == ViewAction::Fit {
            let pan: Vector2D = self.read(self.cells.fit_pan)?;
            let zoom: f64 = self.read(self.cells.fit_zoom)?;
            self.write(self.cells.pan, pan)?;
            self.write(self.cells.zoom, zoom)?;
            self.propagate()?;
            return self.output();
        }
        Ok(output)
    }

    /// Retains a mutating sheet error with independently owned diagnostics.
    ///
    /// - Complexity: O(file name bytes); moves the existing diagnostic snapshot.
    fn diagnostic(diagnostics: SheetDiagnostics, error: adam_rs::Error) -> GraphFailure {
        GraphFailure::Runtime(Box::new(RuntimeFailure {
            error,
            diagnostics: Box::new(diagnostics),
            file_name: FILE_NAME.into(),
        }))
    }

    /// Writes a typed source, retaining its typed mutation error on failure.
    fn write<T: 'static>(&mut self, id: CellId, value: T) -> Result<()> {
        let diagnostics = self.parsed.diagnostics();
        self.parsed
            .write(id, value)
            .map_err(|error| Self::diagnostic(diagnostics, error))
    }

    /// Reads a complete typed value, snapshotting diagnostics only when it fails.
    ///
    /// - Complexity: O(1) when the read succeeds; O(m + e) on error for metadata
    ///   snapshot size m and rendered error sites e.
    fn read<T: Copy + 'static>(&self, id: CellId) -> Result<T> {
        match self.parsed.read::<T>(id).copied() {
            Ok(value) => Ok(value),
            Err(error) => {
                let diagnostics = self.parsed.diagnostics();
                Err(GraphFailure::InvalidInput(
                    Self::diagnostic(diagnostics, error).to_string(),
                ))
            }
        }
    }

    /// Propagates the cached sheet and clears its presentation change flags.
    ///
    /// # Errors
    /// Returns the typed evaluation error with its pre-mutation diagnostic snapshot.
    ///
    /// - Complexity: O(p) time and space in sheet propagation work.
    fn propagate(&mut self) -> Result<()> {
        let diagnostics = self.parsed.diagnostics();
        self.parsed
            .propagate()
            .map_err(|error| Self::diagnostic(diagnostics, error))?;
        self.parsed.clear_changed();
        Ok(())
    }

    /// Reads complete outputs and rejects non-finite or invalid scale results.
    ///
    /// # Errors
    /// Returns an explicit diagnostic for unavailable values or invalid outputs.
    fn output(&self) -> Result<ViewOutput> {
        let view: Transform2D = self.read(self.cells.view)?;
        let fit_pan: Vector2D = self.read(self.cells.fit_pan)?;
        let fit_zoom: f64 = self.read(self.cells.fit_zoom)?;
        let min_zoom: f64 = self.read(self.cells.min_zoom)?;
        let max_zoom: f64 = self.read(self.cells.max_zoom)?;
        if !([
            view.m11, view.m12, view.m21, view.m22, view.m31, view.m32, fit_pan.x, fit_pan.y,
            fit_zoom, min_zoom, max_zoom,
        ]
        .into_iter()
        .all(f64::is_finite)
            && fit_zoom > 0.0
            && min_zoom > 0.0
            && min_zoom <= view.m11
            && view.m11 <= max_zoom)
        {
            return Err(GraphFailure::InvalidInput(format!(
                "{FILE_NAME}: invalid or unrepresentable view/fit outputs"
            )));
        }
        Ok(ViewOutput {
            view: ViewTransform {
                x: view.m31,
                y: view.m32,
                k: view.m11,
            },
            fit: ViewTransform {
                x: fit_pan.x,
                y: fit_pan.y,
                k: fit_zoom,
            },
            min_zoom,
            max_zoom,
        })
    }
}

/// Rejects invalid geometry and proposals before any sheet source is written.
///
/// Positive extents must advance the represented endpoint.
///
/// # Errors
/// Returns a named input error for invalid bounds, viewport, or proposal.
fn validate_input(input: &ViewInput) -> anyhow::Result<()> {
    let bounds = input.bounds;
    for (origin, extent, end) in [
        (bounds.origin.x, bounds.size.width, bounds.max_x()),
        (bounds.origin.y, bounds.size.height, bounds.max_y()),
    ] {
        ensure!(
            origin.is_finite()
                && extent.is_finite()
                && extent >= 0.0
                && end.is_finite()
                && end >= origin
                && (end - origin).is_finite()
                && (extent == 0.0 || end > origin),
            "invalid bounds: expected finite ordered endpoints and representable nonnegative extents"
        );
    }

    ensure!(
        input.viewport.width.is_finite()
            && input.viewport.height.is_finite()
            && input.viewport.width > 0.0
            && input.viewport.height > 0.0,
        "invalid viewport: expected positive finite extents"
    );
    if let ViewAction::Propose { view } = input.action {
        ensure!(
            view.x.is_finite() && view.y.is_finite(),
            "invalid proposed pan: expected finite coordinates"
        );
        ensure!(
            view.k.is_finite() && view.k > 0.0,
            "invalid proposed zoom: expected a positive finite scale"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A mutating error discards its candidate; later work never invokes that sheet.
    #[test]
    fn runtime_failure_retires_without_invoking_sheet_again() {
        let calls = std::rc::Rc::new(std::cell::Cell::new(0));
        let observed = calls.clone();
        let mut model = ViewConstraints::new().unwrap();
        let candidate = model.candidate.as_mut().unwrap();
        let output = candidate.parsed.add_cell(0.0_f64);
        candidate
            .parsed
            .add_relationship(vec![adam_rs::Method::from_fn_1_1(
                candidate.cells.bounds,
                output,
                move |_bounds: &Rect| {
                    observed.set(observed.get() + 1);
                    Err::<f64, _>(anyhow::anyhow!("graph fixture propagation failure"))
                },
            )])
            .unwrap();
        let input = ViewInput {
            bounds: Rect::default(),
            viewport: Size2D::new(400.0, 300.0),
            action: ViewAction::Fit,
        };
        let error = model.update(input).unwrap_err();
        let GraphFailure::Runtime(failure) = error else {
            panic!("typed runtime failure required");
        };
        assert_eq!(failure.file_name, FILE_NAME);
        assert_eq!(calls.get(), 1);
        assert!(matches!(
            model.update(input),
            Err(GraphFailure::Unavailable)
        ));
        assert_eq!(calls.get(), 1);
    }
}
