use std::{any::Any, cell::RefCell, rc::Rc};

use arrayvec::ArrayVec;
use smallvec::SmallVec;

use crate::runtime::{LuaminUserValue, LuaminValue};

#[cfg(test)]
mod tests;

/// Consumes a script value and attempts to convert it into a Rust value.
///
/// Built-in implementations cover integers, booleans, script values, optional
/// values, user values, and lists. `Option<T>` maps nil to `Some(None)`;
/// `None` as the outer result means conversion failed. Arrays require exactly
/// their declared length; `ArrayVec` rejects lists exceeding its capacity.
/// List collection conversions consume unshared storage or clone elements
/// from shared storage, and fail if the shared list cannot be borrowed.
pub trait TryFromLuamin
where
    Self: Sized,
{
    /// Returns the converted value, or `None` for an incompatible type, length, or list borrow.
    fn try_from_luamin(element: LuaminValue) -> Option<Self>;
}

/// Converts an owned Rust value into a script value.
///
/// `None` becomes nil. Arrays and collections become new shared lists;
/// slices clone their elements. Passing an existing script list preserves
/// its shared ownership. Custom user values are boxed.
pub trait IntoLuamin {
    /// Consumes this value and returns its script representation.
    ///
    /// ```rust
    /// use luamin::{IntoLuamin, TryFromLuamin};
    /// let value = vec![1_i32, 2, 3].into_luamin();
    /// assert_eq!(Vec::<i32>::try_from_luamin(value), Some(vec![1, 2, 3]));
    /// ```
    fn into_luamin(self) -> LuaminValue;
}

// MARK: i32

impl TryFromLuamin for i32 {
    fn try_from_luamin(element: LuaminValue) -> Option<Self> {
        match element {
            LuaminValue::Number(value) => Some(value),
            _ => None,
        }
    }
}

impl IntoLuamin for i32 {
    fn into_luamin(self) -> LuaminValue {
        LuaminValue::Number(self)
    }
}

// MARK: bool

impl TryFromLuamin for bool {
    fn try_from_luamin(element: LuaminValue) -> Option<Self> {
        match element {
            LuaminValue::Bool(value) => Some(value),
            _ => None,
        }
    }
}

impl IntoLuamin for bool {
    fn into_luamin(self) -> LuaminValue {
        LuaminValue::Bool(self)
    }
}

// MARK: LuaminValue

impl TryFromLuamin for LuaminValue {
    fn try_from_luamin(element: LuaminValue) -> Option<Self> {
        Some(element)
    }
}

impl IntoLuamin for LuaminValue {
    fn into_luamin(self) -> LuaminValue {
        self
    }
}

// MARK: Option

impl<T> TryFromLuamin for Option<T>
where
    T: TryFromLuamin,
{
    fn try_from_luamin(element: LuaminValue) -> Option<Self> {
        let value = match element {
            LuaminValue::Nil => None,
            element => Some(T::try_from_luamin(element)?),
        };
        Some(value)
    }
}

impl<T> IntoLuamin for Option<T>
where
    T: IntoLuamin,
{
    fn into_luamin(self) -> LuaminValue {
        match self {
            None => LuaminValue::Nil,
            Some(value) => value.into_luamin(),
        }
    }
}

// MARK: LuaminUserValue

impl TryFromLuamin for Box<dyn LuaminUserValue> {
    fn try_from_luamin(element: LuaminValue) -> Option<Self> {
        match element {
            LuaminValue::UserValue(value) => Some(value),
            _ => None,
        }
    }
}

impl IntoLuamin for Box<dyn LuaminUserValue> {
    fn into_luamin(self) -> LuaminValue {
        LuaminValue::UserValue(self)
    }
}

impl<T> TryFromLuamin for T
where
    T: LuaminUserValue + Any,
{
    fn try_from_luamin(element: LuaminValue) -> Option<Self> {
        match element {
            LuaminValue::UserValue(value) => {
                if let Ok(value) = (value as Box<dyn Any>).downcast::<T>() {
                    Some(*value)
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

impl<T> IntoLuamin for T
where
    T: LuaminUserValue,
{
    fn into_luamin(self) -> LuaminValue {
        let b: Box<dyn LuaminUserValue> = Box::new(self);
        LuaminValue::UserValue(b)
    }
}

// MARK: Rc List

impl TryFromLuamin for Rc<RefCell<Vec<LuaminValue>>> {
    fn try_from_luamin(element: LuaminValue) -> Option<Self> {
        match element {
            LuaminValue::List(list) => Some(list),
            _ => None,
        }
    }
}

impl IntoLuamin for Rc<RefCell<Vec<LuaminValue>>> {
    fn into_luamin(self) -> LuaminValue {
        LuaminValue::List(self)
    }
}

// MARK: Array

impl<T, const N: usize> TryFromLuamin for [T; N]
where
    T: TryFromLuamin + Sized,
{
    fn try_from_luamin(element: LuaminValue) -> Option<Self> {
        handle_list::<ArrayHandler<T, N>, _, _>(element)
    }
}

impl<T, const N: usize> IntoLuamin for [T; N]
where
    T: IntoLuamin + Sized,
{
    fn into_luamin(self) -> LuaminValue {
        LuaminValue::List(Rc::new(RefCell::new(
            self.into_iter().map(IntoLuamin::into_luamin).collect(),
        )))
    }
}

// MARK: ArrayVec

impl<T, const N: usize> TryFromLuamin for ArrayVec<T, N>
where
    T: TryFromLuamin,
{
    fn try_from_luamin(element: LuaminValue) -> Option<Self> {
        handle_list::<ArrayVecHandler<T, N>, _, _>(element)
    }
}

impl<T, const N: usize> IntoLuamin for ArrayVec<T, N>
where
    T: IntoLuamin,
{
    fn into_luamin(self) -> LuaminValue {
        LuaminValue::List(Rc::new(RefCell::new(
            self.into_iter().map(IntoLuamin::into_luamin).collect(),
        )))
    }
}

// MARK: Vec

impl<T> TryFromLuamin for Vec<T>
where
    T: TryFromLuamin,
{
    fn try_from_luamin(element: LuaminValue) -> Option<Self> {
        handle_list::<CollectHandler<Self>, _, _>(element)
    }
}

impl<T> IntoLuamin for Vec<T>
where
    T: IntoLuamin,
{
    fn into_luamin(self) -> LuaminValue {
        LuaminValue::List(Rc::new(RefCell::new(
            self.into_iter().map(IntoLuamin::into_luamin).collect(),
        )))
    }
}

// MARK: SmallVec

impl<T, A> TryFromLuamin for SmallVec<A>
where
    T: TryFromLuamin,
    A: smallvec::Array<Item = T>,
{
    fn try_from_luamin(element: LuaminValue) -> Option<Self> {
        handle_list::<CollectHandler<Self>, _, _>(element)
    }
}

impl<T, A> IntoLuamin for SmallVec<A>
where
    T: IntoLuamin,
    A: smallvec::Array<Item = T>,
{
    fn into_luamin(self) -> LuaminValue {
        LuaminValue::List(Rc::new(RefCell::new(
            self.into_iter().map(IntoLuamin::into_luamin).collect(),
        )))
    }
}

// MARK: Slice ref

impl<T> IntoLuamin for &[T]
where
    T: IntoLuamin + Clone,
{
    fn into_luamin(self) -> LuaminValue {
        LuaminValue::List(Rc::new(RefCell::new(
            self.iter().cloned().map(IntoLuamin::into_luamin).collect(),
        )))
    }
}

// MARK: Aux

trait IterHandlerAux<T, R> {
    fn handle_iter<I>(iter: I) -> Option<R>
    where
        I: ExactSizeIterator<Item = Option<T>>;
}

struct ArrayHandler<T, const N: usize>(std::marker::PhantomData<T>);

impl<T, const N: usize> IterHandlerAux<T, [T; N]> for ArrayHandler<T, N> {
    fn handle_iter<I>(iter: I) -> Option<[T; N]>
    where
        I: ExactSizeIterator<Item = Option<T>>,
    {
        if iter.len() != N {
            return None;
        }

        let mut values = ArrayVec::<T, N>::new();
        for value in iter {
            values.try_push(value?).ok()?;
        }
        values.into_inner().ok()
    }
}

struct ArrayVecHandler<T, const N: usize>(std::marker::PhantomData<T>);

impl<T, const N: usize> IterHandlerAux<T, ArrayVec<T, N>> for ArrayVecHandler<T, N> {
    fn handle_iter<I>(iter: I) -> Option<ArrayVec<T, N>>
    where
        I: ExactSizeIterator<Item = Option<T>>,
    {
        if iter.len() > N {
            return None;
        }
        let mut ret = ArrayVec::new();
        for value in iter {
            ret.push(value?);
        }
        Some(ret)
    }
}

struct CollectHandler<C>(std::marker::PhantomData<C>);

impl<C, T> IterHandlerAux<T, C> for CollectHandler<C>
where
    C: FromIterator<T>,
{
    fn handle_iter<I>(iter: I) -> Option<C>
    where
        I: ExactSizeIterator<Item = Option<T>>,
    {
        iter.collect()
    }
}

fn handle_list<H, T, R>(element: LuaminValue) -> Option<R>
where
    T: TryFromLuamin,
    H: IterHandlerAux<T, R>,
{
    match element {
        LuaminValue::List(list) => match Rc::try_unwrap(list) {
            Ok(list) => {
                let list = list.into_inner();
                H::handle_iter(list.into_iter().map(T::try_from_luamin))
            }
            Err(list) => {
                let list = list.try_borrow().ok()?;
                H::handle_iter(list.iter().cloned().map(T::try_from_luamin))
            }
        },
        _ => None,
    }
}
