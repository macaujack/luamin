use quote::quote;
use syn::{GenericParam, parse_macro_input};

pub fn luamin_user_value_derive(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(input as syn::DeriveInput);
    let runtime = match crate::runtime_path(&input) {
        Ok(path) => path,
        Err(error) => return error.into_compile_error().into(),
    };
    let type_name = &input.ident;

    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let mut predicates: Vec<_> = where_clause
        .map(|where_clause| {
            where_clause
                .predicates
                .iter()
                .map(|p| quote! {#p})
                .collect()
        })
        .unwrap_or_default();
    for generic_param in &input.generics.params {
        let ident = match generic_param {
            GenericParam::Type(ty) => &ty.ident,
            _ => continue,
        };
        predicates.push(quote! {
            #ident: ::std::fmt::Debug + ::std::clone::Clone + ::std::cmp::PartialEq + 'static
        });
    }

    let where_clause = if predicates.is_empty() {
        quote! {}
    } else {
        quote! {where #(#predicates),*}
    };

    quote! {
        impl #impl_generics #runtime::runtime::LuaminUserValue for #type_name #ty_generics
        #where_clause
        {}
    }
    .into()
}
