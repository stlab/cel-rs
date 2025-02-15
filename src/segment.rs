use std::any::TypeId;

type Operation = fn(&RawSequence, usize, &mut RawStack) -> usize;

struct Segment;

trait Push {
    fn push(self, segment: &mut Segment);
}

// For zero-argument functions:
impl<R, F> Push for F
where
    F: Fn() -> R + 'static,
    R: 'static,
{
    fn push(self, segment: &mut Segment) {
        segment.push_op0(self);
    }
}

// For one-argument functions:
impl<T, R, F> Push for F
where
    F: Fn(T) -> R + 'static,
    T: 'static,
    R: 'static,
{
    fn push(self, segment: &mut Segment) {
        segment.push_op1(self);
    }
}

// For two-argument functions:
impl<T, U, R, F> Push for F
where
    F: Fn(T, U) -> R + 'static,
    T: 'static,
    U: 'static,
    R: 'static,
{
    fn push(self, segment: &mut Segment) {
        segment.push_op2(self);
    }
}

// For three-argument functions:
impl<T, U, V, R, F> Push for F
where
    F: Fn(T, U, V) -> R + 'static,
    T: 'static,
    U: 'static,
    V: 'static,
    R: 'static,
{
    fn push(self, segment: &mut Segment) {
        segment.push_op3(self);
    }
}

pub struct Segment {
    ops: Vec<Operation>,
    storage: RawSequence,
    dropper: Vec<fn(&mut RawSequence, usize) -> usize>,
    type_ids: Vec<TypeId>,
}

impl Segment {
    pub fn new() -> Self {
        Segment {
            ops: Vec::new(),
            storage: RawSequence::new(),
            dropper: Vec::new(),
            type_ids: Vec::new(),
        }
    }

    fn pop_type<T>(&mut self)
    where
        T: 'static,
    {
        match self.type_ids.pop() {
            Some(tid) if tid == TypeId::of::<T>() => {}
            _ => {
                panic!("Type mismatch: expected {}", std::any::type_name::<T>());
            }
        }
    }

    fn push_storage<T>(&mut self, value: T)
    where
        T: 'static,
    {
        self.storage.push(value);
        self.dropper
            .push(|storage, p| unsafe { storage.drop_in_place::<T>(p) });
    }

    pub fn push_op<F>(&mut self, op: F)
    where
        F: Push + 'static,
    {
        f.push(self);
    }

    pub fn push_op0<R, F>(&mut self, op: F)
    where
        F: Fn() -> R + 'static,
        R: 'static,
    {
        self.push_storage(op);
        self.ops.push(|storage, p, stack| {
            let (f, r) = unsafe { storage.next::<F>(p) };
            stack.push(f());
            r
        });
        self.type_ids.push(TypeId::of::<R>());
    }

    pub fn push_op1<T, R, F>(&mut self, op: F)
    where
        F: Fn(T) -> R + 'static,
        T: 'static,
        R: 'static,
    {
        self.pop_type::<T>();
        self.push_storage(op);
        self.ops.push(|storage, p, stack| {
            let (f, r) = unsafe { storage.next::<F>(p) };
            let x: T = unsafe { stack.pop() };
            stack.push(f(x));
            r
        });
        self.type_ids.push(TypeId::of::<R>());
    }

    pub fn push_op2<T, U, R, F>(&mut self, op: F)
    where
        F: Fn(T, U) -> R + 'static,
        T: 'static,
        U: 'static,
        R: 'static,
    {
        self.pop_type::<U>();
        self.pop_type::<T>();
        self.push_storage(op);
        self.ops.push(|storage, p, stack| {
            let (f, r) = unsafe { storage.next::<F>(p) };
            let y: U = unsafe { stack.pop() };
            let x: T = unsafe { stack.pop() };
            stack.push(f(x, y));
            r
        });
        self.type_ids.push(TypeId::of::<R>());
    }

    pub fn push_op3<T, U, V, R, F>(&mut self, op: F)
    where
        F: Fn(T, U, V) -> R + 'static,
        T: 'static,
        U: 'static,
        V: 'static,
        R: 'static,
    {
        self.pop_type::<V>();
        self.pop_type::<U>();
        self.pop_type::<T>();
        self.push_storage(op);
        self.ops.push(|storage, p, stack| {
            let (f, r) = unsafe { storage.next::<F>(p) };
            let z: V = unsafe { stack.pop() };
            let y: U = unsafe { stack.pop() };
            let x: T = unsafe { stack.pop() };
            stack.push(f(x, y, z));
            r
        });
        self.type_ids.push(TypeId::of::<R>());
    }

    pub fn drop(&mut self) {
        let mut p = 0;
        for e in self.dropper.iter() {
            p = e(&mut self.storage, p);
        }
        assert!(self.storage.buffer.len() == 0, "Storage not empty");
    }

    pub fn run<T>(&mut self) -> T
    where
        T: 'static,
    {
        self.pop_type::<T>();
        if self.type_ids.len() != 0 {
            panic!("Value(s) left on execution stack");
        }

        let mut stack = RawStack::new();
        let mut p = 0;
        for op in self.ops.iter() {
            p = op(&self.storage, p, &mut stack);
        }
        unsafe { stack.pop() }
    }
}

fn main() {
    // Create a vector for stack operations.
    let mut operations = Segment::new();

    // Add a binary operation (addition).
    operations.push_op(|| -> u32 { 30 });
    operations.push_op(|| -> u32 { 12 });
    operations.push_op(|x: u32, y: u32| -> u32 { x + y });
    operations.push_op(|| -> u32 { 100 });
    operations.push_op(|| -> u32 { 10 });
    // Add a ternary operation (x + y - z).
    operations.push_op(|x: u32, y: u32, z: u32| -> u32 { x + y - z });
    operations.push_op(|x: u32| -> String { format!("result: {}", x.to_string()) });

    let final_result: String = operations.run();
    println!("{}", final_result);
}
