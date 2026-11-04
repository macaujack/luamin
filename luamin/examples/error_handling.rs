//! Distinguish parsing, compilation, runtime, and host errors.
//! Run: cargo run -p luamin --example error_handling

use luamin::{
    GasError, HostFuncLibBuf, LuaminCompileError, LuaminCompiler, LuaminExecutor, LuaminGas,
    LuaminRuntimeError, codegen::LuaminCodegenError, func_lib::FuncPointerCallError,
    luamin_bindgen,
};

#[derive(LuaminExecutor)]
struct Host;

#[derive(Debug, thiserror::Error)]
enum HostError {
    #[error("incorrect type in {0}, argument {1} (1-based)")]
    Argument(&'static str, usize),
    #[error("host rejected a zero divisor")]
    ZeroDivisor,
}

fn wrong_argument(function: &'static str, argument: usize) -> HostError {
    HostError::Argument(function, argument)
}

#[luamin_bindgen(
    private_lifetime_provider_struct,
    error_type = HostError,
    incorrect_arg_type_error_func = wrong_argument
)]
impl Host {
    fn divide(&self, value: i32, divisor: i32) -> Result<i32, HostError> {
        if divisor == 0 {
            return Err(HostError::ZeroDivisor);
        }
        Ok(value.wrapping_div(divisor))
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let library = HostFuncLibBuf::new(HostLifetimeProvider::new_func_defs())?;
    let mut compiler = LuaminCompiler::new(&library, 1);

    let parse_error = compiler
        .compile(&[], "local =")
        .expect_err("invalid syntax");
    if let LuaminCompileError::ParseError(error) = &parse_error {
        println!(
            "Parsing: {error}; line={}, column={}",
            error.line, error.column
        );
    } else {
        panic!("expected a parsing error: {parse_error}");
    }

    let compile_error = compiler
        .compile(&[], "return missing_variable")
        .expect_err("unresolved variable");
    assert!(matches!(
        &compile_error,
        LuaminCompileError::CodegenError(LuaminCodegenError::UndeclaredIdent(_))
    ));
    println!("Compilation: {compile_error}");

    // The same compiler remains reusable after failed compilations.
    let (script, _) = compiler.compile(&[], "return 1 // 0")?;
    let runtime_error = Host
        .execute_script(&mut LuaminGas::new(100), &script, &[])
        .expect_err("division by zero");
    assert!(matches!(
        &runtime_error,
        GasError::Other(LuaminRuntimeError::DivisionByZero)
    ));
    println!("Runtime: {runtime_error}");

    for (source, expect_argument_error) in [
        ("return divide(10, 0)", false),
        ("return divide(true, 2)", true),
    ] {
        let (script, _) = compiler.compile(&[], source)?;
        let error = Host
            .execute_script(&mut LuaminGas::new(100), &script, &[])
            .expect_err("host or argument error");
        match &error {
            GasError::Other(LuaminRuntimeError::FuncPointerCallError(
                FuncPointerCallError::LibError(host_error),
            )) => match (host_error.as_ref(), expect_argument_error) {
                (HostError::Argument("divide", 1), true) => {}
                (HostError::ZeroDivisor, false) => {}
                other => panic!("unexpected host error: {other:?}"),
            },
            other => panic!("unexpected execution error: {other:?}"),
        }
        println!("Host/argument conversion: {error}");
    }
    // Errors do not roll back state changes performed before the failure.
    Ok(())
}
