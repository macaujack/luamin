mod gen_func_pointer;
mod gen_luamin_func;
mod luamin_executor_derive;
mod luamin_user_value_derive;

#[proc_macro]
pub fn gen_func_pointer(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    gen_func_pointer::gen_func_pointer(input)
}

#[proc_macro_derive(LuaminExecutor)]
pub fn luamin_executor_derive(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    luamin_executor_derive::luamin_executor_derive(input)
}

#[proc_macro_attribute]
pub fn gen_luamin_func(
    attr: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    gen_luamin_func::gen_luamin_func(attr, item)
}

#[proc_macro_derive(LuaminUserValue)]
pub fn luamin_user_value_derive(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    luamin_user_value_derive::luamin_user_value_derive(input)
}
