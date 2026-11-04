# luamin-macro

**Experimental:** This crate evolves alongside Luamin. Macro options and generated
APIs may change incompatibly between releases.

Procedural macros for binding Rust application state and methods to Luamin
scripts. Requires Rust 1.99 or newer.

Most applications should depend on `luamin`, which re-exports the binding and
derive macros alongside the runtime. The macros generate code that uses that
runtime; this crate alone does not provide a script compiler or interpreter.

## Macros

| Macro | Purpose |
| --- | --- |
| `luamin_bindgen` | Generates script-callable wrappers, host function definitions, and a state lifetime provider from an inherent impl. |
| `LuaminExecutor` | Derives the runtime execution convenience trait for a host state type. |
| `LuaminUserValue` | Derives the marker trait for a custom script value. |
| `gen_func_pointer` | Generates the runtime's low-level function-pointer dispatch types. |

## Binding methods

Import `luamin_bindgen` from `luamin` and annotate an inherent impl. The following
is an attribute/signature fragment; `Host`, `HostError`, and the conversion error
function must be defined by the application:

```text
#[luamin_bindgen(
    error_type = HostError,
    incorrect_arg_type_error_func = wrong_argument
)]
impl Host {
    fn score(&self) -> Result<i32, HostError> { /* ... */ }
    fn add_score(&mut self, amount: i32) -> Result<(), HostError> { /* ... */ }
}
```

`wrong_argument` receives the function name and a one-based argument index and
returns `HostError`. Each method is registered under its Rust name, with raw
identifier prefixes removed. Methods must use `&self` or `&mut self` and return
`Result<T, E>`. Method generics, async and unsafe methods, variadics, and explicit
ABIs are not supported.

For `Host`, the macro generates `HostLifetimeProvider`, including:

- `new_func_defs()`, returning an array of `HostFuncDef` values.
- `new_func_def_statics()`, returning an array of `HostFuncDefStatic` values.
- Private conversion wrappers for the registered methods.

The arrays follow method declaration order. Pass them to `HostFuncLibBuf::new`
or `HostFuncLibArray::new_const` to create the sorted library used by the compiler.
The original methods remain callable from Rust.

See the [luamin crate guide](https://docs.rs/luamin) for a complete executable
example that builds a library, compiles a script, and executes it.

## Options and metering

- `private_lifetime_provider_struct` makes the generated provider private.
- `const_func_lib` makes both function-definition constructors const.
- `func_def_extra_struct = Path` selects a metadata struct; method attributes
  such as `#[extra(priority = 1)]` supply its fields. Unspecified fields use
  `Default::default()`, or `Path::CONST_DEFAULT` with `const_func_lib`.
- `func_def_extra_struct_fields_required` omits that default update, requiring
  all metadata fields to be provided.
- `crate = path` changes the runtime path from its default, `::luamin`.

Ordinary wrappers charge one gas unit before the host method runs, after argument
conversion. `#[gas(N)]` selects a nonnegative fixed charge. For manual metering,
accept `&mut LuaminGas` immediately after the receiver and return
`Result<T, GasError<E>>`. Charge work with `gas.try_consume(cost)?`; manual
metering cannot be combined with `#[gas(...)]`. Runtime dispatch charges are
additional to the host wrapper's charge.

The binding macro supports up to four distinct non-static state lifetimes,
fifteen script inputs, and three script outputs per method. Unit results produce
no outputs; tuple results produce one output per element. Conditional `cfg`
attributes on impls and methods are supported; `cfg_attr` is rejected there.

## Derives and custom runtime paths

`LuaminExecutor` preserves the type's generics and where clause and implements
the execution trait using its default methods. It does not register methods.

`LuaminUserValue` supplies only the marker implementation. The type must also
support `Debug`, `Clone`, `PartialEq`, `CloneGasCost`, `PartialEqGasCost`, and
`'static`. Copy types receive blanket gas-cost implementations from the runtime;
other types provide their own. For type parameters, the derive adds
`Debug + Clone + PartialEq + 'static` bounds in addition to existing predicates.

Both derives accept `#[luamin(crate = path)]` on their input type. Use this with
the binding macro's `crate = path` option when the runtime dependency is renamed
or re-exported.

See the [macro API reference](https://docs.rs/luamin-macro) for full signature,
conversion, and generated-item details.

## License

Licensed under the MIT License. Copyright (c) 2026 macaujack.
The full license text is in the repository's root `LICENSE` file and is also
included as `LICENSE` in this crate's package.
