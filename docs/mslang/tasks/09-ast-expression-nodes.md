# AST 表达式节点定义

## 所属阶段
Phase 1.4a - 基础设施

## 前置任务
02-token-definition

## 目标
定义完整的 AST 表达式节点，覆盖 [03-syntax](../03-syntax.md) 中所有表达式类型。

## 设计规格

参照 [03-syntax](../03-syntax.md) § 表达式：

### 运算符优先级（15 级）

| 优先级 | 运算符 | 结合性 |
|---|---|---|
| 1（最低） | `=` `+=` `-=` 等 | 右 |
| 2 | `if...else`（三元） | 右 |
| 3 | `or` | 左 |
| 4 | `and` | 左 |
| 5 | `not` | 右（一元） |
| 6 | `== != < > <= >= in is` | 左 |
| 7 | `\|` | 左 |
| 8 | `^` | 左 |
| 9 | `&` | 左 |
| 10 | `<< >>` | 左 |
| 11 | `+ -` | 左 |
| 12 | `* / // %` | 左 |
| 13 | `- ~ <-`（一元） | 右 |
| 14 | `**` | 右 |
| 15（最高） | `() [] .`（后缀） | 左 |

### 表达式类型清单

来自 [03-syntax](../03-syntax.md) § 初等表达式 及各优先级层：

- 字面量：Int, Float, String, Bool, Nil
- 标识符引用
- 二元运算
- 一元运算（前缀：`-`, `not`, `~`, `<-`）
- 赋值表达式
- 函数调用
- 下标访问
- 属性访问（`.`）
- 切片
- 三元表达式（`if...else`）
- 列表字面量
- Dict 字面量
- Set 字面量
- 元组字面量
- 匿名函数
- 推导式（列表、Dict、Set，支持嵌套 for 子句）
- 生成器表达式
- super 访问
- yield / yield from
- await
- go 表达式

## 实现细节

### 文件位置

`src/ast/node.rs`

### Expression 枚举

```rust
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
```

### Literal 枚举

```rust
#[derive(Debug, Clone)]
pub enum Literal {
    Int(i64),
    Float(f64),
    String(String),
    Bool(bool),
    Nil,
}
```

### BinaryOp 枚举

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinaryOp {
    Add, Sub, Mul, Div, FloorDiv, Mod, Power,
    Equal, NotEqual, Less, Greater, LessEqual, GreaterEqual,
    BitAnd, BitOr, BitXor, LeftShift, RightShift,
    And, Or,
    In, Is,
}
```

### UnaryOp 枚举

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnaryOp {
    Negate,
    Not,
    BitNot,
    ChannelReceive,
}
```

### AssignOp 枚举

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AssignOp {
    Assign,
    PlusAssign, MinusAssign, StarAssign, SlashAssign,
    DoubleSlashAssign, PercentAssign, DoubleStarAssign,
    BitAndAssign, BitOrAssign, BitXorAssign,
    LeftShiftAssign, RightShiftAssign,
}
```

### Param 结构体

```rust
#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub default: Option<Expr>,
    pub is_variadic: bool,
}
```

### ForClause 结构体

推导式中支持嵌套 for 子句：

```rust
#[derive(Debug, Clone)]
pub struct ForClause {
    pub targets: Vec<String>,
    pub iterable: Box<Expr>,
}
```

### Display 实现

```rust
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
            Expr::Ternary { condition, then_expr, else_expr } => {
                write!(f, "({} if {} else {})", then_expr, condition, else_expr)
            }
            Expr::ListLiteral { elements } => {
                let els: Vec<_> = elements.iter().map(|e| format!("{}", e)).collect();
                write!(f, "[{}]", els.join(", "))
            }
            Expr::DictLiteral { pairs } => {
                let ps: Vec<_> = pairs.iter()
                    .map(|(k, v)| format!("{}: {}", k, v))
                    .collect();
                write!(f, "{{{}}}", ps.join(", "))
            }
            Expr::Await { expr } => write!(f, "await {}", expr),
            Expr::Yield { value } => {
                match value {
                    Some(v) => write!(f, "yield {}", v),
                    None => write!(f, "yield"),
                }
            }
            Expr::YieldFrom { iterable } => write!(f, "yield from {}", iterable),
            Expr::Go { expr } => write!(f, "go {}", expr),
            Expr::Grouping { expr } => write!(f, "({})", expr),
            // ... 其他变体
            _ => write!(f, "<expr>"),
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
            BinaryOp::Add => "+", BinaryOp::Sub => "-", BinaryOp::Mul => "*",
            BinaryOp::Div => "/", BinaryOp::FloorDiv => "//", BinaryOp::Mod => "%",
            BinaryOp::Power => "**", BinaryOp::Equal => "==", BinaryOp::NotEqual => "!=",
            BinaryOp::Less => "<", BinaryOp::Greater => ">",
            BinaryOp::LessEqual => "<=", BinaryOp::GreaterEqual => ">=",
            BinaryOp::BitAnd => "&", BinaryOp::BitOr => "|", BinaryOp::BitXor => "^",
            BinaryOp::LeftShift => "<<", BinaryOp::RightShift => ">>",
            BinaryOp::And => "and", BinaryOp::Or => "or",
            BinaryOp::In => "in", BinaryOp::Is => "is",
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
```

## 验证标准

1. `cargo build` 编译通过
2. Expression 枚举覆盖 [03-syntax](../03-syntax.md) 所有表达式类型
3. Display 实现可正确输出 AST
4. 递归类型使用 `Box<T>` 避免无限大小

## 测试用例

无 `.ms` 测试。Rust 单元测试：

```rust
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
            op: BinaryOp::Mul,
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
```
