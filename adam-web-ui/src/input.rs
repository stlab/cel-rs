//! Model-independent numeric types and editor diagnostics.
//!
//! Use [`NumberKind::range`] for exact primitive endpoints.

/// Distinguishes input syntax, concrete range, editor support, and locale failures.
///
/// # Examples
/// ```
/// use adam_web_ui::input::InputError;
/// assert!(matches!(InputError::Lexical("bad input".into()), InputError::Lexical(_)));
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputError {
    /// Reports malformed numeric syntax.
    Lexical(String),
    /// Reports a value outside its concrete primitive range.
    TypeRange {
        /// Identifies the target primitive.
        kind: NumberKind,
        /// Preserves the rejected canonical text.
        value: String,
    },
    /// Reports an unsupported editor or primitive width.
    UnsupportedEditor(String),
    /// Reports locale resolution, symbol, serialization, or transport failure.
    Locale(String),
}

/// Identifies a concrete primitive without erasing its integer width.
///
/// Only Rust primitive widths 8, 16, 32, 64, and 128 are supported.
///
/// # Examples
/// ```
/// use adam_web_ui::input::NumberKind;
/// assert_eq!(NumberKind::Unsigned(8).range().unwrap().max, "255");
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumberKind {
    /// Identifies a signed integer by bit width.
    Signed(u32),
    /// Identifies an unsigned integer by bit width.
    Unsigned(u32),
    /// Identifies an IEEE 754 single-precision float.
    F32,
    /// Identifies an IEEE 754 double-precision float.
    F64,
}

/// Carries exact canonical primitive endpoints, excluding nonfinite floats.
///
/// # Examples
/// ```
/// use adam_web_ui::input::NumberKind;
/// assert_eq!(NumberKind::Signed(8).range().unwrap().min, "-128");
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NumericRange {
    /// Contains the inclusive minimum.
    pub min: String,
    /// Contains the inclusive maximum.
    pub max: String,
}

impl NumberKind {
    /// Returns exact finite bounds for a Rust numeric primitive.
    ///
    /// # Errors
    /// Returns `UnsupportedEditor` for nonprimitive integer widths.
    ///
    /// # Examples
    /// ```
    /// use adam_web_ui::input::NumberKind;
    /// assert_eq!(NumberKind::Unsigned(8).range().unwrap().max, "255");
    /// ```
    pub fn range(self) -> Result<NumericRange, InputError> {
        self.validate()?;
        let (min, max) = match self {
            Self::Signed(128) => (i128::MIN.to_string(), i128::MAX.to_string()),
            Self::Signed(bits) => {
                let magnitude = 1_i128 << (bits - 1);
                (
                    magnitude.checked_neg().unwrap().to_string(),
                    magnitude.checked_sub(1).unwrap().to_string(),
                )
            }
            Self::Unsigned(128) => ("0".into(), u128::MAX.to_string()),
            Self::Unsigned(bits) => ("0".into(), ((1_u128 << bits) - 1).to_string()),
            Self::F32 => (f32::MIN.to_string(), f32::MAX.to_string()),
            Self::F64 => (f64::MIN.to_string(), f64::MAX.to_string()),
        };
        Ok(NumericRange { min, max })
    }

    /// Rejects widths that do not identify a Rust primitive.
    ///
    /// # Errors
    /// Returns `UnsupportedEditor` for nonprimitive integer widths.
    pub(crate) fn validate(self) -> Result<(), InputError> {
        match self {
            Self::Signed(8 | 16 | 32 | 64 | 128)
            | Self::Unsigned(8 | 16 | 32 | 64 | 128)
            | Self::F32
            | Self::F64 => Ok(()),
            _ => Err(InputError::UnsupportedEditor(format!(
                "unsupported numeric kind: {self:?}"
            ))),
        }
    }
}

impl std::fmt::Display for InputError {
    /// Writes a model-independent diagnostic.
    ///
    /// - Complexity: O(diagnostic bytes) time.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Lexical(message) => write!(f, "Invalid numeric input: {message}"),
            Self::TypeRange { kind, value } => {
                write!(f, "{value} is outside the finite range of {kind:?}")
            }
            Self::UnsupportedEditor(message) => write!(f, "Unsupported editor: {message}"),
            Self::Locale(message) => write!(f, "Numeric locale error: {message}"),
        }
    }
}

impl std::error::Error for InputError {}
