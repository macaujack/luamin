//! Pass an application-defined value through host functions and script execution.
//! Run: cargo run -p luamin --example user_values

use luamin::{
    FromLuamin, HostFuncLibBuf, LuaminCompiler, LuaminExecutor, LuaminGas, LuaminUserValue,
    LuaminValue, ToLuamin, luamin_bindgen,
    runtime::{CloneGasCost, PartialEqGasCost},
};

// The marker derive does not generate Clone, PartialEq, or gas-cost impls.
// Deliberately omit Copy to demonstrate explicit gas costs.
#[derive(Debug, Clone, PartialEq, LuaminUserValue)]
struct Position {
    x: i32,
    y: i32,
}

impl CloneGasCost for Position {
    fn clone_gas_cost(&self) -> i32 {
        1
    }
}

impl PartialEqGasCost for Position {
    fn partial_eq_gas_cost(&self, _other: &Self) -> i32 {
        1
    }
}

#[derive(LuaminExecutor)]
struct Host;

#[derive(Debug, thiserror::Error)]
#[error("incorrect type in {0}, argument {1} (1-based)")]
struct HostError(&'static str, usize);

fn wrong_argument(function: &'static str, argument: usize) -> HostError {
    HostError(function, argument)
}

#[luamin_bindgen(
    private_lifetime_provider_struct,
    error_type = HostError,
    incorrect_arg_type_error_func = wrong_argument
)]
impl Host {
    fn shift(&self, position: Position, dx: i32, dy: i32) -> Result<Position, HostError> {
        Ok(Position {
            x: position.x.wrapping_add(dx),
            y: position.y.wrapping_add(dy),
        })
    }

    fn x(&self, position: Position) -> Result<i32, HostError> {
        Ok(position.x)
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let library = HostFuncLibBuf::new(HostLifetimeProvider::new_func_defs())?;
    let (script, _) = LuaminCompiler::new(&library, 1).compile(
        &[],
        r#"
        #param position
        local moved = shift(position, 3, -2)
        return moved, x(moved), moved == position
        "#,
    )?;
    // Custom fields are accessed through host methods, not script field syntax.
    let position = Position { x: 4, y: 5 };
    let mut values =
        Host.execute_luamin_script(&mut LuaminGas::new(1_000), &script, &[position.to_luamin()])?;
    let moved = Position::from_luamin(values.next().expect("moved position"))
        .expect("the expected concrete user-value type");
    assert_eq!(moved, Position { x: 7, y: 3 });
    assert_eq!(values.next(), Some(LuaminValue::Number(7)));
    assert_eq!(values.next(), Some(LuaminValue::Bool(false)));
    assert_eq!(values.next(), None);
    println!("Moved position: {moved:?}; x=7; equal to original: false");
    Ok(())
}
