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
    let (directive, stmts) = luamin_parse(INPUT1).unwrap();
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
    let (directive, stmts) = luamin_parse(INPUT2).unwrap();
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
    let (_directive, stmts) = luamin_parse(
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
    match luamin_parse(input) {
        Ok(stmts) => {
            println!("Lex success");
            println!("{:#?}", stmts);
        }
        Err(e) => {
            println!("Parse error");
            println!("{}", e);
        }
    }
}
