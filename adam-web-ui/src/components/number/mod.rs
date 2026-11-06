//! Locale-aware numeric editing without model or widget dependencies.
//!
//! Resolve a [`NumberLocale`], then pass native events to [`NumberDraft::reduce`].
mod draft;
mod locale;
pub use draft::{DraftEffect, DraftEvent, NumberDraft};
pub use locale::{
    NormalizedDraft, NumberLocale, decode_format_reply, decode_locale_reply, format_number_script,
    normalize_draft, number_locale_script, step_number,
};

#[cfg(test)]
mod tests;
