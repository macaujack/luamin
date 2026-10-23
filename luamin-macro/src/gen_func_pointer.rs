use quote::{format_ident, quote};
use syn::{Ident, LitInt, Path, Token, parse::Parse, parse_macro_input};

struct MacroInput {
    enum_name: Ident,
    parameter_type_path: Path,
    gas_type_path: Path,
    gas_error_type_path: Path,
    func_pointer_property_type_path: Path,
    func_pointer_call_error_type_path: Path,
    func_pointer_call_stack_op_trait_path: Path,
    lifetime_provider_trait_name: Ident,
    lifetime_provider_lifetime_count: usize,
    input_count_limit: usize,
    output_count_limit: usize,
}

impl Parse for MacroInput {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let enum_name: Ident = input.parse()?;
        let _: Token![,] = input.parse()?;
        let parameter_type_path: Path = input.parse()?;
        let _: Token![,] = input.parse()?;
        let gas_type_path: Path = input.parse()?;
        let _: Token![,] = input.parse()?;
        let gas_error_type_path: Path = input.parse()?;
        let _: Token![,] = input.parse()?;
        let func_pointer_property_type_path: Path = input.parse()?;
        let _: Token![,] = input.parse()?;
        let func_pointer_call_error_type_path: Path = input.parse()?;
        let _: Token![,] = input.parse()?;
        let func_pointer_call_stack_op_trait_path: Path = input.parse()?;
        let _: Token![,] = input.parse()?;
        let lifetime_provider_trait_name: Ident = input.parse()?;
        let _: Token![,] = input.parse()?;
        let lifetime_provider_lifetime_count: LitInt = input.parse()?;
        let _: Token![,] = input.parse()?;
        let input_count_limit: LitInt = input.parse()?;
        let _: Token![,] = input.parse()?;
        let output_count_limit: LitInt = input.parse()?;
        Ok(MacroInput {
            enum_name,
            parameter_type_path,
            gas_type_path,
            gas_error_type_path,
            func_pointer_property_type_path,
            func_pointer_call_error_type_path,
            func_pointer_call_stack_op_trait_path,
            lifetime_provider_trait_name,
            lifetime_provider_lifetime_count: lifetime_provider_lifetime_count.base10_parse()?,
            input_count_limit: input_count_limit.base10_parse()?,
            output_count_limit: output_count_limit.base10_parse()?,
        })
    }
}

pub fn gen_func_pointer(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let MacroInput {
        enum_name,
        parameter_type_path,
        gas_type_path,
        gas_error_type_path,
        func_pointer_property_type_path,
        func_pointer_call_error_type_path,
        func_pointer_call_stack_op_trait_path,
        lifetime_provider_trait_name,
        lifetime_provider_lifetime_count,
        input_count_limit,
        output_count_limit,
    } = parse_macro_input!(input as MacroInput);

    if !(1..=9).contains(&lifetime_provider_lifetime_count)
        || !(1..=99).contains(&input_count_limit)
        || !(1..=9).contains(&output_count_limit)
    {
        return syn::Error::new_spanned(
            enum_name,
            "Expected 1-9 state lifetimes, 1-99 input variants, and 1-9 output variants",
        )
        .into_compile_error()
        .into();
    }

    let lifetime_generic_names: Vec<_> = (0..lifetime_provider_lifetime_count)
        .map(|i| syn::Lifetime::new(&format!("'r{i}"), proc_macro2::Span::call_site()))
        .collect();

    let repetition_count_limit = std::cmp::max(input_count_limit, output_count_limit);
    let repetitions = vec![parameter_type_path.clone(); repetition_count_limit - 1];

    let mut enum_variants = Vec::with_capacity(input_count_limit * output_count_limit * 2);
    let mut type_aliases = Vec::with_capacity(input_count_limit * output_count_limit * 2);
    let mut impl_clone_arms = Vec::with_capacity(input_count_limit * output_count_limit * 2);
    let mut impl_debug_arms = Vec::with_capacity(input_count_limit * output_count_limit * 2);
    let mut impl_property_arms = Vec::with_capacity(input_count_limit * output_count_limit * 2);
    let mut impl_call_ro_arms = Vec::with_capacity(input_count_limit * output_count_limit);
    let mut impl_call_mut_arms = Vec::with_capacity(input_count_limit * output_count_limit);

    for i in 0..input_count_limit {
        let input = &repetitions[0..i];
        for j in 0..output_count_limit {
            let output = &repetitions[0..j];
            for is_mutable in [false, true] {
                let variant_name = if is_mutable {
                    format_ident!("F{}{}Mut", i, j)
                } else {
                    format_ident!("F{}{}", i, j)
                };

                let fn_pointer_type = if is_mutable {
                    quote! {
                        for<'a, #(#lifetime_generic_names),*> fn(
                            &'a mut <SLP as #lifetime_provider_trait_name>::StateWithLifetimes<#(#lifetime_generic_names),*>,
                            gas: &'a mut #gas_type_path,
                            #(#input),*
                        ) -> ::core::result::Result<(#(#output),*), #gas_error_type_path<LE>>
                    }
                } else {
                    quote! {
                        for<'a, #(#lifetime_generic_names),*> fn(
                            &'a <SLP as #lifetime_provider_trait_name>::StateWithLifetimes<#(#lifetime_generic_names),*>,
                            gas: &'a mut #gas_type_path,
                            #(#input),*
                        ) -> ::core::result::Result<(#(#output),*), #gas_error_type_path<LE>>
                    }
                };

                enum_variants.push(quote! {
                    #variant_name(#variant_name<SLP, LE>)
                });
                type_aliases.push(quote! {
                    pub type #variant_name<SLP, LE> = #fn_pointer_type;
                });
                impl_clone_arms.push(quote! {
                    #enum_name::#variant_name(fn_pointer) => #enum_name::#variant_name(*fn_pointer)
                });
                let variant_name_str = variant_name.to_string();
                impl_debug_arms.push(quote! {
                    #enum_name::#variant_name(fn_pointer) => f.debug_tuple(#variant_name_str).field(fn_pointer).finish()
                });
                impl_property_arms.push(quote! {
                    #enum_name::#variant_name(_) => #func_pointer_property_type_path {
                        input_count: #i,
                        output_count: #j,
                        is_mutable: #is_mutable,
                    }
                });

                let input_idents: Vec<_> = (0..i)
                    .map(|input_num| format_ident!("input{}", input_num))
                    .collect();
                let output_idents: Vec<_> = (0..j)
                    .map(|output_num| format_ident!("output{}", output_num))
                    .collect();
                let declare_inputs_stmts: Vec<_> = (0..i)
                    .rev()
                    .map(|input_num| {
                        let input_ident = &input_idents[input_num];
                        quote! {
                            let #input_ident = stack
                                .pop_element()
                                .ok_or(#gas_error_type_path::Other(#func_pointer_call_error_type_path::InsufficientLuaminValues))?;
                        }
                    })
                    .collect();
                let push_outputs_stmts: Vec<_> = (0..j)
                    .map(|output_num| {
                        let output_ident = &output_idents[output_num];
                        quote! {
                            stack.push_element(#output_ident);
                        }
                    })
                    .collect();

                let call_arm = quote! {
                    #enum_name::#variant_name(fn_pointer) => {
                        #(#declare_inputs_stmts)*
                        let (#(#output_idents),*) = fn_pointer(state, gas, #(#input_idents),*)?;
                        #(#push_outputs_stmts)*
                    }
                };

                if !is_mutable {
                    impl_call_ro_arms.push(call_arm.clone());
                }
                impl_call_mut_arms.push(call_arm);
            }
        }
    }

    quote! {
        pub trait #lifetime_provider_trait_name {
            type StateWithLifetimes<#(#lifetime_generic_names),*>;
        }

        #(#type_aliases)*

        pub enum #enum_name<SLP: #lifetime_provider_trait_name, LE: 'static> {
            #(#enum_variants),*
        }

        impl<SLP: #lifetime_provider_trait_name, LE: 'static> Clone for #enum_name<SLP, LE> {
            fn clone(&self) -> Self {
                match self {
                    #(#impl_clone_arms),*
                }
            }
        }

        impl<SLP: #lifetime_provider_trait_name, LE: 'static> Copy for #enum_name<SLP, LE> {}

        impl<SLP: #lifetime_provider_trait_name, LE: 'static> ::std::fmt::Debug for #enum_name<SLP, LE> {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                match self {
                    #(#impl_debug_arms),*
                }
            }
        }

        impl<SLP: #lifetime_provider_trait_name, LE: 'static> #enum_name<SLP, LE> {
            pub fn property(&self) -> #func_pointer_property_type_path {
                match self {
                    #(#impl_property_arms),*
                }
            }

            pub fn call<'a, #(#lifetime_generic_names),* , STK>(
                &'a self,
                state: &'a SLP::StateWithLifetimes<#(#lifetime_generic_names),*>,
                gas: &'a mut #gas_type_path,
                stack: &'a mut STK,
            ) -> ::core::result::Result<(), #gas_error_type_path<#func_pointer_call_error_type_path<LE>>>
            where
                STK: #func_pointer_call_stack_op_trait_path<LE>,
            {
                match self {
                    #(#impl_call_ro_arms),*
                    _ => return ::core::result::Result::Err(#gas_error_type_path::Other(#func_pointer_call_error_type_path::IncorrectStateMutability)),
                }
                ::core::result::Result::Ok(())
            }

            pub fn call_mut<'a, #(#lifetime_generic_names),* , STK>(
                &'a self,
                state: &'a mut SLP::StateWithLifetimes<#(#lifetime_generic_names),*>,
                gas: &'a mut #gas_type_path,
                stack: &'a mut STK,
            ) -> ::core::result::Result<(), #gas_error_type_path<#func_pointer_call_error_type_path<LE>>>
            where
                STK: #func_pointer_call_stack_op_trait_path<LE>,
            {
                match self {
                    #(#impl_call_mut_arms),*
                }
                ::core::result::Result::Ok(())
            }
        }
    }
    .into()
}
