//! Compile once and execute with different inputs.
//! Run: cargo run -p luamin --example basic

use std::convert::Infallible;

use luamin::{
    HostFuncLibArray, HostStateProvider, IntoLuamin, LuaminCompiler, LuaminGas, LuaminValue,
    luamin_execute,
};

// No host functions are needed, so the execution state is simply ().
struct StateProvider;
impl HostStateProvider for StateProvider {
    type StateWithLifetimes<'r0, 'r1, 'r2, 'r3> = ();
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let library = HostFuncLibArray::<0, StateProvider, Infallible>::new([]);
    let mut compiler = LuaminCompiler::new(&library, 1);
    let (script, used_functions) = compiler.compile(
        &[],
        r#"
        #param a, b
        return a + b, a * b
        "#,
    )?;
    assert!(used_functions.is_empty());

    // Initial values match the source parameter order. Each run gets fresh gas.
    for (a, b) in [(3_i32, 4_i32), (10, 2)] {
        let mut gas = LuaminGas::new(100);
        let values: Vec<_> =
            luamin_execute(&(), &mut gas, &script, &[a.into_luamin(), b.into_luamin()])?.collect();
        assert_eq!(
            values,
            vec![LuaminValue::Number(a + b), LuaminValue::Number(a * b)]
        );
        println!("a={a}, b={b}: {values:?}; remaining gas: {gas}");
    }
    Ok(())
}
