#![doc = include_str!("../README.md")]

mod gen_func_pointer;
mod luamin_bindgen;
mod luamin_executor_derive;
mod luamin_user_value_derive;

/// Generates a host function-pointer enum, pointer aliases, and a state lifetime trait.
///
/// This low-level macro is used by the runtime to build its dispatch interface.
/// Its eleven positional arguments, separated by commas, are:
///
/// 1. Generated enum name.
/// 2. Script value type path.
/// 3. Gas budget type path.
/// 4. Generic gas error type path.
/// 5. Function property type path, with `input_count`, `output_count`, and `is_mutable` fields.
/// 6. Generic call error type path, with `InsufficientLuaminValues` and `IncorrectStateMutability` variants.
/// 7. Stack operation trait path, with `push_element` and `pop_element` methods.
/// 8. Generated state lifetime-provider trait name.
/// 9. Number of state lifetimes (1 through 9).
/// 10. Exclusive input-count limit (1 through 99).
/// 11. Exclusive output-count limit (1 through 9).
///
/// For every input/output count below the limits, generates `Fij` and `FijMut`
/// variants and aliases, where `i` is the input count and `j` the output count.
/// Functions receive state, gas, and script inputs, and return script outputs
/// inside `Result<_, GasError<LE>>`. Zero outputs use `()`; one output is a
/// single value, and multiple outputs use a tuple. Generated enum methods expose
/// signature properties and dispatch calls using shared or exclusive state.
/// The trait's `StateWithLifetimes` associated type maps the supplied lifetimes
/// to the application's state type. Invalid limits produce a compile error.
#[proc_macro]
pub fn gen_func_pointer(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    gen_func_pointer::gen_func_pointer(input)
}

/// Implements the runtime's `LuaminExecutor` trait using its default methods.
///
/// Preserves the input type's generics and where clause. This derive does not
/// register host functions; use [`luamin_bindgen`] on an inherent impl for that.
///
/// The runtime path defaults to `::luamin`. Set
/// `#[luamin(crate = path::to::runtime)]` on the type when the dependency is
/// renamed or re-exported.
#[proc_macro_derive(LuaminExecutor, attributes(luamin))]
pub fn luamin_executor_derive(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    luamin_executor_derive::luamin_executor_derive(input)
}

/// Generates script-callable wrappers and host function definitions for an inherent impl.
///
/// Every method in the annotated impl is registered under its Rust name, without
/// any raw-identifier prefix. The impl must contain only methods, each with a
/// shorthand `&self` or `&mut self` receiver. Unsafe, async, ABI-qualified,
/// variadic, or method-generic signatures are rejected. At most four distinct
/// non-static lifetimes in the state type, fifteen script inputs per method,
/// and three outputs per method are supported.
///
/// # Required options
///
/// - `error_type = E`: host application error type.
/// - `incorrect_arg_type_error_func = path`: function taking the method name
///   as `&str` and a one-based argument index as `usize`, returning `E`.
///
/// # Optional options
///
/// - `crate = path`: runtime crate path (default `::luamin`).
/// - `private_lifetime_provider_struct`: makes the generated provider private;
///   otherwise it is public.
/// - `const_func_lib`: makes both definition-array constructors const.
/// - `func_def_extra_struct = Path`: metadata struct attached to every definition.
/// - `func_def_extra_struct_fields_required`: omits the default struct update;
///   every metadata field must then be supplied through `#[extra(...)]`.
///
/// With metadata enabled, `#[extra(field = expression, ...)]` on a method
/// supplies its fields. Missing fields use `Default::default()`, or the metadata
/// type's `CONST_DEFAULT` associated constant when `const_func_lib` is set.
/// Without metadata, the extra type is `()`.
///
/// # Arguments and results
///
/// Ordinary methods return `Result<T, E>`. Owned arguments are converted with
/// `FromLuamin`. Reference arguments borrow converted local values; slices
/// are converted through a temporary `SmallVec`. Mutating these temporaries
/// does not write them back into the script argument, though explicitly shared
/// list handles still alias their original storage. Conversion failures call
/// `incorrect_arg_type_error_func` with the offending argument's index.
///
/// A unit success value produces no script outputs, a tuple produces one output
/// per tuple element, and another supported type produces one output. Outputs
/// use `ToLuamin`; referenced outputs are cloned, while slices use their slice
/// conversion. Owned array/path types and references to supported types are
/// accepted; unsupported type syntax produces a compile error.
///
/// # Gas accounting
///
/// Ordinary wrappers charge one gas unit after argument conversion and before
/// the host call. `#[gas(N)]` overrides that charge with a nonnegative `i32`
/// integer literal. Runtime dispatch charges additional gas for input/output counts.
///
/// A method whose first parameter after the receiver is `&mut LuaminGas` handles
/// its own gas charges and must return `Result<T, GasError<E>>`. The gas parameter
/// is not a script input. Manual gas and `#[gas(...)]` cannot be combined.
///
/// # Generated items
///
/// For a state named `State`, emits `StateLifetimeProvider`, implementing
/// `FuncPointerStateLifetimeProvider`. It carries the state's type and const
/// generics and maps its lifetime parameters through the associated state type.
/// It provides `new_func_defs()` and `new_func_def_statics()`, returning arrays
/// in method declaration order. Build a function library from these arrays to
/// obtain the sorted indices used by compiled scripts. Conversion wrappers are
/// private associated functions named `luamin_<method>`. The original methods
/// remain available.
///
/// `#[cfg(...)]` on the impl and methods is propagated to generated items.
/// `cfg_attr` on either is rejected; method parameter attributes are also rejected.
#[proc_macro_attribute]
pub fn luamin_bindgen(
    attr: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    luamin_bindgen::luamin_bindgen(attr.into(), item.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Implements the runtime's `LuaminUserValue` marker trait for an application value.
///
/// The type must also implement `Debug`, `Clone`, `PartialEq`, `CloneGasCost`,
/// and `PartialEqGasCost`, and satisfy `'static`. These implementations are
/// not generated. Copy types receive blanket gas-cost implementations from
/// the runtime; other types must implement those traits themselves.
///
/// Preserves existing predicates and adds `Debug + Clone + PartialEq + 'static`
/// bounds to each type parameter. The full type must satisfy the runtime trait's
/// supertraits. The runtime path defaults to `::luamin`; override it with
/// `#[luamin(crate = path::to::runtime)]` on the type.
#[proc_macro_derive(LuaminUserValue, attributes(luamin))]
pub fn luamin_user_value_derive(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    luamin_user_value_derive::luamin_user_value_derive(input)
}

fn runtime_path(input: &syn::DeriveInput) -> syn::Result<syn::Path> {
    let mut runtime = syn::parse_quote!(::luamin);
    for attr in &input.attrs {
        if attr.path().is_ident("luamin") {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("crate") {
                    runtime = meta.value()?.parse()?;
                    Ok(())
                } else {
                    Err(meta.error("Expected crate = path"))
                }
            })?;
        }
    }
    Ok(runtime)
}

fn conditional_attrs(attrs: &[syn::Attribute]) -> syn::Result<Vec<syn::Attribute>> {
    let mut result = Vec::new();
    for attr in attrs {
        if attr.path().is_ident("cfg") {
            result.push(attr.clone());
        }
        if attr.path().is_ident("cfg_attr") {
            // Conditional metadata such as #[gas] cannot be interpreted before
            // rustc expands cfg_attr. Reject it instead of silently misbinding.
            return Err(syn::Error::new_spanned(
                attr,
                "cfg_attr is not supported here; use #[cfg(...)] on the impl or method",
            ));
        }
    }
    Ok(result)
}

fn with_conditional_attrs(
    tokens: proc_macro2::TokenStream,
    attrs: &[syn::Attribute],
) -> syn::Result<proc_macro2::TokenStream> {
    use quote::quote;
    let mut file: syn::File = syn::parse2(tokens)?;
    for item in &mut file.items {
        let item_attrs = match item {
            syn::Item::Impl(i) => &mut i.attrs,
            syn::Item::Struct(s) => &mut s.attrs,
            _ => return Err(syn::Error::new_spanned(item, "Unexpected generated item")),
        };
        item_attrs.extend_from_slice(attrs);
    }
    Ok(quote! { #file })
}
