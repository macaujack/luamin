use std::collections::{HashMap, hash_map::Entry};

use quote::{format_ident, quote};
use syn::{
    Expr, FnArg, GenericArgument, GenericParam, ImplItem, ImplItemFn, LitInt, Meta, Path,
    PathArguments, ReturnType, Type, WherePredicate, ext::IdentExt, parse::Parser,
};

pub fn gen_luamin_func(
    attr: proc_macro2::TokenStream,
    item: proc_macro2::TokenStream,
) -> syn::Result<proc_macro2::TokenStream> {
    let mut runtime: Path = syn::parse_quote!(::luamin);
    // MARK: Attr
    let mut private_lifetime_provider_struct = false;
    let mut incorrect_arg_type_error_func: Option<Path> = None;
    let mut error_type: Option<Type> = None;
    let mut func_def_extra_struct: Option<Path> = None;
    let mut func_def_extra_struct_fields_required: bool = false;
    let mut const_func_lib = false;

    let attr_parser = syn::meta::parser(|meta| {
        if meta.path.is_ident("crate") {
            runtime = meta.value()?.parse()?;
            Ok(())
        } else if meta.path.is_ident("private_lifetime_provider_struct") {
            private_lifetime_provider_struct = true;
            Ok(())
        } else if meta.path.is_ident("incorrect_arg_type_error_func") {
            incorrect_arg_type_error_func = Some(meta.value()?.parse()?);
            Ok(())
        } else if meta.path.is_ident("error_type") {
            error_type = Some(meta.value()?.parse()?);
            Ok(())
        } else if meta.path.is_ident("func_def_extra_struct") {
            func_def_extra_struct = Some(meta.value()?.parse()?);
            Ok(())
        } else if meta.path.is_ident("func_def_extra_struct_fields_required") {
            func_def_extra_struct_fields_required = true;
            Ok(())
        } else if meta.path.is_ident("const_func_lib") {
            const_func_lib = true;
            Ok(())
        } else {
            Err(meta.error("unrecognized property"))
        }
    });

    attr_parser.parse2(attr)?;

    let incorrect_arg_type_error_func = match incorrect_arg_type_error_func {
        Some(error_func) => error_func,
        None => {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "incorrect_arg_type_error_func property is required",
            ));
        }
    };
    let error_type = match error_type {
        Some(error_type) => error_type,
        None => {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "error_type property is required",
            ));
        }
    };

    // MARK: Item

    let mut item: syn::ItemImpl = syn::parse2(item)?;
    if item.trait_.is_some() {
        return Err(syn::Error::new_spanned(&item, "Expected an inherent impl"));
    }
    let impl_cfg = crate::conditional_attrs(&item.attrs)?;
    let item_generics = &item.generics;
    let impl_self_ty = match item.self_ty.as_ref() {
        Type::Path(type_path) => type_path,
        _ => {
            return Err(syn::Error::new_spanned(
                &item.self_ty,
                "Self type must be a path type",
            ));
        }
    };

    let lp_struct_ident = format_ident!(
        "{}LifetimeProvider",
        &impl_self_ty.path.segments.last().unwrap().ident
    );
    let lp_struct_name = lp_struct_ident.to_string();
    let lp_generics = {
        let mut lp_generics = item_generics.clone();
        lp_generics.params.clear();
        lp_generics
            .params
            .extend(item_generics.type_params().cloned().map(GenericParam::Type));
        lp_generics.params.extend(
            item_generics
                .const_params()
                .cloned()
                .map(GenericParam::Const),
        );
        if let Some(where_clause) = &mut lp_generics.where_clause {
            where_clause.predicates.clear();
            let impl_generics_where_predicates =
                &item_generics.where_clause.as_ref().unwrap().predicates;
            where_clause.predicates.extend(
                impl_generics_where_predicates
                    .iter()
                    .filter(|p| !matches!(p, WherePredicate::Lifetime(_)))
                    .cloned(),
            );

            if where_clause.predicates.is_empty() {
                lp_generics.where_clause = None;
            }
        }

        lp_generics
    };
    let (lp_impl_generics, lp_type_generics, lp_where_clause) = lp_generics.split_for_impl();
    let lp_struct_pub_token = if private_lifetime_provider_struct {
        quote! {}
    } else {
        quote! { pub }
    };
    let lp_struct_def = if lp_generics.params.is_empty() {
        quote! {
            /// Maps execution lifetimes to the host state for generated bindings.
            #[derive(Debug, Clone, Copy)]
            #lp_struct_pub_token struct #lp_struct_ident;
        }
    } else {
        let phantom_defs = lp_generics
            .type_params()
            .enumerate()
            .map(|(i, tp)| {
                let field_ident = format_ident!("_phantom{}", i);
                let ty = &tp.ident;
                quote! {
                    #field_ident: ::std::marker::PhantomData<#ty>,
                }
            })
            .collect::<Vec<_>>();
        quote! {
            /// Maps execution lifetimes to the generic host state for generated bindings.
            #lp_struct_pub_token struct #lp_struct_ident #lp_impl_generics
            #lp_where_clause
            {
                #(#phantom_defs)*
            }

            impl #lp_impl_generics ::std::fmt::Debug for #lp_struct_ident #lp_type_generics
            #lp_where_clause
            {
                fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                    f.debug_struct(#lp_struct_name)
                        .finish()
                }
            }

            impl #lp_impl_generics ::core::clone::Clone for #lp_struct_ident #lp_type_generics
            #lp_where_clause
            {
                fn clone(&self) -> Self {
                    *self
                }
            }
            impl #lp_impl_generics ::core::marker::Copy for #lp_struct_ident #lp_type_generics
            #lp_where_clause
            {}
        }
    };

    let impl_self_ty_for_lp = {
        let mut impl_self_ty_for_lp = impl_self_ty.clone();
        let mut lifetime_ident_mapping: HashMap<syn::Ident, syn::Ident> = HashMap::new();
        let mut unique_lifetime_count: usize = 0;
        for segment in &mut impl_self_ty_for_lp.path.segments {
            let path_args = match &mut segment.arguments {
                PathArguments::AngleBracketed(path_args) => path_args,
                _ => continue,
            };

            for generic_arg in &mut path_args.args {
                let lifetime_arg = match generic_arg {
                    GenericArgument::Lifetime(lifetime_arg) => lifetime_arg,
                    _ => continue,
                };
                if lifetime_arg.ident == "static" {
                    continue;
                }
                match lifetime_ident_mapping.entry(lifetime_arg.ident.clone()) {
                    Entry::Occupied(occ) => {
                        lifetime_arg.ident = occ.get().clone();
                    }
                    Entry::Vacant(vac) => {
                        let new_ident = format_ident!("r{}", unique_lifetime_count);
                        unique_lifetime_count += 1;
                        if unique_lifetime_count > 4 {
                            return Err(syn::Error::new_spanned(
                                lifetime_arg,
                                "At most 4 lifetime parameters are supported",
                            ));
                        }
                        vac.insert(new_ident.clone());
                        lifetime_arg.ident = new_ident;
                    }
                }
            }
        }
        impl_self_ty_for_lp
    };

    let mut func_def_array: Vec<proc_macro2::TokenStream> = Vec::with_capacity(item.items.len());
    let mut func_def_static_array: Vec<proc_macro2::TokenStream> =
        Vec::with_capacity(item.items.len());
    let mut luamin_func_items: Vec<proc_macro2::TokenStream> = Vec::with_capacity(item.items.len());

    let mut count_entries = Vec::new();
    for item in &mut item.items {
        let impl_fn = match item {
            ImplItem::Fn(impl_fn) => impl_fn,
            _ => return Err(syn::Error::new_spanned(item, "Only methods are supported")),
        };

        let cfg = crate::conditional_attrs(&impl_fn.attrs)?;
        count_entries.push(quote! { #(#cfg)* () });
        let handle_result = handle_impl_fn(
            &runtime,
            impl_fn,
            const_func_lib,
            &incorrect_arg_type_error_func,
            &error_type,
            func_def_extra_struct.as_ref(),
            func_def_extra_struct_fields_required,
        )?;
        func_def_array.push(handle_result.func_def_array_element);
        func_def_static_array.push(handle_result.func_def_static_array_element);
        luamin_func_items.push(handle_result.luamin_func_item);
    }

    let func_def_extra_type = match func_def_extra_struct {
        Some(func_def_extra_struct) => quote! { #func_def_extra_struct },
        None => quote! { () },
    };

    let func_def_count = quote! { <[()]>::len(&[#(#count_entries),*]) };
    let conditional_const_token = if const_func_lib {
        quote! { const }
    } else {
        quote! {}
    };

    let generated = quote! {
        #item

        #lp_struct_def

        impl #lp_impl_generics #runtime::func_lib::FuncPointerStateLifetimeProvider for #lp_struct_ident #lp_type_generics
        #lp_where_clause
        {
            type StateWithLifetimes<'r0, 'r1, 'r2, 'r3> = #impl_self_ty_for_lp;
        }

        impl #lp_impl_generics #lp_struct_ident #lp_type_generics
        #lp_where_clause
        {
            /// Returns registered function definitions in method declaration order.
            ///
            /// Construct a function library to sort these definitions and reject duplicate names.
            pub #conditional_const_token fn new_func_defs() -> [#runtime::func_lib::FuncDef<Self, #error_type, #func_def_extra_type>; #func_def_count] {
                [
                    #(#func_def_array),*
                ]
            }
            /// Returns definitions with static names in method declaration order.
            ///
            /// These can be passed to the fixed-size library constructor.
            pub #conditional_const_token fn new_func_def_statics() -> [#runtime::func_lib::FuncDefStatic<Self, #error_type, #func_def_extra_type>; #func_def_count] {
                [
                    #(#func_def_static_array),*
                ]
            }

            #(#luamin_func_items)*
        }
    };
    crate::with_conditional_attrs(generated, &impl_cfg)
}

struct HandleImplFnResult {
    func_def_array_element: proc_macro2::TokenStream,
    func_def_static_array_element: proc_macro2::TokenStream,
    luamin_func_item: proc_macro2::TokenStream,
}

fn handle_impl_fn(
    runtime: &Path,
    method: &mut ImplItemFn,
    const_func_lib: bool,
    incorrect_arg_type_error_func: &Path,
    error_type: &Type,
    extra_type: Option<&Path>,
    extra_fields_required: bool,
) -> syn::Result<HandleImplFnResult> {
    let cfg = crate::conditional_attrs(&method.attrs)?;
    let mut extra_tokens = Vec::new();
    let mut gas_cost = None;
    let mut kept_attrs = Vec::new();
    for attr in std::mem::take(&mut method.attrs) {
        if attr.path().is_ident("gas") {
            if gas_cost.is_some() {
                return Err(syn::Error::new_spanned(attr, "Duplicate gas attribute"));
            }
            let value: LitInt = attr.parse_args()?;
            let cost = value.base10_parse::<i32>()?;
            if cost < 0 {
                return Err(syn::Error::new_spanned(
                    value,
                    "Gas cost must be nonnegative",
                ));
            }
            gas_cost = Some(cost);
        } else if attr.path().is_ident("extra") {
            let Meta::List(list) = &attr.meta else {
                return Err(syn::Error::new_spanned(attr, "Expected #[extra(...)]"));
            };
            if extra_type.is_none() {
                return Err(syn::Error::new_spanned(
                    attr,
                    "extra requires func_def_extra_struct",
                ));
            }
            extra_tokens.push(list.tokens.clone());
        } else {
            kept_attrs.push(attr);
        }
    }
    method.attrs = kept_attrs;
    let sig = &method.sig;
    let error = |message: &str| syn::Error::new_spanned(sig, message);
    if !matches!(sig.safety, syn::Safety::Default) {
        return Err(error("Unsafe methods are not supported"));
    }
    if sig.asyncness.is_some() {
        return Err(error("Async methods are not supported"));
    }
    if sig.abi.is_some() {
        return Err(error("ABI declarations are not supported"));
    }
    if !sig.generics.params.is_empty() || sig.generics.where_clause.is_some() {
        return Err(error("Method generics are not supported"));
    }
    if sig.variadic.is_some() {
        return Err(error("Variadic methods are not supported"));
    }
    let Some(FnArg::Receiver(receiver)) = sig.inputs.first() else {
        return Err(error("A &self or &mut self receiver is required"));
    };
    if !receiver.attrs.is_empty() {
        return Err(error("Receiver attributes are not supported"));
    }
    let syn::ReceiverKind::Reference(_, _, mutability) = &receiver.kind else {
        return Err(error("Receiver must be shorthand &self or &mut self"));
    };
    let manual_gas = match sig.inputs.iter().nth(1) {
        Some(FnArg::Typed(arg)) => match arg.ty.as_ref() {
            Type::Reference(reference) => match reference.elem.as_ref() {
                Type::Path(path)
                    if path
                        .path
                        .segments
                        .last()
                        .is_some_and(|s| s.ident == "LuaminGas") =>
                {
                    if reference.mutability.is_none() {
                        return Err(syn::Error::new_spanned(
                            &arg.ty,
                            "LuaminGas must be a mutable reference",
                        ));
                    }
                    true
                }
                _ => false,
            },
            _ => false,
        },
        _ => false,
    };
    let skip = 1 + usize::from(manual_gas);
    let input_count = sig.inputs.len() - skip;
    if input_count > 15 {
        return Err(error("At most 15 script inputs are supported"));
    }
    let ReturnType::Type(_, output) = &sig.output else {
        return Err(error("Method must return Result<T, E>"));
    };
    let Type::Path(output_path) = output.as_ref() else {
        return Err(error("Method must return Result<T, E>"));
    };
    if output_path.qself.is_some() {
        return Err(error("Qualified return types are not supported"));
    }
    let result = output_path
        .path
        .segments
        .last()
        .ok_or_else(|| error("Method must return Result<T, E>"))?;
    if result.ident != "Result" {
        return Err(error("Method must return Result<T, E>"));
    }
    let PathArguments::AngleBracketed(args) = &result.arguments else {
        return Err(error("Result requires success and error types"));
    };
    if args.args.len() != 2 {
        return Err(error("Result requires exactly two type arguments"));
    }
    let GenericArgument::Type(success_type) = &args.args[0] else {
        return Err(error("Expected a success type"));
    };
    let GenericArgument::Type(return_error) = &args.args[1] else {
        return Err(error("Expected an error type"));
    };
    let expected_error: Type = if manual_gas {
        syn::parse_quote!(#runtime::func_lib::GasError<#error_type>)
    } else {
        error_type.clone()
    };
    // GasError can be imported or qualified; compare its arguments structurally.
    let error_matches = if manual_gas {
        match return_error {
            Type::Path(p) => p.path.segments.last().is_some_and(|s| {
                s.ident == "GasError" && matches!(&s.arguments, PathArguments::AngleBracketed(a) if a.args.len() == 1 && a.args[0] == GenericArgument::Type(error_type.clone()))
            }),
            _ => false,
        }
    } else {
        return_error == &expected_error
    };
    if !error_matches {
        return Err(syn::Error::new_spanned(
            return_error,
            if manual_gas {
                "Manual gas methods must return GasError<error_type>"
            } else {
                "Return error must match error_type; GasError requires a &mut LuaminGas parameter"
            },
        ));
    }
    if manual_gas && gas_cost.is_some() {
        return Err(error("Manual gas cannot be combined with #[gas(...)]"));
    }

    let outputs = output_types(success_type);
    if outputs.len() > 3 {
        return Err(error("At most 3 script outputs are supported"));
    }
    let mut cloned = Vec::new();
    for ty in &outputs {
        cloned.push(output_should_clone(ty)?);
    }
    let output_count = outputs.len();
    let original = &sig.ident;
    let name = original.unraw().to_string();
    let wrapper = format_ident!("luamin_{}", name);
    let variant = format_ident!(
        "F{}{}{}",
        input_count,
        output_count,
        if mutability.is_some() { "Mut" } else { "" }
    );
    let mut declarations = Vec::new();
    let mut calling_args = Vec::new();
    let input_names: Vec<_> = (1..=input_count)
        .map(|i| format_ident!("input{i}"))
        .collect();
    for (index, arg) in sig.inputs.iter().skip(skip).enumerate() {
        let FnArg::Typed(arg) = arg else {
            return Err(error("Unexpected receiver"));
        };
        if !arg.attrs.is_empty() {
            return Err(syn::Error::new_spanned(
                arg,
                "Parameter attributes are not supported",
            ));
        }
        let (declaration, calling_arg) = input_conversion(
            runtime,
            index + 1,
            &arg.ty,
            incorrect_arg_type_error_func,
            &name,
        )?;
        declarations.push(declaration);
        calling_args.push(calling_arg);
    }
    let output_names: Vec<_> = (1..=output_count)
        .map(|i| format_ident!("output{i}"))
        .collect();
    let converted: Vec<_> = output_names.iter().zip(cloned).map(|(name, clone)| {
        if clone { quote! { #runtime::convert::ToLuamin::to_luamin(::core::clone::Clone::clone(&*#name)) } }
        else { quote! { #runtime::convert::ToLuamin::to_luamin(#name) } }
    }).collect();
    let output_value_types = vec![quote! { #runtime::runtime::LuaminValue }; output_count];
    let call = if manual_gas {
        quote! { state.#original(gas, #(#calling_args),*)? }
    } else {
        quote! { state.#original(#(#calling_args),*).map_err(#runtime::func_lib::GasError::Other)? }
    };
    let consume = if manual_gas {
        quote! {}
    } else {
        let cost = gas_cost.unwrap_or(1);
        quote! { gas.try_consume(#cost)?; }
    };
    let mut extra_fields = Vec::new();
    for tokens in extra_tokens {
        syn::meta::parser(|meta| {
            let key = meta
                .path
                .get_ident()
                .ok_or_else(|| meta.error("Expected a field name"))?;
            let value: Expr = meta.value()?.parse()?;
            extra_fields.push(quote! { #key: #value });
            Ok(())
        })
        .parse2(tokens)?;
    }
    let extra = match extra_type {
        Some(ty) => {
            if !extra_fields_required {
                extra_fields.push(if const_func_lib {
                    quote! { ..#ty::CONST_DEFAULT }
                } else {
                    quote! { ..::core::default::Default::default() }
                });
            }
            quote! { #ty { #(#extra_fields),* } }
        }
        None => quote! { () },
    };
    Ok(HandleImplFnResult {
        func_def_array_element: quote! {
            #(#cfg)* #runtime::func_lib::FuncDef {
                func_name: #runtime::smol_str::SmolStr::new_static(#name),
                func_pointer: #runtime::func_lib::FuncPointer::<Self, #error_type>::#variant(Self::#wrapper),
                extra: #extra,
            }
        },
        func_def_static_array_element: quote! {
            #(#cfg)* #runtime::func_lib::FuncDefStatic {
                func_name: #name,
                func_pointer: #runtime::func_lib::FuncPointer::<Self, #error_type>::#variant(Self::#wrapper),
                extra: #extra,
            }
        },
        luamin_func_item: quote! {
            #(#cfg)* fn #wrapper<'r0, 'r1, 'r2, 'r3>(
                state: & #mutability <Self as #runtime::func_lib::FuncPointerStateLifetimeProvider>::StateWithLifetimes<'r0, 'r1, 'r2, 'r3>,
                gas: &mut #runtime::runtime::LuaminGas,
                #(#input_names: #runtime::runtime::LuaminValue),*
            ) -> ::core::result::Result<(#(#output_value_types),*), #runtime::func_lib::GasError<#error_type>> {
                #(#declarations)*
                #consume
                let (#(#output_names),*) = #call;
                ::core::result::Result::Ok((#(#converted),*))
            }
        },
    })
}

fn output_types(ty: &Type) -> Vec<&Type> {
    match ty {
        Type::Paren(p) => output_types(&p.elem),
        Type::Tuple(t) => t.elems.iter().collect(),
        _ => vec![ty],
    }
}

fn output_should_clone(ty: &Type) -> syn::Result<bool> {
    match ty {
        Type::Array(_) | Type::Path(_) => Ok(false),
        Type::Reference(r) => match r.elem.as_ref() {
            Type::Slice(_) => Ok(false),
            Type::Array(_) | Type::Path(_) | Type::TraitObject(_) => Ok(true),
            _ => Err(syn::Error::new_spanned(
                ty,
                "Unsupported referenced output type",
            )),
        },
        Type::Paren(p) => output_should_clone(&p.elem),
        _ => Err(syn::Error::new_spanned(ty, "Unsupported output type")),
    }
}

fn input_conversion(
    runtime: &Path,
    index: usize,
    ty: &Type,
    error_fn: &Path,
    function: &str,
) -> syn::Result<(proc_macro2::TokenStream, proc_macro2::TokenStream)> {
    let ident = format_ident!("input{index}");
    let (conversion_ty, mutable, call) = match ty {
        Type::Array(_) | Type::Path(_) => (quote! { #ty }, None, quote! { #ident }),
        Type::Reference(r) => {
            let mutable = r.mutability;
            match r.elem.as_ref() {
                Type::Slice(s) => {
                    let elem = &s.elem;
                    let call = if mutable.is_some() {
                        quote! { #ident.as_mut() }
                    } else {
                        quote! { #ident.as_ref() }
                    };
                    (
                        quote! { #runtime::smallvec::SmallVec<[#elem; 24]> },
                        mutable,
                        call,
                    )
                }
                inner => (quote! { #inner }, mutable, quote! { & #mutable #ident }),
            }
        }
        Type::Paren(p) => return input_conversion(runtime, index, &p.elem, error_fn, function),
        _ => return Err(syn::Error::new_spanned(ty, "Unsupported input type")),
    };
    Ok((
        quote! {
            let #mutable #ident = <#conversion_ty as #runtime::convert::FromLuamin>::from_luamin(#ident)
                .ok_or_else(|| #runtime::func_lib::GasError::Other(#error_fn(#function, #index)))?;
        },
        call,
    ))
}
