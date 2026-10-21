use crate::c_stack_list::{CNil, CStackList};
use crate::dyn_segment::DynSegment;
use crate::list_traits::{EmptyList, IntoList, List, ListTypeIteratorAdvance, TypeIdIterator};
use crate::raw_segment::RawSegment;
use crate::raw_stack::RawStack;
use crate::{CStackListHeadLimit, CStackListHeadPadded, ReverseList};
use anyhow::{Result, ensure};
use std::any::TypeId;
use std::result::Result::Ok;

/// Extracts a typed segment's result or its empty-stack marker.
///
/// # Examples
/// ```
/// use cel_runtime::{Segment, SegmentCallable};
/// assert!(Segment::<()>::new().call(()).is_ok());
/// ```
pub trait StackResult: List {
    /// Moves the final result out, or returns the empty-stack marker.
    ///
    /// # Safety
    /// The raw stack's live values must match this type-level stack exactly.
    unsafe fn take_result(stack: &mut RawStack) -> Self::Head;
}

impl StackResult for CNil<()> {
    /// Returns the empty-stack marker without reading a nonexistent slot.
    unsafe fn take_result(_stack: &mut RawStack) -> crate::list_traits::Undefined {
        crate::list_traits::Undefined
    }
}

impl<H: 'static, T: List + CStackListHeadLimit> StackResult for CStackList<H, T> {
    /// Transfers the concrete head out of its owned stack slot.
    unsafe fn take_result(stack: &mut RawStack) -> H {
        unsafe { stack.pop::<H>(Self::HEAD_PADDED) }
    }
}

/// A type-safe segment that represents a sequence of operations.
///
/// The segment takes input arguments of type `Args` and maintains a type stack `Stack` that tracks
/// the types of values produced by operations.
///
/// # Type Parameters
/// - `Args`: The input argument types, must implement `IntoList`
/// - `Stack`: The type stack tracking operation results, defaults to the reverse of `Args`
///
/// # Examples
///
/// ```rust
/// use cel_runtime::segment::*;
///
/// assert_eq!(Segment::<(i32,)>::new() // create a new segment that takes an i32 argument
///     .op1(|x| x * 2)                 // push a unary operation that multiplies the argument by 2
///     .op0(|| 10)                     // push a nullary operation that returns 10
///     .op2(|x, y| x + y)              // push a binary operation that adds two arguments
///     .op1(|x| x.to_string())         // push a unary operation returning a string of the argument
///     .call((42, )).unwrap(),         // call the segment with an argument 42
///     "94");                          // the result is (42 * 2 + 10).to_string()
/// ```
pub struct Segment<
    Args: IntoList + 'static,
    Stack: List = ReverseList<<Args as IntoList>::Output<CNil<()>>>,
> {
    segment: RawSegment,
    _phantom: std::marker::PhantomData<(Args, Stack)>,
}

impl<Args: IntoList + 'static> Default for Segment<Args> {
    fn default() -> Self {
        Self::new()
    }
}

impl<Args: IntoList + 'static> Segment<Args> {
    /// Creates a new empty segment with no operations.
    #[must_use]
    pub fn new() -> Segment<Args> {
        Segment {
            segment: RawSegment::new(),
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<Args: IntoList + 'static, Stack: List + 'static> TryFrom<DynSegment> for Segment<Args, Stack>
where
    ReverseList<<Args as IntoList>::Output<CNil<()>>>: ListTypeIteratorAdvance<TypeId>,
    Stack: ListTypeIteratorAdvance<TypeId>,
{
    type Error = anyhow::Error;
    fn try_from(value: DynSegment) -> Result<Self, Self::Error> {
        type ArgList<Args> = ReverseList<<Args as IntoList>::Output<CNil<()>>>;

        ensure!(
            ArgList::<Args>::LENGTH == value.argument_ids.len()
                && TypeIdIterator::<ArgList::<Args>>::new().eq(value
                    .argument_ids
                    .iter()
                    .rev()
                    .copied()),
            "argument type ids do not match"
        );
        for info in &value.stack_ids {
            info.value_type
                .validate_opaque_leaf(info.value_type.type_id())?;
        }
        ensure!(
            value.argument_reads.is_empty() && value.base_stack_index == 0,
            "typed conversion requires a direct preloaded frame without borrowed reads"
        );
        ensure!(
            Stack::LENGTH == value.stack_ids.len()
                && TypeIdIterator::<Stack>::new().eq(value
                    .stack_ids
                    .iter()
                    .rev()
                    .map(|info| info.value_type.type_id())),
            "stack type ids do not match"
        );
        Ok(Segment {
            segment: value.segment,
            _phantom: std::marker::PhantomData,
        })
    }
}

impl<Args: IntoList + 'static, Stack> Segment<Args, Stack>
where
    Stack: StackResult + 'static,
    Stack::Tail: CStackListHeadLimit,
{
    /// Private method to change the stack type.
    fn into<NewStack: List + 'static>(self) -> Segment<Args, NewStack> {
        Segment {
            segment: self.segment,
            _phantom: std::marker::PhantomData,
        }
    }

    /*     pub fn indirect<N: 'static>(mut self) -> Segment<Args, Stack::PushFront<&Stack::Index<N>>> {
        self.segment.indirect::<N, T>();
        self.into()
    } */

    /// Pushes a nullary operation that takes no arguments and returns a value of type R.
    pub fn op0<R, F>(mut self, op: F) -> Segment<Args, Stack::Push<R>>
    where
        F: Fn() -> R + 'static,
        R: 'static,
    {
        self.segment.push_op0(op);
        self.into()
    }

    /// Pushes a fallible nullary operation that returns `Result<R>`.
    pub fn op0r<R, F>(mut self, op: F) -> Segment<Args, Stack::Push<R>>
    where
        F: Fn() -> Result<R> + 'static,
        R: 'static,
    {
        self.segment.raw0(move |_| op());
        self.into()
    }

    /// Pushes a unary operation that takes the current stack value and returns a new value.
    pub fn op1<R, F>(mut self, op: F) -> Segment<Args, CStackList<R, Stack::Tail>>
    where
        Stack: CStackListHeadPadded,
        F: Fn(Stack::Head) -> R + 'static,
        R: 'static,
    {
        self.segment.push_op1(op, Stack::HEAD_PADDED);
        self.into()
    }

    /// Pushes a fallible unary operation that takes the current stack value and returns `Result<R>`.
    pub fn op1r<R, F>(mut self, op: F) -> Segment<Args, CStackList<R, Stack::Tail>>
    where
        Stack: CStackListHeadPadded,
        F: Fn(Stack::Head) -> Result<R> + 'static,
        R: 'static,
    {
        self.segment.raw1(move |_, x| op(x), Stack::HEAD_PADDED);
        self.into()
    }
    /// Pushes a binary operation that takes the top two stack values and returns a new value.
    pub fn op2<R, F>(
        mut self,
        op: F,
    ) -> Segment<Args, <<Stack::Tail as List>::Tail as List>::Push<R>>
    where
        Stack: CStackListHeadPadded,
        Stack::Tail: CStackListHeadPadded,
        F: Fn(<Stack::Tail as List>::Head, Stack::Head) -> R + 'static,
        R: 'static,
    {
        self.segment.push_op2(
            op,
            <Stack::Tail as CStackListHeadPadded>::HEAD_PADDED,
            Stack::HEAD_PADDED,
        );
        self.into()
    }

    /// Executes all operations in the segment and returns the final result.
    pub(crate) fn call0(&self) -> Result<Stack::Head> {
        self.finish(RawStack::with_base_alignment(self.segment.base_alignment()))
    }

    /// Executes the owned frame and extracts its statically described result.
    fn finish(&self, mut stack: RawStack) -> Result<Stack::Head> {
        unsafe {
            self.segment.call0_stack(&mut stack)?;
            Ok(Stack::take_result(&mut stack))
        }
    }

    /// Executes a one-argument segment and returns its statically described result.
    pub(crate) fn call1<A: 'static>(&self, (a,): (A,)) -> Result<Stack::Head> {
        let mut stack = RawStack::with_base_alignment(self.segment.base_alignment());
        stack.push(a);
        self.finish(stack)
    }

    /// Executes a two-argument segment and returns its statically described result.
    pub(crate) fn call2<A: 'static, B: 'static>(&self, (a, b): (A, B)) -> Result<Stack::Head> {
        let mut stack = RawStack::with_base_alignment(self.segment.base_alignment());
        stack.push(a);
        stack.push(b);
        self.finish(stack)
    }
}

/// Callable segment abstraction for uniform invocation across arities.
///
/// This trait invokes statically typed stack segments; [`crate::Callable`] is
/// the separate shared native/interpreted value facade.
///
/// # Examples
/// ```
/// use cel_runtime::{Segment, SegmentCallable};
/// let body = Segment::<()>::new().op0(|| 7i32);
/// assert_eq!(body.call(()).unwrap(), 7);
/// ```
pub trait SegmentCallable<Args> {
    /// The result type produced by calling with `Args`.
    type Output;
    /// Invokes the segment with the supplied arguments.
    fn call(&self, args: Args) -> Self::Output;
}

impl<T: StackResult + 'static> SegmentCallable<()> for Segment<(), T>
where
    T::Tail: EmptyList + CStackListHeadLimit,
{
    type Output = Result<T::Head>;
    fn call(&self, _args: ()) -> Self::Output {
        self.call0()
    }
}

impl<T: StackResult + 'static, A: 'static> SegmentCallable<(A,)> for Segment<(A,), T>
where
    T::Tail: EmptyList + CStackListHeadLimit,
{
    type Output = Result<T::Head>;
    fn call(&self, args: (A,)) -> Self::Output {
        self.call1(args)
    }
}

impl<T: StackResult + 'static, A: 'static, B: 'static> SegmentCallable<(A, B)>
    for Segment<(A, B), T>
where
    T::Tail: EmptyList + CStackListHeadLimit,
{
    type Output = Result<T::Head>;
    fn call(&self, args: (A, B)) -> Self::Output {
        self.call2(args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering;

    #[test]
    fn segment_from_dyn_segment() -> Result<()> {
        let mut dyn_segment = DynSegment::new::<(i32,)>();
        dyn_segment.op0(|| 42);
        dyn_segment.op2(|x: i32, y: i32| x + y)?;
        let segment = Segment::<(i32,), CStackList<i32, CNil<()>>>::try_from(dyn_segment)?;

        assert_eq!(segment.call((10,)).unwrap(), 52);
        Ok(())
    }

    #[test]
    fn unit_result() {
        let segment = Segment::new();
        let result = segment.call(());
        assert!(result.is_ok());
    }

    /// Returns the empty-stack marker after consuming one or two input values.
    #[test]
    fn consumed_arguments_allow_empty_results() {
        let mut one = DynSegment::new::<(String,)>();
        one.drop_value().unwrap();
        let one = Segment::<(String,), CNil<()>>::try_from(one).unwrap();
        assert!(one.call((String::from("one"),)).is_ok());

        let mut two = DynSegment::new::<(u8, String)>();
        two.drop_value().unwrap();
        two.drop_value().unwrap();
        let two = Segment::<(u8, String), CNil<()>>::try_from(two).unwrap();
        assert!(two.call((1, String::from("two"))).is_ok());
    }

    #[test]
    fn drop_on_error() {
        struct DropCounter(Arc<AtomicUsize>);

        impl Drop for DropCounter {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        impl Clone for DropCounter {
            fn clone(&self) -> Self {
                DropCounter(self.0.clone())
            }
        }

        let drop_count = Arc::new(AtomicUsize::new(0));
        let tracker = DropCounter(drop_count.clone());

        let segment = Segment::new()
            .op0(move || tracker.clone())
            .op0r(|| -> Result<u32> { Err(anyhow::anyhow!("error")) })
            .op2(|_: DropCounter, _: u32| 42u32);

        assert_eq!(drop_count.load(Ordering::SeqCst), 0); // Nothing dropped yet
        let result = segment.call(());
        assert!(matches!(result, Err(e) if e.to_string() == "error"));
        assert_eq!(drop_count.load(Ordering::SeqCst), 1); // The DropCounter from op0 was dropped
    }

    #[test]
    fn op1r_error_does_not_panic() {
        let parse_and_double = Segment::<(&str,)>::new()
            .op1r(|s: &str| s.parse::<u32>().map_err(anyhow::Error::from))
            .op1(|n| n * 2);

        let result = parse_and_double.call(("nope",));
        assert!(result.is_err());
    }

    #[test]
    fn op1r_drops_remaining_stack_exactly_once_on_error() {
        struct DropCounter(Arc<AtomicUsize>);

        impl Drop for DropCounter {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        impl Clone for DropCounter {
            fn clone(&self) -> Self {
                DropCounter(self.0.clone())
            }
        }

        let drop_count = Arc::new(AtomicUsize::new(0));
        let tracker = DropCounter(drop_count.clone());

        let segment = Segment::new()
            .op0(move || tracker.clone())
            .op0(|| "nope")
            .op1r(|s: &str| s.parse::<u32>().map_err(anyhow::Error::from))
            .op2(|_: DropCounter, _: u32| 42u32);

        assert_eq!(drop_count.load(Ordering::SeqCst), 0); // Nothing dropped yet
        let result = segment.call(());
        assert!(result.is_err());
        assert_eq!(drop_count.load(Ordering::SeqCst), 1); // The DropCounter was dropped exactly once
    }

    #[test]
    fn binary_operation_args_of_different_types() {
        let result = Segment::new()
            .op2(|x, y| format!("{} {}", x, y))
            .call(("Hello", 12));

        assert_eq!(result.unwrap(), "Hello 12");
    }

    #[test]
    fn type_safe_operations() {
        let result = Segment::new()
            .op0(|| 42)
            .op0(|| 10)
            .op2(|x, y| x + y)
            .op1(|x| x * 2)
            .op1(|x| format!("{}", x))
            .call(());

        assert_eq!(result.unwrap(), "104");
    }

    #[test]
    fn chain_operations() {
        let result = Segment::<(&str,)>::new()
            .op1(|s| s.len())
            .op1(|n| n * 2)
            .op1(|n| format!("Length * 2 = {}", n))
            .call(("Hello",));

        assert_eq!(result.unwrap(), "Length * 2 = 10");
    }

    #[test]
    fn call_with_args() {
        let result = Segment::new() //
            .op1(|x: i32| x * 2) //
            .call((21,));

        assert_eq!(result.unwrap(), 42);
    }
}
