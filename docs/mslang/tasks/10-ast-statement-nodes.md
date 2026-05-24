# AST 语句节点定义

## 所属阶段
Phase 1.4b - 基础设施

## 前置任务
09-ast-expression-nodes

## 目标
定义完整的 AST 语句节点和程序顶层节点，覆盖 [03-syntax](../03-syntax.md) 中所有语句类型。

## 设计规格

参照 [03-syntax](../03-syntax.md) § 语句：

### 语句类型

- 变量声明：`var x = expr`
- 短变量声明：`x := expr`
- 常量声明：`const NAME = expr`
- 赋值语句（含复合赋值）：`x += expr`
- 多目标赋值：`a, b = 1, 2`
- 表达式语句
- 块语句：`{ stmts }`
- if / elif / else
- while 循环
- for..in 循环（单变量和双变量）
- break / continue
- return（含多返回值）
- 函数声明
- class 声明
- defer 语句
- try / except / finally
- with 语句
- import 语句
- throw 语句

### 程序结构

参照 [03-syntax](../03-syntax.md) § 程序结构：

```
program = statement*
```

## 实现细节

### 文件位置

`src/ast/node.rs`（与表达式节点同文件，或 `src/ast/statement.rs` 单独文件）

### Stmt 枚举

```rust
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
    MultiAssign {
        targets: Vec<Expr>,
        values: Vec<Expr>,
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
    },
    FromImport {
        module_path: Vec<String>,
        targets: Vec<(String, Option<String>)>,
    },
    Throw {
        expr: Expr,
    },
}
```

### ExceptClause 结构体

```rust
#[derive(Debug, Clone)]
pub struct ExceptClause {
    pub type_name: Option<String>,
    pub alias: Option<String>,
    pub body: Vec<Stmt>,
}
```

### Program 结构体

```rust
#[derive(Debug, Clone)]
pub struct Program {
    pub statements: Vec<Stmt>,
}
```

### Display 实现

```rust
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
            Stmt::MultiAssign { targets, values } => {
                let ts: Vec<_> = targets.iter().map(|t| format!("{}", t)).collect();
                let vs: Vec<_> = values.iter().map(|v| format!("{}", v)).collect();
                write!(f, "{} = {}", ts.join(", "), vs.join(", "))
            }
            Stmt::ExprStmt { expr } => write!(f, "{}", expr),
            Stmt::Block { statements } => {
                let stmts: Vec<_> = statements.iter().map(|s| format!("{}", s)).collect();
                write!(f, "{{\n{}\n}}", stmts.join("\n"))
            }
            Stmt::If { condition, then_block, elif_clauses, else_block } => {
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
            Stmt::ForIn { variable, second_variable, iterable, body } => {
                let b: Vec<_> = body.iter().map(|s| format!("{}", s)).collect();
                match second_variable {
                    Some(v2) => write!(f, "for {}, {} in {} {{\n{}\n}}", variable, v2, iterable, b.join("\n")),
                    None => write!(f, "for {} in {} {{\n{}\n}}", variable, iterable, b.join("\n")),
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
            Stmt::FnDecl { name, params, body } => {
                let ps: Vec<_> = params.iter().map(|p| p.name.clone()).collect();
                let b: Vec<_> = body.iter().map(|s| format!("{}", s)).collect();
                write!(f, "fn {}({}) {{\n{}\n}}", name, ps.join(", "), b.join("\n"))
            }
            Stmt::ClassDecl { name, parent, methods, class_vars: _ } => {
                let parent_str = match parent {
                    Some(p) => format!(" < {}", p),
                    None => String::new(),
                };
                let ms: Vec<_> = methods.iter().map(|s| format!("{}", s)).collect();
                write!(f, "class {}{} {{\n{}\n}}", name, parent_str, ms.join("\n"))
            }
            Stmt::Defer { expr } => write!(f, "defer {}", expr),
            Stmt::Try { try_block, except_clauses, finally_block } => {
                let tb: Vec<_> = try_block.iter().map(|s| format!("{}", s)).collect();
                write!(f, "try {{\n{}\n}}", tb.join("\n"))?;
                for clause in except_clauses {
                    let cb: Vec<_> = clause.body.iter().map(|s| format!("{}", s)).collect();
                    match (&clause.type_name, &clause.alias) {
                        (Some(t), Some(a)) => write!(f, " except {} as {} {{\n{}\n}}", t, a, cb.join("\n"))?,
                        (Some(t), None) => write!(f, " except {} {{\n{}\n}}", t, cb.join("\n"))?,
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
            Stmt::With { expression, alias, body } => {
                let b: Vec<_> = body.iter().map(|s| format!("{}", s)).collect();
                match alias {
                    Some(a) => write!(f, "with {} as {} {{\n{}\n}}", expression, a, b.join("\n")),
                    None => write!(f, "with {} {{\n{}\n}}", expression, b.join("\n")),
                }
            }
            Stmt::Import { module_path, alias } => {
                let path = module_path.join(".");
                match alias {
                    Some(a) => write!(f, "import {} as {}", path, a),
                    None => write!(f, "import {}", path),
                }
            }
            Stmt::FromImport { module_path, targets } => {
                let path = module_path.join(".");
                let ts: Vec<_> = targets.iter()
                    .map(|(name, alias)| match alias {
                        Some(a) => format!("{} as {}", name, a),
                        None => name.clone(),
                    })
                    .collect();
                write!(f, "from {} import {}", path, ts.join(", "))
            }
            Stmt::Throw { expr } => write!(f, "throw {}", expr),
        }
    }
}

impl std::fmt::Display for Program {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let stmts: Vec<_> = self.statements.iter().map(|s| format!("{}", s)).collect();
        write!(f, "{}", stmts.join("\n"))
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
```

## 验证标准

1. `cargo build` 编译通过
2. Stmt 枚举覆盖 [03-syntax](../03-syntax.md) 所有语句类型
3. Display 可正确输出语句
4. Program 作为顶层节点正确聚合

## 测试用例

无 `.ms` 测试。Rust 单元测试：

```rust
#[cfg(test)]
mod tests {
    use super::*;

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
            then_block: vec![Stmt::ExprStmt { expr: Expr::Call {
                callee: Box::new(Expr::Identifier("print".into())),
                args: vec![Expr::Literal(Literal::String("yes".into()))],
            }}],
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
                Param { name: "a".into(), default: None, is_variadic: false },
                Param { name: "b".into(), default: None, is_variadic: false },
            ],
            body: vec![Stmt::Return {
                values: vec![Expr::Binary {
                    left: Box::new(Expr::Identifier("a".into())),
                    op: BinaryOp::Add,
                    right: Box::new(Expr::Identifier("b".into())),
                }],
            }],
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
            body: vec![Stmt::ExprStmt { expr: Expr::Call {
                callee: Box::new(Expr::Identifier("print".into())),
                args: vec![Expr::Identifier("i".into())],
            }}],
        };
        let s = format!("{}", stmt);
        assert!(s.contains("for i in"));
    }

    #[test]
    fn test_import() {
        let stmt = Stmt::Import {
            module_path: vec!["os".into(), "path".into()],
            alias: Some("pathutil".into()),
        };
        assert_eq!(format!("{}", stmt), "import os.path as pathutil");
    }

    #[test]
    fn test_program() {
        let prog = Program {
            statements: vec![
                Stmt::VarDecl { name: "x".into(), initializer: Expr::Literal(Literal::Int(1)) },
                Stmt::VarDecl { name: "y".into(), initializer: Expr::Literal(Literal::Int(2)) },
            ],
        };
        let s = format!("{}", prog);
        assert!(s.contains("var x = 1"));
        assert!(s.contains("var y = 2"));
    }
}
```
