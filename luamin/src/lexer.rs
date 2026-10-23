use smol_str::SmolStr;
use winnow::{
    ascii::{dec_int, escaped, multispace0},
    combinator::{
        Infix, Prefix, alt, cut_err, delimited, dispatch, empty, expression, fail, opt, preceded,
        repeat, separated, seq, terminated,
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

#[derive(Debug, Clone)]
#[repr(transparent)]
pub struct LexError(String);

impl std::fmt::Display for LexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for LexError {}

pub fn luamin_lex(input: &str) -> Result<(Directive, Vec<Stmt>), LexError> {
    parse_program
        .parse(input.trim_start())
        .map_err(|e| LexError(e.to_string()))
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
    expression(parse_non_op_expr)
        .prefix(parse_expression_prefix)
        .infix(parse_expression_infix)
        .parse_next(input)
}

fn parse_expression_prefix<'a>(
    input: &mut &'a str,
) -> winnow::ModalResult<Prefix<&'a str, ValueExpr, ErrMode<ContextError>>> {
    use Operator::*;

    macro_rules! prefix {
        ($precedence:literal, $op:ident) => {
            winnow::combinator::Prefix($precedence, |_, expr| {
                Ok(ValueExpr::PrefixOperator($op, Box::new(expr)))
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
                Ok(ValueExpr::InfixOperator($op, Box::new(a), Box::new(b)))
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
mod tests {
    use super::*;

    use std::assert_matches;

    macro_rules! assert_ident {
        ($expression:expr, $expected:literal $(,)?) => {
            assert!(check_identx(&$expression, $expected));
        };
        ($expression:expr, $expected:literal, $($arg:tt)+) => {
            assert!(check_identx(&$expression, $expected), $($arg)+);
        };
    }

    fn check_identx(expr: &ValueExpr, expected: &str) -> bool {
        let identx = match expr {
            ValueExpr::Identx(identx) => identx,
            _ => panic!(),
        };
        identx.ident == expected
    }

    fn unwrap_infix(expr: ValueExpr) -> (Operator, ValueExpr, ValueExpr) {
        if let ValueExpr::InfixOperator(op, lhs, rhs) = expr {
            (op, *lhs, *rhs)
        } else {
            panic!();
        }
    }

    fn unwrap_prefix(expr: ValueExpr) -> (Operator, ValueExpr) {
        if let ValueExpr::PrefixOperator(op, expr) = expr {
            (op, *expr)
        } else {
            panic!();
        }
    }

    fn unwrap_parens(expr: ValueExpr) -> ValueExpr {
        if let ValueExpr::Parens(expr) = expr {
            *expr
        } else {
            panic!();
        }
    }

    #[test]
    fn test_luamin_lex() {
        const INPUT1: &str = r#"

            local foo,bar=1, nil,3

            foo,bar=1, nil,3

        "#;
        let (directive, stmts) = luamin_lex(INPUT1).unwrap();
        assert!(directive.param_list.is_none());
        assert!(directive.list_base_index.is_none());
        assert_eq!(stmts.len(), 2);

        const INPUT2: &str = r#"
            #param p1,p2
            #index 0
            #index 1
            local foo,bar=1, nil,3

            foo,bar=1, nil,3

        "#;
        let (directive, stmts) = luamin_lex(INPUT2).unwrap();
        assert_eq!(directive.param_list.unwrap(), vec!["p1", "p2"]);
        assert_eq!(directive.list_base_index.unwrap(), 1);
        assert_eq!(stmts.len(), 2);
    }

    #[test]
    fn test_parse_stmt() {
        let stmt = parse_stmt.parse("local foo,bar=1, nil,3").unwrap();
        assert_matches!(stmt, Stmt::Decl(_));
        let stmt = parse_stmt.parse("foo,bar=1, nil,3").unwrap();
        assert_matches!(stmt, Stmt::Assign(_));
    }

    #[test]
    fn test_parse_param_list_stmt() {
        let stmt = parse_param_list.parse("#param foo,bar").unwrap();
        assert_eq!(stmt, vec!["foo", "bar"]);
    }

    #[test]
    fn test_parse_decl_stmt() {
        let stmt = parse_decl_stmt.parse("local foo,bar=1, nil,3").unwrap();
        assert_eq!(stmt.idents, vec!["foo", "bar"]);
        assert_eq!(stmt.exprs.len(), 3);
        assert_matches!(stmt.exprs[0], ValueExpr::Literal(LiteralExpr::Number(1)));
        assert_matches!(stmt.exprs[1], ValueExpr::Literal(LiteralExpr::Nil));
        assert_matches!(stmt.exprs[2], ValueExpr::Literal(LiteralExpr::Number(3)));
    }

    #[test]
    fn test_parse_do_stmt() {
        let stmt = parse_do_stmt.parse("do foo,bar=1, nil,3; end").unwrap();
        assert_eq!(stmt.len(), 2);
        assert_matches!(stmt[0], Stmt::Assign(_));
        assert_matches!(stmt[1], Stmt::Semicolon);
    }

    #[test]
    fn test_parse_if_stmt() {
        let stmt = parse_if_stmt.parse("if a then b = 10 end").unwrap();
        assert_matches!(stmt.condition, ValueExpr::Identx(IdentxExpr { ident, indexing_exprs, .. }) if ident == "a" && indexing_exprs.is_empty());
        assert!(stmt.elseif_branches.is_empty());
        assert!(stmt.else_branch.is_none());

        let stmt = parse_if_stmt
            .parse("if a then b = 10 elseif c then d = 20 elseif e then f = 30 end")
            .unwrap();
        assert_eq!(stmt.elseif_branches.len(), 2);
        assert!(stmt.else_branch.is_none());

        let stmt = parse_if_stmt
            .parse("if a then b = 10 else c = 20 end")
            .unwrap();
        assert!(stmt.elseif_branches.is_empty());
        assert!(stmt.else_branch.is_some());

        let stmt = parse_if_stmt
            .parse("if a then b = 10 elseif c then d = 20 else e = 30 end")
            .unwrap();
        assert_eq!(stmt.elseif_branches.len(), 1);
        assert!(stmt.else_branch.is_some());
    }

    #[test]
    fn test_parse_assign_stmt() {
        let stmt = parse_assign_stmt.parse("foo,bar=1, nil,3").unwrap();
        assert_eq!(stmt.left_exprs.len(), 2);
        assert_matches!(stmt.left_exprs[0], PlaceExpr { ident: ref s, .. } if s == "foo");
        assert_matches!(stmt.left_exprs[1], PlaceExpr { ident: ref s, .. } if s == "bar");
        assert_eq!(stmt.right_exprs.len(), 3);
        assert_matches!(
            stmt.right_exprs[0],
            ValueExpr::Literal(LiteralExpr::Number(1))
        );
        assert_matches!(stmt.right_exprs[1], ValueExpr::Literal(LiteralExpr::Nil));
        assert_matches!(
            stmt.right_exprs[2],
            ValueExpr::Literal(LiteralExpr::Number(3))
        );
    }

    #[test]
    fn test_parse_expr() {
        let expr = parse_value_expr.parse("1 +   foo*_Bar").unwrap();
        let (op, lhs, rhs) = unwrap_infix(expr);
        assert_eq!(op, Operator::Plus);
        assert_matches!(lhs, ValueExpr::Literal(LiteralExpr::Number(1)));
        let (op, lhs, rhs) = unwrap_infix(rhs);
        assert_eq!(op, Operator::Asterisk);
        assert_ident!(lhs, "foo");
        assert_ident!(rhs, "_Bar");

        let expr = parse_value_expr.parse("foo*(  3 + -a)<bar").unwrap();
        let (op, lhs, rhs) = unwrap_infix(expr);
        assert_eq!(op, Operator::Lt);
        assert_ident!(rhs, "bar");
        let (op, lhs, rhs) = unwrap_infix(lhs);
        assert_eq!(op, Operator::Asterisk);
        assert_ident!(lhs, "foo");
        let rhs = unwrap_parens(rhs);
        let (op, lhs, rhs) = unwrap_infix(rhs);
        assert_eq!(op, Operator::Plus);
        assert_matches!(lhs, ValueExpr::Literal(LiteralExpr::Number(3)));
        let (op, expr) = unwrap_prefix(rhs);
        assert_eq!(op, Operator::Minus);
        assert_ident!(expr, "a");

        // A lexically valid expression but is not semantically valid. Only for testing.
        let expr = parse_value_expr.parse("a<<3<4<=5").unwrap();
        let (op, lhs, rhs) = unwrap_infix(expr);
        assert_eq!(op, Operator::Lte);
        assert_matches!(rhs, ValueExpr::Literal(LiteralExpr::Number(5)));
        let (op, lhs, rhs) = unwrap_infix(lhs);
        assert_eq!(op, Operator::Lt);
        assert_matches!(rhs, ValueExpr::Literal(LiteralExpr::Number(4)));
        let (op, lhs, rhs) = unwrap_infix(lhs);
        assert_eq!(op, Operator::Lshift);
        assert_ident!(lhs, "a");
        assert_matches!(rhs, ValueExpr::Literal(LiteralExpr::Number(3)));
    }

    #[test]
    fn test_parse_non_op_expr() {
        let expr = parse_non_op_expr.parse("0").unwrap();
        assert_matches!(expr, ValueExpr::Literal(LiteralExpr::Number(0)));
        let expr = parse_non_op_expr.parse("42").unwrap();
        assert_matches!(expr, ValueExpr::Literal(LiteralExpr::Number(42)));
        let expr = parse_non_op_expr.parse("-42  ").unwrap();
        assert_matches!(expr, ValueExpr::Literal(LiteralExpr::Number(-42)));
        let expr = parse_non_op_expr.parse("nil").unwrap();
        assert_matches!(expr, ValueExpr::Literal(LiteralExpr::Nil));
        let expr = parse_non_op_expr.parse("true  ").unwrap();
        assert_matches!(expr, ValueExpr::Literal(LiteralExpr::Bool(true)));

        let expr = parse_non_op_expr.parse("fooBar").unwrap();
        assert_ident!(expr, "fooBar");
        let expr = parse_non_op_expr.parse("_foo   ").unwrap();
        assert_ident!(expr, "_foo");
        assert!(parse_non_op_expr.parse("1foo").is_err());
    }

    #[test]
    fn test_parse_literal_list() {
        let expr = parse_literal_list.parse("{1,2,  a }  ").unwrap();
        assert_eq!(expr.len(), 3);
        assert_matches!(expr[0], ValueExpr::Literal(LiteralExpr::Number(1)));
        assert_matches!(expr[1], ValueExpr::Literal(LiteralExpr::Number(2)));
        assert_ident!(expr[2], "a");
    }

    #[test]
    fn test_parse_identx_expr() {
        let expr = parse_identx.parse("foo [  1][a  ]").unwrap();
        assert_eq!(expr.ident, "foo");
        assert_eq!(expr.indexing_exprs.len(), 2);
        assert!(expr.call_args.is_none());
        assert_matches!(
            expr.indexing_exprs[0],
            ValueExpr::Literal(LiteralExpr::Number(1))
        );
        let expr = match &expr.indexing_exprs[1] {
            ValueExpr::Identx(identx) => identx,
            _ => panic!(),
        };
        assert_eq!(expr.ident, "a");

        let expr = parse_identx.parse("foo ( 1, 2 )").unwrap();
        assert_eq!(expr.ident, "foo");
        assert!(expr.indexing_exprs.is_empty());
        let call_args = expr.call_args.unwrap();
        assert_eq!(call_args.len(), 2);
        assert_matches!(call_args[0], ValueExpr::Literal(LiteralExpr::Number(1)));
        assert_matches!(call_args[1], ValueExpr::Literal(LiteralExpr::Number(2)));

        let expr = parse_identx.parse("foo()[1][2]").unwrap();
        assert_eq!(expr.ident, "foo");
        assert_eq!(expr.indexing_exprs.len(), 2);
        let call_args = expr.call_args.unwrap();
        assert!(call_args.is_empty());
    }

    #[test]
    fn test_parse_place_expr() {
        let PlaceExpr {
            ident,
            indexing_exprs,
        } = parse_place_expr.parse("foo[1]").unwrap();
        assert_eq!(ident, "foo");
        assert_eq!(indexing_exprs.len(), 1);
        assert_matches!(
            indexing_exprs[0],
            ValueExpr::Literal(LiteralExpr::Number(1))
        );

        let PlaceExpr {
            ident,
            indexing_exprs,
        } = parse_place_expr.parse("foo [  1][2  ]").unwrap();
        assert_eq!(ident, "foo");
        assert_eq!(indexing_exprs.len(), 2);
        assert_matches!(
            indexing_exprs[0],
            ValueExpr::Literal(LiteralExpr::Number(1))
        );
        assert_matches!(
            indexing_exprs[1],
            ValueExpr::Literal(LiteralExpr::Number(2))
        );

        let PlaceExpr {
            ident,
            indexing_exprs,
        } = parse_place_expr.parse("foo [  bar[a]][2  ]").unwrap();
        assert_eq!(ident, "foo");
        assert_eq!(indexing_exprs.len(), 2);
        assert_matches!(
            indexing_exprs[1],
            ValueExpr::Literal(LiteralExpr::Number(2))
        );

        let identx = match indexing_exprs[0].clone() {
            ValueExpr::Identx(identx) => identx,
            _ => panic!(),
        };
        assert_eq!(identx.ident, "bar");
        assert_eq!(identx.indexing_exprs.len(), 1);
        let expr = match &identx.indexing_exprs[0] {
            ValueExpr::Identx(identx) => identx,
            _ => panic!(),
        };
        assert_eq!(expr.ident, "a");
    }

    #[test]
    fn test_parse_literal_hash_string() {
        let expr = parse_literal_hash_string
            .parse(r#"h" Hello. I'm a \"string\"!\n ""#)
            .unwrap();
        assert_eq!(expr, " Hello. I'm a \"string\"!\n ");

        let result = parse_literal_hash_string.parse(r#" h"Invalid unescaped "double quote"" "#);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_with_comments() {
        let (_directive, stmts) = luamin_lex(
            r#"
            -- initial comment
            local a = 42   -- forty-two  

            a+=100--increment by 100
            -- a whole line comment --
            -- another whole line comment
            a -= 10--
            local -- separated by comment
            b--
            =--
            --
            {--
            true -- 
            }--
            return a --return the value
        "#,
        )
        .unwrap();
        assert_eq!(stmts.len(), 5);
        assert_matches!(stmts[0], Stmt::Decl(_));
        assert_matches!(stmts[1], Stmt::CompoundAssign(_));
        assert_matches!(stmts[2], Stmt::CompoundAssign(_));
        assert_matches!(stmts[3], Stmt::Decl(_));
        assert_matches!(stmts[4], Stmt::Return(_));
    }

    #[test]
    #[ignore]
    fn test_show_ast() {
        let input = r#"
            local function = 4
        "#;
        match luamin_lex(input) {
            Ok(stmts) => {
                println!("Lex success");
                println!("{:#?}", stmts);
            }
            Err(e) => {
                println!("Lex error");
                println!("{}", e);
            }
        }
    }
}
