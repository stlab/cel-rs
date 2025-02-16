mod dyn_segment;
mod raw_segment;
mod raw_sequence;
mod raw_stack;
mod segment;

pub use dyn_segment::DynSegment;
pub use raw_segment::RawSegment;
pub use raw_sequence::RawSequence;
pub use raw_stack::RawStack;
pub use segment::Segment;

pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}
