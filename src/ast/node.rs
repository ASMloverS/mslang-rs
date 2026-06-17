// AST 表达式节点定义（task 09）。
// 完整规格见 docs/mslang/tasks/09-ast-expression-nodes.md。

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

// 前向声明：FnLiteral 的函数体为语句序列。完整 Stmt 枚举由 task 10（AST 语句节点）定义；
// 此占位仅保证 task 09 可独立编译，task 10 将替换为完整变体集。
#[derive(Debug, Clone)]
pub enum Stmt {
    Placeholder,
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
}
