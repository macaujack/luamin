use super::*;

#[test]
fn const_library_supports_empty_arrays() {
    let lib = HostFuncLibArray::<0, StateLifetimeProvider, TestError>::new_const([]);
    let lib: &HostFuncLib<StateLifetimeProvider, TestError> = &lib;
    assert!(lib.get_func_def("missing").is_none());
}

#[test]
fn const_library_sorts_and_preserves_extra_values() {
    let lib = HostFuncLibArray::new_const([
        HostFuncDefStatic {
            func_name: "z",
            func_pointer: FuncPointer::<StateLifetimeProvider, TestError>::F00Mut(f00_mut),
            extra: 42_u32,
        },
        HostFuncDefStatic {
            func_name: "a",
            func_pointer: FuncPointer::<StateLifetimeProvider, TestError>::F00Mut(f00_mut),
            extra: 7_u32,
        },
    ]);
    let lib: &HostFuncLib<StateLifetimeProvider, TestError, u32> = &lib;
    assert_eq!(lib.get_func_def("a").unwrap().0, 0);
    assert_eq!(lib.get_func_def("z").unwrap().1.extra, 42);
}

#[test]
#[should_panic(expected = "Duplicated function name")]
fn const_library_rejects_duplicate_names() {
    let _ = HostFuncLibArray::new_const([
        HostFuncDefStatic {
            func_name: "same",
            func_pointer: FuncPointer::<StateLifetimeProvider, TestError>::F00Mut(f00_mut),
            extra: (),
        },
        HostFuncDefStatic {
            func_name: "same",
            func_pointer: FuncPointer::<StateLifetimeProvider, TestError>::F00Mut(f00_mut),
            extra: (),
        },
    ]);
}

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

fn f11(s: &State, gas: &mut LuaminGas, a: LuaminValue) -> Result<LuaminValue, GasError<TestError>> {
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
        HostFuncDef::<StateLifetimeProvider, TestError, FuncDefExtra>::new_with_extra(
            "f00_mut",
            FuncPointer::<StateLifetimeProvider, TestError>::F00Mut(f00_mut),
            FuncDefExtra { extra: 0 },
        ),
        HostFuncDef::new_with_extra(
            "f22_mut",
            FuncPointer::<StateLifetimeProvider, TestError>::F22Mut(f22_mut),
            FuncDefExtra { extra: 2 },
        ),
        HostFuncDef::new_with_extra(
            "f11",
            FuncPointer::<StateLifetimeProvider, TestError>::F11(f11),
            FuncDefExtra { extra: 1 },
        ),
    ];
    let func_lib = HostFuncLibBuf::new(func_defs.clone()).unwrap();
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
    assert!(HostFuncLibBuf::new(func_defs).is_err());
}
