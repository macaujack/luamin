extern crate self as luamin;

// The AST/codegen modules intentionally remain available to advanced consumers.
// Common embedding APIs are re-exported below to avoid requiring those details.
pub mod codegen;
pub mod component;
pub mod convert;
pub mod func_lib;
pub mod lexer;
mod limits;
pub use limits::{MAX_INSTRUCTIONS, MAX_SOURCE_BYTES, MAX_SYNTAX_DEPTH, MAX_SYNTAX_NODES};
pub mod runtime;

pub use convert::{FromLuamin, ToLuamin};
pub use func_lib::{DuplicateFunctionError, FuncDef, FuncDefStatic, FuncLibArray, FuncLibBuf};
pub use luamin_macro::{LuaminExecutor, gen_luamin_func};
pub use smallvec;
pub use smol_str;

use smol_str::SmolStr;
use xxhash_rust::{const_xxh3, xxh3};

pub use crate::{
    codegen::{LuaminCodegen, LuaminCodegenError, LuaminScript, LuaminScriptMut},
    func_lib::{FuncLib, FuncPointerStateLifetimeProvider, GasError},
    lexer::{LexError, LexErrorKind, luamin_lex},
    runtime::{
        LuaminGas, LuaminRuntimeError, LuaminUserValue, LuaminValue, luamin_execute,
        luamin_execute_mut,
    },
};

pub const MAX_RUNTIME_STACK_SIZE: usize = 255;

#[derive(Debug, Clone)]
pub struct LuaminCompiler<'f, SLP: FuncPointerStateLifetimeProvider, LE: 'static, X: 'static = ()> {
    codegen: LuaminCodegen<'f, SLP, LE, X>,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum LuaminCompileError {
    #[error("Lex error: {0}")]
    LexError(#[from] LexError),
    #[error("Codegen error: {0}")]
    CodegenError(#[from] LuaminCodegenError),
}

impl<'f, SLP: FuncPointerStateLifetimeProvider, LE: 'static, X: 'static>
    LuaminCompiler<'f, SLP, LE, X>
{
    pub fn new(func_lib: &'f FuncLib<SLP, LE, X>, default_list_base_index: i8) -> Self {
        Self {
            codegen: LuaminCodegen::new(func_lib, default_list_base_index),
        }
    }

    /// Compiles the source code into a `LuaminScript`, which cannot mutate state.
    ///
    /// Returns the compiled `LuaminScript` and the indices of the used lib functions.
    #[allow(clippy::type_complexity)]
    pub fn compile(
        &mut self,
        intrinsic_param_idents: &[SmolStr],
        input: &str,
    ) -> Result<(LuaminScript<'f, SLP, LE, X>, Vec<usize>), LuaminCompileError> {
        let (directive, stmts) = luamin_lex(input)?;
        self.codegen
            .codegen(intrinsic_param_idents, &directive, &stmts)
            .map_err(|e| e.into())
    }

    /// Compiles the source code into a `LuaminScriptMut`, which CAN mutate state.
    ///
    /// Returns the compiled `LuaminScriptMut` and the indices of the used lib functions.
    #[allow(clippy::type_complexity)]
    pub fn compile_mut(
        &mut self,
        intrinsic_param_idents: &[SmolStr],
        input: &str,
    ) -> Result<(LuaminScriptMut<'f, SLP, LE, X>, Vec<usize>), LuaminCompileError> {
        let (directive, stmts) = luamin_lex(input)?;
        self.codegen
            .codegen_mut(intrinsic_param_idents, &directive, &stmts)
            .map_err(|e| e.into())
    }
}

pub trait LuaminExecutor
where
    Self: Sized,
{
    fn execute_luamin_script<'f, 'a, 'r0, 'r1, 'r2, 'r3, SLP, LE, X>(
        &'a self,
        gas: &'a mut LuaminGas,
        luamin_script: &'a LuaminScript<'f, SLP, LE, X>,
        initial_stack: &'a [LuaminValue],
    ) -> Result<
        impl ExactSizeIterator<Item = LuaminValue> + std::fmt::Debug + use<Self, SLP, LE, X>,
        GasError<LuaminRuntimeError<LE>>,
    >
    where
        SLP: FuncPointerStateLifetimeProvider<StateWithLifetimes<'r0, 'r1, 'r2, 'r3> = Self>,
        LE: 'static,
        X: 'static,
    {
        luamin_execute(self, gas, luamin_script, initial_stack)
    }

    fn execute_luamin_script_mut<'f, 'a, 'r0, 'r1, 'r2, 'r3, SLP, LE, X>(
        &mut self,
        gas: &'a mut LuaminGas,
        luamin_script_mut: &LuaminScriptMut<'f, SLP, LE, X>,
        initial_stack: &[LuaminValue],
    ) -> Result<
        impl ExactSizeIterator<Item = LuaminValue> + std::fmt::Debug + use<Self, SLP, LE, X>,
        GasError<LuaminRuntimeError<LE>>,
    >
    where
        SLP: FuncPointerStateLifetimeProvider<StateWithLifetimes<'r0, 'r1, 'r2, 'r3> = Self>,
        LE: 'static,
        X: 'static,
    {
        luamin_execute_mut(self, gas, luamin_script_mut, initial_stack)
    }
}

pub fn hash_string(s: &str) -> i32 {
    xxh3::xxh3_64(s.as_bytes()) as i32
}

pub const fn hash_string_const(s: &str) -> i32 {
    const_xxh3::xxh3_64(s.as_bytes()) as i32
}

// MARK: Test
