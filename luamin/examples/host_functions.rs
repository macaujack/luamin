//! Bind shared and mutable Rust methods to script functions.
//! Run: cargo run -p luamin --example host_functions

use luamin::{
    HostFuncLibBuf, LuaminCompiler, LuaminExecutor, LuaminGas, LuaminValue, gen_luamin_func,
};

#[derive(LuaminExecutor)]
struct Counter {
    score: i32,
}

#[derive(Debug, thiserror::Error)]
#[error("incorrect type in {function}, argument {argument} (1-based)")]
struct HostError {
    function: &'static str,
    argument: usize,
}

fn wrong_argument(function: &'static str, argument: usize) -> HostError {
    HostError { function, argument }
}

#[gen_luamin_func(
    private_lifetime_provider_struct,
    error_type = HostError,
    incorrect_arg_type_error_func = wrong_argument
)]
impl Counter {
    fn score(&self) -> Result<i32, HostError> {
        Ok(self.score)
    }

    fn add_score(&mut self, amount: i32) -> Result<(), HostError> {
        self.score += amount;
        Ok(())
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The macro generates this provider and its function-definition constructor.
    let library = HostFuncLibBuf::new(CounterLifetimeProvider::new_func_defs())?;
    let mut compiler = LuaminCompiler::new(&library, 1);
    let mut counter = Counter { score: 10 };

    let (read_script, _) = compiler.compile(&[], "return score()")?;
    let values: Vec<_> = counter
        .execute_luamin_script(&mut LuaminGas::new(100), &read_script, &[])?
        .collect();
    assert_eq!(values, vec![LuaminValue::Number(10)]);
    println!("Before update: {values:?}");

    // compile_mut permits &mut self host methods; execution borrows state mutably.
    let (update_script, used_functions) = compiler.compile_mut(
        &[],
        r#"
        add_score(7)
        return score()
        "#,
    )?;
    let values: Vec<_> = counter
        .execute_luamin_script_mut(&mut LuaminGas::new(100), &update_script, &[])?
        .collect();
    assert_eq!(counter.score, 17);
    assert_eq!(values, vec![LuaminValue::Number(17)]);
    println!("After update: {values:?}; used library indices: {used_functions:?}");
    Ok(())
}
