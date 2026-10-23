use crate::component::{PlaceExpr, Stmt, ValueExpr};

pub const MAX_SOURCE_BYTES: usize = 1_048_576;
pub const MAX_SYNTAX_DEPTH: usize = 32;
pub const MAX_SYNTAX_NODES: usize = 16_384;
pub const MAX_INSTRUCTIONS: usize = 65_536;

enum Node<'a> {
    Stmt(&'a Stmt),
    Expr(&'a ValueExpr),
    Place(&'a PlaceExpr),
}

pub(crate) fn validate(stmts: &[Stmt]) -> bool {
    walk(stmts.iter().map(|s| (Node::Stmt(s), 0)).collect())
}

pub(crate) fn validate_expression(expr: &ValueExpr) -> bool {
    walk(vec![(Node::Expr(expr), 0)])
}

fn walk(mut pending: Vec<(Node<'_>, usize)>) -> bool {
    let mut count = 0;
    while let Some((node, depth)) = pending.pop() {
        count += 1;
        if depth >= MAX_SYNTAX_DEPTH || count > MAX_SYNTAX_NODES {
            return false;
        }
        let depth = depth + 1;
        macro_rules! exprs {
            ($items:expr) => {
                pending.extend(($items).iter().map(|e| (Node::Expr(e), depth)))
            };
        }
        macro_rules! stmts {
            ($items:expr) => {
                pending.extend(($items).iter().map(|s| (Node::Stmt(s), depth)))
            };
        }
        match node {
            Node::Place(p) => exprs!(&p.indexing_exprs),
            Node::Expr(e) => match e {
                ValueExpr::PrefixOperator(_, e) | ValueExpr::Parens(e) => {
                    pending.push((Node::Expr(e), depth))
                }
                ValueExpr::InfixOperator(_, a, b) => {
                    pending.push((Node::Expr(a), depth));
                    pending.push((Node::Expr(b), depth));
                }
                ValueExpr::LiteralList(es) => exprs!(es),
                ValueExpr::Identx(i) => {
                    if let Some(args) = &i.call_args {
                        exprs!(args);
                    }
                    exprs!(&i.indexing_exprs);
                }
                ValueExpr::Literal(_) => {}
            },
            Node::Stmt(s) => match s {
                Stmt::Decl(s) => {
                    if s.idents.is_empty() {
                        return false;
                    }
                    exprs!(&s.exprs);
                }
                Stmt::Assign(s) => {
                    if s.left_exprs.is_empty() {
                        return false;
                    }
                    pending.extend(s.left_exprs.iter().map(|p| (Node::Place(p), depth)));
                    exprs!(&s.right_exprs);
                }
                Stmt::CompoundAssign(s) => {
                    pending.push((Node::Place(&s.left_expr), depth));
                    pending.push((Node::Expr(&s.right_expr), depth));
                }
                Stmt::Call(s) => exprs!(&s.args),
                Stmt::Return(es) => exprs!(es),
                Stmt::Do(ss) => stmts!(ss),
                Stmt::If(s) => {
                    pending.push((Node::Expr(&s.condition), depth));
                    stmts!(&s.stmts);
                    for b in &s.elseif_branches {
                        pending.push((Node::Expr(&b.condition), depth));
                        stmts!(&b.stmts);
                    }
                    if let Some(ss) = &s.else_branch {
                        stmts!(ss);
                    }
                }
                Stmt::While(s) => {
                    pending.push((Node::Expr(&s.condition), depth));
                    stmts!(&s.stmts);
                }
                Stmt::Repeat(s) => {
                    pending.push((Node::Expr(&s.condition), depth));
                    stmts!(&s.stmts);
                }
                Stmt::NumericFor(s) => {
                    if s.step == 0 {
                        return false;
                    }
                    pending.push((Node::Expr(&s.start), depth));
                    pending.push((Node::Expr(&s.end), depth));
                    stmts!(&s.stmts);
                }
                Stmt::ListFor(s) => {
                    pending.push((Node::Expr(&s.list), depth));
                    stmts!(&s.stmts);
                }
                Stmt::Break | Stmt::Continue | Stmt::Semicolon => {}
            },
        }
        if pending.len() > MAX_SYNTAX_NODES {
            return false;
        }
    }
    true
}
