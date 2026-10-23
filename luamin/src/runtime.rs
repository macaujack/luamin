use std::{cell::RefCell, rc::Rc};

use dyn_clone::DynClone;
pub use luamin_macro::LuaminUserValue;
use smallvec::SmallVec;

use crate::{
    MAX_RUNTIME_STACK_SIZE,
    codegen::{LuaminScript, LuaminScriptMut},
    component::Ix,
    func_lib::{
        FuncDef, FuncLib, FuncPointerCallError, FuncPointerStateLifetimeProvider, GasError,
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
pub const MAX_LIST_LEN: usize = 65_536;

#[derive(Clone)]
pub enum LuaminValue {
    Nil,
    Number(i32),
    Bool(bool),
    // Lists share ownership and use identity equality. Reference cycles are
    // supported, but must be broken by the embedding application to reclaim them.
    List(Rc<RefCell<Vec<LuaminValue>>>),
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct LuaminGas(i32);

impl std::fmt::Display for LuaminGas {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl LuaminGas {
    pub const INF: Self = Self(1_000_000_000);

    pub const fn new(gas: i32) -> Self {
        Self(if gas < 0 { -1 } else { gas })
    }

    /// Returns true on exhaustion or a negative cost. Exhaustion is sticky.
    #[must_use]
    pub const fn consume(&mut self, gas_cost: i32) -> bool {
        if gas_cost < 0 || self.0 < 0 || gas_cost > self.0 {
            self.0 = -1;
        } else {
            self.0 -= gas_cost;
        }
        self.has_exceeded()
    }

    pub const fn has_exceeded(&self) -> bool {
        self.0 < 0
    }

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

pub trait CloneGasCost {
    fn clone_gas_cost(&self) -> i32;
}

pub trait PartialEqGasCost {
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

#[derive(Debug, Clone, thiserror::Error)]
pub enum LuaminRuntimeError<LE> {
    #[error("List length limit exceeded")]
    ListTooLarge,
    #[error("Unable to allocate list storage")]
    AllocationFailed,
    #[error("Stack overflow")]
    StackOverflow,
    #[error("Insufficient stack elements")]
    InsufficientLuaminValues,
    #[error("Function call error: {0}")]
    FuncPointerCallError(FuncPointerCallError<LE>),
    #[error("Stack index out of bound")]
    StackIndexOutOfBound,
    #[error("Invalid list index type")]
    InvalidListIndexType,
    #[error("Invalid list index when setting")]
    InvalidListIndexWhenSetting,
    #[error("Stack item not list")]
    StackItemNotList,
    #[error("List unable to borrow")]
    ListUnableToBorrow,
    #[error("Stack item not number")]
    StackItemNotNumber,
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
                        func_def: &FuncDef<SLP, LE, X>| {
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
                   func_def: &FuncDef<SLP, LE, X>| {
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
    func_lib: &FuncLib<SLP, LE, X>,
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
        &FuncDef<SLP, LE, X>,
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
