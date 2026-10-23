use smol_str::SmolStr;
use winnow::{
    ascii::{dec_int, escaped, multispace0},
    combinator::{
        Infix, Prefix, alt, cut_err, delimited, dispatch, empty, fail, opt, preceded, repeat,
        separated, seq, terminated,
    },
    error::{ContextError, ErrMode, StrContext, StrContextValue},
    prelude::*,
    token::{any, none_of, one_of, take_till, take_while},
};

use crate::component::{
    AssignStmt, CallStmt, CompoundAssignOperator, CompoundAssignStmt, DeclStmt, Directive,
    ElseifBranch, IdentxExpr, IfStmt, ListForStmt, LiteralExpr, NumericForStmt, Operator,
    PlaceExpr, RepeatStmt, Stmt, ValueExpr, WhileStmt,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum LexErrorKind {
    Syntax,
    SourceTooLarge,
    NestingTooDeep,
    TooManyTokens,
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("{message} at line {line}, column {column}")]
pub struct LexError {
    pub kind: LexErrorKind,
    pub offset: usize,
    pub line: usize,
    pub column: usize,
    message: String,
}

impl LexError {
    fn new(input: &str, offset: usize, kind: LexErrorKind, message: impl Into<String>) -> Self {
        let before = &input[..offset];
        let line = before.bytes().filter(|b| *b == b'\n').count() + 1;
        let column = before.rsplit('\n').next().unwrap_or("").chars().count() + 1;
        Self {
            kind,
            offset,
            line,
            column,
            message: message.into(),
        }
    }
}

pub fn luamin_lex(input: &str) -> Result<(Directive, Vec<Stmt>), LexError> {
    preflight(input)?;
    let trimmed = input.trim_start();
    let skipped = input.len() - trimmed.len();
    let (directive, stmts) = parse_program.parse(trimmed).map_err(|e| {
        let kind = if e
            .inner()
            .context()
            .any(|c| matches!(c, StrContext::Label("syntax nesting limit")))
        {
            LexErrorKind::NestingTooDeep
        } else {
            LexErrorKind::Syntax
        };
        LexError::new(input, skipped + e.offset(), kind, e.inner().to_string())
    })?;
    if !crate::limits::validate(&stmts) {
        return Err(LexError::new(
            input,
            0,
            LexErrorKind::NestingTooDeep,
            "Syntax tree limit exceeded",
        ));
    }
    Ok((directive, stmts))
}

// Bound parser recursion before allocating an AST. Ignore comments and hash
// strings, so their punctuation cannot accidentally consume a nesting budget.
fn preflight(input: &str) -> Result<(), LexError> {
    use crate::limits::{MAX_SOURCE_BYTES, MAX_SYNTAX_DEPTH, MAX_SYNTAX_NODES};
    if input.len() > MAX_SOURCE_BYTES {
        return Err(LexError::new(
            input,
            0,
            LexErrorKind::SourceTooLarge,
            "Source byte limit exceeded",
        ));
    }
    let bytes = input.as_bytes();
    let (mut i, mut depth, mut tokens) = (0, 0usize, 0);
    while i < bytes.len() {
        if bytes[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if bytes[i..].starts_with(b"--") {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        let start = i;
        if bytes[i..].starts_with(b"h\"") {
            i += 2;
            while i < bytes.len() {
                if bytes[i] == b'\\' {
                    i = (i + 2).min(bytes.len());
                } else if bytes[i] == b'"' {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
        } else if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
            i += 1;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            match &input[start..i] {
                "if" | "do" | "repeat" => depth += 1,
                "end" | "until" => depth = depth.saturating_sub(1),
                _ => {}
            }
        } else {
            match bytes[i] {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth = depth.saturating_sub(1),
                _ => {}
            }
            // Advance over a full character so error positions are UTF-8 boundaries.
            i += input[i..].chars().next().unwrap().len_utf8();
        }
        tokens += 1;
        if depth >= MAX_SYNTAX_DEPTH {
            return Err(LexError::new(
                input,
                start,
                LexErrorKind::NestingTooDeep,
                "Syntax nesting limit exceeded",
            ));
        }
        if tokens > MAX_SYNTAX_NODES {
            return Err(LexError::new(
                input,
                start,
                LexErrorKind::TooManyTokens,
                "Syntax token limit exceeded",
            ));
        }
    }
    Ok(())
}

fn bounded_expression(expr: ValueExpr) -> winnow::ModalResult<ValueExpr> {
    if crate::limits::validate_expression(&expr) {
        Ok(expr)
    } else {
        Err(nesting_error())
    }
}

fn nesting_error() -> ErrMode<ContextError> {
    let mut error = ContextError::new();
    error.push(StrContext::Label("syntax nesting limit"));
    ErrMode::Cut(error)
}

fn parse_program(input: &mut &str) -> winnow::ModalResult<(Directive, Vec<Stmt>)> {
    let _initial_comments: () = repeat(0.., parse_comment).parse_next(input)?;
    let directive = parse_directive.parse_next(input)?;
    let stmts = parse_stmts.parse_next(input)?;
    Ok((directive, stmts))
}

// MARK: Directive

fn parse_directive(input: &mut &str) -> winnow::ModalResult<Directive> {
    let mut directive = Directive::default();
    loop {
        if let Ok(param_list) = parse_param_list.parse_next(input) {
            directive.param_list = Some(param_list);
        } else if let Ok(list_base_index) = parse_list_base_index.parse_next(input) {
            directive.list_base_index = Some(list_base_index);
        } else {
            break;
        }
    }
    Ok(directive)
}

fn parse_param_list(input: &mut &str) -> winnow::ModalResult<Vec<SmolStr>> {
    let _ = token_pad("#param").parse_next(input)?;
    let idents: Vec<SmolStr> =
        separated(1.., cut_err(token_pad(parse_ident)), token_pad(',')).parse_next(input)?;
    Ok(idents)
}

fn parse_list_base_index(input: &mut &str) -> winnow::ModalResult<i8> {
    let _ = token_pad("#index").parse_next(input)?;
    let index = cut_err(token_pad(dec_int)).parse_next(input)?;
    Ok(index)
}

// MARK: Stmt

fn parse_stmts(input: &mut &str) -> winnow::ModalResult<Vec<Stmt>> {
    repeat(0.., parse_stmt).parse_next(input)
}

fn parse_stmt(input: &mut &str) -> winnow::ModalResult<Stmt> {
    let _depth = ParseDepth::enter()?;
    alt((
        alt((
            parse_compound_assign_stmt.map(Stmt::CompoundAssign),
            parse_assign_stmt.map(Stmt::Assign),
            parse_call_stmt.map(Stmt::Call),
            parse_decl_stmt.map(Stmt::Decl),
            parse_return_stmt.map(Stmt::Return),
            parse_if_stmt.map(Stmt::If),
            parse_list_for_stmt.map(Stmt::ListFor),
            parse_numeric_for_stmt.map(Stmt::NumericFor),
        )),
        alt((
            parse_while_stmt.map(Stmt::While),
            parse_break_stmt.value(Stmt::Break),
            parse_continue_stmt.value(Stmt::Continue),
            parse_repeat_stmt.map(Stmt::Repeat),
            parse_do_stmt.map(Stmt::Do),
            parse_semicolon_stmt.value(Stmt::Semicolon),
            fail.context(StrContext::Label("statement")),
        )),
    ))
    .parse_next(input)
}

fn parse_decl_stmt(input: &mut &str) -> winnow::ModalResult<DeclStmt> {
    let _ = token_pad("local").parse_next(input)?;
    let idents: Vec<SmolStr> =
        separated(1.., cut_err(token_pad(parse_ident)), token_pad(',')).parse_next(input)?;
    let exprs = opt(parse_assign_rhs).parse_next(input)?.unwrap_or_default();
    Ok(DeclStmt { idents, exprs })
}

fn parse_assign_stmt(input: &mut &str) -> winnow::ModalResult<AssignStmt> {
    seq! {AssignStmt {
        left_exprs: separated(1.., parse_place_expr, token_pad(',')),
        right_exprs: parse_assign_rhs,
    }}
    .parse_next(input)
}

fn parse_compound_assign_stmt(input: &mut &str) -> winnow::ModalResult<CompoundAssignStmt> {
    seq! {
        CompoundAssignStmt {
            left_expr: parse_place_expr,
            operator: parse_compound_assign_operator,
            right_expr: parse_value_expr,
        }
    }
    .parse_next(input)
}

fn parse_compound_assign_operator(input: &mut &str) -> winnow::ModalResult<CompoundAssignOperator> {
    dispatch!(any;
        '+' => token_pad('=').value(CompoundAssignOperator::PlusAssign),
        '-' => token_pad('=').value(CompoundAssignOperator::MinusAssign),
        '*' => token_pad('=').value(CompoundAssignOperator::MulAssign),
        '/' => alt((
            token_pad("/=").value(CompoundAssignOperator::DivFloorAssign),
            cut_err(fail.context(StrContext::Label("floor div assign operator")).context(StrContext::Expected(StrContextValue::Description("double slashes")))),
        )),
        '%' => token_pad('=').value(CompoundAssignOperator::ModAssign),
        '&' => token_pad('=').value(CompoundAssignOperator::BitAndAssign),
        '|' => token_pad('=').value(CompoundAssignOperator::BitOrAssign),
        '~' => token_pad('=').value(CompoundAssignOperator::BitXorAssign),
        '<' => token_pad("<=").value(CompoundAssignOperator::LshiftAssign),
        '>' => token_pad(">=").value(CompoundAssignOperator::RshiftAssign),
        _ => fail,
    )
    .parse_next(input)
}

fn parse_call_stmt(input: &mut &str) -> winnow::ModalResult<CallStmt> {
    seq! {CallStmt {
        ident: token_pad(parse_ident),
        args: parse_arg_list,
    }}
    .parse_next(input)
}

fn parse_return_stmt(input: &mut &str) -> winnow::ModalResult<Vec<ValueExpr>> {
    let _return = token_pad("return").parse_next(input)?;
    parse_csv::<0>.parse_next(input)
}

fn parse_do_stmt(input: &mut &str) -> winnow::ModalResult<Vec<Stmt>> {
    delimited(token_pad("do"), parse_stmts, token_pad("end")).parse_next(input)
}

fn parse_if_stmt(input: &mut &str) -> winnow::ModalResult<IfStmt> {
    let _if = token_pad("if").parse_next(input)?;
    let condition = parse_value_expr.parse_next(input)?;
    let _then = token_pad("then").parse_next(input)?;
    let stmts = parse_stmts.parse_next(input)?;

    let elseif_branches: Vec<ElseifBranch> = repeat(0.., parse_elseif_branch).parse_next(input)?;

    let else_branch = opt(preceded(token_pad("else"), parse_stmts)).parse_next(input)?;

    let _end = token_pad("end").parse_next(input)?;

    Ok(IfStmt {
        condition,
        stmts,
        elseif_branches,
        else_branch,
    })
}

fn parse_while_stmt(input: &mut &str) -> winnow::ModalResult<WhileStmt> {
    let _while = token_pad("while").parse_next(input)?;
    let condition = parse_value_expr.parse_next(input)?;
    let _do = token_pad("do").parse_next(input)?;
    let stmts = parse_stmts.parse_next(input)?;
    let _end = token_pad("end").parse_next(input)?;
    Ok(WhileStmt { condition, stmts })
}

fn parse_repeat_stmt(input: &mut &str) -> winnow::ModalResult<RepeatStmt> {
    let _repeat = token_pad("repeat").parse_next(input)?;
    let stmts = parse_stmts.parse_next(input)?;
    let _until = token_pad("until").parse_next(input)?;
    let condition = parse_value_expr.parse_next(input)?;
    Ok(RepeatStmt { stmts, condition })
}

fn parse_numeric_for_stmt(input: &mut &str) -> winnow::ModalResult<NumericForStmt> {
    let _for = token_pad("for").parse_next(input)?;
    let ident = cut_err(token_pad(parse_ident)).parse_next(input)?;
    let _eq = token_pad('=').parse_next(input)?;
    let start = parse_value_expr.parse_next(input)?;
    let _comma = token_pad(',').parse_next(input)?;
    let end = parse_value_expr.parse_next(input)?;
    let step = opt(preceded(token_pad(','), token_pad(dec_int)))
        .parse_next(input)?
        .unwrap_or(1);

    if step == 0 {
        return fail
            .context(StrContext::Label("step"))
            .context(StrContext::Expected(StrContextValue::Description(
                "non-zero integer",
            )))
            .parse_next(input);
    }

    let _do = token_pad("do").parse_next(input)?;

    let stmts = parse_stmts.parse_next(input)?;

    let _end = token_pad("end").parse_next(input)?;

    Ok(NumericForStmt {
        ident,
        start,
        end,
        step,
        stmts,
    })
}

fn parse_list_for_stmt(input: &mut &str) -> winnow::ModalResult<ListForStmt> {
    let _for = token_pad("for").parse_next(input)?;
    let index_ident = cut_err(token_pad(parse_ident)).parse_next(input)?;
    let _comma = token_pad(',').parse_next(input)?;
    let element_ident = cut_err(token_pad(parse_ident)).parse_next(input)?;
    let _in = token_pad("in").parse_next(input)?;
    let _ipairs = token_pad("ipairs").parse_next(input)?;
    let list = parse_parens.parse_next(input)?;
    let _do = token_pad("do").parse_next(input)?;
    let stmts = parse_stmts.parse_next(input)?;
    let _end = token_pad("end").parse_next(input)?;
    Ok(ListForStmt {
        index_ident,
        element_ident,
        list,
        stmts,
    })
}

fn parse_elseif_branch(input: &mut &str) -> winnow::ModalResult<ElseifBranch> {
    let _elseif = token_pad("elseif").parse_next(input)?;
    let condition = parse_value_expr.parse_next(input)?;
    let _then = token_pad("then").parse_next(input)?;
    let stmts = parse_stmts.parse_next(input)?;
    Ok(ElseifBranch { condition, stmts })
}

fn parse_break_stmt(input: &mut &str) -> winnow::ModalResult<()> {
    token_pad("break").value(()).parse_next(input)
}

fn parse_continue_stmt(input: &mut &str) -> winnow::ModalResult<()> {
    token_pad("continue").value(()).parse_next(input)
}

fn parse_semicolon_stmt(input: &mut &str) -> winnow::ModalResult<()> {
    token_pad(';').value(()).parse_next(input)
}

fn parse_assign_rhs(input: &mut &str) -> winnow::ModalResult<Vec<ValueExpr>> {
    let _equal_sign = token_pad('=').parse_next(input)?;
    parse_csv::<1>.parse_next(input)
}

// MARK: Expr

fn parse_value_expr(input: &mut &str) -> winnow::ModalResult<ValueExpr> {
    parse_expression(input, i64::MIN)
}

thread_local! {
    static PARSE_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

struct ParseDepth;

impl ParseDepth {
    fn enter() -> winnow::ModalResult<Self> {
        PARSE_DEPTH.with(|depth| {
            if depth.get() >= crate::MAX_SYNTAX_DEPTH {
                return Err(nesting_error());
            }
            depth.set(depth.get() + 1);
            Ok(Self)
        })
    }
}

impl Drop for ParseDepth {
    fn drop(&mut self) {
        PARSE_DEPTH.with(|depth| depth.set(depth.get() - 1));
    }
}

// Pratt parsing with a bound on every recursive operand, including unary
// chains. Checking the resulting tree alone would happen too late.
fn parse_expression(input: &mut &str, min_power: i64) -> winnow::ModalResult<ValueExpr> {
    let _depth = ParseDepth::enter()?;
    let mut operand = if let Some(operand) = opt(parse_non_op_expr).parse_next(input)? {
        operand
    } else {
        let Prefix(power, fold) = parse_expression_prefix(input)?;
        let operand = parse_expression(input, power)?;
        fold(input, operand)?
    };
    loop {
        let start = *input;
        let Some(infix) = opt(parse_expression_infix).parse_next(input)? else {
            break;
        };
        let (left, right, fold) = match infix {
            Infix::Left(power, fold) => (power, power + 1, fold),
            Infix::Right(power, fold) => (power, power - 1, fold),
            Infix::Neither(power, fold) => (power, power + 1, fold),
        };
        if left < min_power {
            *input = start;
            break;
        }
        let rhs = parse_expression(input, right)?;
        operand = fold(input, operand, rhs)?;
    }
    Ok(operand)
}

fn parse_expression_prefix<'a>(
    input: &mut &'a str,
) -> winnow::ModalResult<Prefix<&'a str, ValueExpr, ErrMode<ContextError>>> {
    use Operator::*;

    macro_rules! prefix {
        ($precedence:literal, $op:ident) => {
            winnow::combinator::Prefix($precedence, |_, expr| {
                bounded_expression(ValueExpr::PrefixOperator($op, Box::new(expr)))
            })
        };
    }

    dispatch!(any;
        'n' => token_pad("ot").value(prefix!(11, LitNot)),
        '#' => multispace0.value(prefix!(11, Hashtag)),
        '-' => multispace0.value(prefix!(11, Minus)),
        '~' => multispace0.value(prefix!(11, Tilde)),
        _ => fail,
    )
    .parse_next(input)
}

fn parse_expression_infix<'a>(
    input: &mut &'a str,
) -> winnow::ModalResult<Infix<&'a str, ValueExpr, ErrMode<ContextError>>> {
    use Operator::*;

    macro_rules! infix {
        ($precedence:literal, $op:ident) => {
            winnow::combinator::Infix::Left($precedence, |_, a, b| {
                bounded_expression(ValueExpr::InfixOperator($op, Box::new(a), Box::new(b)))
            })
        };
    }

    dispatch!(any;
        '*' => multispace0.value(infix!(10, Asterisk)),
        '/' => alt((
            token_pad('/').value(infix!(10, DoubleSlash)),
            cut_err(fail.context(StrContext::Label("floor div operator")).context(StrContext::Expected(StrContextValue::Description("double slashes")))),
        )),
        '%' => multispace0.value(infix!(10, Percent)),
        '+' => multispace0.value(infix!(9, Plus)),
        '-' => multispace0.value(infix!(9, Minus)),
        '<' => alt((token_pad('=').value(infix!(3, Lte)), token_pad('<').value(infix!(7, Lshift)), multispace0.value(infix!(3, Lt)))),
        '>' => alt((token_pad('=').value(infix!(3, Gte)), token_pad('>').value(infix!(7, Rshift)), multispace0.value(infix!(3, Gt)))),
        '&' => multispace0.value(infix!(6, Ampersand)),
        '~' => alt((token_pad('=').value(infix!(3, Neq)), multispace0.value(infix!(5, Tilde)))),
        '|' => multispace0.value(infix!(4, Bar)),
        '=' => token_pad('=').value(infix!(3, Eq)),
        'a' => token_pad("nd").value(infix!(2, LitAnd)),
        'o' => token_pad("r").value(infix!(1, LitOr)),
        _ => fail,
    )
    .parse_next(input)
}

fn parse_place_expr(input: &mut &str) -> winnow::ModalResult<PlaceExpr> {
    let ident = token_pad(parse_ident).parse_next(input)?;
    let indexing_exprs: Vec<ValueExpr> = repeat(0.., parse_brackets).parse_next(input)?;
    Ok(PlaceExpr {
        ident,
        indexing_exprs,
    })
}

fn parse_non_op_expr(input: &mut &str) -> winnow::ModalResult<ValueExpr> {
    alt((
        parse_parens.map(|expr| ValueExpr::Parens(Box::new(expr))),
        token_pad(parse_literal.map(ValueExpr::Literal)),
        parse_literal_list.map(ValueExpr::LiteralList),
        parse_identx.map(ValueExpr::Identx),
    ))
    .parse_next(input)
}

fn parse_parens(input: &mut &str) -> winnow::ModalResult<ValueExpr> {
    delimited(token_pad('('), parse_value_expr, token_pad(')')).parse_next(input)
}

fn parse_literal(input: &mut &str) -> winnow::ModalResult<LiteralExpr> {
    // TODO: Support hexadecimal
    alt((
        dec_int.map(LiteralExpr::Number),
        "nil".value(LiteralExpr::Nil),
        "true".value(LiteralExpr::Bool(true)),
        "false".value(LiteralExpr::Bool(false)),
        parse_literal_hash_string.map(LiteralExpr::HashString),
    ))
    .parse_next(input)
}

fn parse_literal_hash_string(input: &mut &str) -> winnow::ModalResult<String> {
    delimited("h\"", parse_and_escape_string, "\"").parse_next(input)
}

fn parse_and_escape_string(input: &mut &str) -> winnow::ModalResult<String> {
    escaped(
        none_of(['\\', '"']),
        '\\',
        dispatch!(any;
            '\\' => empty.value('\\'),
            '"' => empty.value('"'),
            'n' => empty.value('\n'),
            'r' => empty.value('\r'),
            't' => empty.value('\t'),
            _ => fail.context(StrContext::Label("escape character")),
        ),
    )
    .parse_next(input)
}

fn parse_literal_list(input: &mut &str) -> winnow::ModalResult<Vec<ValueExpr>> {
    delimited(token_pad('{'), parse_csv::<0>, token_pad('}')).parse_next(input)
}

fn parse_ident(input: &mut &str) -> winnow::ModalResult<SmolStr> {
    let ident = (
        one_of(|c: char| c.is_ascii_alphabetic() || c == '_'),
        take_while(0.., |c: char| c.is_ascii_alphanumeric() || c == '_'),
    )
        .take()
        .parse_next(input)?;

    if matches!(
        ident,
        "and"
            | "break"
            | "continue"
            | "do"
            | "else"
            | "elseif"
            | "end"
            | "false"
            | "for"
            | "function"
            | "goto"
            | "if"
            | "in"
            | "local"
            | "nil"
            | "not"
            | "or"
            | "repeat"
            | "return"
            | "then"
            | "true"
            | "until"
            | "while"
    ) {
        return fail
            .context(StrContext::Label("identifier"))
            .context(StrContext::Expected(StrContextValue::Description(
                "non-keyword",
            )))
            .parse_next(input);
    }

    Ok(SmolStr::new(ident))
}

fn parse_brackets(input: &mut &str) -> winnow::ModalResult<ValueExpr> {
    delimited(token_pad('['), token_pad(parse_value_expr), token_pad(']')).parse_next(input)
}

fn parse_identx(input: &mut &str) -> winnow::ModalResult<IdentxExpr> {
    let ident = token_pad(parse_ident).parse_next(input)?;
    let call_args = opt(parse_arg_list).parse_next(input)?;
    let indexing_exprs = repeat(0.., parse_brackets).parse_next(input)?;
    Ok(IdentxExpr {
        ident,
        call_args,
        indexing_exprs,
    })
}

fn parse_arg_list(input: &mut &str) -> winnow::ModalResult<Vec<ValueExpr>> {
    delimited(token_pad('('), parse_csv::<0>, token_pad(')')).parse_next(input)
}

fn parse_csv<const MIN_VALUES: usize>(input: &mut &str) -> winnow::ModalResult<Vec<ValueExpr>> {
    separated(MIN_VALUES.., parse_value_expr, token_pad(',')).parse_next(input)
}

fn token_pad<'a, O>(
    inner: impl Parser<&'a str, O, ErrMode<ContextError>>,
) -> impl Parser<&'a str, O, ErrMode<ContextError>> {
    terminated(
        inner,
        (
            multispace0,
            repeat::<_, &'a str, (), _, _>(0.., parse_comment),
        ),
    )
}

fn parse_comment<'a>(input: &mut &'a str) -> winnow::ModalResult<&'a str> {
    delimited("--", take_till(0.., '\n'), multispace0).parse_next(input)
}

// MARK: Test

#[cfg(test)]
mod tests;
