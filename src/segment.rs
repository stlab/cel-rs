use crate::raw_segment::RawSegment;

pub trait TupleToTypeList {
    type Result: ConsCell;
}

impl TupleToTypeList for () {
    type Result = ();
}

impl<A> TupleToTypeList for (A,) {
    type Result = (A, ());
}

impl<A, B> TupleToTypeList for (A, B) {
    type Result = (A, (B, ()));
}

impl<A, B, C> TupleToTypeList for (A, B, C) {
    type Result = (A, (B, (C, ())));
}

impl<A, B, C, D> TupleToTypeList for (A, B, C, D) {
    type Result = (A, (B, (C, (D, ()))));
}

pub trait ConsCell {
    type CAR;
    type CDR: ConsCell;
    //fn len() -> usize;
}

impl ConsCell for () {
    type CAR = ();
    type CDR = ();
    // fn len() -> usize {
    //    0
    // }
}

impl<T, U> ConsCell for (T, U)
where
    U: ConsCell,
{
    type CAR = T;
    type CDR = U;
    // fn len() -> usize {
    //     1 + U::len()
    // }
}

/**
A type-safe wrapper around RawSegment that tracks the type of the values
on the stack at compile time.

T is a cons cell that represents the stack of values.
*/

pub struct Segment<Args, T: ConsCell = <Args as TupleToTypeList>::Result> {
    segment: RawSegment,
    _phantom: std::marker::PhantomData<(Args, T)>,
}

impl<Args: TupleToTypeList, T: ConsCell> Segment<Args, T> {
    /** Creates a new empty segment with no operations. */
    pub fn new<Bargs: TupleToTypeList>() -> Segment<Bargs, <Bargs as TupleToTypeList>::Result> {
        Segment {
            segment: RawSegment::new(),
            _phantom: std::marker::PhantomData,
        }
    }

    /** Pushes a nullary operation that takes no arguments and returns a value of type R. */
    pub fn push_op0<R, F>(self, op: F) -> Segment<Args, (R, T)>
    where
        F: Fn() -> R + 'static,
        R: 'static,
        T: 'static,
    {
        let mut seg = self.segment;
        seg.push_op0(op);
        Segment {
            segment: seg,
            _phantom: std::marker::PhantomData,
        }
    }

    /** Pushes a unary operation that takes the current stack value and returns a new value. */
    pub fn push_op1<R, F>(self, op: F) -> Segment<Args, (R, T::CDR)>
    where
        F: Fn(T::CAR) -> R + 'static,
        T: 'static,
        R: 'static,
    {
        let mut seg = self.segment;
        seg.push_op1(op);
        Segment {
            segment: seg,
            _phantom: std::marker::PhantomData,
        }
    }

    pub fn push_op2<R, F>(self, op: F) -> Segment<Args, (R, <T::CDR as ConsCell>::CDR)>
    where
        F: Fn(T::CAR, <T::CDR as ConsCell>::CAR) -> R + 'static,
        T: 'static,
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
    pub(crate) unsafe fn call0<U: 'static>(self) -> U
    where
        T: 'static,
    {
        unsafe { self.segment.call0() }
    }

    pub(crate) unsafe fn call1<U: 'static, A>(self, args: A) -> U
    where
        T: 'static,
    {
        unsafe { self.segment.call1(args) }
    }
}

// trait Fn<Args> is currently unstable - so we use a call trait as a temporary workaround.
trait Callable<Args> {
    type Output;
    fn call(self, args: Args) -> Self::Output;
}

impl<T: 'static> Callable<()> for Segment<(), (T, ())> {
    type Output = T;
    fn call(self, _args: ()) -> T {
        unsafe { self.call0() }
    }
}

impl<T: 'static, A> Callable<(A,)> for Segment<(A,), (T, ())> {
    type Output = T;
    fn call(self, args: (A,)) -> T {
        unsafe { self.call1(args) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_type_safe_operations() {
        let result = Segment::<()>::new()
            .push_op0(|| 42)
            .push_op0(|| 10)
            .push_op2(|x, y| x + y)
            .push_op1(|x: i32| x * 2)
            .push_op1(|x: i32| x.to_string())
            .call(());

        assert_eq!(result, "104");
    }

    #[test]
    fn test_chain_operations() {
        let result = Segment::<()>::new()
            .push_op0(|| "Hello")
            .push_op1(|s: &str| s.len())
            .push_op1(|n: usize| n * 2)
            .push_op1(|n: usize| format!("Length * 2 = {}", n))
            .call(());

        assert_eq!(result, "Length * 2 = 10");
    }

    #[test]
    fn test_call_with_args() {
        let result = Segment::<(i32,)>::new()
            .push_op1(|x: i32| x * 2)
            .call((21,));

        assert_eq!(result, 42);
    }   
}
