//! Human-readable labels for an [`adam_rs::Error`]'s
//! [`sites`](adam_rs::Error::sites), for attaching to a rustc-style multi-span
//! diagnostic (see [`cel_parser::SpanLabel`]).

use adam_rs::{CellId, Error, ErrorSite};

/// Describes the `index`-th entry of `e.sites()` in plain language, for use as a
/// [`cel_parser::SpanLabel`]'s label text.
///
/// `cell_name` resolves a `CellId` to its declared name (`None` if the cell has no recorded
/// name, e.g. an internal cell); a `Cell` site without a resolvable name falls back to
/// generic wording.
///
/// - Precondition: `index < e.sites().len()`.
pub(crate) fn site_label(
    e: &Error,
    index: usize,
    cell_name: &impl Fn(CellId) -> Option<String>,
) -> String {
    debug_assert!(index < e.sites().len(), "index out of range for e.sites()");
    let sites = e.sites();
    let Some(site) = sites.get(index) else {
        return "related to this error".to_string();
    };

    match e {
        Error::Cycle { .. } | Error::FilterCycle { .. } => match site {
            ErrorSite::Relationship(_) => "this relationship is part of the cycle".to_string(),
            ErrorSite::Cell(c) => match cell_name(*c) {
                Some(name) => format!("cell `{name}` is part of the cycle"),
                None => "this cell is part of the cycle".to_string(),
            },
            _ => generic_site_label(site, cell_name),
        },

        Error::Conflict { .. } => match site {
            ErrorSite::Relationship(_) => {
                "this relationship is part of an overconstrained group".to_string()
            }
            _ => generic_site_label(site, cell_name),
        },

        Error::MismatchedMethodCells { .. } => match (index, site) {
            (0, _) => "this method's cell set differs from the baseline".to_string(),
            (1, _) => "the baseline method".to_string(),
            (_, ErrorSite::Cell(c)) => match cell_name(*c) {
                Some(name) => format!("cell `{name}` appears in only one method"),
                None => "this cell appears in only one method".to_string(),
            },
            _ => generic_site_label(site, cell_name),
        },

        Error::DuplicateMethodOutputs { .. } => match (index, site) {
            (0, _) => "this method's output set".to_string(),
            (1, _) => "collides with this earlier method".to_string(),
            (_, ErrorSite::Cell(c)) => match cell_name(*c) {
                Some(name) => format!("shared output cell `{name}`"),
                None => "shared output cell".to_string(),
            },
            _ => generic_site_label(site, cell_name),
        },

        Error::InvalidCellKind { .. } => match (index, site) {
            (0, _) => "this method's output cell has the wrong kind".to_string(),
            (_, ErrorSite::Cell(c)) => match cell_name(*c) {
                Some(name) => format!("cell `{name}` has an incompatible kind"),
                None => "this cell has an incompatible kind".to_string(),
            },
            _ => generic_site_label(site, cell_name),
        },

        Error::InvalidConditional { .. } => match site {
            ErrorSite::Relationship(_) => {
                "this relationship makes the conditional invalid".to_string()
            }
            ErrorSite::Cell(c) => match cell_name(*c) {
                Some(name) => format!("cell `{name}` is upstream of the match subject"),
                None => "this cell is upstream of the match subject".to_string(),
            },
            _ => generic_site_label(site, cell_name),
        },

        _ => generic_site_label(site, cell_name),
    }
}

/// Falls back to a plain description of `site` alone, ignoring the enclosing `Error` variant —
/// used for `ErrorSite` kinds a variant's specific wording above doesn't otherwise cover.
fn generic_site_label(site: &ErrorSite, cell_name: &impl Fn(CellId) -> Option<String>) -> String {
    match site {
        ErrorSite::MethodIndex(_) => "this method".to_string(),
        ErrorSite::Method(..) => "this method".to_string(),
        ErrorSite::Relationship(_) => "this relationship".to_string(),
        ErrorSite::Cell(c) => match cell_name(*c) {
            Some(name) => format!("cell `{name}`"),
            None => "this cell".to_string(),
        },
        _ => "related to this error".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn site_label_describes_cycle_relationship_steps() {
        let e = adam_rs::Error::Cycle {
            sites: vec![
                adam_rs::ErrorSite::Relationship(adam_rs::RelationshipId::default()),
                adam_rs::ErrorSite::Cell(adam_rs::CellId::default()),
            ],
        };
        let name = |_id: adam_rs::CellId| Some("x".to_string());
        let l0 = site_label(&e, 0, &name);
        let l1 = site_label(&e, 1, &name);
        assert!(!l0.is_empty());
        assert!(l1.contains("x")); // the cell step names the cell
    }

    #[test]
    fn site_label_describes_mismatched_method_cells() {
        let e = adam_rs::Error::MismatchedMethodCells {
            sites: vec![
                adam_rs::ErrorSite::MethodIndex(1),
                adam_rs::ErrorSite::MethodIndex(0),
                adam_rs::ErrorSite::Cell(adam_rs::CellId::default()),
            ],
        };
        let name = |_id| Some("c".to_string());
        assert!(site_label(&e, 0, &name).to_lowercase().contains("differ"));
        assert!(site_label(&e, 1, &name).to_lowercase().contains("baseline"));
        assert!(site_label(&e, 2, &name).contains("c"));
    }

    #[test]
    fn site_label_describes_duplicate_method_outputs() {
        let e = adam_rs::Error::DuplicateMethodOutputs {
            sites: vec![
                adam_rs::ErrorSite::MethodIndex(1),
                adam_rs::ErrorSite::MethodIndex(0),
                adam_rs::ErrorSite::Cell(adam_rs::CellId::default()),
            ],
        };
        let name = |_id| Some("out".to_string());
        assert!(site_label(&e, 0, &name).to_lowercase().contains("output"));
        assert!(site_label(&e, 1, &name).to_lowercase().contains("earlier"));
        assert!(site_label(&e, 2, &name).contains("out"));
    }

    #[test]
    fn site_label_falls_back_to_generic_wording_for_unnamed_cells() {
        let e = adam_rs::Error::Cycle {
            sites: vec![adam_rs::ErrorSite::Cell(adam_rs::CellId::default())],
        };
        let name = |_id: adam_rs::CellId| None;
        assert!(!site_label(&e, 0, &name).is_empty());
    }
}
