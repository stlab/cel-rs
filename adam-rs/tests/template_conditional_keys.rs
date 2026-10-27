//! Verifies that conditional key factories run only during native construction.

use adam_rs::{MatchExpr, Sheet, TemplateBuilder};
use std::{cell::Cell, rc::Rc};

/// Defers keys and constructs them independently for each instance.
#[test]
fn conditional_keys_are_construction_local() {
    let calls = Rc::new(Cell::new(0));
    let mut builder = TemplateBuilder::new();
    let flag = builder.parameter::<bool>("flag").unwrap();
    let count = calls.clone();
    builder
        .conditional_factory::<bool, _, _>(
            &[flag],
            |ids| Ok(MatchExpr::cell(ids[0])),
            move || {
                count.set(count.get() + 1);
                Ok(vec![vec![true]])
            },
            vec![vec![]],
            vec![],
        )
        .unwrap();
    let template = builder.finish().unwrap();
    assert_eq!(calls.get(), 0);
    for _ in 0..2 {
        let mut sheet = Sheet::new();
        let flag = sheet.add_cell(true);
        template.instantiate(&mut sheet, &[("flag", flag)]).unwrap();
    }
    assert_eq!(calls.get(), 2);
}

/// Rejects invalid deferred key vectors at construction rather than executing them early.
#[test]
fn conditional_factory_validates_deferred_branch_keys() {
    for keys in [vec![], vec![vec![]], vec![vec![true], vec![false]]] {
        let mut builder = TemplateBuilder::new();
        let flag = builder.parameter::<bool>("flag").unwrap();
        builder
            .conditional_factory::<bool, _, _>(
                &[flag],
                |ids| Ok(MatchExpr::cell(ids[0])),
                move || Ok(keys.clone()),
                vec![vec![]],
                vec![],
            )
            .unwrap();
        let template = builder.finish().unwrap();
        let mut sheet = Sheet::new();
        let flag = sheet.add_cell(true);
        assert!(matches!(
            template.instantiate(&mut sheet, &[("flag", flag)]),
            Err(adam_rs::TemplateError::InvalidDefinition { .. })
        ));
    }
}

/// Keeps a key-construction failure in the native typed error chain.
#[test]
fn conditional_key_failure_preserves_cause() {
    let mut builder = TemplateBuilder::new();
    let flag = builder.parameter::<bool>("flag").unwrap();
    builder
        .conditional_factory::<bool, _, _>(
            &[flag],
            |ids| Ok(MatchExpr::cell(ids[0])),
            || anyhow::bail!("key construction failure"),
            vec![vec![]],
            vec![],
        )
        .unwrap();
    let template = builder.finish().unwrap();
    let mut sheet = Sheet::new();
    let flag = sheet.add_cell(true);
    let error = template
        .instantiate(&mut sheet, &[("flag", flag)])
        .unwrap_err();
    assert!(matches!(
        error,
        adam_rs::TemplateError::ConditionalFactory { .. }
    ));
    assert!(std::error::Error::source(&error).is_some());
}
