//! Exercises the accessible messages reported for numeric editor failures.

use super::side_panel::numeric_error_message;
use adam_web_ui::input::{InputError, NumberKind};

/// Maps each numeric failure category to its visible panel message.
#[test]
fn numeric_errors_map_to_accessible_panel_messages() {
    assert_eq!(
        numeric_error_message(&InputError::Lexical("incomplete".into())),
        "Enter a complete number."
    );
    assert_eq!(
        numeric_error_message(&InputError::TypeRange {
            kind: NumberKind::Signed(8),
            value: "999".into(),
        }),
        "The number is outside the supported range."
    );
    assert_eq!(
        numeric_error_message(&InputError::UnsupportedEditor("not supported".into())),
        "not supported"
    );
    assert_eq!(
        numeric_error_message(&InputError::Locale("locale unavailable".into())),
        "locale unavailable"
    );
    assert_eq!(
        numeric_error_message(&InputError::EventSource(
            "native selection unavailable".into()
        )),
        "native selection unavailable"
    );
}
