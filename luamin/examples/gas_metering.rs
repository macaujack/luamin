//! Handle interpreter exhaustion and meter host work with fixed or manual costs.
//! Run: cargo run -p luamin --example gas_metering

use luamin::{
    GasError, HostFuncLibBuf, LuaminCompiler, LuaminExecutor, LuaminGas, LuaminValue,
    luamin_bindgen,
};

#[derive(LuaminExecutor)]
struct Host;

#[derive(Debug, thiserror::Error)]
enum HostError {
    #[error("incorrect type in {0}, argument {1} (1-based)")]
    Argument(&'static str, usize),
    #[error("expected a number between 0 and 100")]
    OutOfRange,
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
    // Ordinary wrappers default to one unit; this method charges five instead.
    #[gas(5)]
    fn double(&self, value: i32) -> Result<i32, HostError> {
        Ok(value.wrapping_mul(2))
    }

    // A gas parameter immediately after self opts into manual metering.
    // Charge before each unit of host work and propagate exhaustion immediately.
    fn sum_to(&self, gas: &mut LuaminGas, n: i32) -> Result<i32, GasError<HostError>> {
        if !(0..=100).contains(&n) {
            return Err(GasError::Other(HostError::OutOfRange));
        }
        let mut total = 0;
        for value in 1..=n {
            gas.try_consume(1)?;
            total += value;
        }
        Ok(total)
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let library = HostFuncLibBuf::new(HostLifetimeProvider::new_func_defs())?;
    let mut compiler = LuaminCompiler::new(&library, 1);
    let (script, _) = compiler.compile(&[], "return double(4), sum_to(5)")?;
    let mut gas = LuaminGas::new(100);
    let values: Vec<_> = Host.execute_script(&mut gas, &script, &[])?.collect();
    assert_eq!(
        values,
        vec![LuaminValue::Number(8), LuaminValue::Number(15)]
    );
    println!("Results: {values:?}; remaining gas: {gas}");
    // Interpreter instructions and argument/result dispatch also consume gas.

    let mut host_gas = LuaminGas::new(2);
    assert!(matches!(
        Host.sum_to(&mut host_gas, 5),
        Err(GasError::GasExceeded)
    ));
    println!("Manual host work stopped when its budget ran out");

    let (loop_script, _) = compiler.compile(&[], "while true do end")?;
    let mut loop_gas = LuaminGas::new(20);
    let error = Host
        .execute_script(&mut loop_gas, &loop_script, &[])
        .expect_err("an unbounded loop must exhaust its finite budget");
    assert!(matches!(error, GasError::GasExceeded));
    assert!(loop_gas.has_exceeded());
    assert!(loop_gas.consume(0)); // Exhaustion remains sticky.
    println!("Script loop stopped: {error}");
    Ok(())
}
