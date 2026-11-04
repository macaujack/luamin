use std::{mem::MaybeUninit, ops::Deref};

use smallvec::SmallVec;
use smol_str::SmolStr;

use crate::runtime::{LuaminGas, LuaminValue};

mod private {
    /// Storage wrapper underlying fixed-size and borrowed function libraries.
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct HostFuncLibHelper<T: ?Sized> {
        pub(crate) func_defs: T,
    }
}

/// Fixed-size, name-sorted host function library suitable for constant construction.
///
/// `N` is the number of definitions, `StateProvider` supplies host state lifetimes,
/// `HostError` is the host error type, and `Extra` is per-function metadata.
/// Coerces to [`HostFuncLib`] for compilation and execution.
pub type HostFuncLibArray<const N: usize, StateProvider, HostError, Extra = ()> =
    private::HostFuncLibHelper<[HostFuncDef<StateProvider, HostError, Extra>; N]>;
/// Borrowed view of a name-sorted host function library.
///
/// Function indices refer to sorted positions, not the original input order.
/// `StateProvider` supplies host state lifetimes, `HostError` is the host error type, and
/// `Extra` is metadata carried without interpretation by the compiler/runtime.
pub type HostFuncLib<StateProvider, HostError, Extra = ()> =
    private::HostFuncLibHelper<[HostFuncDef<StateProvider, HostError, Extra>]>;

/// Heap-allocated, name-sorted library built from an iterator of definitions.
///
/// Owns a fixed set of definitions after construction; it does not expose a growable buffer.
/// Dereferences to [`HostFuncLib`]. Construction rejects duplicate names.
/// `StateProvider`, `HostError`, and `Extra` have the same roles as in [`HostFuncLib`].
#[derive(Debug)]
pub struct HostFuncLibBuf<StateProvider: HostStateProvider, HostError: 'static, Extra: 'static = ()>
{
    func_defs: Box<[HostFuncDef<StateProvider, HostError, Extra>]>,
}

impl<StateProvider: HostStateProvider, HostError: 'static, Extra: 'static> Deref
    for HostFuncLibBuf<StateProvider, HostError, Extra>
{
    type Target = HostFuncLib<StateProvider, HostError, Extra>;

    fn deref(&self) -> &Self::Target {
        // SAFETY: HostFuncLibHelper is repr(transparent) over the same slice type,
        // and the returned reference retains the slice metadata and self lifetime.
        unsafe { &*(self.func_defs.as_ref() as *const _ as *const Self::Target) }
    }
}

/// Named host function and application metadata.
///
/// `StateProvider` supplies state lifetimes, `HostError` is the host error type, and `Extra` is metadata.
pub struct HostFuncDef<StateProvider: HostStateProvider, HostError: 'static, Extra: 'static = ()> {
    /// Name used to resolve calls from script source.
    pub func_name: SmolStr,
    /// Dispatch pointer describing the host function signature and mutability.
    pub func_pointer: FuncPointer<StateProvider, HostError>,
    /// Application metadata; not interpreted by the compiler or runtime.
    pub extra: Extra,
}

impl<StateProvider: HostStateProvider, HostError: 'static, Extra: 'static> Clone
    for HostFuncDef<StateProvider, HostError, Extra>
where
    Extra: Clone,
{
    fn clone(&self) -> Self {
        Self {
            func_name: self.func_name.clone(),
            func_pointer: self.func_pointer,
            extra: self.extra.clone(),
        }
    }
}

impl<StateProvider: HostStateProvider, HostError: 'static, Extra: std::fmt::Debug + 'static>
    std::fmt::Debug for HostFuncDef<StateProvider, HostError, Extra>
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostFuncDef")
            .field("func_name", &self.func_name)
            .field("func_pointer", &self.func_pointer)
            .field("extra", &self.extra)
            .finish()
    }
}

/// Host function definition with a static name, used by `HostFuncLibArray::new`.
///
/// The generic parameters have the same meaning as in [`HostFuncDef`].
pub struct HostFuncDefStatic<
    StateProvider: HostStateProvider,
    HostError: 'static,
    Extra: 'static = (),
> {
    /// Name used to resolve calls from script source.
    pub func_name: &'static str,
    /// Dispatch pointer describing the host function signature and mutability.
    pub func_pointer: FuncPointer<StateProvider, HostError>,
    /// Application metadata; not interpreted by the compiler or runtime.
    pub extra: Extra,
}

impl<StateProvider: HostStateProvider, HostError: 'static, Extra: std::fmt::Debug + 'static>
    std::fmt::Debug for HostFuncDefStatic<StateProvider, HostError, Extra>
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostFuncDefStatic")
            .field("func_name", &self.func_name)
            .field("func_pointer", &self.func_pointer)
            .field("extra", &self.extra)
            .finish()
    }
}

impl<StateProvider: HostStateProvider, HostError: 'static, Extra: Clone + 'static> Clone
    for HostFuncDefStatic<StateProvider, HostError, Extra>
{
    fn clone(&self) -> Self {
        Self {
            func_name: self.func_name,
            func_pointer: self.func_pointer,
            extra: self.extra.clone(),
        }
    }
}

/// Script argument count, return count, and host state access required by a function.
#[derive(Debug, Clone)]
pub struct HostFuncSignature {
    /// Number of script values consumed by one call.
    pub input_count: usize,
    /// Number of script values produced by a successful call.
    pub output_count: usize,
    /// Whether the function requires exclusive host state access.
    pub is_mutable: bool,
}

/// Host function dispatch failure or an application-supplied error.
#[derive(Debug, Clone, thiserror::Error)]
pub enum FuncPointerCallError<HostError> {
    /// A mutable host function was called using shared state access.
    #[error("Incorrect state mutability")]
    IncorrectStateMutability,
    /// The stack does not contain all required script arguments.
    #[error("Insufficient stack elements")]
    InsufficientLuaminValues,
    /// Application error returned by the host function or argument conversion.
    #[error("Library error: {0}")]
    LibError(Box<HostError>),
}

/// Stack operations required by generated host function dispatch.
///
/// Arguments are popped from the end and results pushed in source order.
/// `Vec<LuaminValue>` and `SmallVec` implement this trait. `HostError` identifies
/// the host error type used by the surrounding function-pointer interface.
pub trait FuncPointerCallStackOp<HostError> {
    /// Pushes a value onto the top of the stack.
    fn push_element(&mut self, element: LuaminValue);
    /// Removes the top value, or returns `None` for an empty stack.
    fn pop_element(&mut self) -> Option<LuaminValue>;
}

impl<HostError> FuncPointerCallStackOp<HostError> for Vec<LuaminValue> {
    fn push_element(&mut self, element: LuaminValue) {
        self.push(element);
    }

    fn pop_element(&mut self) -> Option<LuaminValue> {
        self.pop()
    }
}

impl<HostError, A> FuncPointerCallStackOp<HostError> for SmallVec<A>
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

/// Distinguishes an exhausted gas budget from an operation-specific error.
#[derive(Debug, Clone, thiserror::Error)]
#[must_use = "Always check which variant is returned"]
pub enum GasError<E> {
    /// An excessive or invalid gas charge exhausted the budget.
    #[error("Gas budget exceeded")]
    GasExceeded,
    /// An operation failed for a reason other than gas exhaustion.
    #[error("{0}")]
    Other(#[source] E),
}

impl<HostError> From<GasError<HostError>> for GasError<FuncPointerCallError<HostError>> {
    fn from(value: GasError<HostError>) -> Self {
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
    HostFuncSignature,
    FuncPointerCallError,
    FuncPointerCallStackOp,
    HostStateProvider,
    4,
    16,
    4
);

/// Library construction failed because two host functions have the same name.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("Duplicate host function: {name}")]
pub struct DuplicateFunctionError {
    /// Duplicated host function name.
    pub name: SmolStr,
}

impl<StateProvider: HostStateProvider, HostError: 'static, Extra: 'static>
    HostFuncLibBuf<StateProvider, HostError, Extra>
{
    /// Collects definitions and sorts them lexicographically by name.
    ///
    /// # Errors
    ///
    /// Returns [`DuplicateFunctionError`] if any names are repeated.
    pub fn new<I>(
        func_defs: I,
    ) -> Result<HostFuncLibBuf<StateProvider, HostError, Extra>, DuplicateFunctionError>
    where
        I: IntoIterator<Item = HostFuncDef<StateProvider, HostError, Extra>>,
    {
        let func_defs: Vec<HostFuncDef<StateProvider, HostError, Extra>> =
            func_defs.into_iter().collect();
        let mut func_defs: Box<[HostFuncDef<StateProvider, HostError, Extra>]> = func_defs.into();
        func_defs.sort_by(|a, b| a.func_name.cmp(&b.func_name));

        for i in 1..func_defs.len() {
            if func_defs[i - 1].func_name == func_defs[i].func_name {
                return Err(DuplicateFunctionError {
                    name: func_defs[i].func_name.clone(),
                });
            }
        }

        Ok(Self { func_defs })
    }
}

impl<const N: usize, StateProvider: HostStateProvider, HostError: 'static, Extra: 'static>
    HostFuncLibArray<N, StateProvider, HostError, Extra>
{
    /// Constructs a fixed-size library, sorting definitions by name.
    ///
    /// Static names and metadata are retained. This constructor is usable in
    /// constant evaluation.
    ///
    /// # Panics
    ///
    /// Panics if two definitions have the same function name.
    pub const fn new(
        func_def_statics: [HostFuncDefStatic<StateProvider, HostError, Extra>; N],
    ) -> Self {
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
            !std::mem::needs_drop::<Extra>(),
            "HostFuncDef extra type must not implement Drop"
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
            // SAFETY: i < N. Every field is read once; the source array is
            // forgotten below, and Extra is checked not to need dropping.
            unsafe {
                let func_def_static = func_def_statics_ptr.add(i);
                let func_name = std::ptr::addr_of!((*func_def_static).func_name);
                let func_pointer = std::ptr::addr_of!((*func_def_static).func_pointer);
                let extra = std::ptr::addr_of!((*func_def_static).extra);

                func_defs[i].write(HostFuncDef {
                    func_name: SmolStr::new_static(func_name.read()),
                    func_pointer: func_pointer.read(),
                    extra: extra.read(),
                });
            }
            i += 1;
        }
        std::mem::forget(func_def_statics);

        // SAFETY: all N elements have been initialized; MaybeUninit<T> has
        // the layout of T, and the source storage does not drop its contents.
        unsafe {
            let func_defs =
                &func_defs as *const _ as *const [HostFuncDef<StateProvider, HostError, Extra>; N];
            let func_defs = func_defs.read();
            Self { func_defs }
        }
    }
}

impl<StateProvider: HostStateProvider, HostError: 'static, Extra: 'static>
    HostFuncLib<StateProvider, HostError, Extra>
{
    /// Looks up a function by name, returning its sorted index and definition.
    ///
    /// Returns `None` when the name is absent.
    pub fn find_by_name(
        &self,
        func_name: &str,
    ) -> Option<(usize, &HostFuncDef<StateProvider, HostError, Extra>)> {
        let index = self
            .func_defs
            .binary_search_by(|func_def| func_def.func_name.as_str().cmp(func_name))
            .ok()?;
        Some((index, &self.func_defs[index]))
    }

    /// Returns the definition at the zero-based sorted index, or `None` if out of bounds.
    pub fn get(&self, index: usize) -> Option<&HostFuncDef<StateProvider, HostError, Extra>> {
        self.func_defs.get(index)
    }
}

impl<StateProvider: HostStateProvider, HostError: 'static, Extra: 'static>
    HostFuncDef<StateProvider, HostError, Extra>
where
    Extra: Default,
{
    /// Creates a named definition using `Extra::default()` for its metadata.
    pub fn new<FN>(func_name: FN, func_pointer: FuncPointer<StateProvider, HostError>) -> Self
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

impl<StateProvider: HostStateProvider, HostError: 'static, Extra: 'static>
    HostFuncDef<StateProvider, HostError, Extra>
{
    /// Creates a named definition with explicit application metadata.
    pub fn new_with_extra<FN>(
        func_name: FN,
        func_pointer: FuncPointer<StateProvider, HostError>,
        extra: Extra,
    ) -> Self
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
mod tests;
