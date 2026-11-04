//! Expose borrowed application data without leaking it or requiring static state.
//! Run: cargo run -p luamin --example borrowed_state

use luamin::{
    HostFuncLibBuf, LuaminCompiler, LuaminExecutor, LuaminGas, LuaminValue, luamin_bindgen,
};

#[derive(LuaminExecutor)]
struct Measurements<'a> {
    readings: &'a [i32],
}

#[derive(Debug, thiserror::Error)]
enum HostError {
    #[error("incorrect type in {0}, argument {1} (1-based)")]
    Argument(&'static str, usize),
    #[error("measurement index out of bounds: {0}")]
    Index(i32),
    #[error("too many measurements to represent with an i32 length")]
    TooManyMeasurements,
}

fn wrong_argument(function: &'static str, argument: usize) -> HostError {
    HostError::Argument(function, argument)
}

#[luamin_bindgen(
    private_lifetime_provider_struct,
    error_type = HostError,
    incorrect_arg_type_error_func = wrong_argument
)]
impl<'a> Measurements<'a> {
    fn count(&self) -> Result<i32, HostError> {
        i32::try_from(self.readings.len()).map_err(|_| HostError::TooManyMeasurements)
    }

    // This host method deliberately exposes one-based indices to the script.
    fn reading(&self, index: i32) -> Result<i32, HostError> {
        let offset = usize::try_from(index)
            .ok()
            .and_then(|index| index.checked_sub(1))
            .ok_or(HostError::Index(index))?;
        self.readings
            .get(offset)
            .copied()
            .ok_or(HostError::Index(index))
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // These readings live on this stack frame, rather than in static storage.
    let readings = [12, 18, 24];
    let state = Measurements {
        readings: &readings,
    };
    let library = HostFuncLibBuf::new(MeasurementsLifetimeProvider::new_func_defs())?;
    let (script, _) = LuaminCompiler::new(&library, 1).compile(
        &[],
        r#"
        local total = 0
        for i = 1, count() do
            total += reading(i)
        end
        return total
        "#,
    )?;
    // The generated lifetime provider maps execution lifetimes to Measurements<'a>.
    // The compiled script borrows its library, not this particular state instance.
    let values: Vec<_> = state
        .execute_script(&mut LuaminGas::new(1_000), &script, &[])?
        .collect();
    assert_eq!(values, vec![LuaminValue::Number(54)]);
    println!("Borrowed readings: {readings:?}; script total: {values:?}");
    Ok(())
}
