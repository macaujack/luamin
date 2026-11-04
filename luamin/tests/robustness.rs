use std::{cell::RefCell, error::Error as _, rc::Rc};

use luamin::{
    LuaminCompiler, LuaminExecutor, LuaminGas, LuaminRuntimeError, LuaminValue, MAX_SOURCE_BYTES,
    MAX_SYNTAX_DEPTH, ParseErrorKind,
    func_lib::{GasError, HostFuncLibArray},
    parser::luamin_parse,
    runtime::MAX_LIST_LEN,
};

#[derive(luamin::LuaminExecutor)]
struct State;

#[derive(Debug, thiserror::Error)]
#[error("incorrect argument")]
struct HostError;
fn wrong_argument(_: &'static str, _: usize) -> HostError {
    HostError
}

#[luamin::luamin_bindgen(private_lifetime_provider_struct, error_type = HostError, incorrect_arg_type_error_func = wrong_argument)]
impl State {}

fn run(
    source: &str,
    gas: &mut LuaminGas,
) -> Result<Vec<LuaminValue>, GasError<LuaminRuntimeError<HostError>>> {
    let lib = HostFuncLibArray::new(StateLifetimeProvider::new_func_def_statics());
    let (script, _) = LuaminCompiler::new(&lib, 1).compile(&[], source).unwrap();
    State
        .execute_script(gas, &script, &[])
        .map(Iterator::collect)
}

#[test]
fn gas_exhaustion_is_sticky_and_cannot_overflow_or_replenish() {
    let mut max_gas = LuaminGas::MAX;
    assert_eq!(max_gas, LuaminGas::new(i32::MAX));
    assert!(!max_gas.consume(i32::MAX - 1));
    assert!(!max_gas.consume(1));
    assert!(max_gas.consume(1));

    for initial in [i32::MIN, -1, 0, 1, i32::MAX] {
        for cost in [i32::MIN, -1, 0, 1, i32::MAX] {
            let mut gas = LuaminGas::new(initial);
            let exhausted = gas.consume(cost);
            assert_eq!(exhausted, initial < 0 || cost < 0 || cost > initial);
            if exhausted {
                assert!(gas.consume(0));
                assert!(gas.consume(-1));
                assert!(gas.has_exceeded());
            }
        }
    }
    let mut gas = LuaminGas::new(5);
    assert!(gas.try_consume::<HostError>(5).is_ok());
    assert!(gas.try_consume::<HostError>(0).is_ok());
    assert!(matches!(
        gas.try_consume::<HostError>(1),
        Err(GasError::GasExceeded)
    ));
}

#[test]
fn gas_errors_preserve_underlying_error_sources() {
    let error = GasError::Other(wrong_argument("test", 1));
    assert_eq!(error.to_string(), "incorrect argument");
    assert!(error.source().is_some());
    assert!(GasError::<HostError>::GasExceeded.source().is_none());
}

#[test]
fn cyclic_lists_and_borrowed_lists_have_bounded_debug_output() {
    let list = Rc::new(RefCell::new(Vec::new()));
    let value = LuaminValue::List(list.clone());
    list.borrow_mut().push(value.clone());
    assert_eq!(format!("{value:?}"), "{<cycle>}");
    let borrowed = list.borrow_mut();
    assert_eq!(format!("{value:?}"), "<borrowed>");
    drop(borrowed);
    // Lists deliberately share ownership. Break the test's cycle so it drops.
    list.borrow_mut().clear();

    let mut deep = LuaminValue::Nil;
    for _ in 0..64 {
        deep = LuaminValue::List(Rc::new(RefCell::new(vec![deep])));
    }
    assert!(format!("{deep:?}").contains("<depth limit>"));
}

#[test]
fn source_errors_keep_original_utf8_positions() {
    let source = "\n  -- 中文\n  local = 1";
    let error = luamin_parse(source).unwrap_err();
    assert_eq!(error.kind, ParseErrorKind::Syntax);
    assert_eq!(error.line, 3);
    assert!(source.is_char_boundary(error.offset));
    assert_eq!(
        source[..error.offset]
            .lines()
            .last()
            .unwrap()
            .chars()
            .count()
            + 1,
        error.column
    );
}

#[test]
fn parser_limits_reject_deep_or_oversized_source() {
    let deep = format!(
        "return {}1{}",
        "(".repeat(MAX_SYNTAX_DEPTH),
        ")".repeat(MAX_SYNTAX_DEPTH)
    );
    assert_eq!(
        luamin_parse(&deep).unwrap_err().kind,
        ParseErrorKind::NestingTooDeep
    );
    let huge = " ".repeat(MAX_SOURCE_BYTES + 1);
    assert_eq!(
        luamin_parse(&huge).unwrap_err().kind,
        ParseErrorKind::SourceTooLarge
    );
    let operators = format!("return {}1", "not ".repeat(2_000));
    assert!(luamin_parse(&operators).is_err());
    let operators = format!("return 1{}", "+1".repeat(2_000));
    assert!(luamin_parse(&operators).is_err());
    assert!(
        luamin_parse(&format!(
            "return h\"{}\" -- {}",
            "(".repeat(100),
            "do ".repeat(100)
        ))
        .is_ok()
    );
    for depth in 0..=MAX_SYNTAX_DEPTH + 1 {
        let source = format!("return {}1{}", "(".repeat(depth), ")".repeat(depth));
        let _ = luamin_parse(&source);
        let source = format!("{}return 1{}", "do ".repeat(depth), " end".repeat(depth));
        let _ = luamin_parse(&source);
    }
}

#[test]
fn syntax_limits_count_nodes_independently_of_literal_width() {
    use luamin::component::{LiteralExpr, Stmt, ValueExpr};

    for number in [1, 123_456_789] {
        let source = format!("return {{{}}}", vec![number.to_string(); 2_000].join(","));
        let (_, stmts) = luamin_parse(&source).unwrap();
        let [Stmt::Return(exprs)] = stmts.as_slice() else {
            panic!("Expected a return statement");
        };
        let [ValueExpr::LiteralList(values)] = exprs.as_slice() else {
            panic!("Expected a list literal");
        };
        assert_eq!(values.len(), 2_000);
        assert!(values.iter().all(|value| {
            matches!(value, ValueExpr::Literal(LiteralExpr::Number(n)) if *n == number)
        }));
    }

    // Removing the approximate token limit must retain the actual AST node limit.
    let source = format!(
        "return {{{}}}",
        vec!["1"; luamin::MAX_SYNTAX_NODES].join(",")
    );
    assert!(luamin_parse(&source).is_err());
}

#[test]
fn list_growth_and_initial_stacks_are_bounded() {
    let source = format!(
        "local list = {{}}; list[{}] = 1; return list",
        MAX_LIST_LEN + 1
    );
    assert!(matches!(
        run(&source, &mut LuaminGas::new(i32::MAX)),
        Err(GasError::Other(LuaminRuntimeError::ListTooLarge))
    ));
    let lib = HostFuncLibArray::new(StateLifetimeProvider::new_func_def_statics());
    let (script, _) = LuaminCompiler::new(&lib, 1)
        .compile(&[], "return 1")
        .unwrap();
    let oversized = vec![LuaminValue::Nil; luamin::MAX_RUNTIME_STACK_SIZE + 1];
    assert!(matches!(
        State.execute_script(&mut LuaminGas::new(i32::MAX), &script, &oversized),
        Err(GasError::Other(LuaminRuntimeError::StackOverflow))
    ));
}

#[test]
fn integer_arithmetic_wraps_consistently_and_division_is_floored() {
    let result = run(
        "return 2147483647 + 1, -7 // 3, -7 % 3",
        &mut LuaminGas::new(100),
    )
    .unwrap();
    assert_eq!(
        result,
        vec![
            LuaminValue::Number(i32::MIN),
            LuaminValue::Number(-3),
            LuaminValue::Number(2)
        ]
    );
}

#[test]
fn compiler_can_be_reused_after_parse_and_codegen_errors() {
    let lib = HostFuncLibArray::new(StateLifetimeProvider::new_func_def_statics());
    let mut compiler = LuaminCompiler::new(&lib, 1);
    assert!(compiler.compile(&[], "local =").is_err());
    assert!(compiler.compile(&[], "return unknown").is_err());
    let (script, _) = compiler.compile(&[], "return 42").unwrap();
    let result: Vec<_> = State
        .execute_script(&mut LuaminGas::new(100), &script, &[])
        .unwrap()
        .collect();
    assert_eq!(result, vec![LuaminValue::Number(42)]);
}

#[test]
fn generated_arithmetic_matches_rust_wrapping_arithmetic() {
    let mut seed = 0x1234_5678_u32;
    for _ in 0..256 {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let a = (seed & 0x7fff_ffff) as i32;
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let b = (seed & 0x7fff_ffff) as i32;
        let source = format!("return {a}+{b}, {a}-{b}, {a}*{b}");
        assert_eq!(
            run(&source, &mut LuaminGas::new(100)).unwrap(),
            vec![
                LuaminValue::Number(a.wrapping_add(b)),
                LuaminValue::Number(a.wrapping_sub(b)),
                LuaminValue::Number(a.wrapping_mul(b)),
            ]
        );
    }
}

#[test]
fn malformed_source_smoke_fuzz_does_not_panic() {
    let alphabet = [
        'a', ' ', '0', '-', '+', '{', '}', '(', ')', '[', ']', '=', '\n', '"', '\\', '中', '🦀',
    ];
    let mut seed = 0x9876_5432_u32;
    for _ in 0..2_000 {
        let mut source = String::new();
        for _ in 0..48 {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            source.push(alphabet[seed as usize % alphabet.len()]);
        }
        let _ = luamin_parse(&source);
    }
}
