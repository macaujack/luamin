use luamin_macro::LuaminUserValue;
use smallvec::smallvec;

use crate::{
    codegen::LuaminCodegen,
    func_lib::{FuncPointer, HostFuncDef, HostFuncLibBuf, consume_gas},
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

fn get_func_lib() -> &'static HostFuncLib<StateLifetimeProvider, TestError> {
    type FP = FuncPointer<StateLifetimeProvider, TestError>;
    let func_lib = HostFuncLibBuf::new([
        HostFuncDef::new("get_score", FP::F01(State::get_score)),
        HostFuncDef::new("add_score", FP::F10Mut(State::add_score)),
        HostFuncDef::new("sub_score", FP::F10Mut(State::sub_score)),
        HostFuncDef::new("mul_score", FP::F10Mut(State::mul_score)),
        HostFuncDef::new("div_score", FP::F10Mut(State::div_score)),
        HostFuncDef::new("list_score0", FP::F01(State::list_score0)),
        HostFuncDef::new("list_score1", FP::F02(State::list_score1)),
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
fn assignments_and_host_calls() {
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
fn arithmetic_returns_multiple_values() {
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
fn nested_list_assignments_and_multiple_results() {
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
fn extra_parameters_are_truncated() {
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
fn missing_parameters_become_nil() {
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
fn division_by_zero_returns_error() {
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
fn floor_division_and_comparison() {
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
fn return_exits_nested_scope() {
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
fn nested_if_leap_year() {
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
fn early_return_leap_year() {
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
fn while_loop_builds_list() {
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
fn repeat_loop_runs_at_least_once() {
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
fn nested_loops_find_primes() {
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
fn numeric_for_descends() {
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
fn numeric_for_uses_step() {
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
fn numeric_for_defaults_to_step_one() {
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
fn ipairs_sums_indices_and_values() {
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
fn ipairs_return_break_and_continue() {
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
fn loop_control_cleans_local_stack() {
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
fn loop_control_preserves_parameters() {
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
fn return_exits_numeric_loop() {
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
fn and_short_circuits() {
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
fn or_short_circuits() {
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
fn logical_operators_return_operands() {
    const INPUT: &str = r#"
        #param n
        return n and 42, n or 43
    "#;
    let (stack, _) = compile_and_execute_mut(&[LuaminValue::Number(7)], INPUT).unwrap();
    let expected: SmallVec<[LuaminValue; 4]> =
        smallvec![LuaminValue::Number(42), LuaminValue::Number(7)];
    assert_eq!(stack, expected);

    let (stack, _) = compile_and_execute_mut(&[LuaminValue::Nil], INPUT).unwrap();
    let expected: SmallVec<[LuaminValue; 4]> = smallvec![LuaminValue::Nil, LuaminValue::Number(43)];
    assert_eq!(stack, expected);
}

#[test]
fn logical_operators_select_price() {
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
fn compound_assignments_return_values() {
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
fn compound_assignments_clean_stack() {
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
fn zero_based_list_indexing() {
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
fn negative_based_list_indexing() {
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
fn keyword_prefix_is_valid_identifier() {
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
fn locals_after_list_loop() {
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
fn locals_after_numeric_loop() {
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
