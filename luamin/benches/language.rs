use std::{hint::black_box, time::Instant};

use luamin::{FuncLibArray, LuaminCompiler, LuaminExecutor, LuaminGas};

#[derive(luamin::LuaminExecutor)]
struct State;

fn wrong_argument(_: &'static str, _: usize) -> &'static str {
    "incorrect argument"
}

#[luamin::gen_luamin_func(private_lifetime_provider_struct, error_type = &'static str, incorrect_arg_type_error_func = wrong_argument)]
impl State {
    fn increment(&self, value: i32) -> Result<i32, &'static str> {
        Ok(value.wrapping_add(1))
    }
}

fn measure(name: &str, iterations: u32, mut action: impl FnMut()) {
    let started = Instant::now();
    for _ in 0..iterations {
        action();
    }
    println!(
        "{name}: {:?}/iteration ({iterations} iterations)",
        started.elapsed() / iterations
    );
}

fn main() {
    let lib = FuncLibArray::new_const(StateLifetimeProvider::new_func_def_statics());
    let mut compiler = LuaminCompiler::new(&lib, 1);
    let source = "local sum = 0; for i = 1, 100 do sum += i end; return sum";
    measure("compile", 5_000, || {
        black_box(compiler.compile(&[], black_box(source)).unwrap());
    });
    let (script, _) = compiler.compile(&[], source).unwrap();
    measure("execute", 10_000, || {
        let values = State
            .execute_luamin_script(&mut LuaminGas::new(100_000), black_box(&script), &[])
            .unwrap();
        black_box(values.collect::<Vec<_>>());
    });
    let (script, _) = compiler.compile(&[], "return increment(41)").unwrap();
    measure("host call", 100_000, || {
        let values = State
            .execute_luamin_script(&mut LuaminGas::new(100), black_box(&script), &[])
            .unwrap();
        black_box(values.collect::<Vec<_>>());
    });
}
