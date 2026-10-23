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

#[derive(Clone)]
pub enum LuaminValue {
    Nil,
    Number(i32),
    Bool(bool),
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
        match self {
            Self::Nil => write!(f, "nil"),
            Self::Number(number) => write!(f, "{}", number),
            Self::Bool(b) => write!(f, "{}", b),
            Self::List(list) => {
                let raw_ptr = Rc::as_ptr(list);
                write!(f, "{:p} {{", raw_ptr)?;

                let list = match list.try_borrow() {
                    Ok(list) => list,
                    Err(_) => return write!(f, "<locked> }}"),
                };

                for (i, element) in list.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", {:?}", element)?;
                    } else {
                        write!(f, "{:?}", element)?;
                    }
                }

                write!(f, "}}")
            }
            Self::UserValue(value) => write!(f, "{:?}", value),
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
        Self(gas)
    }

    /// Returns true if gas exceeded after consuming.
    pub const fn consume(&mut self, gas_cost: i32) -> bool {
        self.0 -= gas_cost;
        self.has_exceeded()
    }

    pub const fn has_exceeded(&self) -> bool {
        self.0 < 0
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

    let mut stack: SmallVec<[LuaminValue; LIKELY_STACK_CAP]> = SmallVec::from(initial_stack);
    let mut pc: i32 = 0;
    #[cfg(test)]
    println!("{:?}", stack);

    macro_rules! assert_or_ret {
        ($predicate:expr, $variant:ident) => {
            if !($predicate) {
                return Err(GasError::Other(LuaminRuntimeError::$variant));
            }
        };
    }

    while (pc as usize) < ixs.len() {
        let ix = &ixs[pc as usize];
        #[cfg(test)]
        println!("<{}> {:?}", pc, ix);
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
                if list_index >= list.len() {
                    gas.consume_priv((list_index - list.len()) as i32 * GAS_COST_PER_PUSHED_ITEM)?;
                    list.resize(list_index + 1, LuaminValue::Nil);
                }
                gas.consume_priv(GAS_COST_PER_PUSHED_ITEM)?;
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

        #[cfg(test)]
        println!("{:?}", stack);

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
mod tests {
    use luamin_macro::LuaminUserValue;
    use smallvec::smallvec;

    use crate::{
        codegen::LuaminCodegen,
        func_lib::{FuncDef, FuncLibBuf, FuncPointer, consume_gas},
        lexer::luamin_lex,
    };

    use super::*;

    #[derive(Debug, Clone)]
    struct State {
        score: i32,
    }

    #[derive(Debug, Clone)]
    struct StateLifetimeProvider;
    impl FuncPointerStateLifetimeProvider for StateLifetimeProvider {
        type StateWithLifetimes<'r0, 'r1, 'r2, 'r3> = State;
    }

    #[derive(Debug, thiserror::Error)]
    enum TestError {
        #[error("Division by zero")]
        DivByZero,
        #[error("Incorrect type")]
        IncorrectType,
    }

    impl State {
        fn get_score(&self, gas: &mut LuaminGas) -> Result<LuaminValue, GasError<TestError>> {
            consume_gas!(gas, 1);
            Ok(LuaminValue::Number(self.score))
        }
        fn add_score(
            &mut self,
            gas: &mut LuaminGas,
            amount: LuaminValue,
        ) -> Result<(), GasError<TestError>> {
            consume_gas!(gas, 1);
            let amount = match amount {
                LuaminValue::Number(amount) => amount,
                _ => return Err(GasError::Other(TestError::IncorrectType)),
            };
            self.score += amount;
            Ok(())
        }
        fn sub_score(
            &mut self,
            gas: &mut LuaminGas,
            amount: LuaminValue,
        ) -> Result<(), GasError<TestError>> {
            consume_gas!(gas, 1);
            let amount = match amount {
                LuaminValue::Number(amount) => amount,
                _ => return Err(GasError::Other(TestError::IncorrectType)),
            };
            self.score -= amount;
            Ok(())
        }
        fn mul_score(
            &mut self,
            gas: &mut LuaminGas,
            amount: LuaminValue,
        ) -> Result<(), GasError<TestError>> {
            consume_gas!(gas, 1);
            let amount = match amount {
                LuaminValue::Number(amount) => amount,
                _ => return Err(GasError::Other(TestError::IncorrectType)),
            };
            self.score *= amount;
            Ok(())
        }
        fn div_score(
            &mut self,
            gas: &mut LuaminGas,
            amount: LuaminValue,
        ) -> Result<(), GasError<TestError>> {
            consume_gas!(gas, 1);
            let amount = match amount {
                LuaminValue::Number(amount) => amount,
                _ => return Err(GasError::Other(TestError::IncorrectType)),
            };
            if amount == 0 {
                return Err(GasError::Other(TestError::DivByZero));
            }
            self.score /= amount;
            Ok(())
        }
        fn list_score0(&self, gas: &mut LuaminGas) -> Result<LuaminValue, GasError<TestError>> {
            consume_gas!(gas, 1);
            let list = vec![
                LuaminValue::Number(self.score),
                LuaminValue::Number(self.score),
            ];
            Ok(LuaminValue::List(Rc::new(RefCell::new(list))))
        }
        fn list_score1(
            &self,
            gas: &mut LuaminGas,
        ) -> Result<(LuaminValue, LuaminValue), GasError<TestError>> {
            consume_gas!(gas, 1);
            let list = vec![LuaminValue::Number(self.score), LuaminValue::Nil];
            Ok((
                LuaminValue::List(Rc::new(RefCell::new(list))),
                LuaminValue::Number(42),
            ))
        }
    }

    fn get_func_lib() -> &'static FuncLib<StateLifetimeProvider, TestError> {
        type FP = FuncPointer<StateLifetimeProvider, TestError>;
        let func_lib = FuncLibBuf::new([
            FuncDef::new("get_score", FP::F01(State::get_score)),
            FuncDef::new("add_score", FP::F10Mut(State::add_score)),
            FuncDef::new("sub_score", FP::F10Mut(State::sub_score)),
            FuncDef::new("mul_score", FP::F10Mut(State::mul_score)),
            FuncDef::new("div_score", FP::F10Mut(State::div_score)),
            FuncDef::new("list_score0", FP::F01(State::list_score0)),
            FuncDef::new("list_score1", FP::F02(State::list_score1)),
        ])
        .unwrap();

        Box::leak(Box::new(func_lib))
    }

    #[allow(clippy::type_complexity)]
    fn compile_and_execute_mut(
        initial_stack: &[LuaminValue],
        input: &str,
    ) -> Result<(SmallVec<[LuaminValue; 4]>, State), GasError<LuaminRuntimeError<TestError>>> {
        let mut state = State { score: 0 };
        let mut codegen = LuaminCodegen::new(get_func_lib(), 1);
        let (directive, stmts) = luamin_lex(input).unwrap();
        let (script, _) = codegen.codegen_mut(&[], &directive, &stmts).unwrap();
        println!("IXs:");
        for (i, ix) in script.ixs().iter().enumerate() {
            println!("<{}> {:?}", i, ix);
        }
        println!("#################################");
        let mut gas = LuaminGas::INF;
        let stack: SmallVec<[LuaminValue; 4]> =
            luamin_execute_mut(&mut state, &mut gas, &script, initial_stack)?.collect();
        Ok((stack, state))
    }

    #[test]
    fn test_luamin_value_equality() {
        assert_eq!(LuaminValue::Number(42), LuaminValue::Number(42));
        assert_ne!(LuaminValue::Number(42), LuaminValue::Number(41));
        assert_ne!(LuaminValue::Number(42), LuaminValue::Nil);
        assert_ne!(
            LuaminValue::List(Rc::new(RefCell::new(Vec::new()))),
            LuaminValue::List(Rc::new(RefCell::new(Vec::new())))
        );

        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        struct MyTypeA(i32);
        impl LuaminUserValue for MyTypeA {}

        #[derive(Debug, Clone, Copy, PartialEq, Eq, LuaminUserValue)]
        struct MyTypeB(i32);

        #[derive(Debug, Clone, PartialEq, Eq)]
        struct MyTypeC<T>(T);
        impl<T> CloneGasCost for MyTypeC<T>
        where
            T: CloneGasCost,
        {
            fn clone_gas_cost(&self) -> i32 {
                self.0.clone_gas_cost()
            }
        }
        impl<T> PartialEqGasCost for MyTypeC<T>
        where
            T: PartialEqGasCost,
        {
            fn partial_eq_gas_cost(&self, other: &Self) -> i32 {
                self.0.partial_eq_gas_cost(&other.0)
            }
        }
        impl<T> LuaminUserValue for MyTypeC<T> where
            MyTypeC<T>: CloneGasCost
                + dyn_clone::DynClone
                + std::fmt::Debug
                + dyn_partial_eq::DynPartialEq
                + dyn_partial_eq::DynPartialEqGasCost
                + 'static
        {
        }

        assert_eq!(
            LuaminValue::UserValue(Box::new(MyTypeA(42)) as Box<dyn LuaminUserValue>),
            LuaminValue::UserValue(Box::new(MyTypeA(42)) as Box<dyn LuaminUserValue>),
        );
        assert_ne!(
            LuaminValue::UserValue(Box::new(MyTypeA(42)) as Box<dyn LuaminUserValue>),
            LuaminValue::UserValue(Box::new(MyTypeA(41)) as Box<dyn LuaminUserValue>),
        );
        assert_ne!(
            LuaminValue::UserValue(Box::new(MyTypeA(42)) as Box<dyn LuaminUserValue>),
            LuaminValue::UserValue(Box::new(MyTypeB(42)) as Box<dyn LuaminUserValue>),
        );

        assert_eq!(
            LuaminValue::UserValue(Box::new(MyTypeC(42_i32)) as Box<dyn LuaminUserValue>),
            LuaminValue::UserValue(Box::new(MyTypeC(42_i32)) as Box<dyn LuaminUserValue>),
        );
        assert_ne!(
            LuaminValue::UserValue(Box::new(MyTypeC(42_i32)) as Box<dyn LuaminUserValue>),
            LuaminValue::UserValue(Box::new(MyTypeA(42_i32)) as Box<dyn LuaminUserValue>),
        );
        assert_ne!(
            LuaminValue::UserValue(Box::new(MyTypeC(42_i32)) as Box<dyn LuaminUserValue>),
            LuaminValue::UserValue(Box::new(MyTypeC(42_i64)) as Box<dyn LuaminUserValue>),
        );
    }

    #[test]
    fn test_runtime_00() {
        let (stack, state) = compile_and_execute_mut(
            &[],
            r#"
            local foo, bar = 1, 2, 3
            foo = foo + bar
            bar = {bar, 2, 3}
            bar[1] = bar[1] + 10

            local baz = {{nil, {nil, nil, 100}}}

            bar[1], foo, baz[1][2][3] = 41, 42, 43

            add_score(66)

            local x = list_score1()[1]
            local y, z = list_score0(), list_score1()
            local p, q = add_score(x, y), list_score1()
        "#,
        )
        .unwrap();
        assert!(stack.is_empty());
        assert_eq!(state.score, 132);
    }

    #[test]
    fn test_runtime_01() {
        let (stack, state) = compile_and_execute_mut(
            &[],
            r#"
            local a, b, c = 2, 3, 4
            return a + b + c, a * b * c
        "#,
        )
        .unwrap();
        let expected: SmallVec<[LuaminValue; 4]> =
            smallvec![LuaminValue::Number(9), LuaminValue::Number(24),];
        assert_eq!(stack, expected);
        assert_eq!(state.score, 0);
    }

    #[test]
    fn test_runtime_02() {
        let (stack, state) = compile_and_execute_mut(
            &[],
            r#"
            local foo, bar = 1, 2, 3
            foo = foo + bar
            bar = {bar, 2, 3}
            bar[1] = bar[1] + 10

            local baz = {{nil, {nil, nil, 100}}}

            bar[1], foo, baz[1][2][3] = 41, 42, 43

            add_score(66)

            local x = list_score1()[1]
            local y, z = list_score0(), list_score1()
            local p, q = add_score(x, y), list_score1()
            return foo, bar, baz, x, y, z, p, q
        "#,
        )
        .unwrap();
        assert_eq!(stack.len(), 8);

        assert_eq!(stack[0], LuaminValue::Number(42));

        let stack1 = match &stack[1] {
            LuaminValue::List(list) => list,
            _ => panic!(),
        };
        assert_eq!(
            &*stack1.borrow(),
            &vec![
                LuaminValue::Number(41),
                LuaminValue::Number(2),
                LuaminValue::Number(3)
            ]
        );

        {
            let stack2 = match &stack[2] {
                LuaminValue::List(list) => list,
                _ => panic!(),
            };
            let stack2 = stack2.borrow();
            let a1 = match &stack2[0] {
                LuaminValue::List(list) => list,
                _ => panic!(),
            };
            let a1 = a1.borrow();
            assert_eq!(&a1[0], &LuaminValue::Nil);
            let a12 = match &a1[1] {
                LuaminValue::List(list) => list,
                _ => panic!(),
            };
            let a12 = a12.borrow();
            assert_eq!(&a12[0], &LuaminValue::Nil);
            assert_eq!(&a12[1], &LuaminValue::Nil);
            assert_eq!(&a12[2], &LuaminValue::Number(43));
        }

        assert_eq!(stack[3], LuaminValue::Number(66));

        let stack4 = match &stack[4] {
            LuaminValue::List(list) => list,
            _ => panic!(),
        };
        assert_eq!(
            &*stack4.borrow(),
            &vec![LuaminValue::Number(66), LuaminValue::Number(66)]
        );

        let stack5 = match &stack[5] {
            LuaminValue::List(list) => list,
            _ => panic!(),
        };
        assert_eq!(
            &*stack5.borrow(),
            &vec![LuaminValue::Number(66), LuaminValue::Nil]
        );

        assert_eq!(stack[6], LuaminValue::Nil);

        let stack7 = match &stack[7] {
            LuaminValue::List(list) => list,
            _ => panic!(),
        };
        assert_eq!(
            &*stack7.borrow(),
            &vec![LuaminValue::Number(132), LuaminValue::Nil]
        );

        assert_eq!(state.score, 132);
    }

    #[test]
    fn test_runtime_03() {
        let (stack, _) = compile_and_execute_mut(
            &[
                LuaminValue::Number(2),
                LuaminValue::Number(3),
                LuaminValue::Number(4),
                LuaminValue::Number(5),
            ],
            r#"
            #param a, b, c
            return a + b + c
        "#,
        )
        .unwrap();
        let expected: SmallVec<[LuaminValue; 4]> = smallvec![LuaminValue::Number(9)];
        assert_eq!(stack, expected);
    }

    #[test]
    fn test_runtime_04() {
        let err = compile_and_execute_mut(
            &[LuaminValue::Number(2), LuaminValue::Number(3)],
            r#"
            #param a, b, c
            return a + b + c
        "#,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            GasError::Other(LuaminRuntimeError::StackItemNotNumber)
        ));
    }

    #[test]
    fn test_runtime_05() {
        let err = compile_and_execute_mut(
            &[LuaminValue::Number(0)],
            r#"
            #param divisor
            return 100 // divisor < 10
        "#,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            GasError::Other(LuaminRuntimeError::DivisionByZero)
        ));
    }

    #[test]
    fn test_runtime_06() {
        let (stack, _) = compile_and_execute_mut(
            &[LuaminValue::Number(12)],
            r#"
            #param divisor
            return 1+100//divisor,100 // divisor < 10
        "#,
        )
        .unwrap();
        let expected: SmallVec<[LuaminValue; 4]> =
            smallvec![LuaminValue::Number(9), LuaminValue::Bool(true)];
        assert_eq!(stack, expected);
    }

    #[test]
    fn test_runtime_07() {
        let (stack, _) = compile_and_execute_mut(
            &[LuaminValue::Number(12)],
            r#"
            #param a
            do
                return a * 10
            end
            local b = a + 3
            return a, b
        "#,
        )
        .unwrap();
        let expected: SmallVec<[LuaminValue; 4]> = smallvec![LuaminValue::Number(120)];
        assert_eq!(stack, expected);
    }

    fn test_leap_year(input: &str) {
        let (stack, _) = compile_and_execute_mut(&[LuaminValue::Number(2004)], input).unwrap();
        let expected: SmallVec<[LuaminValue; 4]> = smallvec![LuaminValue::Bool(true)];
        assert_eq!(stack, expected);

        let (stack, _) = compile_and_execute_mut(&[LuaminValue::Number(1900)], input).unwrap();
        let expected: SmallVec<[LuaminValue; 4]> = smallvec![LuaminValue::Bool(false)];
        assert_eq!(stack, expected);

        let (stack, _) = compile_and_execute_mut(&[LuaminValue::Number(2000)], input).unwrap();
        let expected: SmallVec<[LuaminValue; 4]> = smallvec![LuaminValue::Bool(true)];
        assert_eq!(stack, expected);

        let (stack, _) = compile_and_execute_mut(&[LuaminValue::Number(2001)], input).unwrap();
        let expected: SmallVec<[LuaminValue; 4]> = smallvec![LuaminValue::Bool(false)];
        assert_eq!(stack, expected);
    }

    #[test]
    fn test_runtime_08() {
        test_leap_year(
            r#"
            #param year
            local is_leap_year
            if year % 4 == 0 then
                if year % 100 == 0 then
                    if year % 400 == 0 then
                        is_leap_year = true
                    else
                        is_leap_year = false
                    end
                else
                    is_leap_year = true
                end
            else
                is_leap_year = false
            end
            return is_leap_year
        "#,
        );
    }

    #[test]
    fn test_runtime_09() {
        test_leap_year(
            r#"
            #param year
            if year % 400 == 0 then
                return true
            end
            if year % 100 == 0 then
                return false
            end
            return year % 4 == 0
        "#,
        );
    }

    #[test]
    fn test_runtime_10() {
        const INPUT: &str = r#"
            #param limit
            local i, ret = 1, {}
            while i <= limit do
                ret[i] = i * 10
                i = i + 1
            end
            return ret
        "#;
        let (stack, _) = compile_and_execute_mut(&[LuaminValue::Number(3)], INPUT).unwrap();
        assert_eq!(stack.len(), 1);
        let list = match &stack[0] {
            LuaminValue::List(list) => list,
            _ => panic!(),
        };
        assert_eq!(
            &*list.borrow(),
            &vec![
                LuaminValue::Number(10),
                LuaminValue::Number(20),
                LuaminValue::Number(30)
            ]
        );

        let (stack, _) = compile_and_execute_mut(&[LuaminValue::Number(0)], INPUT).unwrap();
        assert_eq!(stack.len(), 1);
        let list = match &stack[0] {
            LuaminValue::List(list) => list,
            _ => panic!(),
        };
        assert_eq!(&*list.borrow(), &vec![]);
    }

    #[test]
    fn test_runtime_11() {
        const INPUT: &str = r#"
            #param limit
            local i, ret = 1, {}
            repeat
                ret[i] = i * 10
                i = i + 1
            until i > limit
            return ret
        "#;
        let (stack, _) = compile_and_execute_mut(&[LuaminValue::Number(3)], INPUT).unwrap();
        assert_eq!(stack.len(), 1);
        let list = match &stack[0] {
            LuaminValue::List(list) => list,
            _ => panic!(),
        };
        assert_eq!(
            &*list.borrow(),
            &vec![
                LuaminValue::Number(10),
                LuaminValue::Number(20),
                LuaminValue::Number(30)
            ]
        );

        let (stack, _) = compile_and_execute_mut(&[LuaminValue::Number(0)], INPUT).unwrap();
        assert_eq!(stack.len(), 1);
        let list = match &stack[0] {
            LuaminValue::List(list) => list,
            _ => panic!(),
        };
        assert_eq!(&*list.borrow(), &vec![LuaminValue::Number(10)]);
    }

    #[test]
    fn test_runtime_12() {
        const INPUT: &str = r#"
            #param n
            local ret = {false}
            local i = 2
            while i <= n do
                local j = 2
                local is_prime = true
                while j * j <= i do
                    if i % j == 0 then
                        is_prime = false
                        break
                    end
                    j = j + 1
                end
                ret[i] = is_prime
                i = i + 1
            end
            return ret
        "#;
        let (stack, _) = compile_and_execute_mut(&[LuaminValue::Number(20)], INPUT).unwrap();
        assert_eq!(stack.len(), 1);
        let list = match &stack[0] {
            LuaminValue::List(list) => list,
            _ => panic!(),
        };
        assert_eq!(
            &*list.borrow(),
            &vec![
                LuaminValue::Bool(false),
                LuaminValue::Bool(true),
                LuaminValue::Bool(true),
                LuaminValue::Bool(false),
                LuaminValue::Bool(true),
                LuaminValue::Bool(false),
                LuaminValue::Bool(true),
                LuaminValue::Bool(false),
                LuaminValue::Bool(false),
                LuaminValue::Bool(false),
                LuaminValue::Bool(true),
                LuaminValue::Bool(false),
                LuaminValue::Bool(true),
                LuaminValue::Bool(false),
                LuaminValue::Bool(false),
                LuaminValue::Bool(false),
                LuaminValue::Bool(true),
                LuaminValue::Bool(false),
                LuaminValue::Bool(true),
                LuaminValue::Bool(false),
            ]
        );
    }

    #[test]
    fn test_runtime_13() {
        const INPUT: &str = r#"
            #param n
            local sum = 0
            for i = 1, n do
                if i % 7 == 0 then
                    break
                end
                if i % 3 == 0 then
                    continue
                end
                sum = sum + i
            end
            return sum
        "#;
        let (stack, _) = compile_and_execute_mut(&[LuaminValue::Number(10)], INPUT).unwrap();
        let expected: SmallVec<[LuaminValue; 4]> = smallvec![LuaminValue::Number(12)];
        assert_eq!(stack, expected);
    }

    #[test]
    fn test_runtime_14() {
        const INPUT: &str = r#"
            #param n
            local sum = 0
            for i = n, 1, -2 do
                if i % 7 == 0 then
                    break
                end
                if i % 3 == 0 then
                    continue
                end
                sum = sum + i
            end
            return sum
        "#;
        let (stack, _) = compile_and_execute_mut(&[LuaminValue::Number(20)], INPUT).unwrap();
        let expected: SmallVec<[LuaminValue; 4]> = smallvec![LuaminValue::Number(36)];
        assert_eq!(stack, expected);
    }

    #[test]
    fn test_runtime_15() {
        const INPUT: &str = r#"
            #param n
            local sum = 0
            for i = 1, 10 do
                sum = sum + i
            end
            return sum
        "#;
        let (stack, _) = compile_and_execute_mut(&[LuaminValue::Number(10)], INPUT).unwrap();
        let expected: SmallVec<[LuaminValue; 4]> = smallvec![LuaminValue::Number(55)];
        assert_eq!(stack, expected);
    }

    #[test]
    fn test_runtime_16() {
        const INPUT: &str = r#"
            local list = {10, 20, 30, 40, 50}
            local sum_index, sum = 0, 0
            for i, v in ipairs(list) do
                sum = sum + v
                sum_index = sum_index + i
            end
            return sum_index, sum
        "#;
        let (stack, _) = compile_and_execute_mut(&[LuaminValue::Number(10)], INPUT).unwrap();
        let expected: SmallVec<[LuaminValue; 4]> =
            smallvec![LuaminValue::Number(15), LuaminValue::Number(150)];
        assert_eq!(stack, expected);
    }

    #[test]
    fn test_runtime_17() {
        const INPUT: &str = r#"
            #param return_i, break_i, continue_i
            local list = {10, 20, 30, 40, 50}
            local sum_index, sum = 0, 0
            for i, v in ipairs(list) do
                if i == return_i then
                    return sum_index, sum
                end
                if i == break_i then
                    break
                end
                if i == continue_i then
                    continue
                end
                sum = sum + v
                sum_index = sum_index + i
            end
            return sum_index, sum
        "#;
        let (stack, _) = compile_and_execute_mut(
            &[
                LuaminValue::Number(2),
                LuaminValue::Number(3),
                LuaminValue::Number(4),
            ],
            INPUT,
        )
        .unwrap();
        let expected: SmallVec<[LuaminValue; 4]> =
            smallvec![LuaminValue::Number(1), LuaminValue::Number(10)];
        assert_eq!(stack, expected);

        let (stack, _) = compile_and_execute_mut(
            &[
                LuaminValue::Number(9),
                LuaminValue::Number(3),
                LuaminValue::Number(4),
            ],
            INPUT,
        )
        .unwrap();
        let expected: SmallVec<[LuaminValue; 4]> =
            smallvec![LuaminValue::Number(3), LuaminValue::Number(30)];
        assert_eq!(stack, expected);

        let (stack, _) = compile_and_execute_mut(
            &[
                LuaminValue::Number(9),
                LuaminValue::Number(9),
                LuaminValue::Number(4),
            ],
            INPUT,
        )
        .unwrap();
        let expected: SmallVec<[LuaminValue; 4]> =
            smallvec![LuaminValue::Number(11), LuaminValue::Number(110)];
        assert_eq!(stack, expected);
    }

    #[test]
    fn test_runtime_18() {
        const INPUT: &str = r#"
            #param n
            for i = 1, n do
                local inner0 = 50
                if i % 7 == 0 then
                    local inner00 = 500
                    local inner01 = 501
                    break
                end
                if i % 3 == 0 then
                    local inner10 = 510
                    local inner11 = 511
                    continue
                end
                local inner1 = 51
            end
        "#;
        let (stack, _) = compile_and_execute_mut(&[LuaminValue::Number(10)], INPUT).unwrap();
        assert!(stack.is_empty());
    }

    #[test]
    fn test_runtime_19() {
        const INPUT: &str = r#"
            #param n
            for i = 1, n do
                local inner0 = 50
                if i % 7 == 0 then
                    local inner00 = 500
                    local inner01 = 501
                    break
                end
                if i % 3 == 0 then
                    local inner10 = 510
                    local inner11 = 511
                    continue
                end
                local inner1 = 51
            end
            return n
        "#;
        let (stack, _) = compile_and_execute_mut(&[LuaminValue::Number(10)], INPUT).unwrap();
        let expected: SmallVec<[LuaminValue; 4]> = smallvec![LuaminValue::Number(10)];
        assert_eq!(stack, expected);
    }

    #[test]
    fn test_runtime_20() {
        const INPUT: &str = r#"
            #param n
            for i = 1, n do
                local inner0 = 50
                if i % 7 == 0 then
                    local inner00 = 500
                    local inner01 = 501
                    return i
                end
                if i % 3 == 0 then
                    local inner10 = 510
                    local inner11 = 511
                    continue
                end
                local inner1 = 51
            end
            return n
        "#;
        let (stack, _) = compile_and_execute_mut(&[LuaminValue::Number(10)], INPUT).unwrap();
        let expected: SmallVec<[LuaminValue; 4]> = smallvec![LuaminValue::Number(7)];
        assert_eq!(stack, expected);
    }

    #[test]
    fn test_runtime_21() {
        const INPUT: &str = r#"
            #param n
            local sum = 0
            for i = 1, n do
                if i % 3 == 0 and i % 2 == 0 then
                    continue
                end
                sum = sum + i
            end
            return sum
        "#;
        let (stack, _) = compile_and_execute_mut(&[LuaminValue::Number(7)], INPUT).unwrap();
        let expected: SmallVec<[LuaminValue; 4]> = smallvec![LuaminValue::Number(22)];
        assert_eq!(stack, expected);
    }

    #[test]
    fn test_runtime_22() {
        const INPUT: &str = r#"
            #param n
            local sum = 0
            for i = 1, n do
                if i % 3 == 0 or i % 2 == 0 then
                    continue
                end
                sum = sum + i
            end
            return sum
        "#;
        let (stack, _) = compile_and_execute_mut(&[LuaminValue::Number(7)], INPUT).unwrap();
        let expected: SmallVec<[LuaminValue; 4]> = smallvec![LuaminValue::Number(13)];
        assert_eq!(stack, expected);
    }

    #[test]
    fn test_runtime_23() {
        const INPUT: &str = r#"
            #param n
            return n and 42, n or 43
        "#;
        let (stack, _) = compile_and_execute_mut(&[LuaminValue::Number(7)], INPUT).unwrap();
        let expected: SmallVec<[LuaminValue; 4]> =
            smallvec![LuaminValue::Number(42), LuaminValue::Number(7)];
        assert_eq!(stack, expected);

        let (stack, _) = compile_and_execute_mut(&[LuaminValue::Nil], INPUT).unwrap();
        let expected: SmallVec<[LuaminValue; 4]> =
            smallvec![LuaminValue::Nil, LuaminValue::Number(43)];
        assert_eq!(stack, expected);
    }

    #[test]
    fn test_runtime_24() {
        const INPUT: &str = r#"
            #param is_member
            local price = is_member and 10 or 20
            return price
        "#;
        let (stack, _) = compile_and_execute_mut(&[LuaminValue::Bool(true)], INPUT).unwrap();
        let expected: SmallVec<[LuaminValue; 4]> = smallvec![LuaminValue::Number(10)];
        assert_eq!(stack, expected);

        let (stack, _) = compile_and_execute_mut(&[LuaminValue::Bool(false)], INPUT).unwrap();
        let expected: SmallVec<[LuaminValue; 4]> = smallvec![LuaminValue::Number(20)];
        assert_eq!(stack, expected);
    }

    #[test]
    fn test_runtime_25() {
        const INPUT: &str = r#"
            local n, a, b, c = 0, {0}, {nil, {0}}, {nil, nil, {nil, {0}}}
            n += 142
            n -= 42
            n *= 3
            n //= 2
            n >>= 1
            n ~= 1

            a[1] -= 9
            a[1] //= 2

            b[2][1] += 100
            b[2][1] <<= 2
            b[2][1] |= 3

            c[3][2][1] += 100
            c[3][2][1]%=3

            return n, a[1], b[2][1], c[3][2][1]
        "#;
        let (stack, _) = compile_and_execute_mut(&[], INPUT).unwrap();
        let expected: SmallVec<[LuaminValue; 4]> = smallvec![
            LuaminValue::Number(74),
            LuaminValue::Number(-5),
            LuaminValue::Number(403),
            LuaminValue::Number(1),
        ];
        assert_eq!(stack, expected);
    }

    #[test]
    fn test_runtime_26() {
        const INPUT: &str = r#"
            local n, a, b, c = 0, {0}, {nil, {0}}, {nil, nil, {nil, {0}}}
            n += 142
            n -= 42
            n *= 3
            n //= 2
            n >>= 1
            n ~= 1

            a[1] -= 9
            a[1] //= 2

            b[2][1] += 100
            b[2][1] <<= 2
            b[2][1] |= 3

            c[3][2][1] += 100
            c[3][2][1]%=3
        "#;
        let (stack, _) = compile_and_execute_mut(&[], INPUT).unwrap();
        assert!(stack.is_empty());
    }

    #[test]
    fn test_runtime_27() {
        const INPUT: &str = r#"
            #index 0

            local list = {0, 10, 20, 30}
            return list[0], list[3], list[4]
        "#;
        let (stack, _) = compile_and_execute_mut(&[], INPUT).unwrap();
        let expected: SmallVec<[LuaminValue; 4]> = smallvec![
            LuaminValue::Number(0),
            LuaminValue::Number(30),
            LuaminValue::Nil,
        ];
        assert_eq!(stack, expected);
    }

    #[test]
    fn test_runtime_28() {
        const INPUT: &str = r#"
            #index -1

            local list = {-10, 0, 10, 20}
            return list[-1], list[2], list[3]
        "#;
        let (stack, _) = compile_and_execute_mut(&[], INPUT).unwrap();
        let expected: SmallVec<[LuaminValue; 4]> = smallvec![
            LuaminValue::Number(-10),
            LuaminValue::Number(20),
            LuaminValue::Nil,
        ];
        assert_eq!(stack, expected);
    }

    #[test]
    fn test_runtime_29() {
        const INPUT: &str = r#"
            local returnValue = 7
            returnValue = 8
            return returnValue
        "#;
        let (stack, _) = compile_and_execute_mut(&[], INPUT).unwrap();
        let expected: SmallVec<[LuaminValue; 4]> = smallvec![LuaminValue::Number(8)];
        assert_eq!(stack, expected);
    }

    #[test]
    fn test_runtime_30() {
        const INPUT: &str = r#"
            local revealed = {1, 2, 3}
            local affected = {}
            for _, value in ipairs(revealed) do
                if value > 1 then
                    affected[#affected + 1] = value
                end
            end
            local modifier = 42
            return modifier, #affected, affected[1], affected[2]
        "#;
        let (stack, _) = compile_and_execute_mut(&[], INPUT).unwrap();
        let expected: SmallVec<[LuaminValue; 4]> = smallvec![
            LuaminValue::Number(42),
            LuaminValue::Number(2),
            LuaminValue::Number(2),
            LuaminValue::Number(3),
        ];
        assert_eq!(stack, expected);
    }

    #[test]
    fn test_runtime_31() {
        const INPUT: &str = r#"
            local sum = 0
            for i = 1, 3 do
                sum += i
            end
            local modifier = 42
            return modifier, sum
        "#;
        let (stack, _) = compile_and_execute_mut(&[], INPUT).unwrap();
        let expected: SmallVec<[LuaminValue; 4]> =
            smallvec![LuaminValue::Number(42), LuaminValue::Number(6)];
        assert_eq!(stack, expected);
    }
}
