// use crate::dyn_segment::DynSegment;
use crate::exp_type_list::{CEmptyStackList, CStackList, EmptyList, IntoList, List, TypeHandler};
use crate::raw_aligned_stack::RawAlignedStack;
use crate::raw_segment_aligned_stack::RawSegmentAlignedStack;
use anyhow::*;
// use std::mem::offset_of;
use std::result::Result::Ok;
// use std::any::TypeId;

/* pub trait PaddedList: List {
    const _HEAD_PADDING: usize;
    const _HEAD_OFFSET: usize;
}

impl<H: 'static, T: PaddedList> PaddedList for CStackList<H, T>
where
    T::Tail: PaddedList,
{
    const _HEAD_PADDING: usize =
        Self::_HEAD_OFFSET - (T::Tail::_HEAD_OFFSET + size_of::<<T::Tail as List>::Head>());
    const _HEAD_OFFSET: usize = offset_of!(Self, 1);
}

impl PaddedList for CEmptyStackList {
    const _HEAD_PADDING: usize = 0;
    const _HEAD_OFFSET: usize = 0;
} */

// Create a handler for dropping types
struct DropHandler<'a>(&'a mut RawAlignedStack);

impl TypeHandler for DropHandler<'_> {
    fn invoke<T: List>(&mut self) {
        unsafe { self.0.drop::<T::Head>(T::HEAD_PADDING != 0) };
    }
}

trait DropTop {
    fn drop_top(stack: &mut RawAlignedStack);
}

// Implement DropTop for List
impl<T: List + 'static> DropTop for T {
    fn drop_top(stack: &mut RawAlignedStack) {
        let mut handler = DropHandler(stack);
        T::for_each_type(&mut handler);
    }
}

/*
struct EqListTypeIDListHandler<'a>(&'a [TypeId], &'a mut usize, &'a mut bool);

impl TypeHandler for EqListTypeIDListHandler<'_> {
    fn invoke<T: List + 'static>(&mut self) {
        *self.2 = *self.2 && TypeId::of::<T::Head>() == self.0[*self.1];
        *self.1 += 1;
    }
}

trait EqListTypeIDList {
    fn equal(ids: &[TypeId]) -> bool;
}

impl<T: List> EqListTypeIDList for T {
    fn equal(ids: &[TypeId]) -> bool {
        let mut index: usize = 0;
        let mut result: bool = true;
        let mut handler = EqListTypeIDListHandler(ids, &mut index, &mut result);
        T::for_each_type(&mut handler);
        result
    }
}
*/

/**
A type-safe segment that represents a sequence of operations.

The segment takes input arguments of type `Args` and maintains a type stack `Stack` that tracks
the types of values produced by operations.

# Type Parameters
- `Args`: The input argument types, must implement `IntoList`
- `Stack`: The type stack tracking operation results, defaults to the reverse of `Args`

# Examples

```rust
use cel_rs::segment_aligned_stack::*;

assert_eq!(Segment::<(i32,)>::new() // create a new segment that takes an i32 argument
    .op1(|x| x * 2)                 // push a unary operation that multiplies the argument by 2
    .op0(|| 10)                     // push a nullary operation that returns 10
    .op2(|x, y| x + y)              // push a binary operation that adds two arguments
    .op1(|x| x.to_string())         // push a unary operation that converts the argument to a string
    .call((42, )).unwrap(),         // call the segment with an argument 42
    "94");                          // the result is (42 * 2 + 10).to_string()
```
*/
pub struct Segment<
    Args: IntoList + 'static,
    Stack: List = <<Args as IntoList>::IntoList<CEmptyStackList> as List>::Reverse,
> {
    segment: RawSegmentAlignedStack,
    _phantom: std::marker::PhantomData<(Args, Stack)>,
}

impl<Args: IntoList + 'static> Default for Segment<Args> {
    fn default() -> Self {
        Self::new()
    }
}

impl<Args: IntoList + 'static> Segment<Args> {
    pub fn new() -> Segment<Args> {
        Segment {
            segment: RawSegmentAlignedStack::new(),
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<Args: IntoList + 'static, Stack: List + 'static> Segment<Args, Stack> {
    fn into<NewStack: List + 'static>(self) -> Segment<Args, NewStack> {
        Segment {
            segment: self.segment,
            _phantom: std::marker::PhantomData,
        }
    }

    /*
        pub fn from_dyn_segment(segment: DynSegment) -> Result<Self> {
            type ArgList<Args> = <<Args as TupleTraits>::IntoList<CEmptyStackList> as List>::Reverse;

            ensure!(
                ArgList::<Args>::LENGTH == segment.argument_ids.len()
                    && ArgList::<Args>::equal(&segment.argument_ids),
                "argument type ids do not match"
            );
            ensure!(
                Stack::LENGTH == segment.stack_ids.len() && Stack::equal(&segment.stack_ids),
                "stack type ids do not match"
            );
            Ok(Segment {
                segment: segment.segment,
                _phantom: std::marker::PhantomData,
            })
        }
    */
    /*
    pub fn indirect<N: 'static, T: 'static>(
        mut self,
    ) -> Segment<Args, Stack::PushFront<&'static T>> {
        self.segment.indirect::<N, T>();
        self.into()
    } */

    /** Pushes a nullary operation that takes no arguments and returns a value of type R. */
    pub fn op0<R, F>(mut self, op: F) -> Segment<Args, Stack::PushFront<R>>
    where
        F: Fn() -> R + 'static,
        R: 'static,
    {
        self.segment.push_op0(op);
        self.into()
    }

    pub fn op0r<R, F>(mut self, op: F) -> Segment<Args, Stack::PushFront<R>>
    where
        F: Fn() -> Result<R> + 'static,
        R: 'static,
    {
        self.segment.raw0(move |stack| match op() {
            Ok(r) => Ok(r),
            Err(e) => {
                Stack::drop_top(stack);
                Err(e)
            }
        });
        self.into()
    }

    /** Pushes a unary operation that takes the current stack value and returns a new value. */
    pub fn op1<R, F>(mut self, op: F) -> Segment<Args, CStackList<R, Stack::Tail>>
    where
        F: Fn(Stack::Head) -> R + 'static,
        R: 'static,
    {
        self.segment.push_op1(op, Stack::HEAD_PADDING != 0);
        self.into()
    }

    pub fn op2<R, F>(
        mut self,
        op: F,
    ) -> Segment<Args, <<Stack::Tail as List>::Tail as List>::PushFront<R>>
    where
        F: Fn(<Stack::Tail as List>::Head, Stack::Head) -> R + 'static,
        R: 'static,
    {
        self.segment.push_op2(
            op,
            <Stack::Tail as List>::HEAD_PADDING != 0,
            Stack::HEAD_PADDING != 0,
        );
        self.into()
    }

    /** Executes all operations in the segment and returns the final result. */
    pub(crate) fn call0<U: 'static>(&self) -> Result<U> {
        unsafe { self.segment.call0() }
    }

    pub(crate) fn call1<U: 'static, A>(&self, args: A) -> Result<U> {
        unsafe { self.segment.call1(args) }
    }

    pub(crate) fn call2<U: 'static, A, B>(&self, args: (A, B)) -> Result<U> {
        unsafe { self.segment.call2(args) }
    }
}

/** Creates a new empty segment with no operations. */
// trait Fn<Args> is currently unstable - so we use a call trait as a temporary workaround.
pub trait Callable<Args> {
    type Output;
    fn call(&self, args: Args) -> Self::Output;
}

impl<T: List + 'static> Callable<()> for Segment<(), T>
where
    T::Tail: EmptyList,
{
    type Output = Result<T::Head>;
    fn call(&self, _args: ()) -> Self::Output {
        self.call0()
    }
}

impl<T: List + 'static, A: 'static> Callable<(A,)> for Segment<(A,), T>
where
    T::Tail: EmptyList,
{
    type Output = Result<T::Head>;
    fn call(&self, args: (A,)) -> Self::Output {
        self.call1(args)
    }
}

impl<T: List + 'static, A: 'static, B: 'static> Callable<(A, B)> for Segment<(A, B), T>
where
    T::Tail: EmptyList,
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

    /*
    #[test]
    fn test_segment_from_dyn_segment() -> Result<()> {
        let mut dyn_segment = DynSegment::new::<(i32,)>();
        dyn_segment.op0(|| 42);
        dyn_segment.op2(|x: i32, y: i32| x + y)?;
        let segment = Segment::<(i32,), (i32, ())>::from_dyn_segment(dyn_segment)?;

        assert_eq!(segment.call((10,)).unwrap(), 52);
        Ok(())
    } */

    #[test]
    fn test_unit_result() {
        let segment = Segment::new();
        let result = segment.call(());
        assert!(result.is_ok());
    }

    #[test]
    fn test_drop_on_error() {
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
    fn test_binary_operation_args_of_different_types() {
        let result = Segment::new()
            .op2(|x, y| format!("{} {}", x, y))
            .call(("Hello", 12));

        assert_eq!(result.unwrap(), "Hello 12");
    }
    #[test]
    fn test_type_safe_operations() {
        let result = Segment::new()
            .op0(|| 42)
            .op0(|| 10)
            .op2(|x, y| x + y)
            .op1(|x| x * 2)
            .op1(|x| format!("{}", x))
            .call(());

        assert_eq!(result.unwrap(), "104");
    }

    /*     #[test]
    fn test_references() {
        let result = Segment::new()
            .op0(|| 42)
            .op0(|| 10)
            .op1(|&x, y| x + 7)
            .call(());
        assert_eq!(result.unwrap(), 84);
    } */

    #[test]
    fn test_chain_operations() {
        let result = Segment::<(&str,)>::new()
            .op1(|s| s.len())
            .op1(|n| n * 2)
            .op1(|n| format!("Length * 2 = {}", n))
            .call(("Hello",));

        assert_eq!(result.unwrap(), "Length * 2 = 10");
    }

    #[test]
    fn test_call_with_args() {
        let result = Segment::new() //
            .op1(|x: i32| x * 2) //
            .call((21,));

        assert_eq!(result.unwrap(), 42);
    }
}
