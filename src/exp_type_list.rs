trait ListTraits {
    type Head;
    type Tail: ListTraits;

    const LENGTH: usize;

    type Concat<U: ListTraits + 'static>: ListTraits;
    fn concat<U: ListTraits + 'static>(self, other: U) -> Self::Concat<U>;

    fn head(&self) -> &Self::Head;
    fn tail(&self) -> &Self::Tail;

    fn append<U: 'static>(self, item: U) -> List<U, Self>;
}

struct EmptyList();

impl ListTraits for EmptyList {
    type Head = EmptyList;
    type Tail = EmptyList;

    const LENGTH: usize = 0;

    type Concat<U: ListTraits + 'static> = U;

    fn concat<U: ListTraits + 'static>(self, other: U) -> Self::Concat<U> {
        other
    }

    fn head(&self) -> &Self::Head {
        self
    }

    fn tail(&self) -> &Self::Tail {
        panic!("EmptyList has no tail")
    }

    fn append<U: 'static>(self, item: U) -> List<U, Self> {
        List(item, self)
    }
}

impl<H: 'static, T: ListTraits + 'static> ListTraits for List<H, T> {
    type Head = H;
    type Tail = T;

    const LENGTH: usize = 1 + T::LENGTH;

    type Concat<U: ListTraits + 'static> = List<H, T::Concat<U>>;

    fn concat<U: ListTraits + 'static>(self, other: U) -> Self::Concat<U> {
        List(self.0, self.1.concat(other))
    }

    fn head(&self) -> &Self::Head {
        &self.0
    }

    fn tail(&self) -> &Self::Tail {
        &self.1
    }

    fn append<U: 'static>(self, item: U) -> List<U, Self> {
        List(item, self)
    }
}

use std::fmt::*;

impl Debug for EmptyList {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(f, "()")
    }
}

impl<H: Debug, T: ListTraits + Debug> Debug for List<H, T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(f, "({:?}, {:?})", self.head(), self.tail())
    }
}

pub struct PrintFunction<L: Print + ListTraits>(pub fn(&mut &L) -> Option<PrintFunction<L>>);

trait Print {
    type AtResult<Index: Print>;
    fn at<Index: Print>(list: &impl ListTraits<Index>) -> &Self::AtResult<Index>;
    type PrintResult: Print;
    fn print<Index: Print>(&self) -> Option<PrintFunction<Self::PrintResult>>;
}

impl Print for EmptyList {
    type AtResult<H, T>
        = H
    where
        T: ListTraits;
    fn at<H, T>(list: &impl ListTraits<Head = H, Tail = T>) -> &Self::AtResult<H, T>
    where
        H: Display,
        T: ListTraits + Display + Print,
    {
        list.head()
    }
    type PrintResult = Self;
    fn print<H: Display, T: ListTraits + Display + Print>(
        &self,
    ) -> Option<PrintFunction<Self::PrintResult>> {
        println!("()");
        None
    }
}

impl<Head, Tail> Print for List<Head, Tail>
where
    Head: Display,
    Tail: ListTraits + Display + Print,
{
    type AtResult<H, T>
        = Tail::AtResult<T, T::Tail>
    where
        T: ListTraits;
    fn at<H, T>(list: &impl ListTraits<Head = H, Tail = T>) -> &Self::AtResult<H, T>
    where
        H: Display,
        T: ListTraits + Display + Print,
    {
        Tail::at(list.tail())
    }
    type PrintResult = List<Head, Tail>;
    fn print<H, T>(&self) -> Option<PrintFunction<Self::PrintResult>>
    where
        H: Display,
        T: ListTraits + Display + Print,
    {
        println!("{}", self.head());
        Some(PrintFunction(|list: &Self| {
            println!("{}", <List<H, T> as Print>::at(list));
            Self::print::<List<EmptyList, List<H, T>>>()
        }))
    }
}

struct List<H: 'static, T: ?Sized + ListTraits + 'static>(H, T);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_list() {
        let list = List(1, List(2, List(3, EmptyList()))).append(42.5);
        println!("{:?}", list);
        let list2 = List(42, List(3.5, List("Hello", EmptyList())));
        println!("{:?}", list2);
        let list3 = list.concat(list2);
        println!("{:?}", list3);
    }
}
