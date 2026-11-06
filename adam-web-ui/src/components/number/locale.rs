//! Numeric normalization and checked stepping.
//!
//! Pass a resolved [`NumberLocale`] to [`normalize_draft`], then submit only
//! [`NormalizedDraft::Complete`] values.
use crate::input::{InputError, NumberKind};

/// Builds an eval script that sends a resolved locale or explicit locale failure.
///
/// Every dynamic argument is JSON-encoded; `None` requests the host default.
/// The script sends a serde-compatible `Result` object through `dioxus.send`.
///
/// - Complexity: O(locale bytes) time and space, plus fixed helper source.
///
/// # Errors
/// Returns `Locale` if argument serialization fails.
///
/// # Examples
/// ```
/// use adam_web_ui::components::number::number_locale_script;
/// let script = number_locale_script(Some("de-DE")).unwrap();
/// ```
pub fn number_locale_script(locale: Option<&str>) -> Result<String, InputError> {
    let argument = serde_json::to_string(&locale).map_err(transport_error)?;
    Ok(eval_script(&format!(
        "numberLocale({argument} ?? undefined)"
    )))
}

/// Builds an eval script that formats concretely validated canonical text.
///
/// Every dynamic argument is JSON-encoded. The script sends a serde-compatible
/// `Result<String, String>` through `dioxus.send`; JavaScript never handles
/// instance/revision integers. Capture those in Rust for [`decode_format_reply`].
///
/// - Complexity: O(canonical + locale + options bytes) time and space.
///
/// # Errors
/// Returns the numeric validation error or `Locale` for serialization failure.
///
/// # Examples
/// ```
/// use adam_web_ui::{components::number::format_number_script, input::NumberKind};
/// let script = format_number_script("255", NumberKind::Unsigned(8), "en-US",
///     &serde_json::json!({})).unwrap();
/// ```
pub fn format_number_script(
    canonical: &str,
    kind: NumberKind,
    locale: &str,
    options: &serde_json::Value,
) -> Result<String, InputError> {
    kind.validate()?;
    if !complete_syntax(canonical, kind)? {
        return Err(lexical(canonical));
    }
    validate_number(canonical, kind)?;
    let integer = matches!(kind, NumberKind::Signed(_) | NumberKind::Unsigned(_));
    let arguments =
        serde_json::to_string(&(canonical, integer, locale, options)).map_err(transport_error)?;
    Ok(eval_script(&format!("formatNumber(...{arguments})")))
}

/// Decodes and validates a locale script reply without substituting defaults.
///
/// - Complexity: O(reply bytes) time and space.
///
/// # Errors
/// Returns `Locale` for failed resolution, malformed transport, or invalid symbols.
///
/// # Examples
/// ```
/// use adam_web_ui::components::number::decode_locale_reply;
/// assert!(decode_locale_reply(r#"{"Err":"unsupported locale"}"#).is_err());
/// ```
pub fn decode_locale_reply(reply: &str) -> Result<NumberLocale, InputError> {
    let locale = decode_reply(reply)?;
    validate_locale(&locale)?;
    Ok(locale)
}

/// Decodes a formatter reply with its Rust-captured instance/revision stamp.
///
/// - Complexity: O(reply bytes) time and space.
///
/// # Errors
/// Returns `Locale` for formatting failures or malformed transport.
///
/// # Examples
/// ```
/// use adam_web_ui::components::number::{decode_format_reply, DraftEvent};
/// assert_eq!(decode_format_reply(r#"{"Ok":"1,234"}"#, 2, 3).unwrap(),
///     DraftEvent::Formatted { text: "1,234".into(), generation: 2, revision: 3 });
/// ```
pub fn decode_format_reply(
    reply: &str,
    generation: u64,
    revision: u64,
) -> Result<super::DraftEvent, InputError> {
    Ok(super::DraftEvent::Formatted {
        text: decode_reply(reply)?,
        generation,
        revision,
    })
}

/// Wraps a JSON-encoded expression in the locale helper and an explicit result envelope.
///
/// - Complexity: O(expression bytes) time and space, plus fixed helper source.
fn eval_script(expression: &str) -> String {
    let helpers = include_str!("locale.js").replace("export function", "function");
    format!(
        "{helpers}\ntry {{ dioxus.send({{Ok: {expression}}}); }} catch (error) {{ dioxus.send({{Err: String(error)}}); }}"
    )
}

/// Converts serde transport/serialization errors into model-independent locale diagnostics.
///
/// - Complexity: O(diagnostic bytes) time and space.
fn transport_error(error: serde_json::Error) -> InputError {
    InputError::Locale(error.to_string())
}

/// Decodes the helper's result envelope without hiding failures behind defaults.
///
/// - Complexity: O(reply bytes) time and space.
fn decode_reply<T: serde::de::DeserializeOwned>(reply: &str) -> Result<T, InputError> {
    serde_json::from_str::<Result<T, String>>(reply)
        .map_err(transport_error)?
        .map_err(InputError::Locale)
}

/// Owns the token vocabulary of one resolved decimal locale.
///
/// Digit tokens are ordered zero through nine. Group sizes count digits from
/// the right; the secondary size repeats to the left of the primary group.
/// Nonempty symbols have at most 16 bytes and no prefix ambiguities. Only
/// Arabic letter mark and left/right direction marks are ignorable. Normalization
/// validates these constraints and reports `InputError::Locale` on failure.
///
/// # Examples
/// ```
/// use adam_web_ui::components::number::NumberLocale;
/// let locale: NumberLocale = serde_json::from_str(r#"{
/// "resolved":"en-US","digits":["0","1","2","3","4","5","6","7","8","9"],
/// "decimal":".","group":",","minus":"-","plus":"+","ignorable":[],
/// "primary_group":3,"secondary_group":3}"#).unwrap();
/// assert_eq!(locale.resolved, "en-US");
/// ```
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NumberLocale {
    /// Contains the Intl-resolved locale identifier.
    pub resolved: String,
    /// Contains the localized digits in numeric order.
    pub digits: [String; 10],
    /// Separates the fraction from the integer.
    pub decimal: String,
    /// Separates integer digit groups.
    pub group: String,
    /// Contains the localized negative sign.
    pub minus: String,
    /// Contains the localized positive sign.
    pub plus: String,
    /// Contains ignorable bidi formatting marks.
    pub ignorable: Vec<String>,
    /// Counts digits in the rightmost integer group.
    pub primary_group: usize,
    /// Counts digits in subsequent integer groups.
    pub secondary_group: usize,
}

/// Distinguishes a numeric prefix from a complete, representable canonical draft.
///
/// # Examples
/// ```
/// use adam_web_ui::components::number::NormalizedDraft;
/// let complete = NormalizedDraft::Complete("18446744073709551615".into());
/// assert_ne!(complete, NormalizedDraft::Incomplete);
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NormalizedDraft {
    /// Retains an empty, sign, decimal, or exponent prefix without a write.
    Incomplete,
    /// Carries ASCII decimal syntax validated against the concrete primitive.
    Complete(String),
}

/// Normalizes locale tokens without coercing integer values through floats.
///
/// Empty input, lone signs/decimal separators, and exponent prefixes remain
/// incomplete. Grouping is accepted only in the mantissa's integer part, with
/// the locale's primary/secondary group widths. Space-grouping locales accept
/// ordinary, nonbreaking, and narrow nonbreaking spaces interchangeably.
/// Complete integer input uses decimal integer syntax; incomplete prefixes
/// remain local even when their eventual spelling would require a float.
///
/// - Complexity: O(raw bytes + locale token bytes) time and space.
///
/// # Errors
/// Returns `Lexical` for malformed syntax/grouping, `TypeRange` for overflow,
/// nonfinite conversion, or nonzero values rounded to zero, `UnsupportedEditor`
/// for unknown widths, and `Locale` for ambiguous or malformed vocabularies.
///
/// # Examples
/// ```
/// use adam_web_ui::{components::number::*, input::NumberKind};
/// fn incomplete_prefix(locale: &NumberLocale) {
///     assert_eq!(normalize_draft("-", locale, NumberKind::F64),
///         Ok(NormalizedDraft::Incomplete));
/// }
/// ```
pub fn normalize_draft(
    raw: &str,
    locale: &NumberLocale,
    kind: NumberKind,
) -> Result<NormalizedDraft, InputError> {
    kind.validate()?;
    validate_locale(locale)?;
    let tokens = locale_tokens(locale);
    let mut rest = raw;
    let mut ascii = String::with_capacity(raw.len());
    // Translate a fixed vocabulary while consuming each source token exactly once.
    while !rest.is_empty() {
        if let Some(mark) = locale
            .ignorable
            .iter()
            .find(|mark| rest.starts_with(mark.as_str()))
        {
            rest = &rest[mark.len()..];
            continue;
        }
        if let Some((token, value)) = tokens
            .iter()
            .find(|(token, _)| !token.is_empty() && rest.starts_with(token))
        {
            ascii.push(*value);
            rest = &rest[token.len()..];
            continue;
        }
        let ch = rest.chars().next().unwrap();
        if is_group_space(&locale.group) && matches!(ch, ' ' | '\u{a0}' | '\u{202f}') {
            ascii.push(',');
        } else if ch.is_ascii_digit() || matches!(ch, '+' | '-' | 'e' | 'E') {
            ascii.push(ch);
        } else {
            return Err(lexical(raw));
        }
        rest = &rest[ch.len_utf8()..];
    }
    remove_grouping(&mut ascii, locale)?;
    if !complete_syntax(&ascii, kind)? {
        return Ok(NormalizedDraft::Incomplete);
    }
    validate_number(&ascii, kind)?;
    Ok(NormalizedDraft::Complete(ascii))
}

/// Steps canonical text with checked arithmetic, without clamping aspirations.
///
/// A step must be positive and representable by the bound primitive.
///
/// - Complexity: O(current bytes + step bytes) time and space.
///
/// # Errors
/// Returns `Lexical` for malformed canonical text or nonpositive steps,
/// `TypeRange` for nonfinite values, overflow, or nonzero values rounded to zero,
/// and `UnsupportedEditor` for unknown widths.
///
/// # Examples
/// ```
/// use adam_web_ui::{components::number::step_number, input::NumberKind};
/// assert_eq!(step_number("255", NumberKind::Unsigned(16), "1", true).unwrap(), "256");
/// ```
pub fn step_number(
    current: &str,
    kind: NumberKind,
    step: &str,
    increment: bool,
) -> Result<String, InputError> {
    kind.validate()?;
    for text in [current, step] {
        if !complete_syntax(text, kind)? {
            return Err(lexical(text));
        }
        validate_number(text, kind)?;
    }
    let result = match kind {
        NumberKind::Signed(_) => {
            let value = current
                .parse::<i128>()
                .map_err(|_| range_error(current, kind))?;
            let step = step.parse::<i128>().map_err(|_| range_error(step, kind))?;
            if step <= 0 {
                return Err(lexical("step must be positive"));
            }
            if increment {
                value.checked_add(step)
            } else {
                value.checked_sub(step)
            }
            .ok_or_else(|| range_error(current, kind))?
            .to_string()
        }
        NumberKind::Unsigned(_) => {
            let value = current
                .parse::<u128>()
                .map_err(|_| range_error(current, kind))?;
            let step = step.parse::<u128>().map_err(|_| range_error(step, kind))?;
            if step == 0 {
                return Err(lexical("step must be positive"));
            }
            if increment {
                value.checked_add(step)
            } else {
                value.checked_sub(step)
            }
            .ok_or_else(|| range_error(current, kind))?
            .to_string()
        }
        NumberKind::F32 => {
            let value = current
                .parse::<f32>()
                .map_err(|_| range_error(current, kind))?;
            let step = step.parse::<f32>().map_err(|_| range_error(step, kind))?;
            if step <= 0.0 {
                return Err(lexical("step must be positive"));
            }
            let result = if increment {
                value + step
            } else {
                value - step
            };
            if !result.is_finite() {
                return Err(range_error(current, kind));
            }
            result.to_string()
        }
        NumberKind::F64 => {
            let value = current
                .parse::<f64>()
                .map_err(|_| range_error(current, kind))?;
            let step = step.parse::<f64>().map_err(|_| range_error(step, kind))?;
            if step <= 0.0 {
                return Err(lexical("step must be positive"));
            }
            let result = if increment {
                value + step
            } else {
                value - step
            };
            if !result.is_finite() {
                return Err(range_error(current, kind));
            }
            result.to_string()
        }
    };
    validate_number(&result, kind)?;
    Ok(result)
}

/// Returns fixed-size token mappings; an empty grouping token is unused.
fn locale_tokens(locale: &NumberLocale) -> [(&str, char); 14] {
    std::array::from_fn(|i| match i {
        0..=9 => (locale.digits[i].as_str(), char::from(b'0' + i as u8)),
        10 => (locale.decimal.as_str(), '.'),
        11 => (locale.group.as_str(), ','),
        12 => (locale.minus.as_str(), '-'),
        _ => (locale.plus.as_str(), '+'),
    })
}

/// Rejects empty/ambiguous symbols and unsupported ignorable tokens.
///
/// Symbol tokens have at most 16 bytes; only the three numeric bidi marks are
/// ignorable. This bounds vocabulary lookup independently of draft length.
/// - Complexity: O(locale token bytes) time, O(1) additional space.
fn validate_locale(locale: &NumberLocale) -> Result<(), InputError> {
    let tokens = locale_tokens(locale);
    if locale.resolved.is_empty()
        || locale.primary_group == 0
        || locale.secondary_group == 0
        || locale.ignorable.len() > 3
        || locale
            .ignorable
            .iter()
            .any(|s| !matches!(s.as_str(), "\u{61c}" | "\u{200e}" | "\u{200f}"))
    {
        return Err(InputError::Locale("invalid locale metadata".into()));
    }
    for (i, (token, value)) in tokens.iter().enumerate() {
        if i == 11 && token.is_empty() {
            continue;
        }
        if token.is_empty()
            || token.len() > 16
            || token
                .chars()
                .any(|c| matches!(c, 'e' | 'E' | '\u{61c}' | '\u{200e}' | '\u{200f}'))
            || token.chars().any(|c| {
                (c.is_ascii_digit() || matches!(c, '+' | '-')) && (token.len() != 1 || c != *value)
            })
            || tokens[..i].iter().any(|(other, _)| {
                !other.is_empty() && (token.starts_with(other) || other.starts_with(token))
            })
        {
            return Err(InputError::Locale("ambiguous numeric symbols".into()));
        }
    }
    Ok(())
}

/// Identifies the interchangeable space grouping tokens.
fn is_group_space(token: &str) -> bool {
    matches!(token, " " | "\u{a0}" | "\u{202f}")
}

/// Validates integer grouping before removing its separators.
///
/// - Complexity: O(text bytes) time and O(1) additional space.
fn remove_grouping(text: &mut String, locale: &NumberLocale) -> Result<(), InputError> {
    if !text.contains(',') {
        return Ok(());
    }
    let unsigned = text.strip_prefix(['+', '-']).unwrap_or(text);
    let end = unsigned.find(['.', 'e', 'E']).unwrap_or(unsigned.len());
    let integer = &unsigned[..end];
    if unsigned[end..].contains(',') {
        return Err(lexical(text));
    }
    let mut groups = integer.rsplit(',');
    let last = groups.next().unwrap();
    if last.len() != locale.primary_group || !last.bytes().all(|b| b.is_ascii_digit()) {
        return Err(lexical(text));
    }
    let mut groups = groups.peekable();
    while let Some(group) = groups.next() {
        let valid_width = if groups.peek().is_none() {
            !group.is_empty() && group.len() <= locale.secondary_group
        } else {
            group.len() == locale.secondary_group
        };
        if !valid_width || !group.bytes().all(|b| b.is_ascii_digit()) {
            return Err(lexical(text));
        }
    }
    text.retain(|c| c != ',');
    Ok(())
}

/// Recognizes decimal syntax and its incomplete prefixes from ASCII text.
///
/// - Complexity: O(text bytes) time and O(1) additional space.
fn complete_syntax(text: &str, kind: NumberKind) -> Result<bool, InputError> {
    let bytes = text.as_bytes();
    let mut index = usize::from(bytes.first().is_some_and(|b| matches!(b, b'+' | b'-')));
    let start = index;
    while bytes.get(index).is_some_and(u8::is_ascii_digit) {
        index += 1;
    }
    let mut digits = index - start;
    if bytes.get(index) == Some(&b'.') {
        index += 1;
        let start = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        digits += index - start;
    }
    if digits == 0 {
        return if index == bytes.len() {
            Ok(false)
        } else {
            Err(lexical(text))
        };
    }
    if bytes.get(index).is_some_and(|b| matches!(b, b'e' | b'E')) {
        index += 1;
        if bytes.get(index).is_some_and(|b| matches!(b, b'+' | b'-')) {
            index += 1;
        }
        let start = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        if index == start && index == bytes.len() {
            return Ok(false);
        }
    }
    if index != bytes.len()
        || (matches!(kind, NumberKind::Signed(_) | NumberKind::Unsigned(_))
            && text.contains(['.', 'e', 'E']))
    {
        return Err(lexical(text));
    }
    Ok(true)
}

/// Checks exact integer bounds and concrete finite float conversion.
///
/// - Precondition: `kind` has a supported primitive width.
/// - Complexity: O(text bytes) time and O(1) additional space.
fn validate_number(text: &str, kind: NumberKind) -> Result<(), InputError> {
    debug_assert!(kind.validate().is_ok());
    let valid = match kind {
        NumberKind::Signed(bits) => text.parse::<i128>().is_ok_and(|value| {
            bits == 128 || {
                let magnitude = 1_i128 << (bits - 1);
                value >= magnitude.checked_neg().unwrap() && value < magnitude
            }
        }),
        NumberKind::Unsigned(bits) => text
            .parse::<u128>()
            .is_ok_and(|value| bits == 128 || value < (1_u128 << bits)),
        NumberKind::F32 => text
            .parse::<f32>()
            .is_ok_and(|value| value.is_finite() && (value != 0.0 || zero_mantissa(text))),
        NumberKind::F64 => text
            .parse::<f64>()
            .is_ok_and(|value| value.is_finite() && (value != 0.0 || zero_mantissa(text))),
    };
    if valid {
        Ok(())
    } else {
        Err(range_error(text, kind))
    }
}

/// Creates an owned lexical diagnostic.
///
/// - Complexity: O(text bytes) time and space.
fn lexical(text: &str) -> InputError {
    InputError::Lexical(text.into())
}

/// Preserves the rejected text in a concrete-range diagnostic.
///
/// - Complexity: O(text bytes) time and space.
fn range_error(text: &str, kind: NumberKind) -> InputError {
    InputError::TypeRange {
        kind,
        value: text.into(),
    }
}

/// Checks whether the decimal mantissa contains no nonzero digit.
///
/// - Complexity: O(text bytes) time and O(1) additional space.
fn zero_mantissa(text: &str) -> bool {
    !text
        .bytes()
        .take_while(|b| !matches!(b, b'e' | b'E'))
        .any(|b| matches!(b, b'1'..=b'9'))
}
