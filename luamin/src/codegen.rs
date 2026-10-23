use smallvec::{SmallVec, smallvec};
use smol_str::SmolStr;

use crate::{
    MAX_RUNTIME_STACK_SIZE,
    component::{
        CompoundAssignOperator, Directive, ElseifBranch, IfStmt, Ix, ListForStmt, LiteralExpr,
        NumericForStmt, Operator, PlaceExpr, RepeatStmt, Stmt,
        ValueExpr::{self},
        WhileStmt,
    },
    func_lib::{FuncLib, FuncPointerStateLifetimeProvider},
    hash_string,
};

#[derive(Debug, Clone, thiserror::Error)]
pub enum LuaminCodegenError {
    #[error("Undeclared ident: {0}")]
    UndeclaredIdent(SmolStr),
    #[error("Param list must be the first statement if specified")]
    IncorrectParamListPosition,
    #[error("Stack overflow")]
    StackOverflow,
    #[error("Cannot mutate a readonly variable")]
    MutatingReadonly,
    #[error("Library function not found: {0}")]
    LibFuncNotFound(SmolStr),
    #[error("Return must be the last statement in a block")]
    ReturnMustBeLastStatementInBlock,
    #[error("Break must be inside a loop")]
    BreakMustBeInsideLoop,
    #[error("Break must be the last statement in a block")]
    BreakMustBeLastStatementInBlock,
    #[error("Continue must be inside a loop")]
    ContinueMustBeInsideLoop,
    #[error("Continue must be the last statement in a block")]
    ContinueMustBeLastStatementInBlock,
    #[error("Mutable library function used when codegen to an immutable context")]
    MutableLibFuncInImmutableContext,
}

#[derive(Debug, Clone)]
pub struct LuaminCodegen<'f, SLP: FuncPointerStateLifetimeProvider, LE: 'static, X: 'static = ()> {
    func_lib: &'f FuncLib<SLP, LE, X>,
    default_list_base_index: i8,
    declared_idents: SmallVec<[DeclaredIdent; 10]>,
    list_base_index: i8,
    ixs: Vec<Ix>,
    runtime_stack_size: usize,
    early_return_indices: SmallVec<[usize; 4]>,
    loop_contexts: SmallVec<[LoopContext; 4]>,
    used_lib_func_indices: Vec<usize>,
    is_mutable_state: bool,
}

#[derive(Debug, Clone)]
pub struct LuaminScript<'f, SLP: FuncPointerStateLifetimeProvider, LE: 'static, X: 'static = ()> {
    func_lib: &'f FuncLib<SLP, LE, X>,
    ixs: Box<[Ix]>,
}

impl<'f, SLP: FuncPointerStateLifetimeProvider, LE: 'static, X: 'static>
    LuaminScript<'f, SLP, LE, X>
{
    pub fn func_lib(&self) -> &FuncLib<SLP, LE, X> {
        self.func_lib
    }
    pub fn ixs(&self) -> &[Ix] {
        &self.ixs
    }
}

#[derive(Debug, Clone)]
pub struct LuaminScriptMut<'f, SLP: FuncPointerStateLifetimeProvider, LE: 'static, X: 'static = ()>
{
    func_lib: &'f FuncLib<SLP, LE, X>,
    ixs: Box<[Ix]>,
}

impl<'f, SLP: FuncPointerStateLifetimeProvider, LE: 'static, X: 'static>
    LuaminScriptMut<'f, SLP, LE, X>
{
    pub fn func_lib(&self) -> &FuncLib<SLP, LE, X> {
        self.func_lib
    }
    pub fn ixs(&self) -> &[Ix] {
        &self.ixs
    }
}

const EMPTY_IDENT: SmolStr = SmolStr::new_inline("");

#[derive(Debug, Clone)]
struct DeclaredIdent {
    ident: SmolStr,
    is_readonly: bool,
}

#[derive(Debug, Clone)]
struct LoopContext {
    loop_stack_begin_index: usize,
    loop_level_element_count: usize,
    break_indices: SmallVec<[usize; 4]>,
    continue_indices: SmallVec<[usize; 4]>,
}

impl LoopContext {
    fn new(loop_stack_begin_index: usize, loop_level_element_count: usize) -> Self {
        Self {
            loop_stack_begin_index,
            loop_level_element_count,
            break_indices: SmallVec::new(),
            continue_indices: SmallVec::new(),
        }
    }
}

impl<'f, SLP: FuncPointerStateLifetimeProvider, LE: 'static, X: 'static>
    LuaminCodegen<'f, SLP, LE, X>
{
    pub fn new(func_lib: &'f FuncLib<SLP, LE, X>, default_list_base_index: i8) -> Self {
        Self {
            func_lib,
            default_list_base_index,
            declared_idents: SmallVec::new(),
            list_base_index: default_list_base_index,
            // TODO: Use a better initial capacity.
            ixs: Vec::with_capacity(64),
            runtime_stack_size: 0,
            early_return_indices: SmallVec::new(),
            loop_contexts: SmallVec::new(),
            used_lib_func_indices: Vec::with_capacity(4),
            is_mutable_state: false,
        }
    }

    /// Generates an immutable script. Returns the script and the indices of
    /// the used lib functions.
    #[allow(clippy::type_complexity)]
    pub fn codegen(
        &mut self,
        intrinsic_param_idents: &[SmolStr],
        directive: &Directive,
        stmts: &[Stmt],
    ) -> Result<(LuaminScript<'f, SLP, LE, X>, Vec<usize>), LuaminCodegenError> {
        let (ixs, used_lib_func_indices) =
            self.codegen_aux(intrinsic_param_idents, directive, stmts, false)?;
        Ok((
            LuaminScript {
                func_lib: self.func_lib,
                ixs,
            },
            used_lib_func_indices,
        ))
    }

    /// Generates a mutable script. Returns the script and the indices of
    /// the used lib functions.
    #[allow(clippy::type_complexity)]
    pub fn codegen_mut(
        &mut self,
        intrinsic_param_idents: &[SmolStr],
        directive: &Directive,
        stmts: &[Stmt],
    ) -> Result<(LuaminScriptMut<'f, SLP, LE, X>, Vec<usize>), LuaminCodegenError> {
        let (ixs, used_lib_func_indices) =
            self.codegen_aux(intrinsic_param_idents, directive, stmts, true)?;
        Ok((
            LuaminScriptMut {
                func_lib: self.func_lib,
                ixs,
            },
            used_lib_func_indices,
        ))
    }

    fn codegen_aux(
        &mut self,
        intrinsic_param_idents: &[SmolStr],
        directive: &Directive,
        stmts: &[Stmt],
        is_mutable_state: bool,
    ) -> Result<(Box<[Ix]>, Vec<usize>), LuaminCodegenError> {
        let declared_param_idents = match &directive.param_list {
            Some(idents) => idents.as_slice(),
            None => &[],
        };

        if intrinsic_param_idents.len() + declared_param_idents.len() > MAX_RUNTIME_STACK_SIZE {
            return Err(LuaminCodegenError::StackOverflow);
        }

        self.declared_idents.clear();
        self.list_base_index = directive
            .list_base_index
            .unwrap_or(self.default_list_base_index);
        self.ixs.clear();
        self.runtime_stack_size = 0;
        self.early_return_indices.clear();
        self.loop_contexts.clear();
        self.used_lib_func_indices.clear();
        self.is_mutable_state = is_mutable_state;

        for param_ident in intrinsic_param_idents {
            self.declared_idents.push(DeclaredIdent {
                ident: param_ident.clone(),
                is_readonly: true,
            });
        }
        for param_ident in declared_param_idents {
            self.declared_idents.push(DeclaredIdent {
                ident: param_ident.clone(),
                is_readonly: true,
            });
        }

        let parameter_count = (intrinsic_param_idents.len() + declared_param_idents.len()) as u8;
        self.push_ix(Ix::AdjustStack {
            adjust_count: parameter_count,
        })?;
        let _root_scope_returned = self.gen_scope_statements::<true>(stmts)?;

        let ixs_len = self.ixs.len() as i32;
        for early_return_index in self.early_return_indices.iter().cloned() {
            match &mut self.ixs[early_return_index] {
                Ix::Jump { relative_offset } => {
                    *relative_offset = ixs_len - 1 - early_return_index as i32
                }
                _ => unreachable!("Early return index must be a Jump ix"),
            }
        }
        self.used_lib_func_indices.sort_unstable();
        self.used_lib_func_indices.dedup();

        Ok((
            self.ixs.clone().into_boxed_slice(),
            self.used_lib_func_indices.clone(),
        ))
    }

    fn gen_scope_statements<const IS_ROOT_SCOPE: bool>(
        &mut self,
        stmts: &[Stmt],
    ) -> Result<bool, LuaminCodegenError> {
        let new_scope_stack_begin_index = if IS_ROOT_SCOPE {
            0
        } else {
            self.declared_idents.len()
        };

        debug_assert_eq!(
            self.declared_idents.len(),
            self.runtime_stack_size,
            "Declared idents and runtime stack size mismatch at the beginning of a scope"
        );

        for (stmt_index, stmt) in stmts.iter().enumerate() {
            match stmt {
                Stmt::Decl(s) => {
                    debug_assert!(!s.idents.is_empty(), "Empty decl");

                    let value_count = self.gen_value_exprs_keep_last_multiple(&s.exprs)?;
                    self.gen_adjust_value_count(value_count, s.idents.len())?;

                    self.declared_idents
                        .extend(s.idents.iter().cloned().map(|ident| DeclaredIdent {
                            ident,
                            is_readonly: false,
                        }));
                }
                Stmt::Assign(s) => {
                    debug_assert!(
                        !s.left_exprs.is_empty() && !s.right_exprs.is_empty(),
                        "Empty assign statement"
                    );

                    for left_expr in &s.left_exprs {
                        self.gen_place_expr(left_expr)?;
                    }
                    let value_count = self.gen_value_exprs_keep_last_multiple(&s.right_exprs)?;
                    self.gen_adjust_value_count(value_count, s.left_exprs.len())?;

                    let mut left_ending_index = self.runtime_stack_size - s.left_exprs.len();

                    if s.left_exprs.len() == 1 {
                        let place_expr = &s.left_exprs[0];
                        self.gen_assign_ixs_aux(&mut left_ending_index, place_expr)?;
                    } else {
                        for (i, place_expr) in s.left_exprs.iter().rev().enumerate() {
                            match place_expr.indexing_exprs.len() {
                                0 => {
                                    self.push_ix(Ix::Dup {
                                        stack_index: (self.runtime_stack_size - 1 - i) as u8,
                                    })?;
                                }
                                1.. => {
                                    self.push_ix(Ix::Dup {
                                        stack_index: (left_ending_index - 1) as u8,
                                    })?;
                                    self.push_ix(Ix::Dup {
                                        stack_index: (self.runtime_stack_size - 2 - i) as u8,
                                    })?;
                                }
                            }
                            self.gen_assign_ixs_aux(&mut left_ending_index, place_expr)?;
                        }
                    }

                    debug_assert_eq!(
                        left_ending_index,
                        self.declared_idents.len(),
                        "Left ending index mismatch"
                    );

                    if self.runtime_stack_size > self.declared_idents.len() {
                        let pop_count =
                            (self.runtime_stack_size - self.declared_idents.len()) as u8;
                        self.push_ix(Ix::Pop { pop_count })?;
                    }
                }
                Stmt::CompoundAssign(s) => {
                    use CompoundAssignOperator::*;

                    let stmt_begin_stack_index = self.runtime_stack_size;
                    self.gen_place_expr(&s.left_expr)?;

                    match s.left_expr.indexing_exprs.len() {
                        0 => {
                            let stack_index = self.get_ident_stack_index(&s.left_expr.ident)?;
                            self.push_ix(Ix::Dup {
                                stack_index: stack_index as u8,
                            })?;
                        }
                        1 => {
                            let stack_index = self.get_ident_stack_index(&s.left_expr.ident)?;
                            self.push_ix(Ix::Dup {
                                stack_index: stmt_begin_stack_index as u8,
                            })?;
                            self.push_ix(Ix::GetListElement {
                                stack_index: stack_index as u8,
                                base_index: self.list_base_index,
                            })?;
                        }
                        2.. => {
                            self.push_ix(Ix::Dup {
                                stack_index: (stmt_begin_stack_index + 1) as u8,
                            })?;
                            self.push_ix(Ix::GetListElement {
                                stack_index: stmt_begin_stack_index as u8,
                                base_index: self.list_base_index,
                            })?;
                        }
                    }

                    self.gen_value_expr(&s.right_expr, Some(1))?;

                    let arith_ix = match s.operator {
                        PlusAssign => Ix::Add,
                        MinusAssign => Ix::Sub,
                        MulAssign => Ix::Mul,
                        DivFloorAssign => Ix::DivFloor,
                        ModAssign => Ix::Mod,
                        BitAndAssign => Ix::BitAnd,
                        BitOrAssign => Ix::BitOr,
                        BitXorAssign => Ix::BitXor,
                        LshiftAssign => Ix::Lshift,
                        RshiftAssign => Ix::Rshift,
                    };
                    self.push_ix(arith_ix)?;

                    let mut left_ending_index = self.runtime_stack_size - 1;
                    self.gen_assign_ixs_aux(&mut left_ending_index, &s.left_expr)?;

                    debug_assert_eq!(
                        left_ending_index,
                        self.declared_idents.len(),
                        "Left ending index mismatch in compound assign"
                    );

                    // Pop the temporary left place expr
                    if s.left_expr.indexing_exprs.len() >= 2 {
                        self.push_ix(Ix::Pop { pop_count: 1 })?;
                    }
                }
                Stmt::Call(s) => {
                    let output_count = self.gen_func_call(&s.ident, &s.args)?;
                    if output_count > 0 {
                        self.push_ix(Ix::Pop {
                            pop_count: output_count as u8,
                        })?;
                    }
                }
                Stmt::Return(_) => {
                    if stmt_index < stmts.len() - 1 {
                        return Err(LuaminCodegenError::ReturnMustBeLastStatementInBlock);
                    }
                }
                Stmt::Do(stmts) => {
                    let _returned = self.gen_scope_statements::<false>(stmts)?;
                }
                Stmt::If(IfStmt {
                    condition,
                    stmts,
                    elseif_branches,
                    else_branch,
                }) => {
                    self.gen_value_expr(condition, Some(1))?;
                    let mut jump_to_next_index = self.ixs.len();
                    let mut jump_to_end_indices: SmallVec<[usize; 8]> = smallvec![];
                    self.push_ix(Ix::JumpIfNot {
                        relative_offset: i32::MIN,
                    })?;
                    let returned = self.gen_scope_statements::<false>(stmts)?;

                    if (!elseif_branches.is_empty() || else_branch.is_some()) && !returned {
                        jump_to_end_indices.push(self.ixs.len());
                        self.push_ix(Ix::Jump {
                            relative_offset: i32::MIN,
                        })?;
                    }

                    for (elseif_branch_index, else_if_branch) in elseif_branches.iter().enumerate()
                    {
                        let ElseifBranch { condition, stmts } = else_if_branch;
                        let jump_to = self.ixs.len() as i32;
                        match &mut self.ixs[jump_to_next_index] {
                            Ix::JumpIfNot { relative_offset } => {
                                *relative_offset = jump_to - 1 - jump_to_next_index as i32;
                            }
                            _ => unreachable!(
                                "If/Elseif condition must be followed by a JumpIfNot ix"
                            ),
                        }
                        self.gen_value_expr(condition, Some(1))?;
                        jump_to_next_index = self.ixs.len();
                        self.push_ix(Ix::JumpIfNot {
                            relative_offset: i32::MIN,
                        })?;
                        let returned = self.gen_scope_statements::<false>(stmts)?;

                        if (elseif_branch_index < elseif_branches.len() - 1
                            || else_branch.is_some())
                            && !returned
                        {
                            jump_to_end_indices.push(self.ixs.len());
                            self.push_ix(Ix::Jump {
                                relative_offset: i32::MIN,
                            })?;
                        }
                    }

                    let jump_to = self.ixs.len() as i32;
                    match &mut self.ixs[jump_to_next_index] {
                        Ix::JumpIfNot { relative_offset } => {
                            *relative_offset = jump_to - 1 - jump_to_next_index as i32;
                            jump_to_next_index = usize::MAX;
                        }
                        _ => unreachable!("If/Elseif condition must be followed by a JumpIfNot ix"),
                    }

                    if let Some(stmts) = else_branch {
                        let _returned = self.gen_scope_statements::<false>(stmts)?;
                    }

                    let jump_to = self.ixs.len() as i32;
                    for jump_to_end_index in jump_to_end_indices {
                        match &mut self.ixs[jump_to_end_index] {
                            Ix::Jump { relative_offset } => {
                                *relative_offset = jump_to - 1 - jump_to_end_index as i32;
                            }
                            _ => unreachable!("Elseif/Else branch must be after a Jump ix"),
                        }
                    }
                }
                Stmt::While(WhileStmt { condition, stmts }) => {
                    self.loop_contexts
                        .push(LoopContext::new(self.runtime_stack_size, 0));

                    let initial_jump_index = self.ixs.len();
                    self.push_ix(Ix::Jump {
                        relative_offset: i32::MIN,
                    })?;

                    let while_begin_jump_to = self.ixs.len() as i32;
                    let _returned = self.gen_scope_statements::<false>(stmts)?;

                    // Note here we will generate the condition no matter whether the while body
                    // contains a return statement or not, because this condition will at least be
                    // evaluated once by the initial jump.
                    let initial_jump_to = self.ixs.len() as i32;
                    let continue_jump_to = self.ixs.len() as i32;
                    match &mut self.ixs[initial_jump_index] {
                        Ix::Jump { relative_offset } => {
                            *relative_offset = initial_jump_to - 1 - initial_jump_index as i32;
                        }
                        _ => unreachable!("While statement initial jump must be a Jump ix"),
                    }
                    self.gen_value_expr(condition, Some(1))?;
                    let current_index = self.ixs.len();
                    self.push_ix(Ix::JumpIf {
                        relative_offset: while_begin_jump_to - 1 - current_index as i32,
                    })?;

                    let break_jump_to = self.ixs.len() as i32;

                    self.update_break_continue_jump_ixs(break_jump_to, continue_jump_to);
                }
                Stmt::Repeat(RepeatStmt { stmts, condition }) => {
                    self.loop_contexts
                        .push(LoopContext::new(self.runtime_stack_size, 0));

                    let repeat_begin_jump_to = self.ixs.len() as i32;
                    let _returned = self.gen_scope_statements::<false>(stmts)?;

                    // Note here we will generate the condition no matter whether the repeat body
                    // contains a return statement or not, because this condition will possibly be
                    // evaluated if there's a "continue" to skip the final "return".
                    let continue_jump_to = self.ixs.len() as i32;
                    self.gen_value_expr(condition, Some(1))?;
                    let current_index = self.ixs.len();
                    self.push_ix(Ix::JumpIfNot {
                        relative_offset: repeat_begin_jump_to - 1 - current_index as i32,
                    })?;
                    let break_jump_to = self.ixs.len() as i32;

                    self.update_break_continue_jump_ixs(break_jump_to, continue_jump_to);
                }
                Stmt::NumericFor(NumericForStmt {
                    ident,
                    start,
                    end,
                    step,
                    stmts,
                }) => {
                    debug_assert_ne!(*step, 0, "step must be non-zero");

                    self.loop_contexts
                        .push(LoopContext::new(self.runtime_stack_size, 2));
                    self.gen_value_expr(end, Some(1))?;
                    self.gen_value_expr(start, Some(1))?;
                    self.declared_idents.extend([
                        DeclaredIdent {
                            ident: EMPTY_IDENT,
                            is_readonly: true,
                        },
                        DeclaredIdent {
                            ident: ident.clone(),
                            is_readonly: true,
                        },
                    ]);
                    let end_stack_index = (self.declared_idents.len() - 2) as u8;
                    let cur_stack_index = (self.declared_idents.len() - 1) as u8;

                    let initial_jump_index = self.ixs.len();
                    self.push_ix(Ix::Jump {
                        relative_offset: i32::MIN,
                    })?;

                    let for_begin_jump_to = self.ixs.len() as i32;
                    let _returned = self.gen_scope_statements::<false>(stmts)?;

                    let continue_jump_to = self.ixs.len() as i32;
                    self.push_ix(Ix::PushNumber(*step))?;
                    self.push_ix(Ix::Add)?;

                    let initial_jump_to = self.ixs.len() as i32;
                    match &mut self.ixs[initial_jump_index] {
                        Ix::Jump { relative_offset } => {
                            *relative_offset = initial_jump_to - 1 - initial_jump_index as i32;
                        }
                        _ => unreachable!("For statement initial jump must be a Jump ix"),
                    }
                    self.push_ix(Ix::Dup {
                        stack_index: cur_stack_index,
                    })?;
                    self.push_ix(Ix::Dup {
                        stack_index: end_stack_index,
                    })?;
                    let cmp_ix = if *step > 0 { Ix::Lte } else { Ix::Gte };
                    self.push_ix(cmp_ix)?;
                    let current_index = self.ixs.len();
                    self.push_ix(Ix::JumpIf {
                        relative_offset: for_begin_jump_to - 1 - current_index as i32,
                    })?;

                    // Pop the 2 loop level elements
                    self.push_ix(Ix::Pop { pop_count: 2 })?;
                    self.declared_idents
                        .truncate(self.declared_idents.len() - 2);

                    let break_jump_to = self.ixs.len() as i32;

                    self.update_break_continue_jump_ixs(break_jump_to, continue_jump_to);
                }
                Stmt::ListFor(ListForStmt {
                    index_ident,
                    element_ident,
                    list,
                    stmts,
                }) => {
                    self.loop_contexts
                        .push(LoopContext::new(self.runtime_stack_size, 3));
                    self.gen_value_expr(list, Some(1))?;
                    self.push_ix(Ix::PushNumber(self.list_base_index as i32))?;
                    {
                        let list_stack_index = self.runtime_stack_size - 2;
                        self.push_ix(Ix::GetListElement {
                            stack_index: list_stack_index as u8,
                            base_index: self.list_base_index,
                        })?;
                    }
                    self.push_ix(Ix::PushNumber(self.list_base_index as i32))?;
                    self.declared_idents.extend([
                        DeclaredIdent {
                            ident: EMPTY_IDENT,
                            is_readonly: true,
                        },
                        DeclaredIdent {
                            ident: element_ident.clone(),
                            is_readonly: true,
                        },
                        DeclaredIdent {
                            ident: index_ident.clone(),
                            is_readonly: true,
                        },
                    ]);
                    let list_stack_index = (self.declared_idents.len() - 3) as u8;
                    let list_element_stack_index = (self.declared_idents.len() - 2) as u8;
                    let list_index_stack_index = (self.declared_idents.len() - 1) as u8;

                    let initial_jump_index = self.ixs.len();
                    self.push_ix(Ix::Jump {
                        relative_offset: i32::MIN,
                    })?;

                    let for_begin_jump_to = self.ixs.len() as i32;
                    let _returned = self.gen_scope_statements::<false>(stmts)?;

                    let continue_jump_to = self.ixs.len() as i32;
                    self.push_ix(Ix::PushNumber(1))?;
                    self.push_ix(Ix::Add)?;
                    self.push_ix(Ix::Dup {
                        stack_index: list_index_stack_index,
                    })?;
                    self.push_ix(Ix::GetListElement {
                        stack_index: list_stack_index,
                        base_index: self.list_base_index,
                    })?;
                    self.push_ix(Ix::Set {
                        stack_index: list_element_stack_index,
                    })?;

                    let initial_jump_to = self.ixs.len() as i32;
                    match &mut self.ixs[initial_jump_index] {
                        Ix::Jump { relative_offset } => {
                            *relative_offset = initial_jump_to - 1 - initial_jump_index as i32;
                        }
                        _ => unreachable!("For statement initial jump must be a Jump ix"),
                    }
                    self.push_ix(Ix::Dup {
                        stack_index: list_element_stack_index,
                    })?;
                    self.push_ix(Ix::PushNil { count: 1 })?;
                    self.push_ix(Ix::Eq)?;
                    let current_index = self.ixs.len();
                    self.push_ix(Ix::JumpIfNot {
                        relative_offset: for_begin_jump_to - 1 - current_index as i32,
                    })?;

                    // Pop the 3 loop level elements
                    self.push_ix(Ix::Pop { pop_count: 3 })?;
                    self.declared_idents
                        .truncate(self.declared_idents.len() - 3);

                    let break_jump_to = self.ixs.len() as i32;

                    self.update_break_continue_jump_ixs(break_jump_to, continue_jump_to);
                }
                Stmt::Break => {
                    if self.loop_contexts.is_empty() {
                        return Err(LuaminCodegenError::BreakMustBeInsideLoop);
                    }
                    if stmt_index < stmts.len() - 1 {
                        return Err(LuaminCodegenError::BreakMustBeLastStatementInBlock);
                    }
                }
                Stmt::Continue => {
                    if self.loop_contexts.is_empty() {
                        return Err(LuaminCodegenError::ContinueMustBeInsideLoop);
                    }
                    if stmt_index < stmts.len() - 1 {
                        return Err(LuaminCodegenError::ContinueMustBeLastStatementInBlock);
                    }
                }
                Stmt::Semicolon => {}
            }
        }

        let mut scope_returned = false;
        match stmts.last() {
            Some(Stmt::Return(return_values)) => {
                scope_returned = true;
                let return_value_count = self.gen_value_exprs_keep_last_multiple(return_values)?;
                debug_assert!(
                    return_value_count <= self.runtime_stack_size,
                    "Return value count exceeds runtime stack size"
                );
                if return_value_count < self.runtime_stack_size {
                    let return_remove_stack_ix = if return_value_count == 0 {
                        Ix::Pop {
                            pop_count: self.runtime_stack_size as u8,
                        }
                    } else {
                        Ix::Remove {
                            stack_index: 0,
                            remove_count: (self.runtime_stack_size - return_value_count) as u8,
                        }
                    };
                    self.push_ix(return_remove_stack_ix)?;
                }
                if !IS_ROOT_SCOPE {
                    self.early_return_indices.push(self.ixs.len());
                    self.push_ix(Ix::Jump {
                        relative_offset: i32::MIN,
                    })?;
                }
            }
            Some(Stmt::Break) => {
                self.gen_break_continue_aux::<false>()?;
            }
            Some(Stmt::Continue) => {
                self.gen_break_continue_aux::<true>()?;
            }
            _ => {
                debug_assert!(
                    self.runtime_stack_size >= new_scope_stack_begin_index,
                    "Runtime stack size too small"
                );
                let pop_count = (self.runtime_stack_size - new_scope_stack_begin_index) as u8;
                if pop_count > 0 {
                    self.push_ix(Ix::Pop { pop_count })?;
                }
            }
        }

        self.declared_idents.truncate(new_scope_stack_begin_index);
        self.runtime_stack_size = new_scope_stack_begin_index;

        Ok(scope_returned)
    }

    fn update_break_continue_jump_ixs(&mut self, break_jump_to: i32, continue_jump_to: i32) {
        let loop_context = self.loop_contexts.pop().expect("Loop context must exist");
        for break_index in loop_context.break_indices {
            match &mut self.ixs[break_index] {
                Ix::Jump { relative_offset } => {
                    *relative_offset = break_jump_to - 1 - break_index as i32;
                }
                _ => unreachable!("Break index must be a Jump ix"),
            }
        }
        for continue_index in loop_context.continue_indices {
            match &mut self.ixs[continue_index] {
                Ix::Jump { relative_offset } => {
                    *relative_offset = continue_jump_to - 1 - continue_index as i32;
                }
                _ => unreachable!("Continue index must be a Jump ix"),
            }
        }
    }

    fn gen_break_continue_aux<const IS_CONTINUE: bool>(
        &mut self,
    ) -> Result<(), LuaminCodegenError> {
        let loop_context = self.loop_contexts.last().unwrap();
        let pop_count = (self.runtime_stack_size
            - loop_context.loop_stack_begin_index
            - if IS_CONTINUE {
                loop_context.loop_level_element_count
            } else {
                0
            }) as u8;
        if pop_count > 0 {
            self.push_ix(Ix::Pop { pop_count })?;
        }

        let loop_context = self.loop_contexts.last_mut().unwrap();
        let indices = if IS_CONTINUE {
            &mut loop_context.continue_indices
        } else {
            &mut loop_context.break_indices
        };
        indices.push(self.ixs.len());
        self.push_ix(Ix::Jump {
            relative_offset: i32::MIN,
        })?;
        Ok(())
    }

    fn gen_assign_ixs_aux(
        &mut self,
        left_ending_index: &mut usize,
        place_expr: &PlaceExpr,
    ) -> Result<(), LuaminCodegenError> {
        let stack_index = self.get_ident_stack_index(&place_expr.ident)?;
        if self.declared_idents[stack_index].is_readonly {
            return Err(LuaminCodegenError::MutatingReadonly);
        }
        let stack_index = stack_index as u8;
        match place_expr.indexing_exprs.len() {
            0 => {
                self.push_ix(Ix::Set { stack_index })?;
            }
            1 => {
                *left_ending_index -= 1;
                self.push_ix(Ix::SetListElement {
                    stack_index,
                    base_index: self.list_base_index,
                })?;
            }
            2.. => {
                *left_ending_index -= 2;
                self.push_ix(Ix::SetListElement {
                    stack_index: *left_ending_index as u8,
                    base_index: self.list_base_index,
                })?;
            }
        }
        Ok(())
    }

    fn gen_value_expr(
        &mut self,
        expr: &ValueExpr,
        expected_value_count: Option<usize>,
    ) -> Result<usize, LuaminCodegenError> {
        let value_count = match expr {
            ValueExpr::PrefixOperator(op, expr) => {
                self.gen_value_expr(expr, Some(1))?;
                self.gen_prefix_operator(*op)?;
                1
            }
            ValueExpr::InfixOperator(op, lhs, rhs) => {
                match op {
                    Operator::LitAnd => {
                        self.gen_short_circuit_expr(lhs, rhs, Ix::JumpIfNot { relative_offset: 0 })?
                    }
                    Operator::LitOr => {
                        self.gen_short_circuit_expr(lhs, rhs, Ix::JumpIf { relative_offset: 0 })?
                    }
                    _ => {
                        self.gen_value_expr(lhs, Some(1))?;
                        self.gen_value_expr(rhs, Some(1))?;
                        self.gen_infix_operator(*op)?;
                    }
                }
                1
            }
            ValueExpr::Parens(expr) => {
                self.gen_value_expr(expr, Some(1))?;
                1
            }
            ValueExpr::Literal(lit) => {
                let ix = match lit {
                    LiteralExpr::Number(number) => Ix::PushNumber(*number),
                    LiteralExpr::Bool(bool) => Ix::PushBool(*bool),
                    LiteralExpr::Nil => Ix::PushNil { count: 1 },
                    LiteralExpr::HashString(s) => {
                        let number = hash_string(s);
                        Ix::PushNumber(number)
                    }
                };
                self.push_ix(ix)?;
                1
            }
            ValueExpr::LiteralList(exprs) => {
                let element_count = self.gen_value_exprs_keep_last_multiple(exprs)?;
                self.push_ix(Ix::NewList {
                    size: element_count as u8,
                })?;
                1
            }
            ValueExpr::Identx(identx) => match &identx.call_args {
                None => {
                    let stack_index = self.get_ident_stack_index(&identx.ident)?;
                    self.gen_place_expr_as_value::<false>(stack_index, &identx.indexing_exprs)?;
                    1
                }
                Some(call_args) => {
                    let output_count = self.gen_func_call(&identx.ident, call_args)?;

                    if identx.indexing_exprs.is_empty() {
                        output_count
                    } else {
                        self.gen_adjust_value_count(output_count, 1)?;
                        let stack_index = self.runtime_stack_size - 1;
                        self.gen_place_expr_as_value::<true>(stack_index, &identx.indexing_exprs)?;
                        1
                    }
                }
            },
        };

        match expected_value_count {
            Some(expected_value_count) => {
                self.gen_adjust_value_count(value_count, expected_value_count)?;
                Ok(expected_value_count)
            }
            None => Ok(value_count),
        }
    }

    fn gen_func_call(
        &mut self,
        ident: &SmolStr,
        args: &[ValueExpr],
    ) -> Result<usize, LuaminCodegenError> {
        let (func_index, func_def) = self
            .func_lib
            .get_func_def(ident)
            .ok_or(LuaminCodegenError::LibFuncNotFound(ident.clone()))?;
        self.used_lib_func_indices.push(func_index);
        let func_property = func_def.func_pointer.property();
        if func_property.is_mutable && !self.is_mutable_state {
            return Err(LuaminCodegenError::MutableLibFuncInImmutableContext);
        }

        let value_count = self.gen_value_exprs_keep_last_multiple(args)?;
        self.gen_adjust_value_count(value_count, func_property.input_count)?;
        self.push_ix(Ix::CallLib {
            fn_index: func_index as u32,
        })?;
        Ok(func_property.output_count)
    }

    fn gen_value_exprs_keep_last_multiple(
        &mut self,
        exprs: &[ValueExpr],
    ) -> Result<usize, LuaminCodegenError> {
        let value_count = if exprs.is_empty() {
            0
        } else {
            for expr in &exprs[0..exprs.len() - 1] {
                let _count = self.gen_value_expr(expr, Some(1))?;
            }
            let final_element_count = self.gen_value_expr(&exprs[exprs.len() - 1], None)?;
            exprs.len() - 1 + final_element_count
        };
        Ok(value_count)
    }

    fn gen_adjust_value_count(
        &mut self,
        value_count: usize,
        expected_value_count: usize,
    ) -> Result<(), LuaminCodegenError> {
        if value_count > expected_value_count {
            self.push_ix(Ix::Pop {
                pop_count: (value_count - expected_value_count) as u8,
            })?;
        } else if value_count < expected_value_count {
            self.push_ix(Ix::PushNil {
                count: (expected_value_count - value_count) as u8,
            })?;
        }
        Ok(())
    }

    fn gen_place_expr(&mut self, place_expr: &PlaceExpr) -> Result<(), LuaminCodegenError> {
        let PlaceExpr {
            ident,
            indexing_exprs,
        } = place_expr;

        match indexing_exprs.len() {
            0 => {}
            1 => {
                let _value_count = self.gen_value_expr(&indexing_exprs[0], Some(1))?;
            }
            len @ 2.. => {
                let stack_index = self.get_ident_stack_index(ident)?;
                self.gen_place_expr_as_value::<false>(stack_index, &indexing_exprs[..len - 1])?;
                let _value_count = self.gen_value_expr(&indexing_exprs[len - 1], Some(1))?;
            }
        }

        Ok(())
    }

    fn gen_place_expr_as_value<const SHOULD_REMOVE_LIST: bool>(
        &mut self,
        stack_index: usize,
        indexing_exprs: &[ValueExpr],
    ) -> Result<(), LuaminCodegenError> {
        if indexing_exprs.is_empty() {
            self.push_ix(Ix::Dup {
                stack_index: stack_index as u8,
            })?;
        } else {
            let mut stack_index = stack_index as u8;
            for index_expr in indexing_exprs {
                let _value_count = self.gen_value_expr(index_expr, Some(1))?;
                self.push_ix(Ix::GetListElement {
                    stack_index,
                    base_index: self.list_base_index,
                })?;
                stack_index = (self.runtime_stack_size - 1) as u8;
            }

            let remove_count =
                (indexing_exprs.len() - if SHOULD_REMOVE_LIST { 0 } else { 1 }) as u8;
            if remove_count > 0 {
                self.push_ix(Ix::Remove {
                    stack_index: stack_index - remove_count,
                    remove_count,
                })?;
            }
        }

        Ok(())
    }

    fn get_ident_stack_index(&self, ident: &SmolStr) -> Result<usize, LuaminCodegenError> {
        self.declared_idents
            .iter()
            .rposition(|di| &di.ident == ident)
            .ok_or(LuaminCodegenError::UndeclaredIdent(ident.clone()))
    }

    fn gen_prefix_operator(&mut self, op: Operator) -> Result<(), LuaminCodegenError> {
        let ix = match op {
            Operator::Minus => Ix::Neg,
            Operator::Tilde => Ix::BitNot,
            Operator::LitNot => Ix::LogicalNot,
            Operator::Hashtag => Ix::Length,
            _ => unreachable!("Invalid prefix operator: {:?}", op),
        };
        self.push_ix(ix)?;
        Ok(())
    }

    fn gen_short_circuit_expr(
        &mut self,
        lhs: &ValueExpr,
        rhs: &ValueExpr,
        jump_ix_template: Ix,
    ) -> Result<(), LuaminCodegenError> {
        self.gen_value_expr(lhs, Some(1))?;
        self.push_ix(Ix::Dup {
            stack_index: (self.runtime_stack_size - 1) as u8,
        })?;
        let jump_index = self.ixs.len();
        self.push_ix(jump_ix_template)?;

        self.push_ix(Ix::Pop { pop_count: 1 })?;
        self.gen_value_expr(rhs, Some(1))?;

        let jump_to = self.ixs.len() as i32;
        match &mut self.ixs[jump_index] {
            Ix::JumpIfNot { relative_offset } | Ix::JumpIf { relative_offset } => {
                *relative_offset = jump_to - 1 - jump_index as i32;
            }
            _ => unreachable!("Jump index must be a JumpIfNot/JumpIf ix"),
        }

        Ok(())
    }

    fn gen_infix_operator(&mut self, op: Operator) -> Result<(), LuaminCodegenError> {
        let ix = match op {
            Operator::Plus => Ix::Add,
            Operator::Minus => Ix::Sub,
            Operator::Asterisk => Ix::Mul,
            Operator::DoubleSlash => Ix::DivFloor,
            Operator::Percent => Ix::Mod,
            Operator::Lshift => Ix::Lshift,
            Operator::Rshift => Ix::Rshift,
            Operator::Ampersand => Ix::BitAnd,
            Operator::Bar => Ix::BitOr,
            Operator::Tilde => Ix::BitXor,
            Operator::Lt => Ix::Lt,
            Operator::Lte => Ix::Lte,
            Operator::Gt => Ix::Gt,
            Operator::Gte => Ix::Gte,
            Operator::Neq => Ix::Neq,
            Operator::Eq => Ix::Eq,
            Operator::LitAnd | Operator::LitOr => {
                unreachable!("Should treat short-circuit operators differently: {:?}", op)
            }
            _ => unreachable!("Invalid infix operator: {:?}", op),
        };
        self.push_ix(ix)?;
        Ok(())
    }

    fn push_ix(&mut self, ix: Ix) -> Result<(), LuaminCodegenError> {
        use Ix::*;

        let mut check_stack = |new_element_count: usize| {
            if self.runtime_stack_size + new_element_count > MAX_RUNTIME_STACK_SIZE {
                return Err(LuaminCodegenError::StackOverflow);
            }
            self.runtime_stack_size += new_element_count;
            Ok(())
        };

        match ix {
            PushNumber(..) | PushBool(..) | Dup { .. } => {
                check_stack(1)?;
            }

            AdjustStack { adjust_count } => {
                let adjust_count = adjust_count as usize;
                if adjust_count > MAX_RUNTIME_STACK_SIZE {
                    return Err(LuaminCodegenError::StackOverflow);
                }
                self.runtime_stack_size = adjust_count;
            }

            PushNil { count } => {
                check_stack(count as usize)?;
            }

            GetListElement { .. } | Neg | BitNot | LogicalNot | Length | Jump { .. } => {}

            Set { .. }
            | JumpIf { .. }
            | JumpIfNot { .. }
            | Add
            | Sub
            | Mul
            | DivFloor
            | Mod
            | Lshift
            | Rshift
            | BitAnd
            | BitOr
            | BitXor
            | Eq
            | Neq
            | Lt
            | Lte
            | Gt
            | Gte => {
                debug_assert!(self.runtime_stack_size >= 1, "Runtime stack size too small");
                self.runtime_stack_size -= 1;
            }

            SetListElement { .. } => {
                debug_assert!(self.runtime_stack_size >= 2, "Runtime stack size too small");
                self.runtime_stack_size -= 2;
            }

            NewList { size } => match size {
                0 => check_stack(1)?,
                1.. => {
                    debug_assert!(
                        self.runtime_stack_size >= size as usize - 1,
                        "Runtime stack size too small"
                    );
                    self.runtime_stack_size -= size as usize - 1;
                }
            },

            CallLib { fn_index } => {
                let func_def = self
                    .func_lib
                    .kth_func_def(fn_index as usize)
                    .expect("Function index out of bounds");
                let property = func_def.func_pointer.property();
                debug_assert!(
                    self.runtime_stack_size >= property.input_count,
                    "Runtime stack size too small"
                );
                let new_runtime_stack_size =
                    self.runtime_stack_size - property.input_count + property.output_count;
                if new_runtime_stack_size > MAX_RUNTIME_STACK_SIZE {
                    return Err(LuaminCodegenError::StackOverflow);
                }
                self.runtime_stack_size = new_runtime_stack_size;
            }

            Pop { pop_count: count } => {
                debug_assert!(
                    self.runtime_stack_size >= count as usize,
                    "Runtime stack size too small"
                );
                self.runtime_stack_size -= count as usize;
            }

            Remove {
                stack_index,
                remove_count,
            } => {
                debug_assert!(
                    self.runtime_stack_size >= stack_index as usize + remove_count as usize,
                    "Runtime stack size too small"
                );
                self.runtime_stack_size -= remove_count as usize;
            }
        }

        self.ixs.push(ix);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        func_lib::{FuncDef, FuncLibBuf, FuncPointer, GasError, consume_gas},
        lexer::luamin_lex,
        runtime::{LuaminGas, LuaminValue},
    };

    use super::*;

    #[derive(Debug, Clone)]
    struct State {
        score: i32,
    }

    #[derive(Debug, Clone)]
    struct StateLifetimeProvider;
    impl FuncPointerStateLifetimeProvider for StateLifetimeProvider {
        type StateWithLifetimes<'r0, 'r1, 'r2, 'r3> = State;
    }

    #[derive(Debug, thiserror::Error)]
    enum StateError {}

    impl State {
        fn get_score(&self, gas: &mut LuaminGas) -> Result<LuaminValue, GasError<StateError>> {
            consume_gas!(gas, 1);
            Ok(LuaminValue::Number(self.score))
        }
        fn inc_score(&mut self, gas: &mut LuaminGas) -> Result<(), GasError<StateError>> {
            consume_gas!(gas, 1);
            self.score += 1;
            Ok(())
        }
    }

    fn get_codegen() -> LuaminCodegen<'static, StateLifetimeProvider, StateError> {
        let func_lib = FuncLibBuf::new([
            FuncDef::new(
                "get_score",
                FuncPointer::<StateLifetimeProvider, StateError>::F01(State::get_score),
            ),
            FuncDef::new(
                "inc_score",
                FuncPointer::<StateLifetimeProvider, StateError>::F00Mut(State::inc_score),
            ),
        ])
        .unwrap();

        let func_lib = Box::leak(Box::new(func_lib));

        LuaminCodegen::new(func_lib, 1)
    }

    fn compile(
        input: &str,
    ) -> Result<
        (
            LuaminScript<'static, StateLifetimeProvider, StateError>,
            Vec<usize>,
        ),
        LuaminCodegenError,
    > {
        let mut codegen = get_codegen();
        let (directive, stmts) = luamin_lex(input).unwrap();
        codegen.codegen(&[], &directive, &stmts)
    }

    fn compile_mut(
        input: &str,
    ) -> Result<
        (
            LuaminScriptMut<'static, StateLifetimeProvider, StateError>,
            Vec<usize>,
        ),
        LuaminCodegenError,
    > {
        let mut codegen = get_codegen();
        let (directive, stmts) = luamin_lex(input).unwrap();
        codegen.codegen_mut(&[], &directive, &stmts)
    }

    #[test]
    fn test_error_when_undeclared() {
        let result = compile_mut(
            r#"
            foo = 1
        "#,
        );
        assert!(result.is_err());

        let result = compile_mut(
            r#"
            do
                local bar = 2
            end
            bar = 3
        "#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_error_when_using_mutable_lib_func_in_immutable_context() {
        let err = compile(
            r#"
            local a = 1
            inc_score()
        "#,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            LuaminCodegenError::MutableLibFuncInImmutableContext
        ));
    }

    #[test]
    #[ignore]
    fn test_show_ixs() {
        let (script, _) = compile_mut(
            r#"
            local a = {nil, {42}}
            a[2][1] += 100
        "#,
        )
        .unwrap();
        println!("{:#?}", script.ixs());
    }
}
