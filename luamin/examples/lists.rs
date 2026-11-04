//! Convert Rust collections and observe shared list storage.
//! Run: cargo run -p luamin --example lists

use std::convert::Infallible;

use luamin::{
    FromLuamin, FuncPointerStateLifetimeProvider, HostFuncLibArray, LuaminCompiler, LuaminGas,
    ToLuamin, luamin_execute,
};

struct StateProvider;
impl FuncPointerStateLifetimeProvider for StateProvider {
    type StateWithLifetimes<'r0, 'r1, 'r2, 'r3> = ();
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let library = HostFuncLibArray::<0, StateProvider, Infallible>::new_const([]);
    let mut compiler = LuaminCompiler::new(&library, 1);
    let (script, _) = compiler.compile(
        &[],
        r#"
        #param numbers
        #index 1
        local doubled = {}
        local total = 0
        -- ipairs stops at the first nil element, including the end of the list.
        for i, value in ipairs(numbers) do
            doubled[i] = value * 2
            total += value
        end
        -- Parameter bindings are read-only. A local alias shares list storage.
        local alias = numbers
        alias[1] = 99
        return doubled, total
        "#,
    )?;

    let input = vec![2_i32, 4, 6].to_luamin();
    let mut gas = LuaminGas::new(1_000);
    let mut values = luamin_execute(&(), &mut gas, &script, std::slice::from_ref(&input))?;
    let doubled =
        Vec::<i32>::from_luamin(values.next().expect("doubled list")).expect("a list of integers");
    let total = i32::from_luamin(values.next().expect("total")).expect("an integer");
    assert_eq!(doubled, vec![4, 8, 12]);
    assert_eq!(total, 12);
    assert_eq!(values.next(), None);

    // Execution clones list handles, so even a shared-state script can mutate
    // the original list. This is not a deep copy of its elements.
    let changed_input = Vec::<i32>::from_luamin(input).expect("input list");
    assert_eq!(changed_input, vec![99, 4, 6]);
    println!("Doubled: {doubled:?}; original sum: {total}");
    println!("Input after script mutation: {changed_input:?}");

    Ok(())
}
