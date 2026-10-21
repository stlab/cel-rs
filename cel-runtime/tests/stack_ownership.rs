//! Verifies live-stack ownership across raw instruction execution boundaries.

use cel_runtime::{RawSegment, RawStack};
use std::cell::Cell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;

/// Records release of one live evaluation value.
struct Tracked(Rc<Cell<usize>>);

impl Drop for Tracked {
    /// Increments the shared destruction count.
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}

/// A raw execution error destroys the values produced before that error.
#[test]
fn raw_execution_error_releases_live_stack_values() {
    let drops = Rc::new(Cell::new(0));
    let observed = drops.clone();
    let mut segment = RawSegment::new();
    segment.push_op0(move || Tracked(observed.clone()));
    segment.raw0_(|_| anyhow::bail!("operation failed"));
    let mut stack = RawStack::with_base_alignment(align_of::<Tracked>());

    // SAFETY: The segment produces its own inputs on an empty aligned stack.
    assert!(unsafe { segment.call0_stack(&mut stack) }.is_err());
    assert_eq!(stack.len(), 0);
    assert_eq!(drops.get(), 1);
}

/// A host panic destroys remaining stack values before returning to the caller.
#[test]
fn raw_execution_panic_releases_live_stack_values() {
    let drops = Rc::new(Cell::new(0));
    let observed = drops.clone();
    let mut segment = RawSegment::new();
    segment.push_op0(move || Tracked(observed.clone()));
    segment.raw0_(|_| panic!("host panic"));
    let mut stack = RawStack::with_base_alignment(align_of::<Tracked>());

    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: The segment produces its own inputs on an empty aligned stack.
        unsafe { segment.call0_stack(&mut stack) }
    }));
    assert!(result.is_err());
    assert_eq!(stack.len(), 0);
    assert_eq!(drops.get(), 1);
}
