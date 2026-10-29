//! Save/load [`Document`]s as JSON — `ez-adam`'s native document format.
//! `.adm2` export ([`crate::codegen::generate_adm2`]) is a separate,
//! one-way operation; it is never read back in.

use crate::model::document::{CURRENT_FORMAT_VERSION, Document};

/// Serializes `doc` to pretty-printed JSON.
///
/// # Errors
///
/// Returns an error if `doc` holds a non-finite `f64` (`NaN`/`±inf`) — in a
/// [`Point`](crate::model::geometry::Point) coordinate or an `f64` clamp
/// bound — violates the document's reference/order invariants, or declares an
/// unsupported format version.
///
/// - Complexity: O(n) in the total size of `doc`.
///
/// # Examples
///
/// ```
/// let doc = ez_adam::model::document::Document::new("example");
/// assert!(ez_adam::persistence::to_json(&doc)?.contains("example"));
/// # Ok::<(), serde_json::Error>(())
/// ```
pub fn to_json(doc: &Document) -> Result<String, serde_json::Error> {
    validate_document(doc)?;
    serde_json::to_string_pretty(doc)
}

/// Deserializes a `Document` from JSON text produced by [`to_json`].
///
/// # Errors
///
/// Returns an error if `text` is not valid JSON, does not match
/// [`Document`]'s current shape, or declares a `format_version` other than
/// [`CURRENT_FORMAT_VERSION`] — no migration path exists yet for other
/// versions, or violates the document's reference/order/numeric invariants.
///
/// - Complexity: O(n) in the length of `text`.
///
/// # Examples
///
/// ```
/// let doc = ez_adam::model::document::Document::new("example");
/// let text = ez_adam::persistence::to_json(&doc)?;
/// assert_eq!(ez_adam::persistence::from_json(&text)?, doc);
/// # Ok::<(), serde_json::Error>(())
/// ```
pub fn from_json(text: &str) -> Result<Document, serde_json::Error> {
    use serde::de::Error as _;
    let doc: Document = serde_json::from_str(text)?;
    if doc.format_version != CURRENT_FORMAT_VERSION {
        return Err(serde_json::Error::custom(format!(
            "unsupported document format version {}: only version {CURRENT_FORMAT_VERSION} is supported",
            doc.format_version
        )));
    }
    validate_document(&doc)?;
    Ok(doc)
}

/// Validates references, declaration-order permutations, unique memberships, and finite numbers.
///
/// Editing drafts may contain empty formulas or invalid names; export validates those.
///
/// # Errors
///
/// Returns an error for an unsupported format version, dangling/duplicate references,
/// incomplete orders, empty cell selectors, invalid branch arity, reversed bounds,
/// or non-finite coordinates/bounds.
///
/// - Complexity: expected O(n) time and O(n) space in allocated document slots
///   and reference/order entries.
pub(crate) fn validate_document(doc: &Document) -> Result<(), serde_json::Error> {
    use crate::model::{cell::CellType, conditional_group::ConditionExpr};
    use serde::de::Error as _;
    use std::collections::HashSet;

    /// Tests whether keys enumerate every map entry once.
    ///
    /// - Complexity: expected O(keys.len()) time and space.
    fn is_order<K: slotmap::Key, V>(keys: &[K], map: &slotmap::SlotMap<K, V>) -> bool {
        keys.len() == map.len()
            && keys.iter().all(|key| map.contains_key(*key))
            && keys.iter().copied().collect::<HashSet<_>>().len() == keys.len()
    }
    /// Tests whether a reference list contains live keys without duplicates.
    ///
    /// - Complexity: expected O(keys.len()) time and space.
    fn live_unique<K: slotmap::Key, V>(keys: &[K], map: &slotmap::SlotMap<K, V>) -> bool {
        keys.iter().all(|key| map.contains_key(*key))
            && keys.iter().copied().collect::<HashSet<_>>().len() == keys.len()
    }
    /// Tests whether both coordinates are finite.
    fn finite(point: crate::model::geometry::Point) -> bool {
        point.x.is_finite() && point.y.is_finite()
    }
    /// Tests whether optional endpoints are ordered.
    fn ordered<T: PartialOrd>(min: Option<T>, max: Option<T>) -> bool {
        match (min, max) {
            (Some(min), Some(max)) => min <= max,
            _ => true,
        }
    }
    let valid = doc.format_version == CURRENT_FORMAT_VERSION
        && is_order(&doc.cell_order, &doc.cells)
        && is_order(&doc.relationship_group_order, &doc.relationship_groups)
        && is_order(&doc.conditional_group_order, &doc.conditional_groups)
        && doc.cells.values().all(|cell| match cell.ty {
            CellType::F64 { clamp } => {
                clamp.min.is_none_or(f64::is_finite)
                    && clamp.max.is_none_or(f64::is_finite)
                    && ordered(clamp.min, clamp.max)
            }
            CellType::I64 { clamp } => ordered(clamp.min, clamp.max),
            _ => true,
        })
        && doc
            .cell_nodes
            .values()
            .all(|node| finite(node.position) && doc.cells.contains_key(node.cell))
        && doc.relationship_groups.values().all(|group| {
            finite(group.position)
                && group
                    .members
                    .iter()
                    .all(|(node, _)| doc.cell_nodes.contains_key(*node))
                && group
                    .members
                    .iter()
                    .map(|(node, _)| *node)
                    .collect::<HashSet<_>>()
                    .len()
                    == group.members.len()
        })
        && doc.conditional_groups.values().all(|group| {
            let arity = match &group.condition {
                ConditionExpr::Cells(cells) => cells.len(),
                ConditionExpr::Formula { .. } => 1,
            };
            finite(group.position)
                && arity > 0
                && live_unique(group.condition.referenced_cells(), &doc.cells)
                && live_unique(&group.default, &doc.relationship_groups)
                && group.branches.iter().all(|branch| {
                    branch.values.len() == arity
                        && live_unique(&branch.enabled_groups, &doc.relationship_groups)
                })
        });
    if valid {
        Ok(())
    } else {
        Err(serde_json::Error::custom(
            "invalid document version, references, order, branch arity, or numeric data",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Empty cell selectors fail load, save, and export instead of reaching synthesis preconditions.
    #[test]
    fn empty_cell_conditions_are_rejected_at_document_boundaries() {
        use crate::model::conditional_group::{ConditionExpr, ConditionalGroup};
        let mut doc = Document::new("empty_condition");
        let id = doc.conditional_groups.insert(ConditionalGroup {
            display_name: "empty".into(),
            position: crate::model::geometry::Point::new(0.0, 0.0),
            condition: ConditionExpr::Cells(vec![]),
            branches: vec![],
            default: vec![],
        });
        doc.conditional_group_order.push(id);
        let external = serde_json::to_string(&doc).unwrap();
        assert!(from_json(&external).is_err());
        assert!(to_json(&doc).is_err());
        assert!(crate::codegen::generate_adm2(&doc).is_err());
    }
    use crate::model::cell::CellType;
    use crate::model::geometry::Point;
    use crate::ops::cells::{add_cell, add_cell_node};
    use crate::ops::relationships::{create_relationship, set_member_formula};

    /// Both shipped JSON documents round-trip and export their declarations into type-checked Adam.
    #[test]
    fn authored_json_examples_preserve_declarations_through_export() {
        for (text, expected_names) in [
            (
                include_str!("../examples/playground.json"),
                &[
                    "width_pixels",
                    "height_pixels",
                    "aspect_ratio",
                    "constrain_proportions",
                    "label",
                ][..],
            ),
            (
                include_str!("../examples/resize.json"),
                &[
                    "width_pixels",
                    "height_pixels",
                    "constrain_proportions",
                    "aspect_ratio",
                ][..],
            ),
        ] {
            let doc = from_json(text).unwrap();
            assert_eq!(
                doc.cells_in_order()
                    .map(|(_, cell)| cell.name.as_str())
                    .collect::<Vec<_>>(),
                expected_names
            );
            assert_eq!(from_json(&to_json(&doc).unwrap()).unwrap(), doc);
            let emitted = crate::codegen::generate_adm2(&doc).unwrap();
            let mut lookup = cel_parser::OpLookup::new();
            cel_std::install(&mut lookup);
            let parsed = adam_lang::AdamParser::new(adam_lang::TypeRegistry::new(), lookup)
                .parse_str(&emitted)
                .unwrap();
            assert_eq!(
                parsed
                    .cell_names
                    .keys()
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
                expected_names
            );
        }
    }

    /// All conditional references, order permutations, and branch arities are validated on both boundaries.
    #[test]
    fn rejects_invalid_conditional_structure() {
        use crate::model::conditional_group::ConditionExpr;
        use crate::ops::conditionals::add_conditional_from_bool_cells;
        use slotmap::Key;
        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "a", CellType::Bool);
        let b = add_cell(&mut doc, "b", CellType::i64());
        let na = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let nb = add_cell_node(&mut doc, b, Point::new(1.0, 0.0));
        let g = create_relationship(&mut doc, na, nb, Point::new(2.0, 0.0));
        let c = add_conditional_from_bool_cells(&mut doc, vec![a], g, Point::new(3.0, 0.0));
        for case in 0..9 {
            let mut invalid = doc.clone();
            let conditional = &mut invalid.conditional_groups[c];
            match case {
                0 => invalid.conditional_group_order.push(c),
                1 => invalid.conditional_group_order.clear(),
                2 => conditional.condition = ConditionExpr::Cells(vec![a, a]),
                3 => {
                    conditional.condition =
                        ConditionExpr::Cells(vec![crate::model::cell::CellId::null()])
                }
                4 => conditional.branches[0].values.clear(),
                5 => conditional
                    .default
                    .push(crate::model::relationship_group::RelationshipGroupId::null()),
                6 => conditional.branches[0]
                    .enabled_groups
                    .push(crate::model::relationship_group::RelationshipGroupId::null()),
                7 => conditional.default = vec![g, g],
                8 => conditional.branches[1].enabled_groups.push(g),
                _ => unreachable!(),
            }
            assert!(
                to_json(&invalid).is_err(),
                "save accepted invalid case {case}"
            );
            assert!(
                from_json(&serde_json::to_string(&invalid).unwrap()).is_err(),
                "load accepted invalid case {case}"
            );
        }
    }

    /// Every node kind rejects non-finite coordinates and numeric ranges reject reversed endpoints.
    #[test]
    fn rejects_nonfinite_group_coordinates_and_reversed_bounds() {
        use crate::ops::conditionals::add_conditional_with_formula;
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            for kind in 0..2 {
                for position in [Point::new(value, 0.0), Point::new(0.0, value)] {
                    let mut doc = Document::new("demo");
                    let a = add_cell(&mut doc, "a", CellType::i64());
                    let b = add_cell(&mut doc, "b", CellType::i64());
                    let na = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
                    let nb = add_cell_node(&mut doc, b, Point::new(1.0, 0.0));
                    if kind == 0 {
                        let _ = create_relationship(&mut doc, na, nb, position);
                    } else {
                        let _ = add_conditional_with_formula(&mut doc, vec![a], "a > 0", position);
                    }
                    assert!(to_json(&doc).is_err());
                }
            }
        }
        for ty in [
            CellType::I64 {
                clamp: crate::model::cell::ClampRange {
                    min: Some(2),
                    max: Some(1),
                },
            },
            CellType::F64 {
                clamp: crate::model::cell::ClampRange {
                    min: Some(2.0),
                    max: Some(1.0),
                },
            },
        ] {
            let mut doc = Document::new("demo");
            let _ = add_cell(&mut doc, "value", ty);
            assert!(to_json(&doc).is_err());
            assert!(from_json(&serde_json::to_string(&doc).unwrap()).is_err());
        }
    }

    /// Rejects invalid external references and incomplete declaration orders.
    #[test]
    fn rejects_invalid_document_structure() {
        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "a", CellType::i64());
        let b = add_cell(&mut doc, "b", CellType::i64());
        let an = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let bn = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let g = create_relationship(&mut doc, an, bn, Point::new(5.0, 0.0));
        let mut invalid = doc.clone();
        invalid.cells.remove(a);
        assert!(from_json(&serde_json::to_string(&invalid).unwrap()).is_err());
        invalid = doc.clone();
        invalid.cell_order.push(a);
        assert!(from_json(&serde_json::to_string(&invalid).unwrap()).is_err());
        invalid = doc.clone();
        invalid.cell_order.clear();
        assert!(from_json(&serde_json::to_string(&invalid).unwrap()).is_err());
        invalid = doc.clone();
        invalid.cell_nodes.remove(an);
        assert!(from_json(&serde_json::to_string(&invalid).unwrap()).is_err());
        invalid = doc.clone();
        invalid.relationship_group_order.push(g);
        assert!(from_json(&serde_json::to_string(&invalid).unwrap()).is_err());
        invalid = doc.clone();
        invalid.relationship_groups[g]
            .members
            .push((an, String::new()));
        assert!(from_json(&serde_json::to_string(&invalid).unwrap()).is_err());
    }

    /// Rejects non-finite numeric data before JSON can replace it with null.
    #[test]
    fn rejects_nonfinite_values_on_save() {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            for coordinate in [true, false] {
                let mut doc = Document::new("demo");
                let a = add_cell(&mut doc, "a", CellType::f64());
                let position = if coordinate {
                    Point::new(value, 0.0)
                } else {
                    Point::new(0.0, value)
                };
                let _ = add_cell_node(&mut doc, a, position);
                assert!(to_json(&doc).is_err());
            }

            for minimum in [true, false] {
                let mut doc = Document::new("demo");
                let a = add_cell(&mut doc, "a", CellType::f64());
                let CellType::F64 { clamp } = &mut doc.cells[a].ty else {
                    unreachable!()
                };
                if minimum {
                    clamp.min = Some(value);
                } else {
                    clamp.max = Some(value);
                }
                assert!(to_json(&doc).is_err());
            }
        }
    }

    /// Accepts scalar formula conditions independently of their dependency count.
    #[test]
    fn scalar_formula_dependency_count_is_not_branch_arity() {
        use crate::model::conditional_group::CellValueLiteral;
        use crate::ops::conditionals::{add_branch, add_conditional_with_formula};
        for count in [0, 2] {
            let mut doc = Document::new("demo");
            let a = add_cell(&mut doc, "a", CellType::i64());
            let b = add_cell(&mut doc, "b", CellType::i64());
            let cells = if count == 0 { vec![] } else { vec![a, b] };
            let expr = if count == 0 { "true" } else { "a > b" };
            let conditional =
                add_conditional_with_formula(&mut doc, cells, expr, Point::new(0.0, 0.0));
            add_branch(&mut doc, conditional, vec![CellValueLiteral::Bool(true)]);
            assert!(crate::codegen::generate_adm2(&doc).is_ok());
            let json = to_json(&doc).unwrap();
            assert_eq!(doc, from_json(&json).unwrap());
            doc.conditional_groups[conditional].branches[0]
                .values
                .push(CellValueLiteral::Bool(false));
            assert!(from_json(&serde_json::to_string(&doc).unwrap()).is_err());
        }
    }

    #[test]
    /// Serialization preserves ordered cells, placements, and relationships.
    fn round_trips_a_document_with_cells_and_a_relationship() {
        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "width_pixels", CellType::i64());
        let b = add_cell(&mut doc, "height_pixels", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));
        set_member_formula(&mut doc, group, a_node, "height_pixels * 2");

        let json = to_json(&doc).unwrap();
        let back = from_json(&json).unwrap();
        assert_eq!(doc, back);
    }

    #[test]
    /// Malformed JSON returns a decoding error.
    fn from_json_rejects_malformed_json() {
        assert!(from_json("not json").is_err());
    }

    #[test]
    /// Externally authored unsupported versions return the version diagnostic.
    fn from_json_rejects_an_unsupported_format_version() {
        let mut doc = Document::new("demo");
        doc.format_version = CURRENT_FORMAT_VERSION + 1;
        assert!(to_json(&doc).is_err());
        let json = serde_json::to_string(&doc).unwrap();
        assert!(from_json(&json).is_err());
    }

    #[test]
    /// Serialization preserves text values and scalar formula conditionals.
    fn round_trips_a_document_with_a_formula_conditional_and_a_text_cell() {
        use crate::model::conditional_group::CellValueLiteral;
        use crate::ops::cells::set_restrict;
        use crate::ops::conditionals::{
            add_branch, add_conditional_with_formula, toggle_enabled_group,
        };

        let mut doc = Document::new("demo");
        let label = add_cell(&mut doc, "label", CellType::Text);
        set_restrict(&mut doc, label, Some("_ != \"\"".to_string()));
        let aspect = add_cell(&mut doc, "aspect_ratio", CellType::f64());
        let a = add_cell(&mut doc, "width_pixels", CellType::i64());
        let b = add_cell(&mut doc, "height_pixels", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));

        let cond = add_conditional_with_formula(
            &mut doc,
            vec![aspect],
            "aspect_ratio > 2.0",
            Point::new(0.0, 20.0),
        );
        add_branch(
            &mut doc,
            cond,
            vec![CellValueLiteral::Text("wide".to_string())],
        );
        toggle_enabled_group(&mut doc, cond, 0, group);

        let json = to_json(&doc).unwrap();
        let back = from_json(&json).unwrap();
        assert_eq!(doc, back);
    }
}
