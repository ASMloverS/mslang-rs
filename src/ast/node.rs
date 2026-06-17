// AST 节点定义：表达式（task 09）+ 语句（task 10）。
// 完整规格见 docs/mslang/tasks/09-ast-expression-nodes.md 与 10-ast-statement-nodes.md。

#[derive(Debug, Clone)]
pub enum Literal {
    Int(i64),
    Float(f64),
    String(String),
    Bool(bool),
    Nil,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinaryOp {
    Add,
    Subtract,
    Multiply,
    Divide,
    FloorDiv,
    Modulo,
    Power,
    Equal,
    NotEqual,
    Less,
    Greater,
    LessEqual,
    GreaterEqual,
    BitAnd,
    BitOr,
    BitXor,
    LeftShift,
    RightShift,
    And,
    Or,
    In,
    Is,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnaryOp {
    Negate,
    Not,
    BitNot,
    ChannelReceive,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AssignOp {
    Assign,
    PlusAssign,
    MinusAssign,
    StarAssign,
    SlashAssign,
    DoubleSlashAssign,
    PercentAssign,
    DoubleStarAssign,
    BitAndAssign,
    BitOrAssign,
    BitXorAssign,
    LeftShiftAssign,
    RightShiftAssign,
}

#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub default: Option<Expr>,
    pub is_variadic: bool,
}

#[derive(Debug, Clone)]
pub struct ForClause {
    pub targets: Vec<String>,
    pub iterable: Box<Expr>,
}

#[derive(Debug, Clone)]
pub enum Stmt {
    VarDecl {
        name: String,
        initializer: Expr,
    },
    ShortVarDecl {
        name: String,
        initializer: Expr,
    },
    ConstDecl {
        name: String,
        initializer: Expr,
    },
    Assign {
        target: Expr,
        op: AssignOp,
        value: Expr,
    },
    ExprStmt {
        expr: Expr,
    },
    Block {
        statements: Vec<Stmt>,
    },
    If {
        condition: Expr,
        then_block: Vec<Stmt>,
        elif_clauses: Vec<(Expr, Vec<Stmt>)>,
        else_block: Option<Vec<Stmt>>,
    },
    While {
        condition: Expr,
        body: Vec<Stmt>,
    },
    ForIn {
        variable: String,
        second_variable: Option<String>,
        iterable: Expr,
        body: Vec<Stmt>,
    },
    Break,
    Continue,
    Return {
        values: Vec<Expr>,
    },
    FnDecl {
        name: String,
        params: Vec<Param>,
        body: Vec<Stmt>,
        is_async: bool,
    },
    ClassDecl {
        name: String,
        parent: Option<String>,
        methods: Vec<Stmt>,
        class_vars: Vec<(String, Expr)>,
    },
    Defer {
        expr: Expr,
    },
    Try {
        try_block: Vec<Stmt>,
        except_clauses: Vec<ExceptClause>,
        finally_block: Option<Vec<Stmt>>,
    },
    With {
        expression: Expr,
        alias: Option<String>,
        body: Vec<Stmt>,
    },
    Import {
        module_path: Vec<String>,
        alias: Option<String>,
        is_stdlib: bool,
    },
    FromImport {
        module_path: Vec<String>,
        targets: Vec<(String, Option<String>)>,
        is_stdlib: bool,
    },
    Throw {
        expr: Option<Expr>,
    },
    Nonlocal {
        names: Vec<String>,
    },
    Global {
        names: Vec<String>,
    },
}

#[derive(Debug, Clone)]
pub struct ExceptClause {
    pub type_name: Option<Vec<String>>,
    pub alias: Option<String>,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone)]
pub struct Program {
    pub statements: Vec<Stmt>,
}

#[derive(Debug, Clone)]
pub enum Expr {
    Literal(Literal),
    Identifier(String),
    Binary {
        left: Box<Expr>,
        op: BinaryOp,
        right: Box<Expr>,
    },
    Unary {
        op: UnaryOp,
        operand: Box<Expr>,
    },
    Assign {
        target: Box<Expr>,
        op: AssignOp,
        value: Box<Expr>,
    },
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
    },
    Index {
        object: Box<Expr>,
        index: Box<Expr>,
    },
    Dot {
        object: Box<Expr>,
        name: String,
    },
    Slice {
        object: Box<Expr>,
        start: Option<Box<Expr>>,
        stop: Option<Box<Expr>>,
        step: Option<Box<Expr>>,
    },
    Ternary {
        condition: Box<Expr>,
        then_expr: Box<Expr>,
        else_expr: Box<Expr>,
    },
    ListLiteral {
        elements: Vec<Expr>,
    },
    DictLiteral {
        pairs: Vec<(Expr, Expr)>,
    },
    SetLiteral {
        elements: Vec<Expr>,
    },
    TupleLiteral {
        elements: Vec<Expr>,
    },
    FnLiteral {
        params: Vec<Param>,
        body: Vec<Stmt>,
    },
    ListComprehension {
        expr: Box<Expr>,
        for_clauses: Vec<ForClause>,
        condition: Option<Box<Expr>>,
    },
    DictComprehension {
        key_expr: Box<Expr>,
        value_expr: Box<Expr>,
        for_clauses: Vec<ForClause>,
        condition: Option<Box<Expr>>,
    },
    SetComprehension {
        expr: Box<Expr>,
        for_clauses: Vec<ForClause>,
        condition: Option<Box<Expr>>,
    },
    GeneratorExpression {
        expr: Box<Expr>,
        for_clauses: Vec<ForClause>,
        condition: Option<Box<Expr>>,
    },
    SuperAccess {
        name: String,
    },
    Yield {
        value: Option<Box<Expr>>,
    },
    YieldFrom {
        iterable: Box<Expr>,
    },
    Await {
        expr: Box<Expr>,
    },
    Go {
        expr: Box<Expr>,
    },
    Grouping {
        expr: Box<Expr>,
    },
}

impl std::fmt::Display for Expr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Expr::Literal(lit) => write!(f, "{}", lit),
            Expr::Identifier(name) => write!(f, "{}", name),
            Expr::Binary { left, op, right } => {
                write!(f, "({} {} {})", left, op, right)
            }
            Expr::Unary { op, operand } => {
                write!(f, "({}{})", op, operand)
            }
            Expr::Assign { target, op, value } => {
                write!(f, "({} {} {})", target, op, value)
            }
            Expr::Call { callee, args } => {
                let args_str: Vec<_> = args.iter().map(|a| format!("{}", a)).collect();
                write!(f, "{}({})", callee, args_str.join(", "))
            }
            Expr::Index { object, index } => {
                write!(f, "{}[{}]", object, index)
            }
            Expr::Dot { object, name } => {
                write!(f, "{}.{}", object, name)
            }
            Expr::Ternary {
                condition,
                then_expr,
                else_expr,
            } => {
                write!(f, "({} if {} else {})", then_expr, condition, else_expr)
            }
            Expr::ListLiteral { elements } => {
                let els: Vec<_> = elements.iter().map(|e| format!("{}", e)).collect();
                write!(f, "[{}]", els.join(", "))
            }
            Expr::DictLiteral { pairs } => {
                let ps: Vec<_> = pairs.iter().map(|(k, v)| format!("{}: {}", k, v)).collect();
                write!(f, "{{{}}}", ps.join(", "))
            }
            Expr::Await { expr } => write!(f, "await {}", expr),
            Expr::Yield { value } => match value {
                Some(v) => write!(f, "yield {}", v),
                None => write!(f, "yield"),
            },
            Expr::YieldFrom { iterable } => write!(f, "yield from {}", iterable),
            Expr::Go { expr } => write!(f, "go {}", expr),
            Expr::Grouping { expr } => write!(f, "({})", expr),
            Expr::Slice {
                object,
                start,
                stop,
                step,
            } => {
                let s = start.as_ref().map(|e| format!("{}", e)).unwrap_or_default();
                let e = stop.as_ref().map(|e| format!("{}", e)).unwrap_or_default();
                let sp = step.as_ref().map(|e| format!(":{}", e)).unwrap_or_default();
                write!(f, "{}[{}:{}{}]", object, s, e, sp)
            }
            Expr::SetLiteral { elements } => {
                let els: Vec<_> = elements.iter().map(|e| format!("{}", e)).collect();
                write!(f, "{{{}}}", els.join(", "))
            }
            Expr::TupleLiteral { elements } => {
                let els: Vec<_> = elements.iter().map(|e| format!("{}", e)).collect();
                let trailing = if elements.len() == 1 { "," } else { "" };
                write!(f, "({}{})", els.join(", "), trailing)
            }
            Expr::FnLiteral { params, body: _ } => {
                let ps: Vec<_> = params
                    .iter()
                    .map(|p| {
                        if p.is_variadic {
                            format!("{}...", p.name)
                        } else if let Some(d) = &p.default {
                            format!("{} = {}", p.name, d)
                        } else {
                            p.name.clone()
                        }
                    })
                    .collect();
                write!(f, "fn({}) {{ ... }}", ps.join(", "))
            }
            Expr::ListComprehension {
                expr,
                for_clauses,
                condition,
            } => {
                let fcs: Vec<_> = for_clauses
                    .iter()
                    .map(|c| format!("for {} in {}", c.targets.join(", "), c.iterable))
                    .collect();
                let cond = condition
                    .as_ref()
                    .map(|c| format!(" if {}", c))
                    .unwrap_or_default();
                write!(f, "[{} {}{}]", expr, fcs.join(" "), cond)
            }
            Expr::DictComprehension {
                key_expr,
                value_expr,
                for_clauses,
                condition,
            } => {
                let fcs: Vec<_> = for_clauses
                    .iter()
                    .map(|c| format!("for {} in {}", c.targets.join(", "), c.iterable))
                    .collect();
                let cond = condition
                    .as_ref()
                    .map(|c| format!(" if {}", c))
                    .unwrap_or_default();
                write!(
                    f,
                    "{{{}: {} {}{}}}",
                    key_expr,
                    value_expr,
                    fcs.join(" "),
                    cond
                )
            }
            Expr::SetComprehension {
                expr,
                for_clauses,
                condition,
            } => {
                let fcs: Vec<_> = for_clauses
                    .iter()
                    .map(|c| format!("for {} in {}", c.targets.join(", "), c.iterable))
                    .collect();
                let cond = condition
                    .as_ref()
                    .map(|c| format!(" if {}", c))
                    .unwrap_or_default();
                write!(f, "{{{} {}{}}}", expr, fcs.join(" "), cond)
            }
            Expr::GeneratorExpression {
                expr,
                for_clauses,
                condition,
            } => {
                let fcs: Vec<_> = for_clauses
                    .iter()
                    .map(|c| format!("for {} in {}", c.targets.join(", "), c.iterable))
                    .collect();
                let cond = condition
                    .as_ref()
                    .map(|c| format!(" if {}", c))
                    .unwrap_or_default();
                write!(f, "({} {}{})", expr, fcs.join(" "), cond)
            }
            Expr::SuperAccess { name } => write!(f, "super.{}", name),
        }
    }
}

impl std::fmt::Display for Stmt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Stmt::VarDecl { name, initializer } => {
                write!(f, "var {} = {}", name, initializer)
            }
            Stmt::ShortVarDecl { name, initializer } => {
                write!(f, "{} := {}", name, initializer)
            }
            Stmt::ConstDecl { name, initializer } => {
                write!(f, "const {} = {}", name, initializer)
            }
            Stmt::Assign { target, op, value } => {
                write!(f, "{} {} {}", target, op, value)
            }
            Stmt::ExprStmt { expr } => write!(f, "{}", expr),
            Stmt::Block { statements } => {
                if statements.is_empty() {
                    return write!(f, "{{}}");
                }
                let stmts: Vec<_> = statements.iter().map(|s| format!("{}", s)).collect();
                write!(f, "{{\n{}\n}}", stmts.join("\n"))
            }
            Stmt::If {
                condition,
                then_block,
                elif_clauses,
                else_block,
            } => {
                let body: Vec<_> = then_block.iter().map(|s| format!("{}", s)).collect();
                write!(f, "if {} {{\n{}\n}}", condition, body.join("\n"))?;
                for (cond, block) in elif_clauses {
                    let b: Vec<_> = block.iter().map(|s| format!("{}", s)).collect();
                    write!(f, " elif {} {{\n{}\n}}", cond, b.join("\n"))?;
                }
                if let Some(block) = else_block {
                    let b: Vec<_> = block.iter().map(|s| format!("{}", s)).collect();
                    write!(f, " else {{\n{}\n}}", b.join("\n"))?;
                }
                Ok(())
            }
            Stmt::While { condition, body } => {
                let b: Vec<_> = body.iter().map(|s| format!("{}", s)).collect();
                write!(f, "while {} {{\n{}\n}}", condition, b.join("\n"))
            }
            Stmt::ForIn {
                variable,
                second_variable,
                iterable,
                body,
            } => {
                let b: Vec<_> = body.iter().map(|s| format!("{}", s)).collect();
                match second_variable {
                    Some(v2) => write!(
                        f,
                        "for {}, {} in {} {{\n{}\n}}",
                        variable,
                        v2,
                        iterable,
                        b.join("\n")
                    ),
                    None => write!(
                        f,
                        "for {} in {} {{\n{}\n}}",
                        variable,
                        iterable,
                        b.join("\n")
                    ),
                }
            }
            Stmt::Break => write!(f, "break"),
            Stmt::Continue => write!(f, "continue"),
            Stmt::Return { values } => {
                if values.is_empty() {
                    write!(f, "return")
                } else {
                    let vs: Vec<_> = values.iter().map(|v| format!("{}", v)).collect();
                    write!(f, "return {}", vs.join(", "))
                }
            }
            Stmt::FnDecl {
                name,
                params,
                body,
                is_async,
            } => {
                let prefix = if *is_async { "async " } else { "" };
                let ps: Vec<_> = params.iter().map(|p| p.name.clone()).collect();
                let b: Vec<_> = body.iter().map(|s| format!("{}", s)).collect();
                write!(
                    f,
                    "{}fn {}({}) {{\n{}\n}}",
                    prefix,
                    name,
                    ps.join(", "),
                    b.join("\n")
                )
            }
            Stmt::ClassDecl {
                name,
                parent,
                methods,
                class_vars: _,
            } => {
                let parent_str = match parent {
                    Some(p) => format!(" < {}", p),
                    None => String::new(),
                };
                let ms: Vec<_> = methods.iter().map(|s| format!("{}", s)).collect();
                write!(f, "class {}{} {{\n{}\n}}", name, parent_str, ms.join("\n"))
            }
            Stmt::Defer { expr } => write!(f, "defer {}", expr),
            Stmt::Try {
                try_block,
                except_clauses,
                finally_block,
            } => {
                let tb: Vec<_> = try_block.iter().map(|s| format!("{}", s)).collect();
                write!(f, "try {{\n{}\n}}", tb.join("\n"))?;
                for clause in except_clauses {
                    let cb: Vec<_> = clause.body.iter().map(|s| format!("{}", s)).collect();
                    match (&clause.type_name, &clause.alias) {
                        (Some(t), Some(a)) => write!(
                            f,
                            " except {} as {} {{\n{}\n}}",
                            t.join("."),
                            a,
                            cb.join("\n")
                        )?,
                        (Some(t), None) => {
                            write!(f, " except {} {{\n{}\n}}", t.join("."), cb.join("\n"))?
                        }
                        (None, Some(a)) => write!(f, " except as {} {{\n{}\n}}", a, cb.join("\n"))?,
                        (None, None) => write!(f, " except {{\n{}\n}}", cb.join("\n"))?,
                    }
                }
                if let Some(block) = finally_block {
                    let fb: Vec<_> = block.iter().map(|s| format!("{}", s)).collect();
                    write!(f, " finally {{\n{}\n}}", fb.join("\n"))?;
                }
                Ok(())
            }
            Stmt::With {
                expression,
                alias,
                body,
            } => {
                let b: Vec<_> = body.iter().map(|s| format!("{}", s)).collect();
                match alias {
                    Some(a) => write!(f, "with {} as {} {{\n{}\n}}", expression, a, b.join("\n")),
                    None => write!(f, "with {} {{\n{}\n}}", expression, b.join("\n")),
                }
            }
            Stmt::Import {
                module_path, alias, ..
            } => {
                let path = module_path.join(".");
                match alias {
                    Some(a) => write!(f, "import {} as {}", path, a),
                    None => write!(f, "import {}", path),
                }
            }
            Stmt::FromImport {
                module_path,
                targets,
                ..
            } => {
                let path = module_path.join(".");
                let ts: Vec<_> = targets
                    .iter()
                    .map(|(name, alias)| match alias {
                        Some(a) => format!("{} as {}", name, a),
                        None => name.clone(),
                    })
                    .collect();
                write!(f, "from {} import {}", path, ts.join(", "))
            }
            Stmt::Throw { expr } => match expr {
                Some(e) => write!(f, "throw {}", e),
                None => write!(f, "throw"),
            },
            Stmt::Nonlocal { names } => write!(f, "nonlocal {}", names.join(", ")),
            Stmt::Global { names } => write!(f, "global {}", names.join(", ")),
        }
    }
}

impl std::fmt::Display for Program {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let stmts: Vec<_> = self.statements.iter().map(|s| format!("{}", s)).collect();
        write!(f, "{}", stmts.join("\n"))
    }
}

impl std::fmt::Display for Literal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Literal::Int(v) => write!(f, "{}", v),
            Literal::Float(v) => write!(f, "{}", v),
            Literal::String(v) => write!(f, "\"{}\"", v),
            Literal::Bool(v) => write!(f, "{}", if *v { "true" } else { "false" }),
            Literal::Nil => write!(f, "nil"),
        }
    }
}

impl std::fmt::Display for BinaryOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            BinaryOp::Add => "+",
            BinaryOp::Subtract => "-",
            BinaryOp::Multiply => "*",
            BinaryOp::Divide => "/",
            BinaryOp::FloorDiv => "//",
            BinaryOp::Modulo => "%",
            BinaryOp::Power => "**",
            BinaryOp::Equal => "==",
            BinaryOp::NotEqual => "!=",
            BinaryOp::Less => "<",
            BinaryOp::Greater => ">",
            BinaryOp::LessEqual => "<=",
            BinaryOp::GreaterEqual => ">=",
            BinaryOp::BitAnd => "&",
            BinaryOp::BitOr => "|",
            BinaryOp::BitXor => "^",
            BinaryOp::LeftShift => "<<",
            BinaryOp::RightShift => ">>",
            BinaryOp::And => "and",
            BinaryOp::Or => "or",
            BinaryOp::In => "in",
            BinaryOp::Is => "is",
        };
        write!(f, "{}", s)
    }
}

impl std::fmt::Display for UnaryOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UnaryOp::Negate => write!(f, "-"),
            UnaryOp::Not => write!(f, "not "),
            UnaryOp::BitNot => write!(f, "~"),
            UnaryOp::ChannelReceive => write!(f, "<-"),
        }
    }
}

impl std::fmt::Display for AssignOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            AssignOp::Assign => "=",
            AssignOp::PlusAssign => "+=",
            AssignOp::MinusAssign => "-=",
            AssignOp::StarAssign => "*=",
            AssignOp::SlashAssign => "/=",
            AssignOp::DoubleSlashAssign => "//=",
            AssignOp::PercentAssign => "%=",
            AssignOp::DoubleStarAssign => "**=",
            AssignOp::BitAndAssign => "&=",
            AssignOp::BitOrAssign => "|=",
            AssignOp::BitXorAssign => "^=",
            AssignOp::LeftShiftAssign => "<<=",
            AssignOp::RightShiftAssign => ">>=",
        };
        write!(f, "{}", s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_literal_expr() {
        let expr = Expr::Literal(Literal::Int(42));
        assert_eq!(format!("{}", expr), "42");
    }

    #[test]
    fn test_binary_expr() {
        let expr = Expr::Binary {
            left: Box::new(Expr::Literal(Literal::Int(1))),
            op: BinaryOp::Add,
            right: Box::new(Expr::Literal(Literal::Int(2))),
        };
        assert_eq!(format!("{}", expr), "(1 + 2)");
    }

    #[test]
    fn test_nested_binary() {
        let expr = Expr::Binary {
            left: Box::new(Expr::Binary {
                left: Box::new(Expr::Literal(Literal::Int(1))),
                op: BinaryOp::Add,
                right: Box::new(Expr::Literal(Literal::Int(2))),
            }),
            op: BinaryOp::Multiply,
            right: Box::new(Expr::Literal(Literal::Int(3))),
        };
        assert_eq!(format!("{}", expr), "((1 + 2) * 3)");
    }

    #[test]
    fn test_call_expr() {
        let expr = Expr::Call {
            callee: Box::new(Expr::Identifier("print".into())),
            args: vec![Expr::Literal(Literal::String("hello".into()))],
        };
        assert_eq!(format!("{}", expr), "print(\"hello\")");
    }

    #[test]
    fn test_list_literal() {
        let expr = Expr::ListLiteral {
            elements: vec![
                Expr::Literal(Literal::Int(1)),
                Expr::Literal(Literal::Int(2)),
            ],
        };
        assert_eq!(format!("{}", expr), "[1, 2]");
    }

    #[test]
    fn test_ternary() {
        let expr = Expr::Ternary {
            condition: Box::new(Expr::Identifier("ok".into())),
            then_expr: Box::new(Expr::Literal(Literal::String("yes".into()))),
            else_expr: Box::new(Expr::Literal(Literal::String("no".into()))),
        };
        assert_eq!(format!("{}", expr), "(\"yes\" if ok else \"no\")");
    }

    #[test]
    fn test_unary_not() {
        let expr = Expr::Unary {
            op: UnaryOp::Not,
            operand: Box::new(Expr::Identifier("flag".into())),
        };
        assert_eq!(format!("{}", expr), "(not flag)");
    }

    #[test]
    fn test_var_decl() {
        let stmt = Stmt::VarDecl {
            name: "x".into(),
            initializer: Expr::Literal(Literal::Int(42)),
        };
        assert_eq!(format!("{}", stmt), "var x = 42");
    }

    #[test]
    fn test_if_stmt() {
        let stmt = Stmt::If {
            condition: Expr::Identifier("x".into()),
            then_block: vec![Stmt::ExprStmt {
                expr: Expr::Call {
                    callee: Box::new(Expr::Identifier("print".into())),
                    args: vec![Expr::Literal(Literal::String("yes".into()))],
                },
            }],
            elif_clauses: vec![],
            else_block: None,
        };
        let s = format!("{}", stmt);
        assert!(s.starts_with("if x"));
        assert!(s.contains("print"));
    }

    #[test]
    fn test_fn_decl() {
        let stmt = Stmt::FnDecl {
            name: "add".into(),
            params: vec![
                Param {
                    name: "a".into(),
                    default: None,
                    is_variadic: false,
                },
                Param {
                    name: "b".into(),
                    default: None,
                    is_variadic: false,
                },
            ],
            body: vec![Stmt::Return {
                values: vec![Expr::Binary {
                    left: Box::new(Expr::Identifier("a".into())),
                    op: BinaryOp::Add,
                    right: Box::new(Expr::Identifier("b".into())),
                }],
            }],
            is_async: false,
        };
        let s = format!("{}", stmt);
        assert!(s.contains("fn add(a, b)"));
        assert!(s.contains("return (a + b)"));
    }

    #[test]
    fn test_for_in() {
        let stmt = Stmt::ForIn {
            variable: "i".into(),
            second_variable: None,
            iterable: Expr::Call {
                callee: Box::new(Expr::Identifier("range".into())),
                args: vec![Expr::Literal(Literal::Int(10))],
            },
            body: vec![Stmt::ExprStmt {
                expr: Expr::Call {
                    callee: Box::new(Expr::Identifier("print".into())),
                    args: vec![Expr::Identifier("i".into())],
                },
            }],
        };
        let s = format!("{}", stmt);
        assert!(s.contains("for i in"));
    }

    #[test]
    fn test_import() {
        let stmt = Stmt::Import {
            module_path: vec!["os".into(), "path".into()],
            alias: Some("pathutil".into()),
            is_stdlib: false,
        };
        assert_eq!(format!("{}", stmt), "import os.path as pathutil");
    }

    #[test]
    fn test_program() {
        let prog = Program {
            statements: vec![
                Stmt::VarDecl {
                    name: "x".into(),
                    initializer: Expr::Literal(Literal::Int(1)),
                },
                Stmt::VarDecl {
                    name: "y".into(),
                    initializer: Expr::Literal(Literal::Int(2)),
                },
            ],
        };
        let s = format!("{}", prog);
        assert!(s.contains("var x = 1"));
        assert!(s.contains("var y = 2"));
    }
}
