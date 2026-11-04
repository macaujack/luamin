# Luamin

**Experimental:** Luamin is under active development. Its Rust APIs and script
language semantics may change incompatibly between releases.

Luamin is an embeddable, Lua-like scripting language for Rust applications.
Scripts are first compiled into instructions, then executed on a stack-based
runtime (virtual machine). Compile source once, then run the compiled script
with compatible application-provided host state, input values, and a gas budget.
Execution uses either shared (`&State`) or exclusive (`&mut State`) access to
the host state. Scripts can call registered Rust host methods, and gas accounting
limits execution work.

Shared state access does not imply deep immutability: host state may contain
interior-mutable data, and lists can share mutable storage.

The crate provides the parser, compiler, runtime, host function libraries, and
Rust value conversions. It also re-exports `luamin_bindgen`, the `LuaminExecutor`
derive, and the `LuaminUserValue` derive from `luamin-macro`.

Requires Rust 1.99 or newer. Luamin has its own language subset and semantics;
it is not a general-purpose Lua implementation.

## Main differences from Lua

- **Explicit local variables; no global variables.** Declare each ordinary
  variable with `local` before using it, optionally providing an initializer:
  `local value` or `local value = 42`. Uninitialized locals contain nil.
  Assigning to an undeclared name is a compilation error; it never creates a
  global. Parameters are introduced by `#param` or the compiler's intrinsic
  parameter list, and loop bindings are introduced by their `for` statement.
- **Separate scope for `repeat` bodies.** Variables declared inside a
  `repeat ... until` body are not visible in its `until` condition. Declare a
  variable before the loop if both the body and condition need to access it.
- **Compound assignment operators.** Operators such as `+=` are supported;
  for example, `score += 1` updates an existing variable.
- **`continue` in loops.** Use `continue` to skip the rest of the current loop
  iteration. In a `repeat ... until` loop, it proceeds to the `until` condition.
- **Integer-only numbers.** All numbers are signed 32-bit integers (`i32`);
  floating-point numbers are not supported.
- **No runtime strings.** Hash-string literals such as `h"hello world"` compile
  to integer hashes. They do not create string values.
- **No script-defined functions or closures.** Scripts can call registered Rust
  host functions, but cannot define functions or treat them as first-class values.
- **Lists instead of general-purpose tables.** Lists use integer indices and
  shared mutable storage, compare by identity, and support a configurable base
  index (you can make list index start from 0 instead of 1!). There are no
  Lua-style tables with arbitrary keys or metatables.
- **Explicit host state and gas budgets.** Execution receives shared or exclusive
  access to application state and a gas budget. Registered host methods provide
  application operations; scripts have no built-in Lua standard library.

## Dependency

```toml
[dependencies]
luamin = "0.1.0"
thiserror = "2" # Used to define the host error in the example below.
```

`thiserror` is convenient for this example; host errors can also implement
`Display` and `std::error::Error` manually.

## Bind and execute a script

```rust
use luamin::{
    HostFuncLibBuf, LuaminCompiler, LuaminExecutor, LuaminGas, LuaminValue,
    luamin_bindgen,
};

// Host state passed to the script. The derive adds execution convenience methods.
#[derive(LuaminExecutor)]
struct Counter {
    score: i32,
}

#[derive(Debug, thiserror::Error)]
#[error("incorrect type in {0}, argument {1} (1-based)")]
struct HostError(&'static str, usize);

// Called by generated wrappers when a script argument cannot convert to its Rust type.
fn wrong_argument(function: &'static str, argument: usize) -> HostError {
    HostError(function, argument)
}

// Expose the methods below as script functions and generate CounterLifetimeProvider.
#[luamin_bindgen(
    private_lifetime_provider_struct,
    error_type = HostError,
    incorrect_arg_type_error_func = wrong_argument
)]
impl Counter {
    // &mut self requires exclusive host state access during script execution.
    fn add_score(&mut self, amount: i32) -> Result<i32, HostError> {
        self.score += amount;
        Ok(self.score)
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Build a library from the macro-generated host function definitions.
    let library = HostFuncLibBuf::new(CounterLifetimeProvider::new_func_defs())?;
    // The default index of the first list element is 1.
    let mut compiler = LuaminCompiler::new(&library, 1);
    // Compile to instructions, allowing calls to host methods that take &mut self.
    // The second returned value (ignored here) lists the host function indices used.
    let (script, _) = compiler.compile_mut(
        &[], // No intrinsic parameters; this script declares its parameter with #param.
        r#"
        #param amount
        return add_score(amount)
        "#,
    )?;

    let mut state = Counter { score: 10 };
    // Execution stops with an error if this finite gas budget is exceeded.
    let mut gas = LuaminGas::new(100);
    // Supply amount = 7 on the initial stack and collect the script's return values.
    let values: Vec<_> = state
        .execute_script_mut(&mut gas, &script, &[LuaminValue::Number(7)])?
        .collect();

    // The host method updates the state and returns the new score to the script.
    assert_eq!(state.score, 17);
    assert_eq!(values, vec![LuaminValue::Number(17)]);
    Ok(())
}
```

`luamin_bindgen` generates `CounterLifetimeProvider` and the function-definition
constructors. `HostFuncLibBuf` sorts those definitions and rejects duplicate
names. The compiled script borrows this library and can be executed again with
another compatible state or input values.

`compile_mut` allows host functions taking `&mut self`. Use `compile` and
`execute_script` when only shared host access is needed.

## Values and parameters

- Script values are `i32` numbers, booleans, nil, shared lists, or boxed user
  values. String literals represent integer hashes rather than runtime strings.
- `#param` names values supplied on the initial stack. Intrinsic parameter names
  passed to `compile` or `compile_mut` precede source-declared parameters.
  Missing values become nil; excess values are discarded, subject to the initial
  stack size limit.
- `IntoLuamin` and `TryFromLuamin` convert supported Rust values, options, and
  collections. `IntoLuamin` consumes its receiver; `TryFromLuamin` returns `None`
  when a conversion fails.
- `#index` overrides the compiler's default list base index. `ipairs` iteration
  stops at the first nil element.
- Lists compare by identity and share storage when cloned. Parameter bindings
  are read-only, but local aliases can still mutate shared lists. Reference
  cycles must be broken by the embedding application to reclaim their storage.
- `LuaminUserValue` supports application-defined values with clone, equality,
  and gas-cost implementations. The derive supplies the marker implementation.

## Gas and errors

Create a finite `LuaminGas` budget for each execution. The interpreter charges
for instructions and host dispatch. Ordinary bound methods charge one additional
unit by default; `#[gas(N)]` changes that charge. Methods accepting
`&mut LuaminGas` immediately after the receiver handle their own charges and
return `Result<T, GasError<E>>`. Use `gas.try_consume(cost)?` before performing
the work being charged.

Exhaustion returns `GasError::GasExceeded` and remains sticky. Other execution
failures use `GasError::Other(LuaminRuntimeError<_>)`. Parsing and compilation
failures are reported through `LuaminCompileError`. State changes and gas spent
before a failure are retained.

The compiler and runtime also enforce source, syntax tree, instruction, stack,
and script list-growth limits. Host functions must meter their own work and
control the resources they expose.

## Examples

Each example is a single Rust file containing its own `main`, embedded script,
and assertions of the expected behavior.

| Example | Focus |
| --- | --- |
| [basic](https://docs.rs/crate/luamin/latest/source/examples/basic.rs) | Compile once, pass parameters, return multiple values, and reuse the script. |
| [host_functions](https://docs.rs/crate/luamin/latest/source/examples/host_functions.rs) | Bind shared and mutable Rust methods using `luamin_bindgen`. |
| [lists](https://docs.rs/crate/luamin/latest/source/examples/lists.rs) | Convert collections, iterate and index lists, and observe shared storage. |
| [gas_metering](https://docs.rs/crate/luamin/latest/source/examples/gas_metering.rs) | Fixed host charges, manual charges with `try_consume`, and exhaustion handling. |
| [user_values](https://docs.rs/crate/luamin/latest/source/examples/user_values.rs) | Define custom values with equality and clone/comparison gas costs. |
| [error_handling](https://docs.rs/crate/luamin/latest/source/examples/error_handling.rs) | Distinguish parsing, compilation, runtime, and host failures. |
| [borrowed_state](https://docs.rs/crate/luamin/latest/source/examples/borrowed_state.rs) | Bind host state borrowing application data. |
| [recursive](https://docs.rs/crate/luamin/latest/source/examples/recursive.rs) | Alternate Rust and Luamin calls by re-entering a compiled script with a shared gas budget. |

From the workspace root, run an example by name:

```sh
cargo run -p luamin --example basic
cargo run -p luamin --example host_functions
```

See the [API reference](https://docs.rs/luamin) for individual types and methods.

## License

Luamin and its procedural macros are licensed under the MIT License.
Copyright (c) 2026 macaujack. The full license text is included in `LICENSE`.

## Development

The workspace contains two crates:

- `luamin` provides the parser, compiler, runtime, host function libraries, and
  value conversions. This README is its crate-level guide.
- [luamin-macro](https://docs.rs/luamin-macro) provides the procedural macros for
  binding Rust methods and deriving executor and user-value implementations.
  `luamin` re-exports these macros, so most applications only need a direct
  dependency on `luamin`.

```sh
cargo test --workspace
cargo build -p luamin --examples
cargo doc --workspace --no-deps --open
cargo fmt --all -- --check
```

The repository maintains two READMEs: this file for `luamin` and
`luamin-macro/README.md` for the procedural macro crate. Each supplies its
corresponding crate's documentation landing page.
