//! Undo/redo history: a linear stack of [`Document`] snapshots with a
//! current-position index, following the standard "history + cursor"
//! model — undo moves the cursor back, redo moves it forward, and any new
//! edit truncates everything after the cursor before appending.

use crate::model::document::Document;

/// Returns the history index to restore for an undo from `current_index`,
/// or `None` if already at the oldest recorded state.
#[must_use]
pub(crate) fn undo_target(current_index: usize) -> Option<usize> {
    current_index.checked_sub(1)
}

/// Returns the history index to restore for a redo from `current_index`
/// within a history of `history_len` entries, or `None` if already at the
/// newest recorded state.
#[must_use]
pub(crate) fn redo_target(current_index: usize, history_len: usize) -> Option<usize> {
    let next = current_index + 1;
    (next < history_len).then_some(next)
}

/// Records `current` as a new history entry after `current_index`,
/// discarding any entries after that point (the "redo branch" a fresh
/// edit replaces, matching standard undo/redo behavior once one or more
/// undos have happened). Returns the new current index.
///
/// - Postcondition: the returned index is `history.len() - 1`, and
///   `history[returned index] == current`.
///
/// - Complexity: O(n) in `history.len()` (the truncation).
pub(crate) fn record_history(
    history: &mut Vec<Document>,
    current_index: usize,
    current: Document,
) -> usize {
    history.truncate(current_index + 1);
    history.push(current);
    history.len() - 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_target_at_the_start_of_history_returns_none() {
        assert_eq!(undo_target(0), None);
    }

    #[test]
    fn undo_target_mid_history_returns_the_prior_index() {
        assert_eq!(undo_target(3), Some(2));
    }

    #[test]
    fn redo_target_at_the_end_of_history_returns_none() {
        assert_eq!(redo_target(2, 3), None);
    }

    #[test]
    fn redo_target_mid_history_returns_the_next_index() {
        assert_eq!(redo_target(1, 3), Some(2));
    }

    #[test]
    fn record_history_appends_to_a_fresh_history() {
        let mut history = vec![Document::new("d0")];
        let new_index = record_history(&mut history, 0, Document::new("d1"));

        assert_eq!(new_index, 1);
        assert_eq!(history, vec![Document::new("d0"), Document::new("d1")]);
    }

    #[test]
    fn record_history_truncates_the_redo_branch_before_appending() {
        let mut history = vec![
            Document::new("d0"),
            Document::new("d1"),
            Document::new("d2"),
            Document::new("d3"),
        ];
        // As if the cursor had been moved back to d1 via undo, then a new
        // edit happens — d2/d3 (the old redo branch) must be discarded.
        let new_index = record_history(&mut history, 1, Document::new("new"));

        assert_eq!(new_index, 2);
        assert_eq!(
            history,
            vec![
                Document::new("d0"),
                Document::new("d1"),
                Document::new("new")
            ]
        );
    }
}
