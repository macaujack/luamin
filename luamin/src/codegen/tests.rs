use crate::{
    func_lib::{FuncPointer, GasError, HostFuncDef, HostFuncLibBuf, consume_gas},
    lexer::luamin_lex,
    runtime::{LuaminGas, LuaminValue},
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
enum StateError {}

impl State {
    fn get_score(&self, gas: &mut LuaminGas) -> Result<LuaminValue, GasError<StateError>> {
        consume_gas!(gas, 1);
        Ok(LuaminValue::Number(self.score))
    }
    fn inc_score(&mut self, gas: &mut LuaminGas) -> Result<(), GasError<StateError>> {
        consume_gas!(gas, 1);
        self.score += 1;
        Ok(())
    }
}

fn get_codegen() -> LuaminCodegen<'static, StateLifetimeProvider, StateError> {
    let func_lib = HostFuncLibBuf::new([
        HostFuncDef::new(
            "get_score",
            FuncPointer::<StateLifetimeProvider, StateError>::F01(State::get_score),
        ),
        HostFuncDef::new(
            "inc_score",
            FuncPointer::<StateLifetimeProvider, StateError>::F00Mut(State::inc_score),
        ),
    ])
    .unwrap();

    let func_lib = Box::leak(Box::new(func_lib));

    LuaminCodegen::new(func_lib, 1)
}

fn compile(
    input: &str,
) -> Result<
    (
        LuaminScript<'static, StateLifetimeProvider, StateError>,
        Vec<usize>,
    ),
    LuaminCodegenError,
> {
    let mut codegen = get_codegen();
    let (directive, stmts) = luamin_lex(input).unwrap();
    codegen.codegen(&[], &directive, &stmts)
}

fn compile_mut(
    input: &str,
) -> Result<
    (
        LuaminScriptMut<'static, StateLifetimeProvider, StateError>,
        Vec<usize>,
    ),
    LuaminCodegenError,
> {
    let mut codegen = get_codegen();
    let (directive, stmts) = luamin_lex(input).unwrap();
    codegen.codegen_mut(&[], &directive, &stmts)
}

#[test]
fn test_error_when_undeclared() {
    let result = compile_mut(
        r#"
        foo = 1
    "#,
    );
    assert!(result.is_err());

    let result = compile_mut(
        r#"
        do
            local bar = 2
        end
        bar = 3
    "#,
    );
    assert!(result.is_err());
}

#[test]
fn test_error_when_using_mutable_lib_func_in_immutable_context() {
    let err = compile(
        r#"
        local a = 1
        inc_score()
    "#,
    )
    .unwrap_err();
    assert!(matches!(
        err,
        LuaminCodegenError::MutableLibFuncInImmutableContext
    ));
}

#[test]
#[ignore]
fn test_show_ixs() {
    let (script, _) = compile_mut(
        r#"
        local a = {nil, {42}}
        a[2][1] += 100
    "#,
    )
    .unwrap();
    println!("{:#?}", script.ixs());
}
