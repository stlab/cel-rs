pub trait TypeHandler {
    fn invoke<T>(self: &mut Self);
}

pub trait ValueHandler {
    fn invoke<T: 'static>(self: &mut Self, value: &T);
}

pub trait List {
    type Head: 'static;
    fn head(&self) -> &Self::Head;

    type Tail: List;
    fn tail(&self) -> &Self::Tail;

    const LENGTH: usize = 1 + Self::Tail::LENGTH;

    type PushFront<U: 'static>: List;
    fn push_front<U: 'static>(self, item: U) -> Self::PushFront<U>;

    type Concat<U: List>: List;
    fn concat<U: List>(self, other: U) -> Self::Concat<U>;

    type Reverse: List;
    fn reverse(self) -> Self::Reverse;

    fn for_each_type<H: TypeHandler>(self: &Self, handler: &mut H) {
        handler.invoke::<Self::Head>();
        self.tail().for_each_type(handler);
    }

    fn for_each_value<H: ValueHandler>(self: &Self, handler: &mut H) {
        handler.invoke(self.head());
        self.tail().for_each_value(handler);
    }
}

pub struct Bottom;
pub trait EmptyList {
    type AsList<U: 'static>: List;
    fn as_list<U: 'static>(self, item: U) -> Self::AsList<U>;

    type FromTuple<T: TupleTraits>: List;
    fn from_tuple<T: TupleTraits>(tuple: T) -> Self::FromTuple<T>;

    fn empty() -> Self;
}

impl<T: EmptyList> List for T {
    type Head = Bottom;
    type Tail = T; // Satisfy the List trait
    type PushFront<U: 'static> = T::AsList<U>;
    type Concat<U: List> = U;
    type Reverse = T;
    const LENGTH: usize = 0;

    fn head(&self) -> &Self::Head {
        unreachable!("EmptyList has no head")
    }

    fn tail(&self) -> &Self::Tail {
        unreachable!("EmptyList has no tail")
    }

    fn push_front<U>(self, item: U) -> Self::PushFront<U> {
        self.as_list(item)
    }

    fn concat<U: List>(self, other: U) -> Self::Concat<U> {
        other
    }

    fn reverse(self) -> Self::Reverse {
        self
    }

    fn for_each_type<H: TypeHandler>(self: &Self, _handler: &mut H) {}
    fn for_each_value<H: ValueHandler>(self: &Self, _handler: &mut H) {}
}

pub trait TupleTraits {
    type IntoList<T: EmptyList>: List;
    fn into_list<T: EmptyList>(self) -> Self::IntoList<T>;
}

impl TupleTraits for () {
    type IntoList<T: EmptyList> = T;
    fn into_list<T: EmptyList>(self) -> Self::IntoList<T> {
        T::empty()
    }
}

impl<A: 'static> TupleTraits for (A,) {
    type IntoList<T: EmptyList> = T::AsList<A>;
    fn into_list<T: EmptyList>(self) -> Self::IntoList<T> {
        T::empty().push_front(self.0)
    }
}

impl<A: 'static, B: 'static> TupleTraits for (A, B) {
    type IntoList<T: EmptyList> = <T::AsList<B> as List>::PushFront<A>;
    fn into_list<T: EmptyList>(self) -> Self::IntoList<T> {
        T::empty().push_front(self.1).push_front(self.0)
    }
}

impl<A: 'static, B: 'static, C: 'static> TupleTraits for (A, B, C) {
    type IntoList<T: EmptyList> = <<T::AsList<C> as List>::PushFront<B> as List>::PushFront<A>;
    fn into_list<T: EmptyList>(self) -> Self::IntoList<T> {
        T::empty()
            .push_front(self.2)
            .push_front(self.1)
            .push_front(self.0)
    }
}
impl<A: 'static, B: 'static, C: 'static, D: 'static> TupleTraits for (A, B, C, D) {
    type IntoList<T: EmptyList> =
        <<<T::AsList<D> as List>::PushFront<C> as List>::PushFront<B> as List>::PushFront<A>;
    fn into_list<T: EmptyList>(self) -> Self::IntoList<T> {
        T::empty()
            .push_front(self.3)
            .push_front(self.2)
            .push_front(self.1)
            .push_front(self.0)
    }
}

impl<A: 'static, B: 'static, C: 'static, D: 'static, E: 'static> TupleTraits for (A, B, C, D, E) {
    type IntoList<T: EmptyList> = <<<<T::AsList<E> as List>::PushFront<D> as List>::PushFront<
        C,
    > as List>::PushFront<B> as List>::PushFront<A>;
    fn into_list<T: EmptyList>(self) -> Self::IntoList<T> {
        T::empty()
            .push_front(self.4)
            .push_front(self.3)
            .push_front(self.2)
            .push_front(self.1)
            .push_front(self.0)
    }
}

impl<A: 'static, B: 'static, C: 'static, D: 'static, E: 'static, F: 'static> TupleTraits
    for (A, B, C, D, E, F)
{
    type IntoList<T: EmptyList> = <<<<<T::AsList<F> as List>::PushFront<E> as List>::PushFront<
        D,
    > as List>::PushFront<C> as List>::PushFront<B> as List>::PushFront<A>;
    fn into_list<T: EmptyList>(self) -> Self::IntoList<T> {
        T::empty()
            .push_front(self.5)
            .push_front(self.4)
            .push_front(self.3)
            .push_front(self.2)
            .push_front(self.1)
            .push_front(self.0)
    }
}

impl<A: 'static, B: 'static, C: 'static, D: 'static, E: 'static, F: 'static, G: 'static> TupleTraits
    for (A, B, C, D, E, F, G)
{
    type IntoList<T: EmptyList> = <<<<<<T::AsList<G> as List>::PushFront<F> as List>::PushFront<
        E,
    > as List>::PushFront<D> as List>::PushFront<C> as List>::PushFront<B> as List>::PushFront<A>;
    fn into_list<T: EmptyList>(self) -> Self::IntoList<T> {
        T::empty()
            .push_front(self.6)
            .push_front(self.5)
            .push_front(self.4)
            .push_front(self.3)
            .push_front(self.2)
            .push_front(self.1)
            .push_front(self.0)
    }
}

impl<A: 'static, B: 'static, C: 'static, D: 'static, E: 'static, F: 'static, G: 'static, H: 'static>
    TupleTraits for (A, B, C, D, E, F, G, H)
{
    type IntoList<T: EmptyList> =
        <<<<<<<T::AsList<H> as List>::PushFront<G> as List>::PushFront<F> as List>::PushFront<
            E,
        > as List>::PushFront<D> as List>::PushFront<C> as List>::PushFront<B> as List>::PushFront<
            A,
        >;
    fn into_list<T: EmptyList>(self) -> Self::IntoList<T> {
        T::empty()
            .push_front(self.7)
            .push_front(self.6)
            .push_front(self.5)
            .push_front(self.4)
            .push_front(self.3)
            .push_front(self.2)
            .push_front(self.1)
            .push_front(self.0)
    }
}

impl<
    A: 'static,
    B: 'static,
    C: 'static,
    D: 'static,
    E: 'static,
    F: 'static,
    G: 'static,
    H: 'static,
    I: 'static,
> TupleTraits for (A, B, C, D, E, F, G, H, I)
{
    type IntoList<T: EmptyList> =
        <<<<<<<<T::AsList<I> as List>::PushFront<H> as List>::PushFront<G> as List>::PushFront<
            F,
        > as List>::PushFront<E> as List>::PushFront<D> as List>::PushFront<C> as List>::PushFront<
            B,
        > as List>::PushFront<A>;
    fn into_list<T: EmptyList>(self) -> Self::IntoList<T> {
        T::empty()
            .push_front(self.8)
            .push_front(self.7)
            .push_front(self.6)
            .push_front(self.5)
            .push_front(self.4)
            .push_front(self.3)
            .push_front(self.2)
            .push_front(self.1)
            .push_front(self.0)
    }
}

impl<
    A: 'static,
    B: 'static,
    C: 'static,
    D: 'static,
    E: 'static,
    F: 'static,
    G: 'static,
    H: 'static,
    I: 'static,
    J: 'static,
> TupleTraits for (A, B, C, D, E, F, G, H, I, J)
{
    type IntoList<T: EmptyList> =
        <<<<<<<<<T::AsList<J> as List>::PushFront<I> as List>::PushFront<H> as List>::PushFront<
            G,
        > as List>::PushFront<F> as List>::PushFront<E> as List>::PushFront<D> as List>::PushFront<
            C,
        > as List>::PushFront<B> as List>::PushFront<A>;
    fn into_list<T: EmptyList>(self) -> Self::IntoList<T> {
        T::empty()
            .push_front(self.9)
            .push_front(self.8)
            .push_front(self.7)
            .push_front(self.6)
            .push_front(self.5)
            .push_front(self.4)
            .push_front(self.3)
            .push_front(self.2)
            .push_front(self.1)
            .push_front(self.0)
    }
}

impl<
    A: 'static,
    B: 'static,
    C: 'static,
    D: 'static,
    E: 'static,
    F: 'static,
    G: 'static,
    H: 'static,
    I: 'static,
    J: 'static,
    K: 'static,
> TupleTraits for (A, B, C, D, E, F, G, H, I, J, K)
{
    type IntoList<T: EmptyList> =
        <<<<<<<<<<T::AsList<K> as List>::PushFront<J> as List>::PushFront<I> as List>::PushFront<
            H,
        > as List>::PushFront<G> as List>::PushFront<F> as List>::PushFront<E> as List>::PushFront<
            D,
        > as List>::PushFront<C> as List>::PushFront<B> as List>::PushFront<A>;
    fn into_list<T: EmptyList>(self) -> Self::IntoList<T> {
        T::empty()
            .push_front(self.10)
            .push_front(self.9)
            .push_front(self.8)
            .push_front(self.7)
            .push_front(self.6)
            .push_front(self.5)
            .push_front(self.4)
            .push_front(self.3)
            .push_front(self.2)
            .push_front(self.1)
            .push_front(self.0)
    }
}

impl<
    A: 'static,
    B: 'static,
    C: 'static,
    D: 'static,
    E: 'static,
    F: 'static,
    G: 'static,
    H: 'static,
    I: 'static,
    J: 'static,
    K: 'static,
    L: 'static,
> TupleTraits for (A, B, C, D, E, F, G, H, I, J, K, L)
{
    type IntoList<T: EmptyList> =
        <<<<<<<<<<<T::AsList<L> as List>::PushFront<K> as List>::PushFront<J> as List>::PushFront<
            I,
        > as List>::PushFront<H> as List>::PushFront<G> as List>::PushFront<F> as List>::PushFront<
            E,
        > as List>::PushFront<D> as List>::PushFront<C> as List>::PushFront<B> as List>::PushFront<
            A,
        >;
    fn into_list<T: EmptyList>(self) -> Self::IntoList<T> {
        T::empty()
            .push_front(self.11)
            .push_front(self.10)
            .push_front(self.9)
            .push_front(self.8)
            .push_front(self.7)
            .push_front(self.6)
            .push_front(self.5)
            .push_front(self.4)
            .push_front(self.3)
            .push_front(self.2)
            .push_front(self.1)
            .push_front(self.0)
    }
}

impl EmptyList for () {
    type AsList<U: 'static> = (U, ());
    fn as_list<U: 'static>(self, item: U) -> Self::AsList<U> {
        (item, ())
    }

    type FromTuple<T: TupleTraits> = T::IntoList<()>;
    fn from_tuple<T: TupleTraits>(tuple: T) -> Self::FromTuple<T> {
        tuple.into_list()
    }

    fn empty() -> Self {
        ()
    }
}

impl<H: 'static, T: List> List for (H, T) {
    type Head = H;
    fn head(&self) -> &Self::Head {
        &self.0
    }

    type Tail = T;
    fn tail(&self) -> &Self::Tail {
        &self.1
    }

    type PushFront<U: 'static> = (U, Self);
    fn push_front<U: 'static>(self, item: U) -> Self::PushFront<U> {
        (item, self)
    }

    type Concat<U: List> = <T::Concat<U> as List>::PushFront<H>;
    fn concat<U: List>(self, other: U) -> Self::Concat<U> {
        self.1.concat(other).push_front(self.0)
    }

    type Reverse = <T::Reverse as List>::Concat<(H, ())>;
    fn reverse(self) -> Self::Reverse {
        self.1.reverse().concat((self.0, ()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_empty_list() {
        assert_eq!(<()>::empty(), ());
    }

    #[test]
    fn test_into_list() {
        assert_eq!(().into_list::<()>(), ());
        assert_eq!((1, 2, 3).into_list::<()>(), (1, (2, (3, ()))));
        assert_eq!(
            (1, 2.5, "Hello").into_list::<()>(),
            (1, (2.5, ("Hello", ())))
        );
    }

    #[test]
    fn test_push_front() {
        assert_eq!(().push_front(1), (1, ()));
        assert_eq!(
            (1, 2, 3).into_list::<()>().push_front(4),
            (4, (1, (2, (3, ()))))
        );
    }

    #[test]
    fn test_concat() {
        assert_eq!(
            (1, 2, 3)
                .into_list::<()>()
                .concat((4, 5, 6).into_list::<()>()),
            (1, (2, (3, (4, (5, (6, ()))))))
        );
    }

    #[test]
    fn test_reverse() {
        assert_eq!((1, 2, 3).into_list::<()>().reverse(), (3, (2, (1, ()))));
    }

    #[test]
    fn test_list_length() {
        assert_eq!(<()>::LENGTH, 0);
        assert_eq!(<(i32, ())>::LENGTH, 1);
        assert_eq!(<(i32, (f64, ()))>::LENGTH, 2);
    }

    #[test]
    fn test_for_each_type() {
        struct PrintTypeNames {
            count: usize,
        }

        impl TypeHandler for PrintTypeNames {
            fn invoke<T>(self: &mut Self) {
                println!("{}: {}", self.count, std::any::type_name::<T>());
                self.count += 1;
            }
        }

        (1, 2.5, "Hello")
            .into_list::<()>()
            .for_each_type(&mut PrintTypeNames { count: 0 });
    }

    #[test]
    fn test_for_each_value() {
        use std::any::Any;
        struct Log {
            output: String,
        }

        impl ValueHandler for Log {
            fn invoke<T: 'static>(self: &mut Self, value: &T) {
                let value_any = value as &dyn Any;
                if let Some(i) = value_any.downcast_ref::<i32>() {
                    self.output.push_str(&format!("{}: i32\n", i));
                } else if let Some(f) = value_any.downcast_ref::<f64>() {
                    self.output.push_str(&format!("{}: f64\n", f));
                } else if let Some(s) = value_any.downcast_ref::<&str>() {
                    self.output.push_str(&format!("\"{}\": str\n", s));
                } else {
                    self.output.push_str("unknown: unknown\n");
                }
            }
        }

        let mut collector = Log {
            output: String::new(),
        };
        (1, 2.5, "Hello")
            .into_list::<()>()
            .for_each_value(&mut collector);

        assert_eq!(collector.output, "1: i32\n2.5: f64\n\"Hello\": str\n");
    }

    #[test]
    fn test_tuple_list() {
        let list = (1, 2.5, "Hello").into_list::<()>();
        println!("{:?}", list);
    }
    /*
    #[test]
    fn custom_list_type() {
        use std::fmt::*;

        pub trait DisplayableList: List + Display {}

        struct EmptyDisplayList();

        impl Display for EmptyDisplayList {
            fn fmt(&self, f: &mut Formatter<'_>) -> Result {
                write!(f, "()")
            }
        }
        impl EmptyList for EmptyDisplayList {
            type AsList<U: 'static> = DisplayList<U, EmptyDisplayList>;
            fn as_list<U: 'static>(self, item: U) -> Self::AsList<U> {
                DisplayList(item, EmptyDisplayList())
            }

            type FromTuple<T: TupleTraits> = T::IntoList<EmptyDisplayList>;
            fn from_tuple<T: TupleTraits>(tuple: T) -> Self::FromTuple<T> {
                tuple.into_list()
            }

            fn empty() -> Self {
                EmptyDisplayList()
            }
        }

        struct DisplayList<H: 'static + Display, T: DisplayableList>(H, T);

        impl<H: 'static + Display, T: DisplayableList> Display for DisplayList<H, T> {
            fn fmt(&self, f: &mut Formatter<'_>) -> Result {
                write!(f, "({}, {})", self.0, self.1)
            }
        }

        impl<H: 'static + Display, T: DisplayableList> List for DisplayList<H, T> {
            type Head = H;
            fn head(&self) -> &Self::Head {
                &self.0
            }

            type Tail = T;
            fn tail(&self) -> &Self::Tail {
                &self.1
            }

            type PushFront<U: 'static> = DisplayList<U, Self>;
            fn push_front<U: 'static>(self, item: U) -> Self::PushFront<U> {
                DisplayList(item, self)
            }

            type Concat<U: List> = <T::Concat<U> as List>::PushFront<H>;
            fn concat<U: List>(self, other: U) -> Self::Concat<U> {
                self.1.concat(other).push_front(self.0)
            }

            type Reverse = <T::Reverse as List>::Concat<(H, ())>;
            fn reverse(self) -> Self::Reverse {
                self.1.reverse().concat((self.0, ()))
            }
        }

        impl<H, T> DisplayableList for DisplayList<H, T>
        where
            H: 'static + Display,
            T: DisplayableList,
        {
        }

        let list = (1, 2.5, "Hello").into_list::<EmptyDisplayList>();
        println!("{}", list);
    } */
}
