use std::{cell::RefCell, rc::Rc};

use dyn_clone::DynClone;
pub use luamin_macro::LuaminUserValue;
use smallvec::SmallVec;

use crate::{
    MAX_RUNTIME_STACK_SIZE,
    codegen::{LuaminScript, LuaminScriptMut},
    component::Ix,
    func_lib::{
        FuncPointerCallError, FuncPointerStateLifetimeProvider, GasError, HostFuncDef, HostFuncLib,
    },
};

// MARK: Gas CFG

const GAS_COST_NOP: i32 = 1;
const GAS_COST_BASIC_SCALAR: i32 = 1;
const GAS_COST_LIST_CREATION: i32 = 1;
const GAS_COST_PER_LIST_CREATION_ITEM: i32 = 1;
const GAS_COST_PER_POPPED_ITEM: i32 = 1;
const GAS_COST_PER_REMOVED_ITEM: i32 = 1;
const GAS_COST_SET: i32 = 1;
const GAS_COST_PER_PUSHED_ITEM: i32 = 1;
const GAS_COST_JUMP: i32 = 1;
const GAS_COST_ARITH_BINARY: i32 = 3;
const GAS_COST_ARITH_UNARY: i32 = 2;

// MARK: LuaminValue

const LIKELY_STACK_CAP: usize = 32;
/// Maximum list length produced by script list assignment (65,536 elements).
///
/// Host-provided lists are not validated against this limit on conversion.
pub const MAX_LIST_LEN: usize = 65_536;

/// Dynamically typed value stored on the script stack.
///
/// Cloning a list shares its storage; cloning a user value clones the boxed
/// object. Lists compare by identity, user values by concrete type and equality,
/// and scalar values by value. Only nil and false are falsy; zero, empty lists,
/// and all user values are truthy. Debug formatting handles cycles and active borrows.
#[derive(Clone)]
pub enum LuaminValue {
    /// Absence of a value; also used for missing parameters and list elements.
    Nil,
    /// Signed 32-bit integer, also used for hash-string literals.
    Number(i32),
    /// Boolean value.
    Bool(bool),
    /// Shared, mutable list with identity equality.
    ///
    /// Cloning preserves aliases. Reference cycles must be broken by the
    /// embedding application to reclaim their storage.
    List(Rc<RefCell<Vec<LuaminValue>>>),
    /// Application-defined, clonable value with dynamic equality and gas costs.
    UserValue(Box<dyn LuaminUserValue>),
}

impl PartialEq for LuaminValue {
    fn eq(&self, other: &Self) -> bool {
        use LuaminValue::*;
        match (self, other) {
            (Nil, Nil) => true,
            (Number(a), Number(b)) => a == b,
            (Bool(a), Bool(b)) => a == b,
            (List(a), List(b)) => Rc::ptr_eq(a, b),
            (UserValue(a), UserValue(b)) => a == b,
            _ => false,
        }
    }
}

impl std::fmt::Debug for LuaminValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fmt_value(self, f, &mut Vec::new())
    }
}

fn fmt_value(
    value: &LuaminValue,
    f: &mut std::fmt::Formatter<'_>,
    path: &mut Vec<*const RefCell<Vec<LuaminValue>>>,
) -> std::fmt::Result {
    match value {
        LuaminValue::Nil => f.write_str("nil"),
        LuaminValue::Number(n) => write!(f, "{n}"),
        LuaminValue::Bool(b) => write!(f, "{b}"),
        LuaminValue::UserValue(v) => write!(f, "{v:?}"),
        LuaminValue::List(list) => {
            let ptr = Rc::as_ptr(list);
            if path.contains(&ptr) {
                return f.write_str("<cycle>");
            }
            if path.len() >= 32 {
                return f.write_str("<depth limit>");
            }
            let Ok(list) = list.try_borrow() else {
                return f.write_str("<borrowed>");
            };
            path.push(ptr);
            f.write_str("{")?;
            for (i, value) in list.iter().enumerate() {
                if i != 0 {
                    f.write_str(", ")?;
                }
                fmt_value(value, f, path)?;
            }
            path.pop();
            f.write_str("}")
        }
    }
}

/// Remaining gas budget for script instructions and host calls.
///
/// The default budget is zero. A failed charge marks the budget exhausted;
/// subsequent charges fail even if they cost zero. Reaching exactly zero is
/// allowed. Display prints the remaining budget, or -1 after exhaustion.
/// Gas bounds interpreter work; host functions must charge for their own work.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct LuaminGas(i32);

impl std::fmt::Display for LuaminGas {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl LuaminGas {
    /// Large finite budget of one billion units; this is not unlimited gas.
    pub const INF: Self = Self(1_000_000_000);

    /// Creates a budget; a negative amount creates an already-exhausted budget.
    pub const fn new(gas: i32) -> Self {
        Self(if gas < 0 { -1 } else { gas })
    }

    /// Charges `gas_cost` and returns whether the budget has been exceeded.
    ///
    /// A negative cost, an already-exhausted budget, or a cost exceeding the
    /// remaining budget marks the budget exhausted. Charging the exact remaining
    /// amount succeeds. Callers must stop work when this returns true.
    ///
    /// `try_consume` provides the same operation as a result.
    ///
    /// ```rust
    /// use luamin::LuaminGas;
    /// let mut gas = LuaminGas::new(2);
    /// assert!(!gas.consume(2));
    /// assert!(gas.consume(1));
    /// assert!(gas.has_exceeded());
    /// ```
    #[must_use]
    pub const fn consume(&mut self, gas_cost: i32) -> bool {
        if gas_cost < 0 || self.0 < 0 || gas_cost > self.0 {
            self.0 = -1;
        } else {
            self.0 -= gas_cost;
        }
        self.has_exceeded()
    }

    /// Returns whether a failed or invalid charge has exhausted this budget.
    pub const fn has_exceeded(&self) -> bool {
        self.0 < 0
    }

    /// Charges gas, returning [`GasError::GasExceeded`] on an invalid or excessive charge.
    ///
    /// Has the same sticky exhaustion behavior as [`Self::consume`]. The generic
    /// `E` allows the error to be used directly in the caller's result type.
    ///
    /// Use `?` to stop work immediately when a charge fails.
    ///
    /// ```rust
    /// use luamin::{GasError, LuaminGas};
    /// fn charge(gas: &mut LuaminGas) -> Result<(), GasError<()>> {
    ///     gas.try_consume(2)?;
    ///     Ok(())
    /// }
    /// assert!(charge(&mut LuaminGas::new(2)).is_ok());
    /// assert!(matches!(
    ///     charge(&mut LuaminGas::new(1)),
    ///     Err(GasError::GasExceeded)
    /// ));
    /// ```
    pub const fn try_consume<E>(&mut self, gas_cost: i32) -> Result<(), GasError<E>> {
        if self.consume(gas_cost) {
            Err(GasError::GasExceeded)
        } else {
            Ok(())
        }
    }

    const fn consume_priv<LE>(
        &mut self,
        gas_cost: i32,
    ) -> Result<(), GasError<LuaminRuntimeError<LE>>> {
        if self.consume(gas_cost) {
            Err(GasError::GasExceeded)
        } else {
            Ok(())
        }
    }
}

// MARK: LuaminUserValue

mod dyn_partial_eq {
    use super::PartialEqGasCost;
    use std::any::Any;
    pub trait DynPartialEq: Any {
        fn dyn_eq(&self, other: &dyn DynPartialEq) -> bool;
    }

    impl<T> DynPartialEq for T
    where
        T: PartialEq + Any,
    {
        fn dyn_eq(&self, other: &dyn DynPartialEq) -> bool {
            if let Some(other) = (other as &dyn Any).downcast_ref::<Self>() {
                self == other
            } else {
                false
            }
        }
    }

    impl PartialEq for dyn DynPartialEq {
        fn eq(&self, other: &Self) -> bool {
            self.dyn_eq(other)
        }
    }

    pub trait DynPartialEqGasCost: Any {
        fn dyn_partial_eq_gas_cost(&self, other: &dyn DynPartialEqGasCost) -> i32;
    }

    impl<T> DynPartialEqGasCost for T
    where
        T: PartialEqGasCost + Any,
    {
        fn dyn_partial_eq_gas_cost(&self, other: &dyn DynPartialEqGasCost) -> i32 {
            if let Some(other) = (other as &dyn Any).downcast_ref::<Self>() {
                self.partial_eq_gas_cost(other)
            } else {
                0
            }
        }
    }

    impl PartialEqGasCost for dyn DynPartialEqGasCost {
        fn partial_eq_gas_cost(&self, other: &Self) -> i32 {
            self.dyn_partial_eq_gas_cost(other)
        }
    }
}

/// Gas cost of cloning a value during script execution.
///
/// Costs must be nonnegative. `Copy` types have a blanket implementation based
/// on their size; other user values must provide an implementation. Calling
/// `Clone::clone` directly does not automatically charge this cost.
pub trait CloneGasCost {
    /// Returns the nonnegative charge for cloning this value.
    fn clone_gas_cost(&self) -> i32;
}

/// Additional gas cost of comparing two values for equality.
///
/// `Copy` types have a blanket implementation based on their size. Other user
/// values must provide an implementation. Direct Rust equality does not charge gas.
pub trait PartialEqGasCost {
    /// Returns the nonnegative additional comparison charge for these values.
    fn partial_eq_gas_cost(&self, other: &Self) -> i32;
}

impl<T> CloneGasCost for T
where
    T: Copy,
{
    fn clone_gas_cost(&self) -> i32 {
        let size = std::cmp::max(std::mem::size_of::<T>(), std::mem::size_of::<LuaminValue>());
        size.div_ceil(32) as i32
    }
}

impl<T> PartialEqGasCost for T
where
    T: Copy,
{
    fn partial_eq_gas_cost(&self, _other: &Self) -> i32 {
        self.clone_gas_cost() - 1
    }
}

impl CloneGasCost for LuaminValue {
    fn clone_gas_cost(&self) -> i32 {
        match self {
            LuaminValue::Nil
            | LuaminValue::Number(_)
            | LuaminValue::Bool(_)
            // Note when cloning a list, we are essentially cloning the pointer.
            | LuaminValue::List(_) => GAS_COST_BASIC_SCALAR,
            LuaminValue::UserValue(value) => value.clone_gas_cost(),
        }
    }
}

impl PartialEqGasCost for LuaminValue {
    fn partial_eq_gas_cost(&self, other: &Self) -> i32 {
        use LuaminValue::*;
        match (self, other) {
            (Nil, Nil) | (Number(_), Number(_)) | (Bool(_), Bool(_)) | (List(_), List(_)) => 0,
            (UserValue(a), UserValue(b)) => a.dyn_partial_eq_gas_cost(&**b),
            _ => 0,
        }
    }
}

/// Application-defined value that can be boxed in [`LuaminValue::UserValue`].
///
/// Implementors must be `Clone + Debug + PartialEq + 'static` and supply clone
/// and equality gas costs. Dynamic comparisons of different concrete types are
/// false. The `LuaminUserValue` derive supplies this marker implementation,
/// not the required cloning, equality, debugging, or gas-cost implementations.
pub trait LuaminUserValue:
    DynClone
    + CloneGasCost
    + std::fmt::Debug
    + dyn_partial_eq::DynPartialEq
    + dyn_partial_eq::DynPartialEqGasCost
    + 'static
{
}

impl PartialEq for dyn LuaminUserValue {
    fn eq(&self, other: &Self) -> bool {
        (self as &dyn dyn_partial_eq::DynPartialEq) == (other as &dyn dyn_partial_eq::DynPartialEq)
    }
}

impl Clone for Box<dyn LuaminUserValue> {
    fn clone(&self) -> Self {
        dyn_clone::clone_box(&**self)
    }
}

// MARK: Core

/// Failure during instruction execution, excluding gas exhaustion.
#[derive(Debug, Clone, thiserror::Error)]
pub enum LuaminRuntimeError<LE> {
    /// List length limit exceeded.
    #[error("List length limit exceeded")]
    ListTooLarge,
    /// Unable to allocate list storage.
    #[error("Unable to allocate list storage")]
    AllocationFailed,
    /// The runtime stack would exceed [`MAX_RUNTIME_STACK_SIZE`].
    #[error("Stack overflow")]
    StackOverflow,
    /// An instruction requires more stack values than are available.
    #[error("Insufficient stack elements")]
    InsufficientLuaminValues,
    /// Function call error; the payload contains the underlying error.
    #[error("Function call error: {0}")]
    FuncPointerCallError(FuncPointerCallError<LE>),
    /// An instruction references a stack position that does not exist.
    #[error("Stack index out of bound")]
    StackIndexOutOfBound,
    /// List assignment received an index that is not an integer.
    #[error("Invalid list index type")]
    InvalidListIndexType,
    /// A translated list assignment index is negative or exceeds `i32::MAX`.
    #[error("Invalid list index when setting")]
    InvalidListIndexWhenSetting,
    /// A list operation received a value of another type.
    #[error("Stack item not list")]
    StackItemNotList,
    /// An active `RefCell` borrow conflicts with the requested list access.
    #[error("List unable to borrow")]
    ListUnableToBorrow,
    /// An integer operation received a value of another type.
    #[error("Stack item not number")]
    StackItemNotNumber,
    /// Floor division or modulo received a zero divisor.
    #[error("Division by zero")]
    DivisionByZero,
}

impl<LE> From<GasError<FuncPointerCallError<LE>>> for GasError<LuaminRuntimeError<LE>> {
    fn from(value: GasError<FuncPointerCallError<LE>>) -> Self {
        match value {
            GasError::GasExceeded => GasError::GasExceeded,
            GasError::Other(fp_call_error) => {
                GasError::Other(LuaminRuntimeError::FuncPointerCallError(fp_call_error))
            }
        }
    }
}

/// Executes a compiled script with shared host state access.
///
/// `initial_stack` is cloned in order: intrinsic parameters first, then source
/// parameters. The script pads missing parameters with nil and drops excess
/// values. On success, the returned owned iterator yields script return values
/// in source order. List clones share storage, including initial-stack lists.
///
/// Instructions and host-call dispatch consume `gas`; generated host wrappers
/// also apply their configured charge. Custom host functions must account for
/// their own work. Shared access permits interior mutation and list changes.
/// Changes and gas consumed before an error are not rolled back.
///
/// # Errors
///
/// Returns [`GasError::GasExceeded`] on exhaustion, or
/// [`GasError::Other`] for runtime type, borrow, stack, list, arithmetic,
/// or host errors. Initial stacks above [`MAX_RUNTIME_STACK_SIZE`] are rejected.
pub fn luamin_execute<SLP: FuncPointerStateLifetimeProvider, LE: 'static, X: 'static>(
    state: &SLP::StateWithLifetimes<'_, '_, '_, '_>,
    gas: &mut LuaminGas,
    luamin_script: &LuaminScript<SLP, LE, X>,
    initial_stack: &[LuaminValue],
) -> Result<
    impl ExactSizeIterator<Item = LuaminValue> + std::fmt::Debug + use<SLP, LE, X>,
    GasError<LuaminRuntimeError<LE>>,
> {
    let mut state = state;
    let call_fn = move |state: &mut &SLP::StateWithLifetimes<'_, '_, '_, '_>,
                        gas: &mut LuaminGas,
                        stack: &mut SmallVec<[LuaminValue; LIKELY_STACK_CAP]>,
                        func_def: &HostFuncDef<SLP, LE, X>| {
        func_def.func_pointer.call(state, gas, stack)
    };

    execute_aux(
        &mut state,
        gas,
        luamin_script.func_lib(),
        luamin_script.ixs(),
        initial_stack,
        call_fn,
    )
}

/// Executes a compiled script with exclusive host state access.
///
/// Accepts both shared and mutable host methods. Parameter ordering, returned
/// values, aliasing, gas accounting, and errors follow [`luamin_execute`].
/// Host state and shared list changes made before an error are retained.
pub fn luamin_execute_mut<SLP: FuncPointerStateLifetimeProvider, LE: 'static, X: 'static>(
    state: &mut SLP::StateWithLifetimes<'_, '_, '_, '_>,
    gas: &mut LuaminGas,
    luamin_script_mut: &LuaminScriptMut<SLP, LE, X>,
    initial_stack: &[LuaminValue],
) -> Result<
    impl ExactSizeIterator<Item = LuaminValue> + std::fmt::Debug + use<SLP, LE, X>,
    GasError<LuaminRuntimeError<LE>>,
> {
    let mut state = state;
    let call_fn = |state: &mut &mut SLP::StateWithLifetimes<'_, '_, '_, '_>,
                   gas: &mut LuaminGas,
                   stack: &mut SmallVec<[LuaminValue; LIKELY_STACK_CAP]>,
                   func_def: &HostFuncDef<SLP, LE, X>| {
        func_def.func_pointer.call_mut(state, gas, stack)
    };

    execute_aux(
        &mut state,
        gas,
        luamin_script_mut.func_lib(),
        luamin_script_mut.ixs(),
        initial_stack,
        call_fn,
    )
}

fn execute_aux<SLP: FuncPointerStateLifetimeProvider, LE: 'static, X: 'static, RS, F>(
    state: &mut RS,
    gas: &mut LuaminGas,
    func_lib: &HostFuncLib<SLP, LE, X>,
    ixs: &[Ix],
    initial_stack: &[LuaminValue],
    mut call_fn: F,
) -> Result<
    <SmallVec<[LuaminValue; LIKELY_STACK_CAP]> as IntoIterator>::IntoIter,
    GasError<LuaminRuntimeError<LE>>,
>
where
    F: FnMut(
        &mut RS,
        &mut LuaminGas,
        &mut SmallVec<[LuaminValue; LIKELY_STACK_CAP]>,
        &HostFuncDef<SLP, LE, X>,
    ) -> Result<(), GasError<FuncPointerCallError<LE>>>,
{
    use Ix::*;

    if initial_stack.len() > MAX_RUNTIME_STACK_SIZE {
        return Err(GasError::Other(LuaminRuntimeError::StackOverflow));
    }
    let mut stack: SmallVec<[LuaminValue; LIKELY_STACK_CAP]> = SmallVec::from(initial_stack);
    let mut pc: i32 = 0;

    macro_rules! assert_or_ret {
        ($predicate:expr, $variant:ident) => {
            if !($predicate) {
                return Err(GasError::Other(LuaminRuntimeError::$variant));
            }
        };
    }

    while (pc as usize) < ixs.len() {
        let ix = &ixs[pc as usize];
        match ix {
            PushNumber(number) => {
                gas.consume_priv(GAS_COST_BASIC_SCALAR)?;
                stack.push_element_checked(LuaminValue::Number(*number))?;
            }
            PushBool(bool) => {
                gas.consume_priv(GAS_COST_BASIC_SCALAR)?;
                stack.push_element_checked(LuaminValue::Bool(*bool))?;
            }
            PushNil { count } => {
                let count = *count as usize;
                assert_or_ret!(stack.len() + count <= MAX_RUNTIME_STACK_SIZE, StackOverflow);
                gas.consume_priv(GAS_COST_BASIC_SCALAR * count as i32)?;
                stack.extend(std::iter::repeat_n(LuaminValue::Nil, count));
            }
            NewList { size } => {
                let size = *size as usize;
                assert_or_ret!(stack.len() >= size, InsufficientLuaminValues);
                gas.consume_priv(
                    GAS_COST_LIST_CREATION + GAS_COST_PER_LIST_CREATION_ITEM * size as i32,
                )?;
                let list = stack.drain(stack.len() - size..).collect();
                stack.push_element_checked(LuaminValue::List(Rc::new(RefCell::new(list))))?;
            }
            CallLib { fn_index } => {
                let func_def = func_lib
                    .kth_func_def(*fn_index as usize)
                    .expect("Function index out of bounds");
                let property = func_def.func_pointer.property();
                assert_or_ret!(
                    stack.len() >= property.input_count,
                    InsufficientLuaminValues
                );
                assert_or_ret!(
                    stack.len() - property.input_count + property.output_count
                        <= MAX_RUNTIME_STACK_SIZE,
                    StackOverflow
                );
                gas.consume_priv((property.input_count + property.output_count) as i32)?;
                call_fn(state, gas, &mut stack, func_def)?;
            }
            Dup { stack_index } => {
                let stack_index = *stack_index as usize;
                assert_or_ret!(stack_index < stack.len(), StackIndexOutOfBound);
                let element = &stack[stack_index];
                gas.consume_priv(element.clone_gas_cost())?;
                stack.push_element_checked(element.clone())?;
            }
            GetListElement {
                stack_index,
                base_index,
            } => {
                let list_index = pop_number_1(&mut stack)? as i64 - *base_index as i64;

                let stack_index = *stack_index as usize;
                assert_or_ret!(stack_index < stack.len(), StackIndexOutOfBound);

                let element = {
                    let list = match &stack[stack_index] {
                        LuaminValue::List(list) => list,
                        _ => return Err(GasError::Other(LuaminRuntimeError::StackItemNotList)),
                    };
                    let list = list
                        .try_borrow()
                        .map_err(|_| GasError::Other(LuaminRuntimeError::ListUnableToBorrow))?;
                    list.get(list_index as usize)
                        .cloned()
                        .unwrap_or(LuaminValue::Nil)
                };
                gas.consume_priv(element.clone_gas_cost())?;
                stack.push_element_checked(element)?;
            }
            Pop { pop_count } => {
                let pop_count = *pop_count as usize;
                assert_or_ret!(stack.len() >= pop_count, InsufficientLuaminValues);
                gas.consume_priv(GAS_COST_PER_POPPED_ITEM * pop_count as i32)?;
                stack.truncate(stack.len() - pop_count);
            }
            Remove {
                stack_index,
                remove_count,
            } => {
                let stack_index = *stack_index as usize;
                let remove_count = *remove_count as usize;
                let end_index = stack_index + remove_count;
                assert_or_ret!(end_index <= stack.len(), StackIndexOutOfBound);
                gas.consume_priv(GAS_COST_PER_REMOVED_ITEM * remove_count as i32)?;
                stack.drain(stack_index..end_index);
            }
            Set { stack_index } => {
                assert_or_ret!(!stack.is_empty(), InsufficientLuaminValues);
                let value = stack.pop().unwrap();

                let stack_index = *stack_index as usize;
                assert_or_ret!(stack_index < stack.len(), StackIndexOutOfBound);
                gas.consume_priv(GAS_COST_SET)?;
                stack[stack_index] = value;
            }
            SetListElement {
                stack_index,
                base_index,
            } => {
                assert_or_ret!(stack.len() >= 2, InsufficientLuaminValues);
                let value = stack.pop().unwrap();
                let list_index = match stack.pop().unwrap() {
                    LuaminValue::Number(list_index) => {
                        let list_index = list_index as i64 - *base_index as i64;
                        assert_or_ret!(
                            (0..=i32::MAX as i64).contains(&list_index),
                            InvalidListIndexWhenSetting
                        );
                        list_index as usize
                    }
                    _ => return Err(GasError::Other(LuaminRuntimeError::InvalidListIndexType)),
                };

                let stack_index = *stack_index as usize;
                assert_or_ret!(stack_index < stack.len(), StackIndexOutOfBound);

                let list = match &stack[stack_index] {
                    LuaminValue::List(list) => list,
                    _ => return Err(GasError::Other(LuaminRuntimeError::StackItemNotList)),
                };
                let mut list = list
                    .try_borrow_mut()
                    .map_err(|_| GasError::Other(LuaminRuntimeError::ListUnableToBorrow))?;
                assert_or_ret!(list_index < MAX_LIST_LEN, ListTooLarge);
                let additional = (list_index + 1).saturating_sub(list.len());
                gas.consume_priv(additional.max(1) as i32 * GAS_COST_PER_PUSHED_ITEM)?;
                if additional != 0 {
                    list.try_reserve(additional)
                        .map_err(|_| GasError::Other(LuaminRuntimeError::AllocationFailed))?;
                    list.resize(list_index + 1, LuaminValue::Nil);
                }
                list[list_index] = value;
            }
            AdjustStack { adjust_count } => {
                let adjust_count = *adjust_count as usize;
                assert_or_ret!(adjust_count <= MAX_RUNTIME_STACK_SIZE, StackOverflow);
                if adjust_count > stack.len() {
                    gas.consume_priv((adjust_count - stack.len()) as i32 * GAS_COST_BASIC_SCALAR)?;
                    stack.resize(adjust_count, LuaminValue::Nil);
                } else if adjust_count < stack.len() {
                    gas.consume_priv(
                        (stack.len() - adjust_count) as i32 * GAS_COST_PER_POPPED_ITEM,
                    )?;
                    stack.truncate(adjust_count);
                } else {
                    gas.consume_priv(GAS_COST_NOP)?;
                }
            }

            JumpIf { relative_offset } => {
                gas.consume_priv(GAS_COST_JUMP)?;
                assert_or_ret!(!stack.is_empty(), InsufficientLuaminValues);
                if cast_bool(&stack.pop().unwrap()) {
                    pc += *relative_offset;
                }
            }
            JumpIfNot { relative_offset } => {
                gas.consume_priv(GAS_COST_JUMP)?;
                assert_or_ret!(!stack.is_empty(), InsufficientLuaminValues);
                if !cast_bool(&stack.pop().unwrap()) {
                    pc += *relative_offset;
                }
            }
            Jump { relative_offset } => {
                gas.consume_priv(GAS_COST_JUMP)?;
                pc += *relative_offset;
            }

            Add => {
                gas.consume_priv(GAS_COST_ARITH_BINARY)?;
                execute_arith_2(&mut stack, i32::wrapping_add)?;
            }
            Sub => {
                gas.consume_priv(GAS_COST_ARITH_BINARY)?;
                execute_arith_2(&mut stack, i32::wrapping_sub)?;
            }
            Mul => {
                gas.consume_priv(GAS_COST_ARITH_BINARY)?;
                execute_arith_2(&mut stack, i32::wrapping_mul)?;
            }
            DivFloor => {
                gas.consume_priv(GAS_COST_ARITH_BINARY)?;
                let (a, b) = pop_number_2(&mut stack)?;
                let quo = div_floor(a, b)?;
                stack.push_element_checked(LuaminValue::Number(quo))?;
            }
            Mod => {
                gas.consume_priv(GAS_COST_ARITH_BINARY)?;
                let (a, b) = pop_number_2(&mut stack)?;
                let quo = div_floor(a, b)?;
                let rem = a.wrapping_sub(quo.wrapping_mul(b));
                stack.push_element_checked(LuaminValue::Number(rem))?;
            }
            Neg => {
                gas.consume_priv(GAS_COST_ARITH_UNARY)?;
                execute_arith_1(&mut stack, i32::wrapping_neg)?;
            }

            Lshift => {
                gas.consume_priv(GAS_COST_ARITH_BINARY)?;
                let (a, b) = pop_number_2(&mut stack)?;
                let result = if b >= 0 {
                    a.wrapping_shl(b as u32)
                } else {
                    a.wrapping_shr((-(b as i64)) as u32)
                };
                stack.push_element_checked(LuaminValue::Number(result))?;
            }
            Rshift => {
                gas.consume_priv(GAS_COST_ARITH_BINARY)?;
                let (a, b) = pop_number_2(&mut stack)?;
                let result = if b >= 0 {
                    a.wrapping_shr(b as u32)
                } else {
                    a.wrapping_shl((-(b as i64)) as u32)
                };
                stack.push_element_checked(LuaminValue::Number(result))?;
            }
            BitAnd => {
                use std::ops::BitAnd;
                gas.consume_priv(GAS_COST_ARITH_BINARY)?;
                execute_arith_2(&mut stack, i32::bitand)?;
            }
            BitOr => {
                use std::ops::BitOr;
                gas.consume_priv(GAS_COST_ARITH_BINARY)?;
                execute_arith_2(&mut stack, i32::bitor)?;
            }
            BitXor => {
                use std::ops::BitXor;
                gas.consume_priv(GAS_COST_ARITH_BINARY)?;
                execute_arith_2(&mut stack, i32::bitxor)?;
            }
            BitNot => {
                use std::ops::Not;
                gas.consume_priv(GAS_COST_ARITH_UNARY)?;
                execute_arith_1(&mut stack, i32::not)?;
            }

            LogicalNot => {
                gas.consume_priv(GAS_COST_ARITH_UNARY)?;
                assert_or_ret!(!stack.is_empty(), InsufficientLuaminValues);
                let result = !cast_bool(&stack.pop().unwrap());
                stack.push_element_checked(LuaminValue::Bool(result))?;
            }
            Eq => {
                assert_or_ret!(stack.len() >= 2, InsufficientLuaminValues);
                let b = stack.pop().unwrap();
                let a = stack.pop().unwrap();
                gas.consume_priv(a.partial_eq_gas_cost(&b))?;
                stack.push_element_checked(LuaminValue::Bool(a == b))?;
            }
            Neq => {
                assert_or_ret!(stack.len() >= 2, InsufficientLuaminValues);
                let b = stack.pop().unwrap();
                let a = stack.pop().unwrap();
                gas.consume_priv(a.partial_eq_gas_cost(&b))?;
                stack.push_element_checked(LuaminValue::Bool(a != b))?;
            }
            Lt => {
                gas.consume_priv(GAS_COST_ARITH_BINARY)?;
                compare_2(&mut stack, |a, b| a < b)?;
            }
            Lte => {
                gas.consume_priv(GAS_COST_ARITH_BINARY)?;
                compare_2(&mut stack, |a, b| a <= b)?;
            }
            Gt => {
                gas.consume_priv(GAS_COST_ARITH_BINARY)?;
                compare_2(&mut stack, |a, b| a > b)?;
            }
            Gte => {
                gas.consume_priv(GAS_COST_ARITH_BINARY)?;
                compare_2(&mut stack, |a, b| a >= b)?;
            }

            Length => {
                gas.consume_priv(GAS_COST_ARITH_UNARY)?;
                assert_or_ret!(!stack.is_empty(), InsufficientLuaminValues);
                let list = match stack.pop().unwrap() {
                    LuaminValue::List(list) => list,
                    _ => return Err(GasError::Other(LuaminRuntimeError::StackItemNotList)),
                };
                let list = list
                    .try_borrow()
                    .map_err(|_| GasError::Other(LuaminRuntimeError::ListUnableToBorrow))?;
                stack.push_element_checked(LuaminValue::Number(list.len() as i32))?;
            }
        }

        pc += 1;
    }

    Ok(stack.into_iter())
}

fn pop_number_2<LE>(
    stack: &mut SmallVec<[LuaminValue; LIKELY_STACK_CAP]>,
) -> Result<(i32, i32), GasError<LuaminRuntimeError<LE>>> {
    if stack.len() < 2 {
        return Err(GasError::Other(
            LuaminRuntimeError::InsufficientLuaminValues,
        ));
    }
    let b = match stack.pop().unwrap() {
        LuaminValue::Number(b) => b,
        _ => return Err(GasError::Other(LuaminRuntimeError::StackItemNotNumber)),
    };
    let a = match stack.pop().unwrap() {
        LuaminValue::Number(a) => a,
        _ => return Err(GasError::Other(LuaminRuntimeError::StackItemNotNumber)),
    };
    Ok((a, b))
}

fn pop_number_1<LE>(
    stack: &mut SmallVec<[LuaminValue; LIKELY_STACK_CAP]>,
) -> Result<i32, GasError<LuaminRuntimeError<LE>>> {
    if stack.is_empty() {
        return Err(GasError::Other(
            LuaminRuntimeError::InsufficientLuaminValues,
        ));
    }
    let a = match stack.pop().unwrap() {
        LuaminValue::Number(a) => a,
        _ => return Err(GasError::Other(LuaminRuntimeError::StackItemNotNumber)),
    };
    Ok(a)
}

fn div_floor<LE>(a: i32, b: i32) -> Result<i32, GasError<LuaminRuntimeError<LE>>> {
    if b == 0 {
        return Err(GasError::Other(LuaminRuntimeError::DivisionByZero));
    }
    let quo = a.wrapping_div(b);
    let rem = a.wrapping_rem(b);
    if (quo < 0 || quo == 0 && (a ^ b) < 0) && rem != 0 {
        Ok(quo.wrapping_sub(1))
    } else {
        Ok(quo)
    }
}

fn execute_arith_2<LE>(
    stack: &mut SmallVec<[LuaminValue; LIKELY_STACK_CAP]>,
    op: fn(i32, i32) -> i32,
) -> Result<(), GasError<LuaminRuntimeError<LE>>> {
    let (a, b) = pop_number_2(stack)?;
    stack.push_element_checked(LuaminValue::Number(op(a, b)))?;
    Ok(())
}

fn execute_arith_1<LE>(
    stack: &mut SmallVec<[LuaminValue; LIKELY_STACK_CAP]>,
    op: fn(i32) -> i32,
) -> Result<(), GasError<LuaminRuntimeError<LE>>> {
    let a = pop_number_1(stack)?;
    stack.push_element_checked(LuaminValue::Number(op(a)))?;
    Ok(())
}

fn compare_2<LE>(
    stack: &mut SmallVec<[LuaminValue; LIKELY_STACK_CAP]>,
    op: fn(i32, i32) -> bool,
) -> Result<(), GasError<LuaminRuntimeError<LE>>> {
    let (a, b) = pop_number_2(stack)?;
    stack.push_element_checked(LuaminValue::Bool(op(a, b)))?;
    Ok(())
}

fn cast_bool(element: &LuaminValue) -> bool {
    match element {
        LuaminValue::Bool(b) => *b,
        LuaminValue::Nil => false,
        _ => true,
    }
}

trait StackOp {
    fn push_element_checked<LE>(
        &mut self,
        element: LuaminValue,
    ) -> Result<(), GasError<LuaminRuntimeError<LE>>>;
}

impl StackOp for SmallVec<[LuaminValue; LIKELY_STACK_CAP]> {
    fn push_element_checked<LE>(
        &mut self,
        element: LuaminValue,
    ) -> Result<(), GasError<LuaminRuntimeError<LE>>> {
        if self.len() + 1 > MAX_RUNTIME_STACK_SIZE {
            Err(GasError::Other(LuaminRuntimeError::StackOverflow))
        } else {
            self.push(element);
            Ok(())
        }
    }
}

// MARK: Test

#[cfg(test)]
mod tests;
