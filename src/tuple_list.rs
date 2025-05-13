use crate::list_traits::*;
use std::mem::offset_of;

impl<P: ListTypeProperty> ListTypeIteratorAdvance<P> for () {
    fn advancer<R: List>(_iter: &mut ListTypeIterator<R, P>) -> Option<P::Output> {
        None
    }
}

impl<P: ListTypeProperty, H: 'static, T: ListTypeIteratorAdvance<P>> ListTypeIteratorAdvance<P>
    for (H, T)
{
    fn advancer<R: List>(iter: &mut ListTypeIterator<R, P>) -> Option<P::Output> {
        iter.advance = T::advancer::<R>;
        Some(P::property::<(H, T)>())
    }
}

impl EmptyList for () {
    type PushFirst<U: 'static> = (U, ());
    fn push_first<U: 'static>(self, item: U) -> Self::PushFirst<U> {
        (item, ())
    }

    type FromTuple<T: IntoList> = T::Output<()>;
    fn from_tuple<T: IntoList>(tuple: T) -> Self::FromTuple<T> {
        tuple.into_list()
    }

    type Empty = Self;
    fn empty() -> Self::Empty {
        ()
    }
}

impl<H: 'static, T: List> List for (H, T) {
    type Empty = T::Empty;

    type Head = H;
    fn head(&self) -> &Self::Head {
        &self.0
    }

    type Tail = T;
    fn tail(&self) -> &Self::Tail {
        &self.1
    }

    const HEAD_PADDING: usize = 0; // undefined
    const HEAD_OFFSET: usize = offset_of!(Self, 0);

    type Push<U: 'static> = (U, Self);
    fn push<U: 'static>(self, item: U) -> Self::Push<U> {
        (item, self)
    }

    type Append<U: List> = <T::Append<U> as List>::Push<H>;
    fn append<U: List>(self, other: U) -> Self::Append<U> {
        self.1.append(other).push(self.0)
    }

    type ReverseOnto<U: List> = T::ReverseOnto<U::Push<H>>;
    fn reverse_onto<U: List>(self, other: U) -> Self::ReverseOnto<U> {
        self.1.reverse_onto(other.push(self.0))
    }

    type Reverse = Self::ReverseOnto<()>;
    fn reverse(self) -> Self::Reverse {
        self.reverse_onto(())
    }
}

//--------------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::any::TypeId;

    #[test]
    fn test_type_id_iterator() {
        let ids: [TypeId; 3] = [
            TypeId::of::<u32>(),
            TypeId::of::<f64>(),
            TypeId::of::<&str>(),
        ];
        assert!(TypeIdIterator::<(u32, (f64, (&str, ())))>::new().eq(ids.iter().map(|&id| id)));
    }

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
        assert_eq!(().push_first(1), (1, ()));
        assert_eq!((1, 2, 3).into_list::<()>().push(4), (4, (1, (2, (3, ())))));
    }

    #[test]
    fn test_concat() {
        assert_eq!(
            (1, 2, 3)
                .into_list::<()>()
                .append((4, 5, 6).into_list::<()>()),
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

        <(i32, f64, &str) as IntoList>::Output::<()>::for_each_type(&mut PrintTypeNames {
            count: 0,
        });
    }

    #[test]
    fn test_tuple_list() {
        let list = (1, 2.5, "Hello").into_list::<()>();
        println!("{:?}", list);
    }
}
