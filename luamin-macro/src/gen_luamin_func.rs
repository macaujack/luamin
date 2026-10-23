use std::collections::{HashMap, hash_map::Entry};

use quote::{format_ident, quote};
use syn::{
    AttrStyle, Expr, FnArg, GenericArgument, GenericParam, ImplItem, ImplItemFn, LitInt, Meta,
    Path, PathArguments, ReturnType, Type, WherePredicate, ext::IdentExt, parse::Parser,
    parse_macro_input,
};

pub fn gen_luamin_func(
    attr: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    // MARK: Attr
    let mut private_lifetime_provider_struct = false;
    let mut incorrect_arg_type_error_func: Option<Path> = None;
    let mut error_type: Option<Type> = None;
    let mut func_def_extra_struct: Option<Path> = None;
    let mut func_def_extra_struct_fields_required: bool = false;
    let mut const_func_lib = false;

    let attr_parser = syn::meta::parser(|meta| {
        if meta.path.is_ident("private_lifetime_provider_struct") {
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

    parse_macro_input!(attr with attr_parser);

    let incorrect_arg_type_error_func = match incorrect_arg_type_error_func {
        Some(error_func) => error_func,
        None => panic!("error_func property is required"),
    };
    let error_type = match error_type {
        Some(error_type) => error_type,
        None => panic!("error_type property is required"),
    };
    let func_def_extra_struct = func_def_extra_struct;

    // MARK: Item

    let mut item = parse_macro_input!(item as syn::ItemImpl);
    let item_generics = &item.generics;
    let impl_self_ty = match item.self_ty.as_ref() {
        Type::Path(type_path) => type_path,
        _ => panic!("Self type must be a path type"),
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

            impl #lp_impl_generics Clone for #lp_struct_ident #lp_type_generics
            #lp_where_clause
            {
                fn clone(&self) -> Self {
                    *self
                }
            }
            impl #lp_impl_generics Copy for #lp_struct_ident #lp_type_generics
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
                match lifetime_ident_mapping.entry(lifetime_arg.ident.clone()) {
                    Entry::Occupied(occ) => {
                        lifetime_arg.ident = occ.get().clone();
                    }
                    Entry::Vacant(vac) => {
                        let new_ident = format_ident!("r{}", unique_lifetime_count);
                        unique_lifetime_count += 1;
                        assert!(
                            unique_lifetime_count <= 4,
                            "At most 4 lifetime parameters are supported"
                        );
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

    for item in &mut item.items {
        let impl_fn = match item {
            ImplItem::Fn(impl_fn) => impl_fn,
            _ => panic!("Only function is supported"),
        };

        let handle_result = handle_impl_fn(
            impl_fn,
            const_func_lib,
            &incorrect_arg_type_error_func,
            &error_type,
            func_def_extra_struct.as_ref(),
            func_def_extra_struct_fields_required,
        );
        func_def_array.push(handle_result.func_def_array_element);
        func_def_static_array.push(handle_result.func_def_static_array_element);
        luamin_func_items.push(handle_result.luamin_func_item);
    }

    let func_def_extra_type = match func_def_extra_struct {
        Some(func_def_extra_struct) => quote! { #func_def_extra_struct },
        None => quote! { () },
    };

    let func_def_count = func_def_array.len();
    let conditional_const_token = if const_func_lib {
        quote! { const }
    } else {
        quote! {}
    };

    quote! {
        #item

        #lp_struct_def

        impl #lp_impl_generics ::luamin::func_lib::FuncPointerStateLifetimeProvider for #lp_struct_ident #lp_type_generics
        #lp_where_clause
        {
            type StateWithLifetimes<'r0, 'r1, 'r2, 'r3> = #impl_self_ty_for_lp;
        }

        impl #lp_impl_generics #lp_struct_ident #lp_type_generics
        #lp_where_clause
        {
            pub #conditional_const_token fn new_func_defs() -> [::luamin::func_lib::FuncDef<Self, #error_type, #func_def_extra_type>; #func_def_count] {
                [
                    #(#func_def_array),*
                ]
            }
            pub #conditional_const_token fn new_func_def_statics() -> [::luamin::func_lib::FuncDefStatic<Self, #error_type, #func_def_extra_type>; #func_def_count] {
                [
                    #(#func_def_static_array),*
                ]
            }

            #(#luamin_func_items)*
        }
    }
    .into()
}

struct HandleImplFnResult {
    func_def_array_element: proc_macro2::TokenStream,
    func_def_static_array_element: proc_macro2::TokenStream,
    luamin_func_item: proc_macro2::TokenStream,
}

// MARK: Handle

fn handle_impl_fn(
    impl_fn: &mut ImplItemFn,
    const_func_lib: bool,
    incorrect_arg_type_error_func: &Path,
    error_type: &Type,
    func_def_extra_struct: Option<&Path>,
    func_def_extra_struct_fields_required: bool,
) -> HandleImplFnResult {
    let mut inner_token_streams = Vec::new();
    let mut attributed_gas_cost: Option<i32> = None;
    impl_fn.attrs.retain(|attr| {
        match attr.style {
            AttrStyle::Inner(_) => return true,
            AttrStyle::Outer => {}
        }
        if attr.meta.path().is_ident("extra") {
            match &attr.meta {
                Meta::List(meta_list) => {
                    inner_token_streams.push(meta_list.tokens.clone());
                }
                _ => panic!("Extra attribute must be a list"),
            }
            false
        } else if attr.meta.path().is_ident("gas") {
            let inner_ts = match &attr.meta {
                Meta::List(meta_list) => &meta_list.tokens,
                _ => panic!("Gas attribute must be a list"),
            };
            let lit_int: LitInt =
                syn::parse2(inner_ts.clone()).expect("Gas cost must be an integer");
            let gas_cost = lit_int
                .base10_parse()
                .expect("Gas cost must be a valid i32");
            attributed_gas_cost = Some(gas_cost);
            false
        } else {
            true
        }
    });
    let impl_fn: &_ = impl_fn;
    let inner_token_streams = inner_token_streams;
    let attributed_gas_cost = attributed_gas_cost;

    let sig = &impl_fn.sig;
    let safety = &sig.safety;
    let original_ident = &sig.ident;
    let original_ident_unraw_str = original_ident.unraw().to_string();
    let gen_ident = format_ident!("luamin_{}", original_ident_unraw_str);

    assert!(sig.asyncness.is_none(), "Async function is not supported");
    assert!(sig.abi.is_none(), "Abi is not supported");
    assert!(
        sig.generics.params.is_empty() && sig.generics.where_clause.is_none(),
        "Generics are not supported"
    );
    assert!(sig.variadic.is_none(), "Variadic function is not supported");

    // MARK: Input
    let receiver = match &sig.inputs.first() {
        Some(FnArg::Receiver(receiver)) => receiver,
        _ => panic!("Receiver is required in function signature"),
    };
    assert!(
        receiver.attrs.is_empty(),
        "Receiver attributes are not supported"
    );
    let receiver_mutability = match &receiver.kind {
        syn::ReceiverKind::Reference(_, _, mutability) => mutability,
        syn::ReceiverKind::Typed(..) => panic!("Receiver must be in shorthand form"),
        _ => panic!("Receiver must be a reference"),
    };
    let receiver_is_mutable = receiver_mutability.is_some();

    let is_manual_gas = 'is_manual_gas: {
        if sig.inputs.len() < 2 {
            break 'is_manual_gas false;
        }
        let param = &sig.inputs[1];
        let ty = match param {
            FnArg::Receiver(_) => panic!("Unexpected receiver"),
            FnArg::Typed(pat_type) => pat_type.ty.as_ref(),
        };
        let (mutability, pointee_ty) = match ty {
            Type::Reference(r) => (r.mutability, r.elem.as_ref()),
            _ => break 'is_manual_gas false,
        };
        let path = match pointee_ty {
            Type::Path(type_path) => &type_path.path,
            _ => break 'is_manual_gas false,
        };
        let last_segment = match path.segments.last() {
            Some(last_segment) => last_segment,
            None => break 'is_manual_gas false,
        };
        if last_segment.ident != "LuaminGas" {
            break 'is_manual_gas false;
        }
        assert!(
            mutability.is_some(),
            "LuaminGas must be a mutable reference"
        );

        true
    };
    let non_input_param_count = 1 + if is_manual_gas { 1 } else { 0 };
    let other_input_count = sig.inputs.len() - non_input_param_count;

    // MARK: Output
    let output_type = match &sig.output {
        ReturnType::Type(_, output_type) => output_type.as_ref(),
        ReturnType::Default => panic!("Function must have a return type"),
    };
    let output_type_path = match output_type {
        Type::Path(type_path) => type_path,
        _ => panic!("Function must return a Result"),
    };
    assert!(
        output_type_path.qself.is_none(),
        "Return type must not contain qualified self"
    );
    let last_segment = match output_type_path.path.segments.last() {
        Some(last_segment) => last_segment,
        None => panic!("Function must return a Result"),
    };
    assert_eq!(
        last_segment.ident, "Result",
        "Function must return a Result"
    );
    let result_generics = match &last_segment.arguments {
        PathArguments::AngleBracketed(args) => &args.args,
        _ => panic!("Returned Result must have generics"),
    };
    assert_eq!(
        result_generics.len(),
        2,
        "Returned Result must have exactly 2 generics"
    );
    let success_type = match &result_generics[0] {
        GenericArgument::Type(ty) => ty,
        _ => panic!("The first generic of the returned Result must be a type"),
    };
    let is_gas_error = match &result_generics[1] {
        GenericArgument::Type(ty) => {
            if ty == error_type {
                false
            } else {
                let path = match ty {
                    Type::Path(type_path) => &type_path.path,
                    _ => panic!("The Error type of the returned Result must be a path type"),
                };
                let last_segment = path
                    .segments
                    .last()
                    .expect("Path must have at least one segment");
                assert_eq!(
                    last_segment.ident, "GasError",
                    "The Error type of the returned Result must be either specified error type or GasError"
                );
                let generics = match &last_segment.arguments {
                    PathArguments::AngleBracketed(angle) => &angle.args,
                    _ => panic!("GasError must have a generic argument"),
                };
                assert_eq!(
                    generics.len(),
                    1,
                    "GasError must have exactly 1 generic argument"
                );
                let ty = match &generics[0] {
                    GenericArgument::Type(ty) => ty,
                    _ => panic!("The generic argument of GasError must be a type"),
                };
                assert_eq!(
                    ty, error_type,
                    "GasError's generic argument must be the specified error type"
                );
                true
            }
        }
        _ => panic!("The second generic of the returned Result must be a type generic"),
    };

    let OutputInfo {
        should_clone: output_should_clone,
    } = get_output_info(success_type);
    let output_count = output_should_clone.len();

    // MARK: Gen Def

    let func_def_extra_ts = match func_def_extra_struct {
        Some(func_def_extra_struct) => {
            let mut key_values = Vec::new();
            for inner_ts in &inner_token_streams {
                let key_value_parser = syn::meta::parser(|meta| -> Result<(), syn::Error> {
                    let key = meta
                        .path
                        .get_ident()
                        .ok_or(meta.error("Extra attribute key must be an identifier"))?;
                    let value: Expr = meta.value()?.parse()?;
                    key_values.push(quote! {
                        #key: #value
                    });
                    Ok(())
                });
                if let Err(e) = key_value_parser.parse2(inner_ts.clone()) {
                    panic!("Extra attribute must be a list of key-value pairs: {e}");
                }
            }

            if !func_def_extra_struct_fields_required {
                let ts = if const_func_lib {
                    quote! {
                        ..#func_def_extra_struct::CONST_DEFAULT
                    }
                } else {
                    quote! {
                        ..Default::default()
                    }
                };
                key_values.push(ts);
            }
            quote! {
                #func_def_extra_struct {
                    #(#key_values),*
                }
            }
        }
        None => {
            quote! {()}
        }
    };
    let func_pointer_variant = format_ident!(
        "F{}{}{}",
        other_input_count,
        output_count,
        if receiver_is_mutable { "Mut" } else { "" }
    );
    let func_def_array_element = quote! {
        ::luamin::func_lib::FuncDef {
            func_name: ::luamin::smol_str::SmolStr::new_static(#original_ident_unraw_str),
            func_pointer: ::luamin::func_lib::FuncPointer::<Self, #error_type>::#func_pointer_variant(Self::#gen_ident),
            extra: #func_def_extra_ts,
        }
    };
    let func_def_static_array_element = quote! {
        ::luamin::func_lib::FuncDefStatic {
            func_name: #original_ident_unraw_str,
            func_pointer: ::luamin::func_lib::FuncPointer::<Self, #error_type>::#func_pointer_variant(Self::#gen_ident),
            extra: #func_def_extra_ts,
        }
    };

    // MARK: Gen Func

    let input_param_list: Vec<_> = (1..=other_input_count)
        .map(|i| {
            let input_ident = format_ident!("input{}", i);
            quote! {
                #input_ident: ::luamin::runtime::LuaminValue
            }
        })
        .collect();
    let output_success_type = vec![
        quote! {
            ::luamin::runtime::LuaminValue
        };
        output_count
    ];

    let mut input_decl_stmts = Vec::with_capacity(other_input_count);
    let mut calling_arg_exprs = Vec::with_capacity(other_input_count);
    let output_idents: Vec<_> = (1..=output_count)
        .map(|i| format_ident!("output{}", i))
        .collect();
    let converted_output_value_exprs: Vec<_> = output_idents
        .iter()
        .zip(output_should_clone)
        .map(|(ident, should_clone)| {
            if should_clone {
                quote! {
                    #ident.clone().to_luamin()
                }
            } else {
                quote! {
                    #ident.to_luamin()
                }
            }
        })
        .collect();

    for (i, arg) in sig.inputs.iter().skip(non_input_param_count).enumerate() {
        let input_type = match &arg {
            FnArg::Typed(arg) => arg.ty.as_ref(),
            FnArg::Receiver(_) => panic!("Unexpected receiver"),
        };
        let (decl_stmt, calling_expr) = get_input_decl_and_calling_arg(
            i + 1,
            input_type,
            incorrect_arg_type_error_func,
            &original_ident_unraw_str,
        );
        input_decl_stmts.push(decl_stmt);
        calling_arg_exprs.push(calling_expr);
    }

    if is_gas_error && !is_manual_gas {
        panic!(
            "Function '{}' returns GasError but doesn't manually control gas",
            original_ident_unraw_str
        );
    }
    if !is_gas_error && is_manual_gas {
        panic!(
            "Function '{}' manually controls gas but the Result's error type is not GasError",
            original_ident_unraw_str
        );
    }

    let call_and_output_ts = if is_manual_gas {
        assert!(
            attributed_gas_cost.is_none(),
            "Function '{}' manually controls gas. Shouldn't specify gas cost twice",
            original_ident_unraw_str
        );
        quote! {
            let (#(#output_idents),*) = state.#original_ident(gas, #(#calling_arg_exprs),*)?;
            ::core::result::Result::Ok((#(#converted_output_value_exprs),*))
        }
    } else {
        let gas_cost = attributed_gas_cost.unwrap_or(1);

        quote! {
            if gas.consume(#gas_cost) {
                return ::core::result::Result::Err(::luamin::func_lib::GasError::GasExceeded);
            }
            let (#(#output_idents),*) = state.#original_ident(#(#calling_arg_exprs),*).map_err(::luamin::func_lib::GasError::Other)?;
            ::core::result::Result::Ok((#(#converted_output_value_exprs),*))
        }
    };

    let luamin_func_item = quote! {
        #safety fn #gen_ident<'r0, 'r1, 'r2, 'r3>(
            state: & #receiver_mutability <Self as ::luamin::func_lib::FuncPointerStateLifetimeProvider>::StateWithLifetimes<'r0, 'r1, 'r2, 'r3>,
            gas: &mut ::luamin::runtime::LuaminGas,
            #(#input_param_list),*
        ) -> ::core::result::Result<(#(#output_success_type),*), ::luamin::func_lib::GasError<#error_type>> {
            use ::luamin::convert::ToLuamin;

            #(#input_decl_stmts)*

            #call_and_output_ts
        }
    };

    HandleImplFnResult {
        func_def_array_element,
        func_def_static_array_element,
        luamin_func_item,
    }
}

struct OutputInfo {
    should_clone: Vec<bool>,
}

fn get_output_info(output_type: &Type) -> OutputInfo {
    match output_type {
        Type::Array(_) | Type::Reference(_) | Type::Path(_) => OutputInfo {
            should_clone: vec![output_type_should_clone(output_type)],
        },
        Type::Tuple(tuple) => {
            let mut should_clone = Vec::with_capacity(tuple.elems.len());
            for elem in &tuple.elems {
                should_clone.push(output_type_should_clone(elem));
            }
            OutputInfo { should_clone }
        }
        Type::Paren(type_paren) => get_output_info(&type_paren.elem),
        Type::Slice(_) => panic!("Slice type is not supported"),
        Type::TraitObject(_) => panic!("Trait object type is not supported"),
        Type::Infer(_) => panic!("Infer type is not supported"),
        Type::ImplTrait(_) => panic!("Opaque type is not supported"),
        _ => panic!("Unsupported output type"),
    }
}

fn output_type_should_clone(output_type: &Type) -> bool {
    match output_type {
        Type::Array(_) | Type::Path(_) => false,
        Type::Reference(type_ref) => match type_ref.elem.as_ref() {
            Type::Slice(_) => false, // This doesn't require calling Clone because "&[T]" is handled as a special case in `convert` mod.
            Type::Array(_) | Type::Path(_) | Type::TraitObject(_) => true,
            Type::Tuple(_) => panic!("Function return type must not be a reference to a tuple"),
            Type::Infer(_) => {
                panic!("Function return type must not be a reference to an inferred type")
            }
            Type::ImplTrait(_) => {
                panic!("Function return type must not be a reference to an opaque type")
            }
            _ => panic!("Unsupported function return type"),
        },
        Type::Tuple(_) => {
            panic!("Function return type must not be a tuple (aside from the top level)")
        }
        Type::Slice(_) => panic!("Function return type must not be a slice (which is DST)"),
        Type::TraitObject(_) => {
            panic!("Function return type must not be a trait object (which is DST)")
        }
        Type::Infer(_) => panic!("Function return type must not be inferred"),
        Type::ImplTrait(_) => panic!("Function return type must not be an opaque type"),
        Type::Paren(type_paren) => output_type_should_clone(&type_paren.elem),
        _ => panic!("Unsupported function return type"),
    }
}

/// `input_index` is 1-based.
fn get_input_decl_and_calling_arg(
    input_index: usize,
    input_type: &Type,
    incorrect_arg_type_error_func: &Path,
    original_ident_unraw_str: &str,
) -> (proc_macro2::TokenStream, proc_macro2::TokenStream) {
    let input_ident = format_ident!("input{}", input_index);

    match input_type {
        Type::Array(_) | Type::Path(_) => {
            let decl_stmt = quote! {
                let #input_ident = match <#input_type as ::luamin::convert::FromLuamin>::from_luamin(#input_ident) {
                    Some(#input_ident) => #input_ident,
                    None => {
                        return ::core::result::Result::Err(::luamin::func_lib::GasError::Other(#incorrect_arg_type_error_func(#original_ident_unraw_str, #input_index)));
                    }
                };
            };
            let calling_expr = quote! { #input_ident };
            (decl_stmt, calling_expr)
        }
        Type::Reference(type_ref) => {
            let referenced_type = type_ref.elem.as_ref();
            let (is_slice_ref, inner_type) = match referenced_type {
                Type::Slice(type_slice) => {
                    let element_type = type_slice.elem.as_ref();
                    let ts = quote! { ::luamin::smallvec::SmallVec<[#element_type; 24]> };
                    (true, ts)
                }
                _ => (false, quote! { #referenced_type }),
            };
            let mutability = &type_ref.mutability;
            let decl_stmt = quote! {
                let #mutability #input_ident = match <#inner_type as ::luamin::convert::FromLuamin>::from_luamin(#input_ident) {
                    Some(#input_ident) => #input_ident,
                    None => {
                        return ::core::result::Result::Err(::luamin::func_lib::GasError::Other(#incorrect_arg_type_error_func(#original_ident_unraw_str, #input_index)));
                    }
                };
            };
            let calling_expr = if is_slice_ref {
                if mutability.is_some() {
                    quote! { #input_ident.as_mut() }
                } else {
                    quote! { #input_ident.as_ref() }
                }
            } else {
                quote! { & #mutability #input_ident }
            };
            (decl_stmt, calling_expr)
        }
        Type::Paren(type_paren) => get_input_decl_and_calling_arg(
            input_index,
            &type_paren.elem,
            incorrect_arg_type_error_func,
            original_ident_unraw_str,
        ),
        Type::Tuple(_) => panic!("Function parameter must not be a tuple"),
        Type::Infer(_) => panic!("Function parameter must not be inferred"),
        Type::Slice(_) => panic!("Function parameter must not be a slice (which is DST)"),
        _ => panic!(
            "Unsupported function parameter type at index {}",
            input_index
        ),
    }
}
