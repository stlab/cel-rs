use crate::dyn_segment::DynSegment;
use crate::raw_segment::RawSegment;
use crate::raw_stack::RawStack;
use crate::type_list::{IntoList, List, TypeHandler};
use anyhow::{Result, ensure};
use std::any::TypeId;

// Create a handler for dropping types
struct DropHandler<'a>(&'a mut RawStack);

impl TypeHandler for DropHandler<'_> {
    fn handle<T>(self: &mut Self) {
        unsafe { self.0.drop::<T>() };
    }
}

trait DropTop {
    fn drop_top(stack: &mut RawStack);
}

// Implement DropTop for List
impl<T: List> DropTop for T {
    fn drop_top(stack: &mut RawStack) {
        let mut handler = DropHandler(stack);
        T::for_each_type(&mut handler);
    }
}

struct EqListTypeIDListHandler<'a>(&'a [TypeId], &'a mut usize, &'a mut bool);

impl TypeHandler for EqListTypeIDListHandler<'_> {
    fn handle<T: 'static>(self: &mut Self) {
        *self.2 = *self.2 && TypeId::of::<T>() == self.0[*self.1];
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
/**
A type-safe segment that represents a sequence of operations.

The segment takes input arguments of type `Args` and maintains a type stack `Stack` that tracks
the types of values produced by operations.

# Type Parameters
- `Args`: The input argument types, must implement `IntoList`
- `Stack`: The type stack tracking operation results, defaults to the reverse of `Args`

# Examples

```rust
use cel_rs::segment::*;

assert_eq!(new_segment::<(i32, )>() // create a new segment that takes an i32 argument
    .op1(|x| x * 2)                 // push a unary operation that multiplies the argument by 2
    .op0(|| 10)                     // push a nullary operation that returns 10
    .op2(|x, y| x + y)              // push a binary operation that adds two arguments
    .op1(|x| x.to_string())         // push a unary operation that converts the argument to a string
    .call((42, )).unwrap(),         // call the segment with an argument 42
    "94");                          // the result is (42 * 2 + 10).to_string()
```
*/

pub struct Segment<Args: IntoList, Stack: List = <<Args as IntoList>::Result as List>::Reverse> {
    segment: RawSegment,
    _phantom: std::marker::PhantomData<(Args, Stack)>,
}

impl<Args: IntoList, Stack: List> Segment<Args, Stack> {
    pub fn from_dyn_segment(segment: DynSegment) -> Result<Self> {
        type ArgList<Args> = <<Args as IntoList>::Result as List>::Reverse;

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

    /** Pushes a nullary operation that takes no arguments and returns a value of type R. */
    pub fn op0<R, F>(self, op: F) -> Segment<Args, (R, Stack)>
    where
        F: Fn() -> R + 'static,
        R: 'static,
        Stack: 'static,
    {
        let mut seg = self.segment;
        seg.push_op0(op);
        Segment {
            segment: seg,
            _phantom: std::marker::PhantomData,
        }
    }

    pub fn op0r<R, F>(self, op: F) -> Segment<Args, (R, Stack)>
    where
        F: Fn() -> Result<R> + 'static,
        R: 'static,
        Stack: 'static,
    {
        let mut seg = self.segment;
        seg.raw0(move |stack| match op() {
            Ok(r) => Ok(r),
            Err(e) => {
                Stack::drop_top(stack);
                Err(e)
            }
        });
        Segment {
            segment: seg,
            _phantom: std::marker::PhantomData,
        }
    }

    /** Pushes a unary operation that takes the current stack value and returns a new value. */
    pub fn op1<R, F>(self, op: F) -> Segment<Args, (R, Stack::Tail)>
    where
        F: Fn(Stack::Head) -> R + 'static,
        Stack: 'static,
        R: 'static,
    {
        let mut seg = self.segment;
        seg.push_op1(op);
        Segment {
            segment: seg,
            _phantom: std::marker::PhantomData,
        }
    }

    pub fn op2<R, F>(self, op: F) -> Segment<Args, (R, <Stack::Tail as List>::Tail)>
    where
        F: Fn(<Stack::Tail as List>::Head, Stack::Head) -> R + 'static,
        Stack: 'static,
        R: 'static,
    {
        let mut seg = self.segment;
        seg.push_op2(op);
        Segment {
            segment: seg,
            _phantom: std::marker::PhantomData,
        }
    }

    /** Executes all operations in the segment and returns the final result. */
    pub(crate) fn call0<U: 'static>(&self) -> Result<U>
    where
        Stack: 'static,
    {
        unsafe { self.segment.call0() }
    }

    pub(crate) fn call1<U: 'static, A>(&self, args: A) -> Result<U>
    where
        Stack: 'static,
    {
        unsafe { self.segment.call1(args) }
    }

    pub(crate) fn call2<U: 'static, A, B>(&self, args: (A, B)) -> Result<U>
    where
        Stack: 'static,
    {
        unsafe { self.segment.call2(args) }
    }
}

/** Creates a new empty segment with no operations. */
pub fn new_segment<Args: IntoList>() -> Segment<Args> {
    Segment {
        segment: RawSegment::new(),
        _phantom: std::marker::PhantomData,
    }
}

// trait Fn<Args> is currently unstable - so we use a call trait as a temporary workaround.
pub trait Callable<Args> {
    type Output;
    fn call(&self, args: Args) -> Self::Output;
}

impl<T: List + 'static> Callable<()> for Segment<(), T>
where
    T::Tail: List<Tail = ()>,
{
    type Output = Result<T::Head>;
    fn call(&self, _args: ()) -> Self::Output {
        self.call0()
    }
}

impl<T: List + 'static, A: 'static> Callable<(A,)> for Segment<(A,), T>
where
    T::Tail: List<Tail = ()>,
{
    type Output = Result<T::Head>;
    fn call(&self, args: (A,)) -> Self::Output {
        self.call1(args)
    }
}

impl<T: List + 'static, A: 'static, B: 'static> Callable<(A, B)> for Segment<(A, B), T>
where
    T::Tail: List<Tail = ()>,
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
    fn test_segment_from_dyn_segment() -> Result<()> {
        let mut dyn_segment = DynSegment::new::<(i32,)>();
        dyn_segment.op0(|| 42);
        dyn_segment.op2(|x: i32, y: i32| x + y)?;
        let segment = Segment::<(i32,), (i32, ())>::from_dyn_segment(dyn_segment)?;

        assert_eq!(segment.call((10,)).unwrap(), 52);
        Ok(())
    }

    #[test]
    fn test_unit_result() {
        let segment = new_segment::<()>();
        let result = segment.call(());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), ());
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

        let segment = new_segment::<()>()
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
        let result = new_segment::<(&str, i32)>()
            .op2(|x, y| format!("{} {}", x, y))
            .call(("Hello", 12));

        assert_eq!(result.unwrap(), "Hello 12");
    }
    #[test]
    fn test_type_safe_operations() {
        let result = new_segment::<()>()
            .op0(|| 42)
            .op0(|| 10)
            .op2(|x, y| x + y)
            .op1(|x: i32| x * 2)
            .op1(|x: i32| x.to_string())
            .call(());

        assert_eq!(result.unwrap(), "104");
    }

    #[test]
    fn test_chain_operations() {
        let result = new_segment::<()>()
            .op0(|| "Hello")
            .op1(|s: &str| s.len())
            .op1(|n: usize| n * 2)
            .op1(|n: usize| format!("Length * 2 = {}", n))
            .call(());

        assert_eq!(result.unwrap(), "Length * 2 = 10");
    }

    #[test]
    fn test_call_with_args() {
        let result = new_segment::<(i32,)>().op1(|x: i32| x * 2).call((21,));

        assert_eq!(result.unwrap(), 42);
    }
}
