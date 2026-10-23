use smol_str::SmolStr;

// MARK: Ix

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Ix {
    PushNumber(i32),
    PushBool(bool),
    PushNil { count: u8 },
    NewList { size: u8 },
    CallLib { fn_index: u32 },
    Dup { stack_index: u8 },
    GetListElement { stack_index: u8, base_index: i8 },
    Pop { pop_count: u8 },
    Remove { stack_index: u8, remove_count: u8 },
    Set { stack_index: u8 },
    SetListElement { stack_index: u8, base_index: i8 },
    AdjustStack { adjust_count: u8 },

    JumpIf { relative_offset: i32 },
    JumpIfNot { relative_offset: i32 },
    Jump { relative_offset: i32 },

    Add,
    Sub,
    Mul,
    DivFloor,
    Mod,
    Neg,

    Lshift,
    Rshift,
    BitAnd,
    BitOr,
    BitXor,
    BitNot,

    LogicalNot,
    Eq,
    Neq,
    Lt,
    Lte,
    Gt,
    Gte,

    Length,
}

// MARK: Directive

#[derive(Debug, Clone, Default)]
pub struct Directive {
    pub param_list: Option<Vec<SmolStr>>,
    pub list_base_index: Option<i8>,
}

// MARK: Stmt

#[derive(Debug, Clone)]
pub enum Stmt {
    Decl(DeclStmt),
    Assign(AssignStmt),
    CompoundAssign(CompoundAssignStmt),
    Call(CallStmt),
    Return(Vec<ValueExpr>),
    Do(Vec<Stmt>),
    If(IfStmt),
    While(WhileStmt),
    Repeat(RepeatStmt),
    NumericFor(NumericForStmt),
    ListFor(ListForStmt),
    Break,
    Continue,
    Semicolon,
}

#[derive(Debug, Clone)]
pub struct DeclStmt {
    pub idents: Vec<SmolStr>,
    pub exprs: Vec<ValueExpr>,
}

#[derive(Debug, Clone)]
pub struct AssignStmt {
    pub left_exprs: Vec<PlaceExpr>,
    pub right_exprs: Vec<ValueExpr>,
}

#[derive(Debug, Clone)]
pub struct CompoundAssignStmt {
    pub left_expr: PlaceExpr,
    pub operator: CompoundAssignOperator,
    pub right_expr: ValueExpr,
}

#[derive(Debug, Clone)]
pub struct CallStmt {
    pub ident: SmolStr,
    pub args: Vec<ValueExpr>,
}

#[derive(Debug, Clone)]
pub struct IfStmt {
    pub condition: ValueExpr,
    pub stmts: Vec<Stmt>,
    pub elseif_branches: Vec<ElseifBranch>,
    pub else_branch: Option<Vec<Stmt>>,
}

#[derive(Debug, Clone)]
pub struct WhileStmt {
    pub condition: ValueExpr,
    pub stmts: Vec<Stmt>,
}

#[derive(Debug, Clone)]
pub struct RepeatStmt {
    pub stmts: Vec<Stmt>,
    pub condition: ValueExpr,
}

#[derive(Debug, Clone)]
pub struct NumericForStmt {
    pub ident: SmolStr,
    pub start: ValueExpr,
    pub end: ValueExpr,
    pub step: i32,
    pub stmts: Vec<Stmt>,
}

#[derive(Debug, Clone)]
pub struct ListForStmt {
    pub index_ident: SmolStr,
    pub element_ident: SmolStr,
    pub list: ValueExpr,
    pub stmts: Vec<Stmt>,
}

#[derive(Debug, Clone)]
pub struct ElseifBranch {
    pub condition: ValueExpr,
    pub stmts: Vec<Stmt>,
}

// MARK: Expr

#[derive(Debug, Clone)]
pub enum ValueExpr {
    PrefixOperator(Operator, Box<ValueExpr>),
    InfixOperator(Operator, Box<ValueExpr>, Box<ValueExpr>),
    Parens(Box<ValueExpr>),
    Literal(LiteralExpr),
    LiteralList(Vec<ValueExpr>),
    Identx(IdentxExpr),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LiteralExpr {
    Nil,
    Number(i32),
    Bool(bool),
    HashString(String),
}

#[derive(Debug, Clone)]
pub struct IdentxExpr {
    pub ident: SmolStr,
    pub call_args: Option<Vec<ValueExpr>>,
    pub indexing_exprs: Vec<ValueExpr>,
}

#[derive(Debug, Clone)]
pub struct PlaceExpr {
    pub ident: SmolStr,
    pub indexing_exprs: Vec<ValueExpr>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Operator {
    LitNot,
    Hashtag,
    Minus,
    Tilde,
    Asterisk,
    DoubleSlash,
    Percent,
    Plus,
    Lshift,
    Rshift,
    Ampersand,
    Bar,
    Lt,
    Lte,
    Gt,
    Gte,
    Neq,
    Eq,
    LitAnd,
    LitOr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompoundAssignOperator {
    PlusAssign,
    MinusAssign,
    MulAssign,
    DivFloorAssign,
    ModAssign,
    BitAndAssign,
    BitOrAssign,
    BitXorAssign,
    LshiftAssign,
    RshiftAssign,
}
