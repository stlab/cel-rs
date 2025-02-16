use crate::dyn_segment::DynSegment;

/**
A type-safe wrapper around RawSegment that tracks the type of the last value
on the stack at compile time.

Type parameter T represents the type of the value currently at the top of
the execution stack.
*/
pub struct Segment<T> {
    segment: RawSegment,
}

impl Segment<()> {
    /** Creates a new empty segment with no operations. */
    pub fn new() -> Self {
        Segment {
            segment: RawSegment::new(),
        }
    }
}

impl<T> Segment<T> {
    /** Pushes a nullary operation that takes no arguments and returns a value of type R. */
    pub fn push_op0<R, F>(self, op: F) -> Segment<R>
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
    pub fn push_op1<R, F>(self, op: F) -> Segment<R>
    where
        F: Fn(T) -> R + 'static,
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

    /** Executes all operations in the segment and returns the final result. */
    pub fn run(mut self) -> T
    where
        T: 'static,
    {
        self.segment.run()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_type_safe_operations() {
        let result = Segment::new()
            .push_op0(|| 42)
            .push_op1(|x: i32| x * 2)
            .push_op1(|x: i32| x.to_string())
            .run();

        assert_eq!(result, "84");
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
