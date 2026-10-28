//! Verifies typed array ownership through its public contracts.

use cel_runtime::Array;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Records destruction without requiring element cloning, comparison, or formatting.
struct Tracked {
    id: usize,
    drops: Rc<RefCell<Vec<usize>>>,
}

impl Drop for Tracked {
    /// Appends this element's identifier to the destruction log.
    fn drop(&mut self) {
        self.drops.borrow_mut().push(self.id);
    }
}

/// Creates move-only elements sharing a destruction log in O(count) time and space.
fn tracked(count: usize, drops: &Rc<RefCell<Vec<usize>>>) -> Vec<Tracked> {
    (0..count)
        .map(|id| Tracked {
            id,
            drops: Rc::clone(drops),
        })
        .collect()
}

/// Checks that canonical nested arrays preserve outer and inner allocations.
#[test]
fn nested_array_keeps_outer_allocation() {
    let inner = vec![String::from("leaf")];
    let inner_pointer = inner.as_ptr();
    let mut outer = Vec::with_capacity(6);
    outer.push(Array::from_vec(inner));
    let pointer = outer.as_ptr();
    let capacity = outer.capacity();
    let array: Array<Array<String>> = Array::from_vec(outer);
    assert_eq!(array.as_slice().as_ptr(), pointer);
    assert_eq!(array.as_slice()[0].as_slice().as_ptr(), inner_pointer);
    assert_eq!(array.as_slice()[0].as_slice(), &["leaf"]);
    let outer = array.into_vec();
    assert_eq!(outer.as_ptr(), pointer);
    assert_eq!(outer.capacity(), capacity);
    let inner = outer.into_iter().next().unwrap().into_vec();
    assert_eq!(inner.as_ptr(), inner_pointer);
}

/// Checks empty, reserved-empty, and zero-sized storage without pointer assumptions.
#[test]
fn empty_and_zst_arrays_round_trip() {
    for values in [Vec::<i32>::new(), Vec::with_capacity(8)] {
        let capacity = values.capacity();
        let array = Array::from_vec(values);
        assert!(array.is_empty());
        assert_eq!(array.len(), 0);
        assert!(array.as_slice().is_empty());
        assert_eq!(array.capacity(), capacity);
        let values = array.into_vec();
        assert!(values.is_empty());
        assert_eq!(values.capacity(), capacity);
    }
    for count in [0, 3] {
        let values = vec![(); count];
        let capacity = values.capacity();
        let array = Array::from_vec(values);
        assert_eq!(array.as_slice(), vec![(); count]);
        assert_eq!(array.len(), count);
        assert_eq!(array.capacity(), capacity);
        let values = array.into_vec();
        assert_eq!(values.len(), count);
        assert_eq!(values.capacity(), capacity);
    }
}

/// Wraps a move-only value with greater-than-default alignment.
#[repr(align(128))]
struct Aligned(Tracked);

/// Checks alignment and ownership without any optional element traits.
#[test]
fn overaligned_move_only_elements_round_trip() {
    let drops = Rc::new(RefCell::new(Vec::new()));
    let values: Vec<_> = tracked(3, &drops).into_iter().map(Aligned).collect();
    let pointer = values.as_ptr();
    let capacity = values.capacity();
    let array = Array::from_vec(values);
    assert_eq!(array.as_slice().as_ptr(), pointer);
    assert_eq!((pointer as usize) % 128, 0);
    assert_eq!(array.as_slice()[1].0.id, 1);
    assert!(drops.borrow().is_empty());
    let values = array.into_vec();
    assert_eq!(values.as_ptr(), pointer);
    assert_eq!(values.capacity(), capacity);
    assert!(drops.borrow().is_empty());
    drop(values);
    let mut dropped = drops.borrow_mut();
    dropped.sort_unstable();
    assert_eq!(*dropped, [0, 1, 2]);
}

/// Checks exactly-once reverse destruction under typed ownership.
#[test]
fn typed_array_drop_runs_once_in_reverse_order() {
    let drops = Rc::new(RefCell::new(Vec::new()));
    let array = Array::from_vec(tracked(4, &drops));
    assert!(drops.borrow().is_empty());
    drop(array);
    assert_eq!(*drops.borrow(), [3, 2, 1, 0]);
}

/// Counts zero-sized element destructors independently of storage addresses.
struct ZstDrop;

static ZST_DROPS: AtomicUsize = AtomicUsize::new(0);

impl Drop for ZstDrop {
    /// Counts one destruction.
    fn drop(&mut self) {
        ZST_DROPS.fetch_add(1, Ordering::Relaxed);
    }
}

/// Checks zero-sized destructors run exactly once through both ownership paths.
#[test]
fn zst_destructors_run_once() {
    ZST_DROPS.store(0, Ordering::Relaxed);
    drop(Array::from_vec((0..5).map(|_| ZstDrop).collect()));
    assert_eq!(ZST_DROPS.load(Ordering::Relaxed), 5);
    let values = Array::from_vec((0..3).map(|_| ZstDrop).collect()).into_vec();
    assert_eq!(ZST_DROPS.load(Ordering::Relaxed), 5);
    drop(values);
    assert_eq!(ZST_DROPS.load(Ordering::Relaxed), 8);
}

/// Checks outer reverse destruction composes with inner reverse destruction.
#[test]
fn nested_array_drops_in_reverse_order() {
    let drops = Rc::new(RefCell::new(Vec::new()));
    let mut values = tracked(4, &drops);
    let tail = values.split_off(2);
    drop(Array::from_vec(vec![
        Array::from_vec(values),
        Array::from_vec(tail),
    ]));
    assert_eq!(*drops.borrow(), [3, 2, 1, 0]);
}

/// Requires the same positive thread-safety bounds as a vector.
#[test]
fn thread_safety_follows_elements() {
    /// Accepts only types transferable and shareable between threads.
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Array<String>>();
    assert_send_sync::<Array<Array<String>>>();
}

/// Checks optional value traits delegate to element behavior.
#[test]
fn optional_traits_follow_elements() {
    let array = Array::from_vec(vec![String::from("leaf")]);
    let cloned = array.clone();
    assert_eq!(array, cloned);
    assert_ne!(array.as_slice().as_ptr(), cloned.as_slice().as_ptr());
    assert_ne!(array, Array::from_vec(vec![String::from("other")]));
    assert_eq!(format!("{array:?}"), "Array([\"leaf\"])");
}

/// Records one destructor panic while allowing the remaining cleanup to complete.
struct PanicDrop {
    value: Tracked,
    panic_id: usize,
    panicked: Rc<Cell<bool>>,
}

impl Drop for PanicDrop {
    /// Panics once at the selected element.
    fn drop(&mut self) {
        if self.value.id == self.panic_id && !self.panicked.replace(true) {
            panic!("element destructor");
        }
    }
}

/// Checks remaining elements still drop in reverse order if one destructor unwinds.
#[test]
fn destructor_panic_preserves_reverse_cleanup() {
    let drops = Rc::new(RefCell::new(Vec::new()));
    let panicked = Rc::new(Cell::new(false));
    let array = Array::from_vec(
        tracked(4, &drops)
            .into_iter()
            .map(|value| PanicDrop {
                value,
                panic_id: 2,
                panicked: Rc::clone(&panicked),
            })
            .collect(),
    );
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(array)));
    assert!(result.is_err());
    assert_eq!(*drops.borrow(), [3, 2, 1, 0]);
}
