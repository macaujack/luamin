use std::{mem::MaybeUninit, ops::Deref};

use smallvec::SmallVec;
use smol_str::SmolStr;

use crate::runtime::{LuaminGas, LuaminValue};

mod private {
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct FuncLibHelper<T: ?Sized> {
        pub(crate) func_defs: T,
    }
}

pub type FuncLibArray<const N: usize, SLP, LE, X = ()> =
    private::FuncLibHelper<[FuncDef<SLP, LE, X>; N]>;
pub type FuncLib<SLP, LE, X = ()> = private::FuncLibHelper<[FuncDef<SLP, LE, X>]>;

#[derive(Debug)]
pub struct FuncLibBuf<SLP: FuncPointerStateLifetimeProvider, LE: 'static, X: 'static = ()> {
    func_defs: Box<[FuncDef<SLP, LE, X>]>,
}

impl<SLP: FuncPointerStateLifetimeProvider, LE: 'static, X: 'static> Deref
    for FuncLibBuf<SLP, LE, X>
{
    type Target = FuncLib<SLP, LE, X>;

    fn deref(&self) -> &Self::Target {
        unsafe { &*(self.func_defs.as_ref() as *const _ as *const Self::Target) }
    }
}

pub struct FuncDef<SLP: FuncPointerStateLifetimeProvider, LE: 'static, X: 'static = ()> {
    pub func_name: SmolStr,
    pub func_pointer: FuncPointer<SLP, LE>,
    pub extra: X,
}

impl<SLP: FuncPointerStateLifetimeProvider, LE: 'static, X: 'static> Clone for FuncDef<SLP, LE, X>
where
    X: Clone,
{
    fn clone(&self) -> Self {
        Self {
            func_name: self.func_name.clone(),
            func_pointer: self.func_pointer,
            extra: self.extra.clone(),
        }
    }
}

impl<SLP: FuncPointerStateLifetimeProvider, LE: 'static, X: std::fmt::Debug + 'static>
    std::fmt::Debug for FuncDef<SLP, LE, X>
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FuncDef")
            .field("func_name", &self.func_name)
            .field("func_pointer", &self.func_pointer)
            .field("extra", &self.extra)
            .finish()
    }
}

pub struct FuncDefStatic<SLP: FuncPointerStateLifetimeProvider, LE: 'static, X: 'static = ()> {
    pub func_name: &'static str,
    pub func_pointer: FuncPointer<SLP, LE>,
    pub extra: X,
}

impl<SLP: FuncPointerStateLifetimeProvider, LE: 'static, X: std::fmt::Debug + 'static>
    std::fmt::Debug for FuncDefStatic<SLP, LE, X>
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FuncDefStatic")
            .field("func_name", &self.func_name)
            .field("func_pointer", &self.func_pointer)
            .field("extra", &self.extra)
            .finish()
    }
}

impl<SLP: FuncPointerStateLifetimeProvider, LE: 'static, X: Clone + 'static> Clone
    for FuncDefStatic<SLP, LE, X>
{
    fn clone(&self) -> Self {
        Self {
            func_name: self.func_name,
            func_pointer: self.func_pointer,
            extra: self.extra.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct FuncPointerProperty {
    pub input_count: usize,
    pub output_count: usize,
    pub is_mutable: bool,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum FuncPointerCallError<LE> {
    #[error("Incorrect state mutability")]
    IncorrectStateMutability,
    #[error("Insufficient stack elements")]
    InsufficientLuaminValues,
    #[error("Library error: {0}")]
    LibError(Box<LE>),
}

pub trait FuncPointerCallStackOp<LE> {
    fn push_element(&mut self, element: LuaminValue);
    fn pop_element(&mut self) -> Option<LuaminValue>;
}

impl<LE> FuncPointerCallStackOp<LE> for Vec<LuaminValue> {
    fn push_element(&mut self, element: LuaminValue) {
        self.push(element);
    }

    fn pop_element(&mut self) -> Option<LuaminValue> {
        self.pop()
    }
}

impl<LE, A> FuncPointerCallStackOp<LE> for SmallVec<A>
where
    A: smallvec::Array<Item = LuaminValue>,
{
    fn push_element(&mut self, element: LuaminValue) {
        self.push(element);
    }
    fn pop_element(&mut self) -> Option<LuaminValue> {
        self.pop()
    }
}

#[derive(Debug, Clone)]
#[must_use = "Always check which variant is returned"]
pub enum GasError<E> {
    GasExceeded,
    Other(E),
}

impl<LE> From<GasError<LE>> for GasError<FuncPointerCallError<LE>> {
    fn from(value: GasError<LE>) -> Self {
        match value {
            GasError::GasExceeded => GasError::GasExceeded,
            GasError::Other(le) => GasError::Other(FuncPointerCallError::LibError(Box::new(le))),
        }
    }
}

luamin_macro::gen_func_pointer!(
    FuncPointer,
    LuaminValue,
    LuaminGas,
    GasError,
    FuncPointerProperty,
    FuncPointerCallError,
    FuncPointerCallStackOp,
    FuncPointerStateLifetimeProvider,
    4,
    16,
    4
);

#[macro_export]
#[doc(hidden)]
macro_rules! __consume_gas {
    ($gas:expr, $gas_cost:expr) => {
        if ($gas).consume($gas_cost) {
            return ::core::result::Result::Err(::luamin::func_lib::GasError::GasExceeded);
        }
    };
}

pub use crate::__consume_gas as consume_gas;

impl<SLP: FuncPointerStateLifetimeProvider, LE: 'static, X: 'static> FuncLibBuf<SLP, LE, X> {
    pub fn new<I>(func_defs: I) -> Option<FuncLibBuf<SLP, LE, X>>
    where
        I: IntoIterator<Item = FuncDef<SLP, LE, X>>,
    {
        let func_defs: Vec<FuncDef<SLP, LE, X>> = func_defs.into_iter().collect();
        let mut func_defs: Box<[FuncDef<SLP, LE, X>]> = func_defs.into();
        func_defs.sort_by(|a, b| a.func_name.cmp(&b.func_name));

        for i in 1..func_defs.len() {
            if func_defs[i - 1].func_name == func_defs[i].func_name {
                return None;
            }
        }

        Some(Self { func_defs })
    }
}

impl<const N: usize, SLP: FuncPointerStateLifetimeProvider, LE: 'static, X: 'static>
    FuncLibArray<N, SLP, LE, X>
{
    /// This function will panic if the input array has duplicated function names.
    pub const fn new_const(func_def_statics: [FuncDefStatic<SLP, LE, X>; N]) -> Self {
        const fn str_gt(a: &str, b: &str) -> bool {
            let a_bytes = a.as_bytes();
            let b_bytes = b.as_bytes();

            // Find the minimum length to safely loop without out-of-bounds errors
            let min_len = if a_bytes.len() < b_bytes.len() {
                a_bytes.len()
            } else {
                b_bytes.len()
            };

            let mut i = 0;
            while i < min_len {
                if a_bytes[i] > b_bytes[i] {
                    return true;
                } else if a_bytes[i] < b_bytes[i] {
                    return false;
                }
                i += 1;
            }

            // If all bytes match up to min_len, the longer string is lexicographically larger
            a_bytes.len() > b_bytes.len()
        }
        const fn str_eq(a: &str, b: &str) -> bool {
            let a_bytes = a.as_bytes();
            let b_bytes = b.as_bytes();
            if a_bytes.len() != b_bytes.len() {
                return false;
            }
            let len = a_bytes.len();
            let mut i = 0;
            while i < len {
                if a_bytes[i] != b_bytes[i] {
                    return false;
                }
                i += 1;
            }
            true
        }

        assert!(
            !std::mem::needs_drop::<X>(),
            "FuncDef extra type must not implement Drop"
        );

        let mut func_def_statics = func_def_statics;
        let mut i = 0;
        while i + 1 < N {
            let mut j = 0;
            while j + i + 1 < N {
                if str_gt(
                    func_def_statics[j].func_name,
                    func_def_statics[j + 1].func_name,
                ) {
                    func_def_statics.swap(j, j + 1);
                }
                j += 1;
            }
            i += 1;
        }

        i = 1;
        while i < N {
            assert!(
                !str_eq(
                    func_def_statics[i - 1].func_name,
                    func_def_statics[i].func_name
                ),
                "Duplicated function name",
            );
            i += 1;
        }

        let func_def_statics_ptr = func_def_statics.as_ptr();
        let mut func_defs = [const { MaybeUninit::uninit() }; N];
        i = 0;
        while i < N {
            unsafe {
                let func_def_static = func_def_statics_ptr.add(i);
                let func_name = std::ptr::addr_of!((*func_def_static).func_name);
                let func_pointer = std::ptr::addr_of!((*func_def_static).func_pointer);
                let extra = std::ptr::addr_of!((*func_def_static).extra);

                func_defs[i].write(FuncDef {
                    func_name: SmolStr::new_static(func_name.read()),
                    func_pointer: func_pointer.read(),
                    extra: extra.read(),
                });
            }
            i += 1;
        }
        std::mem::forget(func_def_statics);

        unsafe {
            let func_defs = &func_defs as *const _ as *const [FuncDef<SLP, LE, X>; N];
            let func_defs = func_defs.read();
            Self { func_defs }
        }
    }
}

impl<SLP: FuncPointerStateLifetimeProvider, LE: 'static, X: 'static> FuncLib<SLP, LE, X> {
    pub fn get_func_def(&self, func_name: &str) -> Option<(usize, &FuncDef<SLP, LE, X>)> {
        let index = self
            .func_defs
            .binary_search_by(|func_def| func_def.func_name.as_str().cmp(func_name))
            .ok()?;
        Some((index, &self.func_defs[index]))
    }

    pub fn kth_func_def(&self, kth: usize) -> Option<&FuncDef<SLP, LE, X>> {
        self.func_defs.get(kth)
    }
}

impl<SLP: FuncPointerStateLifetimeProvider, LE: 'static, X: 'static> FuncDef<SLP, LE, X>
where
    X: Default,
{
    pub fn new<FN>(func_name: FN, func_pointer: FuncPointer<SLP, LE>) -> Self
    where
        FN: AsRef<str>,
    {
        Self {
            func_name: SmolStr::new(func_name),
            func_pointer,
            extra: Default::default(),
        }
    }
}

impl<SLP: FuncPointerStateLifetimeProvider, LE: 'static, X: 'static> FuncDef<SLP, LE, X> {
    pub fn new_with_extra<FN>(func_name: FN, func_pointer: FuncPointer<SLP, LE>, extra: X) -> Self
    where
        FN: AsRef<str>,
    {
        Self {
            func_name: SmolStr::new(func_name),
            func_pointer,
            extra,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct State {
        score: i32,
    }

    struct StateLifetimeProvider;
    impl FuncPointerStateLifetimeProvider for StateLifetimeProvider {
        type StateWithLifetimes<'r0, 'r1, 'r2, 'r3> = State;
    }

    #[derive(Debug, Clone)]
    struct FuncDefExtra {
        extra: i32,
    }

    #[derive(Debug, thiserror::Error)]
    enum TestError {
        #[error("Test error")]
        TestError,
    }

    fn f00_mut(s: &mut State, gas: &mut LuaminGas) -> Result<(), GasError<TestError>> {
        consume_gas!(gas, 1);
        s.score += 1;
        Ok(())
    }

    fn f11(
        s: &State,
        gas: &mut LuaminGas,
        a: LuaminValue,
    ) -> Result<LuaminValue, GasError<TestError>> {
        consume_gas!(gas, 1);
        let a = match a {
            LuaminValue::Number(a) => a,
            _ => return Err(GasError::Other(TestError::TestError)),
        };
        Ok(LuaminValue::Number(s.score + a))
    }

    fn f22_mut(
        s: &mut State,
        gas: &mut LuaminGas,
        a: LuaminValue,
        b: LuaminValue,
    ) -> Result<(LuaminValue, LuaminValue), GasError<TestError>> {
        consume_gas!(gas, 1);
        let a = match a {
            LuaminValue::Number(a) => a,
            _ => panic!(),
        };
        let b = match b {
            LuaminValue::Number(b) => b,
            _ => panic!(),
        };
        s.score += a + b;
        Ok((LuaminValue::Number(a + b), LuaminValue::Number(a * b)))
    }

    #[test]
    fn test_func_lib() {
        let mut func_defs = [
            FuncDef::<StateLifetimeProvider, TestError, FuncDefExtra>::new_with_extra(
                "f00_mut",
                FuncPointer::<StateLifetimeProvider, TestError>::F00Mut(f00_mut),
                FuncDefExtra { extra: 0 },
            ),
            FuncDef::new_with_extra(
                "f22_mut",
                FuncPointer::<StateLifetimeProvider, TestError>::F22Mut(f22_mut),
                FuncDefExtra { extra: 2 },
            ),
            FuncDef::new_with_extra(
                "f11",
                FuncPointer::<StateLifetimeProvider, TestError>::F11(f11),
                FuncDefExtra { extra: 1 },
            ),
        ];
        let func_lib = FuncLibBuf::new(func_defs.clone()).unwrap();
        let (_, func_def) = func_lib.get_func_def("f00_mut").unwrap();
        assert_eq!(func_def.func_name, "f00_mut");
        assert_eq!(func_def.extra.extra, 0);
        let (_, func_def) = func_lib.get_func_def("f22_mut").unwrap();
        assert_eq!(func_def.func_name, "f22_mut");
        assert_eq!(func_def.extra.extra, 2);
        let (_, func_def) = func_lib.get_func_def("f11").unwrap();
        assert_eq!(func_def.func_name, "f11");
        assert_eq!(func_def.extra.extra, 1);
        assert!(func_lib.get_func_def("some_random_function_name").is_none());

        func_defs[0].func_name = SmolStr::new("f11");
        assert!(FuncLibBuf::new(func_defs).is_none());
    }
}
