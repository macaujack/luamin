use quote::quote;
use syn::{DeriveInput, parse_macro_input};

pub fn luamin_executor_derive(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let type_name = &input.ident;

    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    quote! {
        impl #impl_generics LuaminExecutor for #type_name #ty_generics
        #where_clause
        {}
    }
    .into()
}
