use std::{any::Any, cell::RefCell, mem::MaybeUninit, rc::Rc};

use arrayvec::ArrayVec;
use smallvec::SmallVec;

use crate::runtime::{LuaminUserValue, LuaminValue};

pub trait FromLuamin
where
    Self: Sized,
{
    fn from_luamin(element: LuaminValue) -> Option<Self>;
}

pub trait ToLuamin {
    fn to_luamin(self) -> LuaminValue;
}

// MARK: i32

impl FromLuamin for i32 {
    fn from_luamin(element: LuaminValue) -> Option<Self> {
        match element {
            LuaminValue::Number(value) => Some(value),
            _ => None,
        }
    }
}

impl ToLuamin for i32 {
    fn to_luamin(self) -> LuaminValue {
        LuaminValue::Number(self)
    }
}

// MARK: bool

impl FromLuamin for bool {
    fn from_luamin(element: LuaminValue) -> Option<Self> {
        match element {
            LuaminValue::Bool(value) => Some(value),
            _ => None,
        }
    }
}

impl ToLuamin for bool {
    fn to_luamin(self) -> LuaminValue {
        LuaminValue::Bool(self)
    }
}

// MARK: LuaminValue

impl FromLuamin for LuaminValue {
    fn from_luamin(element: LuaminValue) -> Option<Self> {
        Some(element)
    }
}

impl ToLuamin for LuaminValue {
    fn to_luamin(self) -> LuaminValue {
        self
    }
}

// MARK: Option

impl<T> FromLuamin for Option<T>
where
    T: FromLuamin,
{
    fn from_luamin(element: LuaminValue) -> Option<Self> {
        let value = match element {
            LuaminValue::Nil => None,
            element => Some(T::from_luamin(element)?),
        };
        Some(value)
    }
}

impl<T> ToLuamin for Option<T>
where
    T: ToLuamin,
{
    fn to_luamin(self) -> LuaminValue {
        match self {
            None => LuaminValue::Nil,
            Some(value) => value.to_luamin(),
        }
    }
}

// MARK: LuaminUserValue

impl FromLuamin for Box<dyn LuaminUserValue> {
    fn from_luamin(element: LuaminValue) -> Option<Self> {
        match element {
            LuaminValue::UserValue(value) => Some(value),
            _ => None,
        }
    }
}

impl ToLuamin for Box<dyn LuaminUserValue> {
    fn to_luamin(self) -> LuaminValue {
        LuaminValue::UserValue(self)
    }
}

impl<T> FromLuamin for T
where
    T: LuaminUserValue + Any,
{
    fn from_luamin(element: LuaminValue) -> Option<Self> {
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

impl<T> ToLuamin for T
where
    T: LuaminUserValue,
{
    fn to_luamin(self) -> LuaminValue {
        let b: Box<dyn LuaminUserValue> = Box::new(self);
        LuaminValue::UserValue(b)
    }
}

// MARK: Rc List

impl FromLuamin for Rc<RefCell<Vec<LuaminValue>>> {
    fn from_luamin(element: LuaminValue) -> Option<Self> {
        match element {
            LuaminValue::List(list) => Some(list),
            _ => None,
        }
    }
}

impl ToLuamin for Rc<RefCell<Vec<LuaminValue>>> {
    fn to_luamin(self) -> LuaminValue {
        LuaminValue::List(self)
    }
}

// MARK: Array

impl<T, const N: usize> FromLuamin for [T; N]
where
    T: FromLuamin + Sized,
{
    fn from_luamin(element: LuaminValue) -> Option<Self> {
        handle_list::<ArrayHandler<T, N>, _, _>(element)
    }
}

impl<T, const N: usize> ToLuamin for [T; N]
where
    T: ToLuamin + Sized,
{
    fn to_luamin(self) -> LuaminValue {
        LuaminValue::List(Rc::new(RefCell::new(
            self.into_iter().map(ToLuamin::to_luamin).collect(),
        )))
    }
}

// MARK: ArrayVec

impl<T, const N: usize> FromLuamin for ArrayVec<T, N>
where
    T: FromLuamin,
{
    fn from_luamin(element: LuaminValue) -> Option<Self> {
        handle_list::<ArrayVecHandler<T, N>, _, _>(element)
    }
}

impl<T, const N: usize> ToLuamin for ArrayVec<T, N>
where
    T: ToLuamin,
{
    fn to_luamin(self) -> LuaminValue {
        LuaminValue::List(Rc::new(RefCell::new(
            self.into_iter().map(ToLuamin::to_luamin).collect(),
        )))
    }
}

// MARK: Vec

impl<T> FromLuamin for Vec<T>
where
    T: FromLuamin,
{
    fn from_luamin(element: LuaminValue) -> Option<Self> {
        handle_list::<CollectHandler<Self>, _, _>(element)
    }
}

impl<T> ToLuamin for Vec<T>
where
    T: ToLuamin,
{
    fn to_luamin(self) -> LuaminValue {
        LuaminValue::List(Rc::new(RefCell::new(
            self.into_iter().map(ToLuamin::to_luamin).collect(),
        )))
    }
}

// MARK: SmallVec

impl<T, A> FromLuamin for SmallVec<A>
where
    T: FromLuamin,
    A: smallvec::Array<Item = T>,
{
    fn from_luamin(element: LuaminValue) -> Option<Self> {
        handle_list::<CollectHandler<Self>, _, _>(element)
    }
}

impl<T, A> ToLuamin for SmallVec<A>
where
    T: ToLuamin,
    A: smallvec::Array<Item = T>,
{
    fn to_luamin(self) -> LuaminValue {
        LuaminValue::List(Rc::new(RefCell::new(
            self.into_iter().map(ToLuamin::to_luamin).collect(),
        )))
    }
}

// MARK: Slice ref

impl<T> ToLuamin for &[T]
where
    T: ToLuamin + Clone,
{
    fn to_luamin(self) -> LuaminValue {
        LuaminValue::List(Rc::new(RefCell::new(
            self.iter().cloned().map(ToLuamin::to_luamin).collect(),
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

        let mut arr: [MaybeUninit<T>; N] = [const { MaybeUninit::uninit() }; N];

        for (i, value) in iter.enumerate() {
            match value {
                Some(value) => {
                    arr[i].write(value);
                }
                None => {
                    let initialized_slice =
                        std::ptr::slice_from_raw_parts_mut(arr.as_mut_ptr() as *mut T, i);
                    unsafe {
                        std::ptr::drop_in_place(initialized_slice);
                    }
                    return None;
                }
            }
        }

        unsafe {
            let ptr = &arr as *const _ as *const [T; N];
            Some(ptr.read())
        }
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
    T: FromLuamin,
    H: IterHandlerAux<T, R>,
{
    match element {
        LuaminValue::List(list) => match Rc::try_unwrap(list) {
            Ok(list) => {
                let list = list.into_inner();
                H::handle_iter(list.into_iter().map(T::from_luamin))
            }
            Err(list) => {
                let list = list.try_borrow().ok()?;
                H::handle_iter(list.iter().cloned().map(T::from_luamin))
            }
        },
        _ => None,
    }
}
