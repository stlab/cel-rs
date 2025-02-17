use crate::raw_segment::RawSegment;

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

pub struct Segment<T> {
    segment: RawSegment,
    _phantom: std::marker::PhantomData<T>,
}

impl<T> Segment<T>
where
    T: ConsCell,
{
    /** Creates a new empty segment with no operations. */
    pub fn new() -> Self {
        Segment {
            segment: RawSegment::new(),
            _phantom: std::marker::PhantomData,
        }
    }

    /** Pushes a nullary operation that takes no arguments and returns a value of type R. */
    pub fn push_op0<R, F>(self, op: F) -> Segment<(R, T)>
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
    pub fn push_op1<R, F>(self, op: F) -> Segment<(R, T::CDR)>
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

    pub fn push_op2<R, F>(self, op: F) -> Segment<(R, <T::CDR as ConsCell>::CDR)>
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
    pub(crate) unsafe fn run_<U: 'static>(self) -> U
    where
        T: 'static,
    {
        self.segment.run()
    }
}

trait Runner {
    type Result;
    fn run(self) -> Self::Result;
}

impl<T: 'static> Runner for Segment<(T, ())> {
    type Result = T;
    fn run(self) -> T {
        unsafe { self.run_() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_type_safe_operations() {
        let result = Segment::new()
            .push_op0(|| 42)
            .push_op0(|| 10)
            .push_op2(|x, y| x + y)
            .push_op1(|x: i32| x * 2)
            .push_op1(|x: i32| x.to_string())
            .run();

        assert_eq!(result, "104");
    }

    #[test]
    fn test_chain_operations() {
        let result = Segment::new()
            .push_op0(|| "Hello")
            .push_op1(|s: &str| s.len())
            .push_op1(|n: usize| n * 2)
            .push_op1(|n: usize| format!("Length * 2 = {}", n))
            .run();

        assert_eq!(result, "Length * 2 = 10");
    }
}
