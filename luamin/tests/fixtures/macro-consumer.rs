use engine::LuaminGas;

#[cfg(feature = "too_many_lifetimes")]
struct TooManyLifetimes<'a, 'b, 'c, 'd, 'e>(&'a i32, &'b i32, &'c i32, &'d i32, &'e i32);

#[cfg(feature = "too_many_lifetimes")]
#[engine::luamin_bindgen(crate = engine, private_lifetime_provider_struct, error_type = Error, incorrect_arg_type_error_func = wrong_argument)]
impl<'a, 'b, 'c, 'd, 'e> TooManyLifetimes<'a, 'b, 'c, 'd, 'e> {
    fn bad(&self) -> Result<(), Error> {
        Ok(())
    }
}

#[cfg(feature = "invalid_derive_option")]
#[derive(engine::LuaminExecutor)]
#[luamin(unknown = engine)]
struct InvalidDerive;
use engine::func_lib::GasError;

#[derive(Debug, Default, engine::LuaminExecutor)]
#[luamin(crate = engine)]
struct Host {
    score: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, engine::runtime::LuaminUserValue)]
#[luamin(crate = engine)]
struct CustomValue(i32);

#[derive(Debug, thiserror::Error)]
#[error("wrong argument")]
struct Error;
fn wrong_argument(_: &'static str, _: usize) -> Error {
    Error
}

#[engine::luamin_bindgen(crate = engine, private_lifetime_provider_struct, error_type = Error, incorrect_arg_type_error_func = wrong_argument)]
impl Host {
    #[cfg(any())]
    fn disabled(&self, _: TypeThatDoesNotExist) -> Result<(), Error> {
        unreachable!()
    }

    fn add(&mut self, amount: i32) -> Result<i32, Error> {
        self.score += amount;
        Ok(self.score)
    }

    fn manually_metered(&self, gas: &mut LuaminGas) -> Result<i32, GasError<Error>> {
        gas.try_consume(2)?;
        Ok(self.score)
    }
}

#[cfg(any())]
#[engine::luamin_bindgen(crate = engine, private_lifetime_provider_struct, error_type = Error, incorrect_arg_type_error_func = wrong_argument)]
impl DoesNotExist {
    fn missing(&self) -> Result<(), Error> {
        Ok(())
    }
}

#[engine::luamin_bindgen(crate = engine, private_lifetime_provider_struct, error_type = Error, incorrect_arg_type_error_func = wrong_argument)]
#[cfg(any())]
impl AlsoDoesNotExist {
    fn missing(&self) -> Result<(), Error> {
        Ok(())
    }
}

#[derive(engine::LuaminExecutor)]
#[luamin(crate = engine)]
struct Borrowed<'a, T, const N: usize> {
    values: &'a [T; N],
}

#[engine::luamin_bindgen(crate = engine, private_lifetime_provider_struct, error_type = Error, incorrect_arg_type_error_func = wrong_argument)]
impl<'a, T: 'static, const N: usize> Borrowed<'a, T, N> {
    fn len(&self) -> Result<i32, Error> {
        Ok(self.values.len() as i32)
    }
}

#[cfg(feature = "missing_option")]
#[engine::luamin_bindgen(crate = engine, private_lifetime_provider_struct, error_type = Error)]
impl Host {
    fn bad(&self) -> Result<(), Error> {
        Ok(())
    }
}

#[cfg(feature = "unsafe_method")]
#[engine::luamin_bindgen(crate = engine, private_lifetime_provider_struct, error_type = Error, incorrect_arg_type_error_func = wrong_argument)]
impl Host {
    unsafe fn bad(&self) -> Result<(), Error> {
        Ok(())
    }
}

#[cfg(feature = "too_many_inputs")]
#[engine::luamin_bindgen(crate = engine, private_lifetime_provider_struct, error_type = Error, incorrect_arg_type_error_func = wrong_argument)]
impl Host {
    fn bad(
        &self,
        a: i32,
        b: i32,
        c: i32,
        d: i32,
        e: i32,
        f: i32,
        g: i32,
        h: i32,
        i: i32,
        j: i32,
        k: i32,
        l: i32,
        m: i32,
        n: i32,
        o: i32,
        p: i32,
    ) -> Result<(), Error> {
        Ok(())
    }
}

#[cfg(feature = "too_many_outputs")]
#[engine::luamin_bindgen(crate = engine, private_lifetime_provider_struct, error_type = Error, incorrect_arg_type_error_func = wrong_argument)]
impl Host {
    fn bad(&self) -> Result<(i32, i32, i32, i32), Error> {
        Ok((1, 2, 3, 4))
    }
}

#[cfg(feature = "invalid_gas")]
#[engine::luamin_bindgen(crate = engine, private_lifetime_provider_struct, error_type = Error, incorrect_arg_type_error_func = wrong_argument)]
impl Host {
    #[gas(-1)]
    fn bad(&self) -> Result<(), Error> {
        Ok(())
    }
}

#[cfg(feature = "conditional_attribute")]
#[engine::luamin_bindgen(crate = engine, private_lifetime_provider_struct, error_type = Error, incorrect_arg_type_error_func = wrong_argument)]
impl Host {
    #[cfg_attr(all(), inline)]
    fn bad(&self) -> Result<(), Error> {
        Ok(())
    }
}

#[cfg(feature = "value_receiver")]
#[engine::luamin_bindgen(crate = engine, private_lifetime_provider_struct, error_type = Error, incorrect_arg_type_error_func = wrong_argument)]
impl Host {
    fn bad(self) -> Result<(), Error> {
        Ok(())
    }
}

#[cfg(feature = "manual_gas")]
#[engine::luamin_bindgen(crate = engine, private_lifetime_provider_struct, error_type = Error, incorrect_arg_type_error_func = wrong_argument)]
impl Host {
    #[gas(2)]
    fn bad(&self, _: &mut LuaminGas) -> Result<(), GasError<Error>> {
        Ok(())
    }
}

#[cfg(feature = "method_generics")]
#[engine::luamin_bindgen(crate = engine, private_lifetime_provider_struct, error_type = Error, incorrect_arg_type_error_func = wrong_argument)]
impl Host {
    fn bad<T>(&self) -> Result<(), Error> {
        Ok(())
    }
}

#[cfg(feature = "unsupported_output")]
#[engine::luamin_bindgen(crate = engine, private_lifetime_provider_struct, error_type = Error, incorrect_arg_type_error_func = wrong_argument)]
impl Host {
    fn bad(&self) -> Result<impl Copy, Error> {
        Ok(1)
    }
}

#[cfg(feature = "wrong_error")]
#[engine::luamin_bindgen(crate = engine, private_lifetime_provider_struct, error_type = Error, incorrect_arg_type_error_func = wrong_argument)]
impl Host {
    fn bad(&self) -> Result<(), ()> {
        Ok(())
    }
}

#[test]
fn renamed_dependency_macros_execute() {
    use engine::LuaminExecutor;
    use engine::LuaminValue;
    let lib = engine::func_lib::HostFuncLibBuf::new(HostLifetimeProvider::new_func_defs()).unwrap();
    assert_eq!(HostLifetimeProvider::new_func_def_statics().len(), 2);
    let mut compiler = engine::LuaminCompiler::new(&lib, 1);
    let (script, _) = compiler
        .compile_mut(&[], "return add(4), manually_metered()")
        .unwrap();
    let mut host = Host::default();
    let values: Vec<_> = host
        .execute_script_mut(&mut LuaminGas::new(100), &script, &[])
        .unwrap()
        .collect();
    assert_eq!(values, vec![LuaminValue::Number(4), LuaminValue::Number(4)]);
    assert_eq!(BorrowedLifetimeProvider::<i32, 2>::new_func_defs().len(), 1);
    let _ = CustomValue(1);
}
