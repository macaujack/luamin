//! Re-enter a compiled script from a Rust host method called by that script.
//! Run: cargo run -p luamin --example recursive
//!
//! This is host-mediated recursion: Luamin does not define a recursive function.
//! Each execution has its own VM stack, while all calls share one gas budget.

use luamin::{
    GasError, HostFuncLibBuf, LuaminCompiler, LuaminExecutor, LuaminGas, LuaminRuntimeError,
    LuaminScriptMut, LuaminValue, TryFromLuamin, luamin_bindgen,
};

// The script borrows its library; the host borrows the compiled script.
// The macro below generates HostLifetimeProvider for this borrowed state type.
#[derive(LuaminExecutor)]
struct Host<'a> {
    script: &'a LuaminScriptMut<'a, HostLifetimeProvider, HostError>,
    host_calls: usize,
}

#[derive(Debug, thiserror::Error)]
enum HostError {
    #[error("incorrect type in {0}, argument {1} (1-based)")]
    Argument(&'static str, usize),
    #[error("recursion depth must be between 0 and 16")]
    InvalidDepth,
    #[error("expected one integer return value")]
    InvalidReturn,
    #[error("nested script failed: {0}")]
    Nested(#[source] Box<LuaminRuntimeError<HostError>>),
}

fn wrong_argument(function: &'static str, argument: usize) -> HostError {
    HostError::Argument(function, argument)
}

// Keep run() in a separate impl: only rust_step() is exposed to scripts.
#[luamin_bindgen(
    private_lifetime_provider_struct,
    error_type = HostError,
    incorrect_arg_type_error_func = wrong_argument
)]
impl<'a> Host<'a> {
    fn rust_step(&mut self, gas: &mut LuaminGas, depth: i32) -> Result<i32, GasError<HostError>> {
        // Manual metering lets the nested execution reuse the caller's budget.
        gas.try_consume(1)?;
        self.host_calls += 1;
        println!("Luamin -> Rust: rust_step({depth})");
        self.run(gas, depth)
    }
}

impl Host<'_> {
    fn run(&mut self, gas: &mut LuaminGas, depth: i32) -> Result<i32, GasError<HostError>> {
        // Nested executions use the native Rust call stack. Bound the depth as
        // well as the gas budget; gas alone is not a native-stack depth limit.
        if !(0..=16).contains(&depth) {
            return Err(GasError::Other(HostError::InvalidDepth));
        }
        println!("Rust -> Luamin: depth = {depth}");
        // Copy the shared script reference, then reborrow the host mutably for
        // this invocation. The compiled instructions themselves stay unchanged.
        let script = self.script;
        let mut values = self
            .execute_script_mut(gas, script, &[LuaminValue::Number(depth)])
            .map_err(|error| match error {
                // Preserve exhaustion so it propagates through every Rust/VM boundary.
                GasError::GasExceeded => GasError::GasExceeded,
                GasError::Other(error) => GasError::Other(HostError::Nested(Box::new(error))),
            })?;
        let value = values
            .next()
            .and_then(i32::try_from_luamin)
            .ok_or(GasError::Other(HostError::InvalidReturn))?;
        if values.next().is_some() {
            return Err(GasError::Other(HostError::InvalidReturn));
        }
        println!("Luamin -> Rust: returned {value} at depth {depth}");
        Ok(value)
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let library = HostFuncLibBuf::new(HostLifetimeProvider::new_func_defs())?;
    let (script, _) = LuaminCompiler::new(&library, 1).compile_mut(
        &[],
        r#"
        #param depth
        if depth == 0 then
            return 0
        end
        return rust_step(depth - 1) + 1
        "#,
    )?;
    let mut host = Host {
        script: &script,
        host_calls: 0,
    };

    let mut gas = LuaminGas::new(1_000);
    let result = host.run(&mut gas, 4)?;
    assert_eq!(result, 4);
    assert_eq!(host.host_calls, 4);
    println!(
        "Result: {result}; Rust callbacks: {}; remaining gas: {gas}",
        host.host_calls
    );

    // A small budget exhausts during a nested invocation and reaches the caller
    // as GasExceeded, rather than being wrapped as a host application error.
    host.host_calls = 0;
    let mut small_budget = LuaminGas::new(20);
    assert!(matches!(
        host.run(&mut small_budget, 4),
        Err(GasError::GasExceeded)
    ));
    assert!(host.host_calls > 0);
    assert!(small_budget.has_exceeded());
    println!("Nested execution stopped when the shared gas budget ran out");
    Ok(())
}
