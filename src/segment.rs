use crate::raw_segment::RawSegment;
use crate::raw_stack::RawStack;
use crate::type_list::{IntoList, List, Reverse};
use anyhow::Result;

pub struct DropTopFn(fn(&mut RawStack) -> Option<DropTopFn>);

pub trait DropTop: List {
    fn drop_top(stack: &mut RawStack) -> Option<DropTopFn>;
}

impl DropTop for () {
    fn drop_top(_: &mut RawStack) -> Option<DropTopFn> {
        None
    }
}

impl<T, U> DropTop for (T, U)
where
    U: DropTop,
{
    fn drop_top(stack: &mut RawStack) -> Option<DropTopFn> {
        unsafe { stack.drop::<T>() };
        Some(DropTopFn(U::drop_top))
    }
}

/**
A type-safe wrapper around RawSegment that tracks the type of the values
on the stack at compile time.

T is a cons cell that represents the stack of values.
*/

pub trait IntoReverseList {
    type Result: List;
}

impl<Args: IntoList> IntoReverseList for Args
where
    Args::Result: Reverse,
{
    type Result = <Args::Result as Reverse>::Result;
}

pub struct Segment<Args: IntoReverseList, Stack: List = <Args as IntoReverseList>::Result> {
    segment: RawSegment,
    _phantom: std::marker::PhantomData<(Args, Stack)>,
}

impl<Args: IntoReverseList, Stack: DropTop> Segment<Args, Stack> {
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
                let mut dropper = Stack::drop_top(stack);
                while let Some(e) = dropper {
                    dropper = e.0(stack);
                }
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
pub fn new_segment<Args: IntoReverseList>() -> Segment<Args> {
    Segment {
        segment: RawSegment::new(),
        _phantom: std::marker::PhantomData,
    }
}

// trait Fn<Args> is currently unstable - so we use a call trait as a temporary workaround.
trait Callable<Args> {
    type Output;
    fn call(&self, args: Args) -> Self::Output;
}

impl<T: 'static> Callable<()> for Segment<(), (T, ())> {
    type Output = Result<T>;
    fn call(&self, _args: ()) -> Self::Output {
        self.call0()
    }
}

impl<T: 'static, A> Callable<(A,)> for Segment<(A,), (T, ())> {
    type Output = Result<T>;
    fn call(&self, args: (A,)) -> Self::Output {
        self.call1(args)
    }
}

impl<T: 'static, A, B> Callable<(A, B)> for Segment<(A, B), (T, ())> {
    type Output = Result<T>;
    fn call(&self, args: (A, B)) -> Self::Output {
        self.call2(args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering;

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
