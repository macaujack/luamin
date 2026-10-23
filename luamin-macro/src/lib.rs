mod gen_func_pointer;
mod gen_luamin_func;
mod luamin_executor_derive;
mod luamin_user_value_derive;

#[proc_macro]
pub fn gen_func_pointer(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    gen_func_pointer::gen_func_pointer(input)
}

#[proc_macro_derive(LuaminExecutor, attributes(luamin))]
pub fn luamin_executor_derive(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    luamin_executor_derive::luamin_executor_derive(input)
}

#[proc_macro_attribute]
pub fn gen_luamin_func(
    attr: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    gen_luamin_func::gen_luamin_func(attr.into(), item.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

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
