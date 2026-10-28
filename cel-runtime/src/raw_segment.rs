//! Registers owned captures and executes an explicitly typed raw frame.
//!
//! Registration records no dynamic operand types. The caller establishes the
//! complete types and padding before entering the unsafe execution boundary.
//!
//! ```
//! use cel_runtime::RawSegment;
//! let mut body = RawSegment::new();
//! body.push_op0(|| String::from("left"));
//! body.push_op1(|value: String| value + "/right", false);
//! assert_eq!(unsafe { body.call0::<String>() }?, "left/right");
//!
//! let mut checked = RawSegment::new();
//! checked.raw0(|_| Ok(3f64));
//! checked.raw1(|_, value: f64| Ok(value.abs()), false);
//! checked.raw0(|_| Ok(2f64));
//! checked.raw2(|_, first: f64, second: f64| Ok(first - second), false, false);
//! checked.raw0(|_| Ok(4f64));
//! checked.raw0(|_| Ok(7f64));
//! checked.raw3(|_, first: f64, second: f64, third: f64| Ok((first, second, third)),
//!     false, false, false);
//! assert_eq!(unsafe { checked.call0::<(f64, f64, f64)>() }?, (1.0, 4.0, 7.0));
//! # Ok::<(), anyhow::Error>(())
//! ```

use crate::raw_sequence::RawSequence;
use crate::raw_stack::RawStack;
use anyhow::Result;
use std::cmp::max;

/// Executes one instruction and returns the next instruction-state position.
type Operation = fn(&RawSequence, usize, &mut RawStack) -> Result<usize>;

/// Advances the state cursor before destroying one stored instruction capture.
type StorageDrop = fn(&mut RawSequence, &mut usize);

/// Owns the remaining instruction-state destruction during unwinding.
struct StorageCleanup<'a> {
    /// Exclusively borrows the remaining captures' backing allocation.
    storage: &'a mut RawSequence,
    /// Visits the remaining capture destructors once each.
    droppers: std::slice::Iter<'a, StorageDrop>,
    /// Tracks the next capture's byte position before invoking its destructor.
    position: usize,
}

impl StorageCleanup<'_> {
    /// Destroys the remaining captures in insertion order.
    ///
    /// - Complexity: O(n) plus destruction costs for n remaining captures.
    fn drop_remaining(&mut self) {
        // Consume each dropper before invoking a potentially panicking destructor.
        for destroy in self.droppers.by_ref() {
            destroy(self.storage, &mut self.position);
        }
    }
}

impl Drop for StorageCleanup<'_> {
    /// Resumes capture destruction if a previous destructor unwinds.
    ///
    /// - Complexity: O(n) plus destruction costs for n remaining captures.
    fn drop(&mut self) {
        self.drop_remaining();
    }
}

/// A segment represents a sequence of operations that can be executed.
///
/// Each operation is stored along with its data in the segment's storage,
/// and can manipulate values on a stack during execution.
/// Destruction releases instruction captures in insertion order, including
/// remaining captures when one destructor panics.
/// Captures need not be `Send` or `Sync`, so the erased program remains thread-confined.
///
/// # Examples
/// See the [module tutorial](self).
pub struct RawSegment {
    /// Dispatches each captured operation in registration order.
    ops: Vec<Operation>,
    /// Owns the captures' aligned byte representations.
    storage: RawSequence,
    /// Destroys each corresponding capture in registration order.
    dropper: Vec<StorageDrop>,
    /// Bounds the alignment of every value emitted by a registered operation.
    base_alignment: usize,
    /// Retains the optional caller-owned byte origin and owned input count.
    frame: Option<(usize, usize)>,
    /// Prevents erased non-`Send`/non-`Sync` captures from crossing thread boundaries.
    thread_bound: std::marker::PhantomData<std::rc::Rc<()>>,
}

impl Default for RawSegment {
    /// Creates an empty segment.
    fn default() -> Self {
        Self::new()
    }
}

impl RawSegment {
    /// Creates a new empty segment.
    ///
    /// # Examples
    /// See [`RawSegment`].
    #[must_use]
    pub fn new() -> Self {
        RawSegment {
            ops: Vec::new(),
            storage: RawSequence::new(),
            dropper: Vec::new(),
            base_alignment: 1,
            frame: None,
            thread_bound: std::marker::PhantomData,
        }
    }

    /// Returns the maximum alignment required by any value pushed onto the stack while executing this segment.
    pub(crate) fn base_alignment(&self) -> usize {
        self.base_alignment
    }

    /// Updates the stack base alignment to be at least `alignment`.
    pub(crate) fn update_base_alignment(&mut self, alignment: usize) {
        debug_assert!(alignment.is_power_of_two());
        self.base_alignment = max(self.base_alignment, alignment);
    }

    /// Sets the caller-owned byte prefix and number of owned input values.
    pub(crate) fn set_frame(&mut self, origin: usize, owned_inputs: usize) {
        self.frame = Some((origin, owned_inputs));
    }

    /// Binds the initial frame unless an enclosing segment already supplied it.
    pub(crate) fn bind_frame(&mut self, origin: usize, owned_inputs: usize) {
        self.frame.get_or_insert((origin, owned_inputs));
    }

    /// Returns the byte length occupied by instruction closure state.
    #[cfg(test)]
    pub(crate) fn storage_size(&self) -> usize {
        self.storage.len()
    }

    /// Pushes a value into the segment's storage and registers its dropper.
    ///
    /// - Complexity: Amortized O(size of `T` + alignment of `T`) capture storage;
    ///   raising alignment also relocates all stored capture bytes.
    fn push_storage<T>(&mut self, value: T)
    where
        T: 'static,
    {
        self.dropper.reserve(1);
        self.storage.push(value);
        self.dropper
            .push(|storage, p| unsafe { storage.drop_next::<T>(p) });
    }

    /// Pushes a closure that manipulates the stack directly and returns `()`.
    ///
    /// Operation errors propagate at execution after frame cleanup.
    /// - Complexity: The capture insertion cost of [`Self::push_op0`].
    ///
    /// # Examples
    /// ```
    /// use cel_runtime::RawSegment;
    /// let mut body = RawSegment::new();
    /// body.raw0_(|stack| { stack.push(7u8); Ok(()) });
    /// assert_eq!(unsafe { body.call0::<u8>() }?, 7);
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn raw0_<F>(&mut self, op: F)
    where
        F: Fn(&mut RawStack) -> Result<()> + 'static,
    {
        self.push_storage(op);
        self.ops.push(|storage, p, stack| {
            let (f, r) = unsafe { storage.next::<F>(p) };
            f(stack)?;
            Ok(r)
        });
    }

    /// Pushes a fallible closure that manipulates the remaining stack and returns `R`.
    ///
    /// Operation errors propagate at execution after frame cleanup.
    /// - Complexity: The capture insertion cost of [`Self::push_op0`].
    ///
    /// # Examples
    /// See [`RawSegment`].
    pub fn raw0<R, F>(&mut self, op: F)
    where
        F: Fn(&mut RawStack) -> Result<R> + 'static,
        R: 'static,
    {
        self.push_storage(op);
        self.ops.push(|storage, p, stack| {
            let (f, r) = unsafe { storage.next::<F>(p) };
            let result = f(stack)?;
            stack.push(result);
            Ok(r)
        });
        self.base_alignment = max(self.base_alignment, align_of::<R>());
    }

    /// Pushes a nullary operation that returns a value of type `R`.
    ///
    /// - Complexity: Amortized O(size of `F` + alignment of `F`) capture storage;
    ///   raising capture alignment additionally moves all stored capture bytes.
    ///
    /// # Examples
    /// See [`RawSegment`].
    pub fn push_op0<R, F>(&mut self, op: F)
    where
        F: Fn() -> R + 'static,
        R: 'static,
    {
        self.push_storage(op);
        self.ops.push(|storage, p, stack| {
            let (f, r) = unsafe { storage.next::<F>(p) };
            stack.push(f());
            Ok(r)
        });
        self.base_alignment = max(self.base_alignment, align_of::<R>());
    }

    /// Pushes the op-dispatch closure for a unary infallible operation with compile-time padding.
    fn push_op1_<const PADDING0: bool, T, R, F>(&mut self)
    where
        F: Fn(T) -> R + 'static,
        T: 'static,
        R: 'static,
    {
        self.ops.push(|storage, p, stack| {
            let (f, r) = unsafe { storage.next::<F>(p) };
            let x: T = unsafe { stack.pop(PADDING0) };
            stack.push(f(x));
            Ok(r)
        });
    }

    /// Pushes the op-dispatch closure for a unary fallible operation with compile-time padding.
    fn push_op1r_<const PADDING0: bool, T, R, F>(&mut self)
    where
        F: Fn(&mut RawStack, T) -> Result<R> + 'static,
        T: 'static,
        R: 'static,
    {
        self.ops.push(|storage, p, stack| {
            let (f, r) = unsafe { storage.next::<F>(p) };
            let x: T = unsafe { stack.pop(PADDING0) };
            let result = f(stack, x)?;
            stack.push(result);
            Ok(r)
        });
    }

    /// Pushes a unary operation that takes one argument of type T and returns a value of type R.
    ///
    /// `padding0` describes the consumed operand's preceding alignment padding.
    /// - Complexity: The capture insertion cost of [`Self::push_op0`].
    ///
    /// # Examples
    /// See [`RawSegment`].
    pub fn push_op1<T, R, F>(&mut self, op: F, padding0: bool)
    where
        F: Fn(T) -> R + 'static,
        T: 'static,
        R: 'static,
    {
        self.push_storage(op);
        if padding0 {
            self.push_op1_::<true, T, R, F>();
        } else {
            self.push_op1_::<false, T, R, F>();
        }
        self.base_alignment = max(self.base_alignment, align_of::<R>());
    }

    /// Pushes a fallible unary operation that can manipulate the remaining stack.
    ///
    /// `padding0` describes the consumed operand's preceding alignment padding.
    /// Operation errors propagate at execution after frame cleanup.
    /// - Complexity: The capture insertion cost of [`Self::push_op0`].
    ///
    /// # Examples
    /// See [`RawSegment`].
    pub fn raw1<T, R, F>(&mut self, op: F, padding0: bool)
    where
        F: Fn(&mut RawStack, T) -> Result<R> + 'static,
        T: 'static,
        R: 'static,
    {
        self.push_storage(op);
        if padding0 {
            self.push_op1r_::<true, T, R, F>();
        } else {
            self.push_op1r_::<false, T, R, F>();
        }
        self.base_alignment = max(self.base_alignment, align_of::<R>());
    }

    /// Pushes the op-dispatch closure for a unary drop operation with compile-time padding.
    fn drop1_<const PADDING0: bool, T, F>(&mut self)
    where
        F: Fn(T) + 'static,
        T: 'static,
    {
        self.ops.push(|storage, p, stack| {
            let (f, r) = unsafe { storage.next::<F>(p) };
            let x: T = unsafe { stack.pop(PADDING0) };
            f(x); // drop the result
            Ok(r)
        });
    }

    /// Pushes an operation that consumes one stack value without producing a result.
    ///
    /// `padding0` describes the consumed operand's preceding alignment padding.
    /// - Complexity: The capture insertion cost of [`Self::push_op0`].
    ///
    /// # Examples
    /// ```
    /// use cel_runtime::RawSegment;
    /// let mut body = RawSegment::new();
    /// body.push_op0(|| String::from("temporary"));
    /// body.drop1(|value: String| drop(value), false);
    /// body.push_op0(|| true);
    /// assert!(unsafe { body.call0::<bool>() }?);
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn drop1<T, F>(&mut self, op: F, padding0: bool)
    where
        F: Fn(T) + 'static,
        T: 'static,
    {
        self.push_storage(op);
        if padding0 {
            self.drop1_::<true, T, F>();
        } else {
            self.drop1_::<false, T, F>();
        }
    }

    /// Pushes the op-dispatch closure for a binary infallible operation with compile-time padding.
    fn push_op2_<const PADDING0: bool, const PADDING1: bool, T, U, R, F>(&mut self)
    where
        F: Fn(T, U) -> R + 'static,
        T: 'static,
        U: 'static,
        R: 'static,
    {
        self.ops.push(|storage, p, stack| {
            let (f, r) = unsafe { storage.next::<F>(p) };
            let y: U = unsafe { stack.pop(PADDING1) };
            let x: T = unsafe { stack.pop(PADDING0) };
            stack.push(f(x, y));
            Ok(r)
        });
    }

    /// Pushes a binary operation that takes two arguments of types T and U and returns a value of
    /// type R.
    ///
    /// Callback arguments and padding flags follow operand source order.
    /// - Complexity: The capture insertion cost of [`Self::push_op0`].
    ///
    /// # Examples
    /// ```
    /// use cel_runtime::RawSegment;
    /// let mut body = RawSegment::new();
    /// body.push_op2(|first: String, second: u8| (first, second), false, false);
    /// assert_eq!(unsafe { body.call2::<_, _, (String, u8)>((String::from("first"), 7u8)) }?,
    ///     (String::from("first"), 7));
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn push_op2<T, U, R, F>(&mut self, op: F, padding0: bool, padding1: bool)
    where
        F: Fn(T, U) -> R + 'static,
        T: 'static,
        U: 'static,
        R: 'static,
    {
        self.push_storage(op);
        match (padding0, padding1) {
            (false, false) => self.push_op2_::<false, false, T, U, R, F>(),
            (false, true) => self.push_op2_::<false, true, T, U, R, F>(),
            (true, false) => self.push_op2_::<true, false, T, U, R, F>(),
            (true, true) => self.push_op2_::<true, true, T, U, R, F>(),
        }
        self.base_alignment = max(self.base_alignment, align_of::<R>());
    }

    /// Pushes the op-dispatch closure for a binary fallible operation with compile-time padding.
    fn push_op2r_<const PADDING0: bool, const PADDING1: bool, T, U, R, F>(&mut self)
    where
        F: Fn(&mut RawStack, T, U) -> Result<R> + 'static,
        T: 'static,
        U: 'static,
        R: 'static,
    {
        self.ops.push(|storage, p, stack| {
            let (f, r) = unsafe { storage.next::<F>(p) };
            let y: U = unsafe { stack.pop(PADDING1) };
            let x: T = unsafe { stack.pop(PADDING0) };
            let result = f(stack, x, y)?;
            stack.push(result);
            Ok(r)
        });
    }

    /// Pushes a fallible binary operation that can manipulate the remaining stack.
    ///
    /// Arguments and padding flags follow source order. Operation errors propagate
    /// at execution after frame cleanup.
    /// - Complexity: The capture insertion cost of [`Self::push_op0`].
    ///
    /// # Examples
    /// See [`RawSegment`].
    pub fn raw2<T, U, R, F>(&mut self, op: F, padding0: bool, padding1: bool)
    where
        F: Fn(&mut RawStack, T, U) -> Result<R> + 'static,
        T: 'static,
        U: 'static,
        R: 'static,
    {
        self.push_storage(op);
        match (padding0, padding1) {
            (false, false) => self.push_op2r_::<false, false, T, U, R, F>(),
            (false, true) => self.push_op2r_::<false, true, T, U, R, F>(),
            (true, false) => self.push_op2r_::<true, false, T, U, R, F>(),
            (true, true) => self.push_op2r_::<true, true, T, U, R, F>(),
        }
        self.base_alignment = max(self.base_alignment, align_of::<R>());
    }

    /// Pushes a ternary operation that takes three arguments of types T, U, and V and returns a
    /// value of type R.
    #[expect(clippy::many_single_char_names, reason = "patterned code")]
    fn push_op3_<const PADDING0: bool, const PADDING1: bool, const PADDING2: bool, T, U, V, R, F>(
        &mut self,
    ) where
        F: Fn(T, U, V) -> R + 'static,
        T: 'static,
        U: 'static,
        V: 'static,
        R: 'static,
    {
        self.ops.push(|storage, p, stack| {
            let (f, r) = unsafe { storage.next::<F>(p) };
            let z: V = unsafe { stack.pop(PADDING2) };
            let y: U = unsafe { stack.pop(PADDING1) };
            let x: T = unsafe { stack.pop(PADDING0) };
            stack.push(f(x, y, z));
            Ok(r)
        });
    }

    /// Pushes a ternary operation that takes three arguments of types T, U, and V and returns a value of type R.
    ///
    /// Arguments and padding flags follow source order.
    /// - Complexity: The capture insertion cost of [`Self::push_op0`].
    ///
    /// # Examples
    /// ```
    /// use cel_runtime::RawSegment;
    /// let mut body = RawSegment::new();
    /// body.push_op0(|| 1u8);
    /// body.push_op0(|| 2u8);
    /// body.push_op0(|| 3u8);
    /// body.push_op3(|first: u8, second: u8, third: u8| (first, second, third), false, false, false);
    /// assert_eq!(unsafe { body.call0::<(u8, u8, u8)>() }?, (1, 2, 3));
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn push_op3<T, U, V, R, F>(&mut self, op: F, padding0: bool, padding1: bool, padding2: bool)
    where
        F: Fn(T, U, V) -> R + 'static,
        T: 'static,
        U: 'static,
        V: 'static,
        R: 'static,
    {
        self.push_storage(op);

        match (padding0, padding1, padding2) {
            (false, false, false) => self.push_op3_::<false, false, false, T, U, V, R, F>(),
            (false, false, true) => self.push_op3_::<false, false, true, T, U, V, R, F>(),
            (false, true, false) => self.push_op3_::<false, true, false, T, U, V, R, F>(),
            (false, true, true) => self.push_op3_::<false, true, true, T, U, V, R, F>(),
            (true, false, false) => self.push_op3_::<true, false, false, T, U, V, R, F>(),
            (true, false, true) => self.push_op3_::<true, false, true, T, U, V, R, F>(),
            (true, true, false) => self.push_op3_::<true, true, false, T, U, V, R, F>(),
            (true, true, true) => self.push_op3_::<true, true, true, T, U, V, R, F>(),
        }
        self.base_alignment = max(self.base_alignment, align_of::<R>());
    }

    /// Pushes the op-dispatch closure for a ternary fallible operation with compile-time padding.
    #[expect(clippy::many_single_char_names, reason = "patterned code")]
    fn push_op3r_<const PADDING0: bool, const PADDING1: bool, const PADDING2: bool, T, U, V, R, F>(
        &mut self,
    ) where
        F: Fn(&mut RawStack, T, U, V) -> Result<R> + 'static,
        T: 'static,
        U: 'static,
        V: 'static,
        R: 'static,
    {
        self.ops.push(|storage, p, stack| {
            let (f, r) = unsafe { storage.next::<F>(p) };
            let z: V = unsafe { stack.pop(PADDING2) };
            let y: U = unsafe { stack.pop(PADDING1) };
            let x: T = unsafe { stack.pop(PADDING0) };
            let result = f(stack, x, y, z)?;
            stack.push(result);
            Ok(r)
        });
    }

    /// Pushes a fallible ternary operation that can manipulate the remaining stack.
    ///
    /// Arguments and padding flags follow source order. Operation errors propagate
    /// at execution after frame cleanup.
    /// - Complexity: The capture insertion cost of [`Self::push_op0`].
    ///
    /// # Examples
    /// See [`RawSegment`].
    pub fn raw3<T, U, V, R, F>(&mut self, op: F, padding0: bool, padding1: bool, padding2: bool)
    where
        F: Fn(&mut RawStack, T, U, V) -> Result<R> + 'static,
        T: 'static,
        U: 'static,
        V: 'static,
        R: 'static,
    {
        self.push_storage(op);
        match (padding0, padding1, padding2) {
            (false, false, false) => self.push_op3r_::<false, false, false, T, U, V, R, F>(),
            (false, false, true) => self.push_op3r_::<false, false, true, T, U, V, R, F>(),
            (false, true, false) => self.push_op3r_::<false, true, false, T, U, V, R, F>(),
            (false, true, true) => self.push_op3r_::<false, true, true, T, U, V, R, F>(),
            (true, false, false) => self.push_op3r_::<true, false, false, T, U, V, R, F>(),
            (true, false, true) => self.push_op3r_::<true, false, true, T, U, V, R, F>(),
            (true, true, false) => self.push_op3r_::<true, true, false, T, U, V, R, F>(),
            (true, true, true) => self.push_op3r_::<true, true, true, T, U, V, R, F>(),
        }
        self.base_alignment = max(self.base_alignment, align_of::<R>());
    }

    /// Executes all operations in the segment on the supplied stack.
    ///
    /// # Errors
    /// Halts execution and returns an error if any operation returns an error.
    ///
    /// # Safety
    ///
    /// The stack's alignment, origin, operand types, and padding must match every
    /// operation. Callback accesses must preserve the frame's ownership.
    /// Violating these requirements can cause undefined behavior.
    ///
    /// - Complexity: O(n) dispatches plus callback costs for n operations.
    ///
    /// # Examples
    /// ```
    /// use cel_runtime::{RawSegment, RawStack};
    /// let mut body = RawSegment::new();
    /// body.push_op0(|| String::from("owned"));
    /// let mut stack = RawStack::with_base_alignment(std::mem::align_of::<String>());
    /// unsafe { body.call0_stack(&mut stack) }?;
    /// assert_eq!(unsafe { stack.pop::<String>(false) }, "owned");
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub unsafe fn call0_stack(&self, stack: &mut RawStack) -> Result<()> {
        let (origin, depth) = self.frame.map_or((0, 0), |(origin, inputs)| {
            (
                origin,
                stack
                    .depth()
                    .checked_sub(inputs)
                    .expect("valid input frame"),
            )
        });
        stack.with_unwind(origin, depth, |stack| {
            let mut p = 0;
            for op in &self.ops {
                p = op(&self.storage, p, stack)?;
            }
            Ok(())
        })
    }

    /// Executes all operations in the segment and returns the final result.
    ///
    /// # Errors
    /// Halts execution and returns an error if any operation returns an error.
    ///
    /// # Safety
    /// Operations must accept an empty initial stack and end in a live `T`
    /// with no preceding alignment gap.
    /// All operand types and padding must match their registered operations.
    /// Violating these requirements can cause undefined behavior.
    ///
    /// - Complexity: O(n) dispatches plus callback costs for n operations;
    ///   allocates the maximum live stack.
    ///
    /// # Examples
    /// See [`RawSegment`].
    pub unsafe fn call0<T>(&self) -> Result<T>
    where
        T: 'static,
    {
        let mut stack = RawStack::with_base_alignment(self.base_alignment);
        unsafe {
            self.call0_stack(&mut stack)?;
        }
        Ok(unsafe { stack.pop(false) })
    }

    /// Executes all operations in the segment with one argument of type A and returns the final
    /// result.
    ///
    /// # Errors
    /// Halts execution and returns an error if any operation returns an error.
    ///
    /// # Safety
    /// Operations must accept the initial `A` and end in a live `T`
    /// with no preceding alignment gap.
    /// All operand types and padding must match their registered operations.
    /// Violating these requirements can cause undefined behavior.
    ///
    /// - Complexity: The execution and allocation costs of [`Self::call0`].
    ///
    /// # Examples
    /// ```
    /// use cel_runtime::RawSegment;
    /// let mut body = RawSegment::new();
    /// body.push_op1(|value: String| value.len(), false);
    /// assert_eq!(unsafe { body.call1::<_, usize>(String::from("owned")) }?, 5);
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub unsafe fn call1<A: 'static, T>(&self, arg: A) -> Result<T>
    where
        T: 'static,
    {
        let mut stack = RawStack::with_base_alignment(self.base_alignment.max(align_of::<A>()));
        stack.push(arg);
        unsafe { self.call0_stack(&mut stack)? };
        Ok(unsafe { stack.pop(false) })
    }

    /// Executes all operations in the segment with two arguments of types A and B and returns the
    /// final result.
    ///
    /// # Errors
    /// Halts execution and returns an error if any operation returns an error.
    ///
    /// # Safety
    /// Operations must accept the initial source-ordered `(A, B)` and end in a
    /// live `T` with no preceding alignment gap. Operand types and padding must match every
    /// registered operation. Violating these requirements can cause undefined behavior.
    ///
    /// - Complexity: The execution and allocation costs of [`Self::call0`].
    ///
    /// # Examples
    /// See [`Self::push_op2`].
    pub unsafe fn call2<A: 'static, B: 'static, T>(&self, arg: (A, B)) -> Result<T>
    where
        T: 'static,
    {
        let mut stack = RawStack::with_base_alignment(
            self.base_alignment
                .max(align_of::<A>())
                .max(align_of::<B>()),
        );
        stack.push(arg.0);
        stack.push(arg.1);
        unsafe { self.call0_stack(&mut stack)? };
        Ok(unsafe { stack.pop(false) })
    }
}

impl Drop for RawSegment {
    /// Releases instruction captures, preserving cleanup across destructor unwinding.
    ///
    /// - Complexity: O(n) plus destruction costs for n captures.
    fn drop(&mut self) {
        let mut cleanup = StorageCleanup {
            storage: &mut self.storage,
            droppers: self.dropper.iter(),
            position: 0,
        };
        cleanup.drop_remaining();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Supports captured values whose alignment exceeds the initial storage alignment.
    #[test]
    fn instruction_capture_alignment_grows_without_losing_earlier_state() {
        /// Requires a wider alignment than the initial instruction buffer.
        #[repr(align(8192))]
        struct Aligned(u8);

        impl Aligned {
            /// Reads the captured value without narrowing the closure's capture.
            fn value(&self) -> u8 {
                self.0
            }
        }

        let captured = Aligned(42);
        let earlier = String::from("retained");
        let mut segment = RawSegment::new();
        segment.push_op0(move || earlier.clone());
        segment.push_op0(move || captured.value());
        segment.push_op2(|first: String, second: u8| (first, second), false, false);
        assert_eq!(
            unsafe { segment.call0::<(String, u8)>() }.unwrap(),
            (String::from("retained"), 42)
        );
    }

    /// Aligns the supplied arguments even when instructions produce only bytes.
    #[test]
    fn over_aligned_call_arguments() {
        /// Requires a stricter alignment than the operation result.
        #[repr(align(4096))]
        struct Aligned(u8);

        let mut one = RawSegment::new();
        one.push_op1(|value: Aligned| value.0, false);
        assert_eq!(unsafe { one.call1::<_, u8>(Aligned(42)) }.unwrap(), 42);

        let mut two = RawSegment::new();
        two.push_op2(|first: Aligned, second: u8| (first.0, second), false, false);
        assert_eq!(
            unsafe { two.call2::<_, _, (u8, u8)>((Aligned(42), 7u8)) }.unwrap(),
            (42, 7)
        );
        let mut reversed = RawSegment::new();
        reversed.push_op2(|first: u8, second: Aligned| (first, second.0), false, true);
        assert_eq!(
            unsafe { reversed.call2::<_, _, (u8, u8)>((7u8, Aligned(42))) }.unwrap(),
            (7, 42)
        );
    }

    /// Releases later instruction captures when an earlier capture destructor panics.
    #[test]
    fn capture_destruction_continues_after_panic() {
        use std::{cell::RefCell, rc::Rc};

        /// Records destruction and optionally starts unwinding.
        struct Capture {
            id: u8,
            panic: bool,
            destroyed: Rc<RefCell<Vec<u8>>>,
        }

        impl Drop for Capture {
            /// Records this capture before any requested panic.
            fn drop(&mut self) {
                self.destroyed.borrow_mut().push(self.id);
                assert!(!self.panic, "capture destructor panic");
            }
        }

        let destroyed = Rc::new(RefCell::new(Vec::new()));
        let mut segment = RawSegment::new();
        for id in 0..3 {
            let capture = Capture {
                id,
                panic: id == 1,
                destroyed: destroyed.clone(),
            };
            segment.push_op0(move || {
                let _keep_capture = &capture;
                42u32
            });
        }
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(segment))).is_err());
        assert_eq!(*destroyed.borrow(), [0, 1, 2]);
    }

    #[test]
    fn nullary_operation() {
        let mut segment = RawSegment::new();
        segment.push_op0(|| 42);
        unsafe {
            assert_eq!(segment.call0::<i32>().unwrap(), 42);
        }
    }

    #[test]
    fn unary_operation() {
        let mut segment = RawSegment::new();
        segment.push_op0(|| 42);
        segment.push_op1(|x: i32| x * 2, false);
        unsafe {
            assert_eq!(segment.call0::<i32>().unwrap(), 84);
        }
    }

    #[test]
    fn binary_operation() {
        let mut segment = RawSegment::new();
        segment.push_op0(|| 10);
        segment.push_op0(|| 5);
        segment.push_op2(|x: i32, y: i32| x + y, false, false);
        unsafe {
            assert_eq!(segment.call0::<i32>().unwrap(), 15);
        }
    }

    #[test]
    fn ternary_operation() {
        let mut segment = RawSegment::new();
        segment.push_op0(|| 2);
        segment.push_op0(|| 3);
        segment.push_op0(|| 4);
        segment.push_op3(|x: i32, y: i32, z: i32| x + y + z, false, false, false);
        unsafe {
            assert_eq!(segment.call0::<i32>().unwrap(), 9);
        }
    }

    #[test]
    fn complex_chain() {
        let mut segment = RawSegment::new();
        segment.push_op0(|| 10);
        segment.push_op1(|x: i32| x * 2, false);
        segment.push_op0(|| 5);
        segment.push_op2(|x: i32, y: i32| x + y, false, false);
        unsafe {
            assert_eq!(segment.call0::<i32>().unwrap(), 25);
        }
    }
}
