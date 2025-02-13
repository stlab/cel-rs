use aligned_vec::{AVec, ConstAlign};
use std::mem;

/** A sequence that stores values with proper alignment.

Values are stored in memory with alignment matching their type requirements.
*/
#[derive(Debug)]
pub struct RawSequence {
    buffer: AVec<u8, ConstAlign<4096>>,
}

const fn truncate_index(align: usize, index: usize) -> usize {
    index & !(align - 1)
}

const fn align_index(align: usize, index: usize) -> usize {
    truncate_index(align, index + align - 1)
}

impl RawSequence {
    pub fn new() -> Self {
        RawSequence {
            buffer: AVec::new(4096),
        }
    }

    // Push a value onto the stack. The value will be stored at an address aligned to max_align().
    pub fn push<T>(&mut self, value: T) {
        assert!(mem::align_of::<T>() <= 4096);
        let len = self.buffer.len();
        let aligned: usize = align_index(mem::align_of::<T>(), len);
        let new_len = aligned + mem::size_of::<T>();

        self.buffer.reserve(new_len - len);
        unsafe {
            self.buffer.set_len(new_len);
            std::ptr::write(self.buffer.as_mut_ptr().add(aligned) as *mut T, value);
        }
    }

    pub unsafe fn drop_in_place<T>(&mut self, p: usize) -> usize {
        let aligned: usize = align_index(mem::align_of::<T>(), p);
        unsafe { std::ptr::drop_in_place(self.buffer.as_ptr().add(aligned) as *mut T) };
        aligned + mem::size_of::<T>()
    }

    pub unsafe fn next<T>(&self, p: usize) -> (&T, usize) {
        let aligned: usize = align_index(mem::align_of::<T>(), p);
        let ptr = unsafe { self.buffer.as_ptr().add(aligned) as *const T };
        unsafe { (&*ptr, aligned + mem::size_of::<T>()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sequence_operations() {
        let mut seq = RawSequence::new();

        seq.push(100u32);
        seq.push(200u32);
        seq.push(42.0f64);
        seq.push("Hello, world!");

        let (value, p) = unsafe { seq.next::<u32>(0) };
        assert_eq!(*value, 100);
        let (value, p) = unsafe { seq.next::<u32>(p) };
        assert_eq!(*value, 200);
        let (value, p) = unsafe { seq.next::<f64>(p) };
        assert_eq!(*value, 42.0);
        let (value, _) = unsafe { seq.next::<&str>(p) };
        assert_eq!(*value, "Hello, world!");
    }
}
