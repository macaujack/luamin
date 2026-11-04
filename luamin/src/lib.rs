#![doc = include_str!(concat!(env!("OUT_DIR"), "/README.md"))]

extern crate self as luamin;

// The AST/codegen modules intentionally remain available to advanced consumers.
// Common embedding APIs are re-exported below to avoid requiring those details.
/// Compilation of parsed syntax trees into executable instructions.
pub mod codegen;
/// Syntax tree nodes, operators, and virtual machine instructions.
pub mod component;
/// Conversions between Rust values and script values.
pub mod convert;
/// Host function definitions, libraries, and gas-aware dispatch.
pub mod func_lib;
mod limits;
/// Source parsing and structured syntax diagnostics.
pub mod parser;
pub use limits::{MAX_INSTRUCTIONS, MAX_SOURCE_BYTES, MAX_SYNTAX_DEPTH, MAX_SYNTAX_NODES};
/// Script values, execution, gas accounting, and runtime errors.
pub mod runtime;

pub use convert::{IntoLuamin, TryFromLuamin};
pub use func_lib::{
    DuplicateFunctionError, HostFuncDef, HostFuncDefStatic, HostFuncLibArray, HostFuncLibBuf,
};
pub use luamin_macro::{LuaminExecutor, luamin_bindgen};
pub use smallvec;
pub use smol_str;

use smol_str::SmolStr;
use xxhash_rust::{const_xxh3, xxh3};

pub use crate::{
    codegen::{LuaminCodegen, LuaminCodegenError, LuaminScript, LuaminScriptMut},
    func_lib::{GasError, HostFuncLib, HostStateProvider},
    parser::{ParseError, ParseErrorKind, luamin_parse},
    runtime::{
        LuaminGas, LuaminRuntimeError, LuaminUserValue, LuaminValue, luamin_execute,
        luamin_execute_mut,
    },
};

/// Maximum runtime stack size (255 values), including parameters and temporaries.
pub const MAX_RUNTIME_STACK_SIZE: usize = 255;

/// Reusable compiler that parses source and generates scripts bound to a host library.
///
/// `StateProvider` supplies the host state type and its lifetimes, `HostError` is the host error
/// type, and `Extra` is application metadata attached to each host function.
/// Compiled scripts borrow the library for `'f`; they do not borrow the compiler.
#[derive(Debug, Clone)]
pub struct LuaminCompiler<
    'f,
    StateProvider: HostStateProvider,
    HostError: 'static,
    Extra: 'static = (),
> {
    codegen: LuaminCodegen<'f, StateProvider, HostError, Extra>,
}

/// Failure while parsing source or generating executable instructions.
#[derive(Debug, Clone, thiserror::Error)]
pub enum LuaminCompileError {
    /// Source syntax or parsing resource-limit error.
    #[error("Parse error: {0}")]
    ParseError(#[from] ParseError),
    /// Semantic validation or instruction-generation error.
    #[error("Codegen error: {0}")]
    CodegenError(#[from] LuaminCodegenError),
}

impl<'f, StateProvider: HostStateProvider, HostError: 'static, Extra: 'static>
    LuaminCompiler<'f, StateProvider, HostError, Extra>
{
    /// Creates a compiler using `func_lib` for host function resolution.
    ///
    /// `default_list_base_index` is the script index corresponding to the first
    /// list element, unless overridden by the source list-base directive.
    pub fn new(
        func_lib: &'f HostFuncLib<StateProvider, HostError, Extra>,
        default_list_base_index: i8,
    ) -> Self {
        Self {
            codegen: LuaminCodegen::new(func_lib, default_list_base_index),
        }
    }

    /// Parses `input` and compiles a script using shared host state access.
    ///
    /// `intrinsic_param_idents` names leading initial-stack values, followed by
    /// parameters declared in the source. Parameters are read-only bindings.
    /// At execution time, missing values are filled with nil and excess values
    /// are discarded. Host methods requiring mutable state are rejected;
    /// shared state may still contain interior-mutable data.
    ///
    /// Returns the script and sorted, unique indices of referenced host functions
    /// in the bound library. The compiler can be reused for subsequent scripts.
    ///
    /// # Errors
    ///
    /// Returns a parsing error for invalid or excessively large source, or a
    /// code-generation error for invalid semantics or exceeded compilation limits.
    #[allow(clippy::type_complexity)]
    pub fn compile(
        &mut self,
        intrinsic_param_idents: &[SmolStr],
        input: &str,
    ) -> Result<
        (
            LuaminScript<'f, StateProvider, HostError, Extra>,
            Vec<usize>,
        ),
        LuaminCompileError,
    > {
        let (directive, stmts) = luamin_parse(input)?;
        self.codegen
            .codegen(intrinsic_param_idents, &directive, &stmts)
            .map_err(|e| e.into())
    }

    /// Parses `input` and compiles a script that may call mutable host methods.
    ///
    /// Parameters and returned function indices follow [`Self::compile`].
    /// Execution requires an exclusive reference to the host state.
    ///
    /// # Errors
    ///
    /// Returns parsing, semantic validation, or compilation-limit errors,
    /// as with [`Self::compile`].
    #[allow(clippy::type_complexity)]
    pub fn compile_mut(
        &mut self,
        intrinsic_param_idents: &[SmolStr],
        input: &str,
    ) -> Result<
        (
            LuaminScriptMut<'f, StateProvider, HostError, Extra>,
            Vec<usize>,
        ),
        LuaminCompileError,
    > {
        let (directive, stmts) = luamin_parse(input)?;
        self.codegen
            .codegen_mut(intrinsic_param_idents, &directive, &stmts)
            .map_err(|e| e.into())
    }
}

/// Convenience execution methods for a host state type.
///
/// The `LuaminExecutor` derive implements this trait using its default methods.
/// `StateProvider` must resolve to `Self` for the selected state lifetimes; `HostError` and `Extra`
/// are the library error and per-function metadata types.
pub trait LuaminExecutor
where
    Self: Sized,
{
    /// Executes a script with shared access to this state.
    ///
    /// See [`luamin_execute`] for initial-stack ordering, return values, gas
    /// accounting, and errors. Shared access does not prevent interior mutation.
    fn execute_script<'f, 'a, 'r0, 'r1, 'r2, 'r3, StateProvider, HostError, Extra>(
        &'a self,
        gas: &'a mut LuaminGas,
        luamin_script: &'a LuaminScript<'f, StateProvider, HostError, Extra>,
        initial_stack: &'a [LuaminValue],
    ) -> Result<
        impl ExactSizeIterator<Item = LuaminValue>
        + std::fmt::Debug
        + use<Self, StateProvider, HostError, Extra>,
        GasError<LuaminRuntimeError<HostError>>,
    >
    where
        StateProvider: HostStateProvider<StateWithLifetimes<'r0, 'r1, 'r2, 'r3> = Self>,
        HostError: 'static,
        Extra: 'static,
    {
        luamin_execute(self, gas, luamin_script, initial_stack)
    }

    /// Executes a script with exclusive access to this state.
    ///
    /// See [`luamin_execute_mut`] for execution behavior and errors. Changes
    /// made before an error are retained.
    fn execute_script_mut<'f, 'a, 'r0, 'r1, 'r2, 'r3, StateProvider, HostError, Extra>(
        &mut self,
        gas: &'a mut LuaminGas,
        luamin_script_mut: &LuaminScriptMut<'f, StateProvider, HostError, Extra>,
        initial_stack: &[LuaminValue],
    ) -> Result<
        impl ExactSizeIterator<Item = LuaminValue>
        + std::fmt::Debug
        + use<Self, StateProvider, HostError, Extra>,
        GasError<LuaminRuntimeError<HostError>>,
    >
    where
        StateProvider: HostStateProvider<StateWithLifetimes<'r0, 'r1, 'r2, 'r3> = Self>,
        HostError: 'static,
        Extra: 'static,
    {
        luamin_execute_mut(self, gas, luamin_script_mut, initial_stack)
    }
}

/// Hashes UTF-8 bytes with XXH3-64 and truncates the result to a signed 32-bit value.
///
/// This is the representation used for script hash-string literals. Hashes may
/// collide and are not suitable for cryptographic use.
pub fn hash_string(s: &str) -> i32 {
    xxh3::xxh3_64(s.as_bytes()) as i32
}

/// Constant-evaluation counterpart of [`hash_string`], with identical output.
pub const fn hash_string_const(s: &str) -> i32 {
    const_xxh3::xxh3_64(s.as_bytes()) as i32
}

// MARK: Test
