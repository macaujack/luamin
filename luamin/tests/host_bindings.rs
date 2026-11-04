use std::{assert_matches, cell::RefCell, rc::Rc};

use luamin::{
    func_lib::{HostFuncLibArray, HostFuncLibBuf},
    runtime::{CloneGasCost, LuaminUserValue, PartialEqGasCost},
};

use luamin::smol_str::SmolStr;
use luamin::*;

#[derive(Debug, Clone, LuaminExecutor, Default)]
struct State<T> {
    score: i32,
    details: Vec<i32>,
    player_info: PlayerInfo,
    _phantom: std::marker::PhantomData<T>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, LuaminUserValue)]
struct PlayerInfo {
    name: String,
    register_time: i64,
}

impl CloneGasCost for PlayerInfo {
    fn clone_gas_cost(&self) -> i32 {
        10
    }
}
impl PartialEqGasCost for PlayerInfo {
    fn partial_eq_gas_cost(&self, _other: &Self) -> i32 {
        10
    }
}

struct FuncDefExtra {
    precedence: usize,
}

#[gen_luamin_func(
    private_lifetime_provider_struct,
    incorrect_arg_type_error_func = new_incorrect_type_error,
    error_type = StateError,
    func_def_extra_struct = FuncDefExtra,
    func_def_extra_struct_fields_required,
    const_func_lib
)]
impl<T> State<T> {
    #[extra(precedence = 100)]
    pub fn get_multiple_of_score(&self, multiplier: i32) -> Result<i32, StateError> {
        Ok(self.score * multiplier)
    }

    #[extra(precedence = 200)]
    pub fn add_score(&mut self, score_delta: i32) -> Result<i32, StateError> {
        self.score += score_delta;
        Ok(self.score)
    }

    #[extra(precedence = 300)]
    pub fn preview_sum_scores(&self, scores: &[i32]) -> Result<i32, StateError> {
        let mut sum = 0;
        for score in scores {
            sum += *score;
        }
        Ok(sum)
    }

    #[extra(precedence = 300)]
    pub fn list_scores(&self) -> Result<&[i32], StateError> {
        Ok(&self.details)
    }

    #[extra(precedence = 300)]
    pub fn preview_add_score_ref(&self, score_delta: &i32) -> Result<i32, StateError> {
        Ok(self.score + *score_delta)
    }

    #[extra(precedence = 300)]
    pub fn add_2_scores(&mut self, scores: [i32; 2]) -> Result<(), StateError> {
        self.score += scores[0] + scores[1];
        Ok(())
    }

    #[extra(precedence = 300)]
    pub fn get_shorter_list(
        &self,
        list1: Rc<RefCell<Vec<LuaminValue>>>,
        list2: Rc<RefCell<Vec<LuaminValue>>>,
    ) -> Result<Rc<RefCell<Vec<LuaminValue>>>, StateError> {
        if list1
            .try_borrow()
            .map_err(|_| StateError::ListLocked)?
            .len()
            <= list2
                .try_borrow()
                .map_err(|_| StateError::ListLocked)?
                .len()
        {
            Ok(list1)
        } else {
            Ok(list2)
        }
    }

    #[extra(precedence = 300)]
    pub fn get_score_ref(&self) -> Result<&i32, StateError> {
        Ok(&self.score)
    }

    #[extra(precedence = 300)]
    pub fn get_player_info(
        &self,
        gas: &mut LuaminGas,
    ) -> Result<&PlayerInfo, GasError<StateError>> {
        gas.try_consume(3)?;
        Ok(&self.player_info)
    }

    #[extra(precedence = 300)]
    #[gas(5)]
    pub fn get_owned_player_info(&self) -> Result<PlayerInfo, StateError> {
        Ok(self.player_info.clone())
    }
}

#[derive(Debug, thiserror::Error)]
enum StateError {
    #[error("Incorrect type in function call '{0}', argument {1} (1-based)")]
    IncorrectType(&'static str, usize),
    #[error("List is locked")]
    ListLocked,
}

fn new_incorrect_type_error(func_name: &'static str, arg_index: usize) -> StateError {
    StateError::IncorrectType(func_name, arg_index)
}

fn get_compiler() -> LuaminCompiler<'static, StateLifetimeProvider<()>, StateError, FuncDefExtra> {
    static FUNC_LIB: &HostFuncLib<StateLifetimeProvider<()>, StateError, FuncDefExtra> =
        &HostFuncLibArray::new_const(StateLifetimeProvider::<()>::new_func_def_statics());

    LuaminCompiler::new(FUNC_LIB, 1)
}

#[test]
fn test_func_lib() {
    let func_lib = HostFuncLibBuf::new(StateLifetimeProvider::<()>::new_func_defs()).unwrap();
    let (_, func_def) = func_lib.get_func_def("add_score").unwrap();
    assert_eq!(func_def.extra.precedence, 200);
}

#[test]
fn test_compile_and_execute() {
    let mut compiler = get_compiler();
    let (luamin_script, _) = compiler
        .compile(
            &[SmolStr::new_static("a"), SmolStr::new_static("b")],
            r#"
            return a + b
        "#,
        )
        .unwrap();
    let state = State::default();
    let mut gas = LuaminGas::INF;
    let mut return_values = state
        .execute_luamin_script(
            &mut gas,
            &luamin_script,
            &[LuaminValue::Number(3), LuaminValue::Number(4)],
        )
        .unwrap();
    assert_eq!(return_values.len(), 1);
    assert_eq!(return_values.next().unwrap(), LuaminValue::Number(7));
}

#[test]
fn test_array_conversion() {
    let mut compiler = get_compiler();
    let (luamin_script, _) = compiler
        .compile_mut(
            &[SmolStr::new_static("a"), SmolStr::new_static("b")],
            r#"
            add_2_scores({a, b})
        "#,
        )
        .unwrap();
    let mut state = State::default();
    let mut gas = LuaminGas::INF;
    let return_values = state
        .execute_luamin_script_mut(
            &mut gas,
            &luamin_script,
            &[LuaminValue::Number(600), LuaminValue::Number(66)],
        )
        .unwrap();
    assert_eq!(return_values.len(), 0);
    assert_eq!(state.score, 666);
}

#[test]
fn test_raw_list_conversion() {
    let mut compiler = get_compiler();
    let (luamin_script, _) = compiler
        .compile(
            &[],
            r#"
            local list1 = {1, 2, 3, 4}
            local list2 = {5, 6, 7}
            local shorter_list = get_shorter_list(list1, list2)
            shorter_list[#shorter_list + 1] = nil
            return list2
        "#,
        )
        .unwrap();
    let state = State::default();
    let mut gas = LuaminGas::INF;
    let return_values = state
        .execute_luamin_script(&mut gas, &luamin_script, &[])
        .unwrap();
    assert_eq!(return_values.len(), 1);
    let returned_list = match return_values.into_iter().next().unwrap() {
        LuaminValue::List(list) => list,
        _ => panic!(),
    };
    let returned_list = Rc::try_unwrap(returned_list).unwrap().into_inner();
    let expected: Vec<LuaminValue> = vec![
        LuaminValue::Number(5),
        LuaminValue::Number(6),
        LuaminValue::Number(7),
        LuaminValue::Nil,
    ];
    assert_eq!(returned_list, expected);
}

#[test]
fn test_gas_exceeded() {
    let mut compiler = get_compiler();
    let (luamin_script, _) = compiler
        .compile(
            &[],
            r#"
            local a = get_owned_player_info()
        "#,
        )
        .unwrap();
    let state = State::default();

    let mut gas = LuaminGas::INF;
    let return_values = state
        .execute_luamin_script(&mut gas, &luamin_script, &[])
        .unwrap();
    assert_eq!(return_values.len(), 0);

    let mut gas = LuaminGas::new(4);
    let err = state
        .execute_luamin_script(&mut gas, &luamin_script, &[])
        .unwrap_err();
    assert_matches!(err, GasError::GasExceeded);
}
