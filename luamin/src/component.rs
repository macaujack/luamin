use smol_str::SmolStr;

// MARK: Instruction

/// Virtual machine instruction. Stack indices are zero-based from the bottom.
///
/// Jump offsets are relative to the next instruction. Generated scripts provide
/// valid indices and control flow; these instructions are exposed for inspection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Instruction {
    /// Pushes the supplied integer.
    PushNumber(i32),
    /// Pushes the supplied boolean.
    PushBool(bool),
    /// Pushes `count` nil values.
    PushNil {
        /// Number of nil values to push.
        count: u8,
    },
    /// Consumes the top `size` values to create a list, preserving their order.
    NewList {
        /// Number of stack values to collect.
        size: u8,
    },
    /// Calls the host function at the sorted library index `fn_index`.
    CallLib {
        /// Zero-based index in the bound, sorted host library.
        fn_index: u32,
    },
    /// Clones the value at `stack_index` onto the top of the stack.
    Dup {
        /// Zero-based stack position counted from the bottom.
        stack_index: u8,
    },
    /// Pops an index and pushes the selected element of the list at `stack_index`; missing elements yield nil.
    GetListElement {
        /// Zero-based stack position counted from the bottom.
        stack_index: u8,
        /// Script index corresponding to the first list element.
        base_index: i8,
    },
    /// Discards `pop_count` values from the top of the stack.
    Pop {
        /// Number of top values to discard.
        pop_count: u8,
    },
    /// Removes `remove_count` values starting at `stack_index`.
    Remove {
        /// Zero-based stack position counted from the bottom.
        stack_index: u8,
        /// Number of consecutive values to remove.
        remove_count: u8,
    },
    /// Pops a value and replaces the value at `stack_index`.
    Set {
        /// Zero-based stack position counted from the bottom.
        stack_index: u8,
    },
    /// Pops a value and index, then updates the list at `stack_index`, padding new elements with nil.
    SetListElement {
        /// Zero-based stack position counted from the bottom.
        stack_index: u8,
        /// Script index corresponding to the first list element.
        base_index: i8,
    },
    /// Truncates or pads the stack with nil to reach `adjust_count` values.
    AdjustStack {
        /// Required total stack length.
        adjust_count: u8,
    },

    /// Pops a condition and jumps if truthy.
    JumpIf {
        /// Signed displacement from the next instruction.
        relative_offset: i32,
    },
    /// Pops a condition and jumps if falsy.
    JumpIfNot {
        /// Signed displacement from the next instruction.
        relative_offset: i32,
    },
    /// Jumps unconditionally.
    Jump {
        /// Signed displacement from the next instruction.
        relative_offset: i32,
    },

    /// Wrapping integer addition.
    Add,
    /// Wrapping integer subtraction.
    Sub,
    /// Wrapping integer multiplication.
    Mul,
    /// Floor integer division; a zero divisor is an error.
    DivFloor,
    /// Integer modulo paired with floor division; a zero divisor is an error.
    Mod,
    /// Wrapping integer negation.
    Neg,

    /// Left shift with the count reduced modulo 32; negative counts shift right.
    Lshift,
    /// Arithmetic right shift with the count reduced modulo 32; negative counts shift left.
    Rshift,
    /// Bitwise AND.
    BitAnd,
    /// Bitwise OR.
    BitOr,
    /// Bitwise exclusive OR.
    BitXor,
    /// Bitwise complement.
    BitNot,

    /// Logical negation using script truthiness.
    LogicalNot,
    /// Value equality, with identity comparison for lists.
    Eq,
    /// Negated value equality.
    Neq,
    /// Integer less-than comparison.
    Lt,
    /// Integer less-than-or-equal comparison.
    Lte,
    /// Integer greater-than comparison.
    Gt,
    /// Integer greater-than-or-equal comparison.
    Gte,

    /// Replaces a list value with its element count.
    Length,
}

// MARK: Directive

/// Optional source-level parameter names and list indexing convention.
#[derive(Debug, Clone, Default)]
pub struct Directive {
    /// Source-declared parameter names; follow intrinsic parameters on the initial stack.
    pub param_list: Option<Vec<SmolStr>>,
    /// Script index of the first list element; `None` uses the compiler default.
    pub list_base_index: Option<i8>,
}

// MARK: Stmt

/// Parsed statement in a script block.
#[derive(Debug, Clone)]
pub enum Stmt {
    /// Declares local variables.
    Decl(DeclStmt),
    /// Assigns values to existing places.
    Assign(AssignStmt),
    /// Updates a place with an operator.
    CompoundAssign(CompoundAssignStmt),
    /// Calls a host function and discards its outputs.
    Call(CallStmt),
    /// Returns values from the script.
    Return(Vec<ValueExpr>),
    /// Executes a nested lexical scope.
    Do(Vec<Stmt>),
    /// Selects a conditional branch.
    If(IfStmt),
    /// Repeats while a condition is truthy.
    While(WhileStmt),
    /// Repeats until a condition becomes truthy.
    Repeat(RepeatStmt),
    /// Iterates over an integer range.
    NumericFor(NumericForStmt),
    /// Iterates over a list until the first nil element.
    ListFor(ListForStmt),
    /// Exits the innermost loop.
    Break,
    /// Starts the next iteration of the innermost loop.
    Continue,
    /// Empty statement.
    Semicolon,
}

/// Local variable declaration with parallel initializers.
#[derive(Debug, Clone)]
pub struct DeclStmt {
    /// Local names in declaration order.
    pub idents: Vec<SmolStr>,
    /// Initializer expressions; missing values are padded with nil.
    pub exprs: Vec<ValueExpr>,
}

/// Parallel assignment to variables or indexed list elements.
#[derive(Debug, Clone)]
pub struct AssignStmt {
    /// Assignment destinations in source order.
    pub left_exprs: Vec<PlaceExpr>,
    /// Expressions supplying assigned values.
    pub right_exprs: Vec<ValueExpr>,
}

/// Assignment combining a target value with a right-hand operand.
#[derive(Debug, Clone)]
pub struct CompoundAssignStmt {
    /// Destination read and updated by the operation.
    pub left_expr: PlaceExpr,
    /// Operation combining the old and new values.
    pub operator: CompoundAssignOperator,
    /// Right-hand operand.
    pub right_expr: ValueExpr,
}

/// Host function call used as a statement; returned values are discarded.
#[derive(Debug, Clone)]
pub struct CallStmt {
    /// Host function name.
    pub ident: SmolStr,
    /// Argument expressions in call order.
    pub args: Vec<ValueExpr>,
}

/// Conditional statement with optional elseif and else branches.
#[derive(Debug, Clone)]
pub struct IfStmt {
    /// Condition selecting the first branch.
    pub condition: ValueExpr,
    /// Body executed when the first condition is truthy.
    pub stmts: Vec<Stmt>,
    /// Additional conditions and bodies, tried in order.
    pub elseif_branches: Vec<ElseifBranch>,
    /// Fallback body when all conditions are falsy.
    pub else_branch: Option<Vec<Stmt>>,
}

/// Loop that tests its condition before each iteration.
#[derive(Debug, Clone)]
pub struct WhileStmt {
    /// Condition evaluated before each iteration.
    pub condition: ValueExpr,
    /// Loop body.
    pub stmts: Vec<Stmt>,
}

/// Loop that executes its body before testing the termination condition.
#[derive(Debug, Clone)]
pub struct RepeatStmt {
    /// Loop body, executed at least once.
    pub stmts: Vec<Stmt>,
    /// Termination condition evaluated after the body.
    pub condition: ValueExpr,
}

/// Numeric loop with an inclusive endpoint and a constant nonzero step.
#[derive(Debug, Clone)]
pub struct NumericForStmt {
    /// Read-only loop counter binding.
    pub ident: SmolStr,
    /// Initial counter value.
    pub start: ValueExpr,
    /// Inclusive endpoint.
    pub end: ValueExpr,
    /// Constant increment; must not be zero.
    pub step: i32,
    /// Loop body.
    pub stmts: Vec<Stmt>,
}

/// Loop binding an index and element until the first nil element.
///
/// Iteration begins at the selected list base index and ends at the first nil,
/// including a missing element beyond the current list length.
#[derive(Debug, Clone)]
pub struct ListForStmt {
    /// Read-only index binding using the selected list base.
    pub index_ident: SmolStr,
    /// Read-only element binding.
    pub element_ident: SmolStr,
    /// Expression providing the iterated list.
    pub list: ValueExpr,
    /// Loop body.
    pub stmts: Vec<Stmt>,
}

/// Additional conditional branch within an if statement.
#[derive(Debug, Clone)]
pub struct ElseifBranch {
    /// Condition selecting this branch.
    pub condition: ValueExpr,
    /// Branch body.
    pub stmts: Vec<Stmt>,
}

// MARK: Expr

/// Expression that produces script values.
#[derive(Debug, Clone)]
pub enum ValueExpr {
    /// Unary operator applied to an operand.
    PrefixOperator(Operator, Box<ValueExpr>),
    /// Binary operator applied to left and right operands.
    InfixOperator(Operator, Box<ValueExpr>, Box<ValueExpr>),
    /// Parenthesized expression.
    Parens(Box<ValueExpr>),
    /// Scalar literal.
    Literal(LiteralExpr),
    /// List literal with element expressions.
    LiteralList(Vec<ValueExpr>),
    /// Variable access or host call with optional indexing.
    Identx(IdentxExpr),
}

/// Scalar literal; hash strings compile to signed 32-bit hashes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LiteralExpr {
    /// Nil literal.
    Nil,
    /// Signed integer literal.
    Number(i32),
    /// Boolean literal.
    Bool(bool),
    /// String whose UTF-8 bytes are hashed with [`crate::hash_string`] at compilation.
    HashString(String),
}

/// Identifier optionally followed by a host call and list indexing operations.
#[derive(Debug, Clone)]
pub struct IdentxExpr {
    /// Variable or host function name.
    pub ident: SmolStr,
    /// `Some` for a host call, including an empty argument list; `None` for a variable.
    pub call_args: Option<Vec<ValueExpr>>,
    /// Successive list indices, evaluated from left to right.
    pub indexing_exprs: Vec<ValueExpr>,
}

/// Assignable variable or nested list element.
#[derive(Debug, Clone)]
pub struct PlaceExpr {
    /// Base variable name.
    pub ident: SmolStr,
    /// Successive list indices; empty for direct variable assignment.
    pub indexing_exprs: Vec<ValueExpr>,
}

/// Parsed unary or binary operator; its role depends on expression position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Operator {
    /// Logical `not`.
    LitNot,
    /// List length (`#`).
    Hashtag,
    /// Subtraction or unary negation (`-`).
    Minus,
    /// Bitwise XOR or unary complement (`~`).
    Tilde,
    /// Multiplication (`*`).
    Asterisk,
    /// Floor division (`//`).
    DoubleSlash,
    /// Modulo (`%`).
    Percent,
    /// Addition (`+`).
    Plus,
    /// Left shift (`<<`).
    Lshift,
    /// Right shift (`>>`).
    Rshift,
    /// Bitwise AND (`&`).
    Ampersand,
    /// Bitwise OR (`|`).
    Bar,
    /// Less than (`<`).
    Lt,
    /// Less than or equal (`<=`).
    Lte,
    /// Greater than (`>`).
    Gt,
    /// Greater than or equal (`>=`).
    Gte,
    /// Not equal (`~=`).
    Neq,
    /// Equal (`==`).
    Eq,
    /// Short-circuit logical `and`.
    LitAnd,
    /// Short-circuit logical `or`.
    LitOr,
}

/// Arithmetic or bitwise operator used in a compound assignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompoundAssignOperator {
    /// Addition assignment (`+=`).
    PlusAssign,
    /// Subtraction assignment (`-=`).
    MinusAssign,
    /// Multiplication assignment (`*=`).
    MulAssign,
    /// Floor division assignment (`//=`).
    DivFloorAssign,
    /// Modulo assignment (`%=`).
    ModAssign,
    /// Bitwise AND assignment (`&=`).
    BitAndAssign,
    /// Bitwise OR assignment (`|=`).
    BitOrAssign,
    /// Bitwise XOR assignment (`~=`).
    BitXorAssign,
    /// Left shift assignment (`<<=`).
    LshiftAssign,
    /// Right shift assignment (`>>=`).
    RshiftAssign,
}
