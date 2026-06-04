# select 语句（多 channel 复用）

## 所属阶段
Phase 7.4 - 并发

## 前置任务
54-channel, 53-async-await

## 目标

实现 `select` 语句，支持同时等待多个 channel 操作，任一分支就绪即执行。

## 设计规格

参照 [08-concurrency](../08-concurrency.md) § select：

### 语法

```
select_stmt = "select" "{" select_case+ ("default" block)? "}"
select_case = "case" channel_op block
channel_op  = IDENTIFIER "=" "<-" IDENTIFIER    // 接收
            | IDENTIFIER "<-" expression          // 发送
```

`select`、`case`、`default` 为保留字（见 [01-lexical](../01-lexical.md) § 保留字），不可用作变量名。

### 语义

- 多个 `case` 分支同时就绪时，**随机选择一个**执行（避免饥饿）
- `default` 分支在所有 channel 操作均未就绪时立即执行（非阻塞）
- 无 `default` 时，`select` 阻塞直到某个 case 就绪
- 空 `select {}`（无任何 case）永久阻塞当前协程

## 实现细节

### 文件位置

- `src/ast/node.rs` — AST 节点
- `src/parser/advanced_statement.rs` — 解析
- `src/compiler/statement.rs` — 编译
- `src/vm/mod.rs` — VM 执行
- `src/vm/select.rs` — select 实现

### AST 节点

```rust
#[derive(Debug, Clone)]
pub struct SelectCase {
    pub operation: SelectOp,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone)]
pub enum SelectOp {
    Receive {
        channel: String,
        target: String,
    },
    Send {
        channel: String,
        value: Expr,
    },
}
```

在 `Stmt` 枚举中添加：

```rust
Select {
    cases: Vec<SelectCase>,
    default_block: Option<Vec<Stmt>>,
},
```

### 解析

`select` 为保留字，词法分析器返回 `TokenKind::Identifier("select")` 时需检查保留字列表并报错。在 `parse_statement()` 中，当遇到保留字 `select` 时调用 `parse_select()`：

```rust
fn parse_select(&mut self) -> Result<Stmt> {
    self.advance(); // consume 'select'
    self.expect(TokenKind::LeftBrace, "expected '{' after 'select'")?;
    self.skip_newlines();

    let mut cases = Vec::new();
    let mut default_block = None;

    while !self.check(&TokenKind::RightBrace) {
        let tok = self.peek();
        match &tok.kind {
            TokenKind::Identifier(name) if name == "case" => {
                self.advance();
                let op = self.parse_select_op()?;
                let body = self.parse_block()?;
                cases.push(SelectCase { operation: op, body });
            }
            TokenKind::Identifier(name) if name == "default" => {
                self.advance();
                default_block = Some(self.parse_block()?);
            }
            _ => {
                return Err(MspError::ParseError {
                    line: tok.span.start.line,
                    column: tok.span.start.column,
                    message: "expected 'case' or 'default' in select".into(),
                });
            }
        }
        self.skip_newlines();
    }

    self.expect(TokenKind::RightBrace, "expected '}' after select")?;
    Ok(Stmt::Select { cases, default_block })
}

fn parse_select_op(&mut self) -> Result<SelectOp> {
    let name = self.expect_identifier("expected identifier in case")?;

    if self.match_token(&[TokenKind::Equal]) {
        self.expect(TokenKind::LeftArrow, "expected '<-' in receive case")?;
        let channel = self.expect_identifier("expected channel name")?;
        Ok(SelectOp::Receive { channel, target: name })
    } else if self.match_token(&[TokenKind::LeftArrow]) {
        let value = self.parse_expression()?;
        Ok(SelectOp::Send { channel: name, value })
    } else {
        Err(MspError::ParseError {
            line: self.previous().span.start.line,
            column: self.previous().span.start.column,
            message: "expected '=' or '<-' in select case".into(),
        })
    }
}
```

### 编译

编译 `select` 语句的核心逻辑：

1. 评估所有 channel 和发送值
2. 非阻塞尝试每个操作（try_receive / try_send）
3. 如果有就绪分支，随机选择一个执行
4. 如果都未就绪且有 `default`，执行 default 块
5. 如果都未就绪且无 `default`，注册所有 channel 的唤醒回调，暂停协程

### VM SELECT 指令

| OpCode | 操作数 | 说明 |
|---|---|---|
| `SELECT` | `case_count(1)` | 执行 select 语义 |

```rust
OpCode::Select => {
    let case_count = self.read_byte() as usize;
    let has_default = self.read_byte() != 0;

    let mut ready_cases = Vec::new();
    for i in 0..case_count {
        // 非阻塞尝试每个 case 的 channel 操作
        // ready_cases.push(i) if operation can proceed immediately
    }

    if !ready_cases.is_empty() {
        // 随机选择一个就绪分支
        let chosen = ready_cases[rand::random::<usize>() % ready_cases.len()];
        // 跳转到对应分支的字节码偏移
    } else if has_default {
        // 跳转到 default 分支
    } else {
        // 暂停协程，等待任一 channel 就绪
    }
}
```

## 验证标准

1. 单 case 接收正确执行
2. 多 case 同时就绪时随机选择
3. `default` 在无就绪分支时立即执行
4. 无 `default` 时阻塞直到有分支就绪
5. 空 `select {}` 永久阻塞

## 测试用例

```ms
ch1 = channel(1)
ch2 = channel(1)

go fn() {
    ch1 <- "hello"
}()

select {
    case val = <-ch1 {
        print("from ch1: " + val)
    }
    case ch2 <- data {
        print("sent to ch2")
    }
    default {
        print("no activity")
    }
}
```

预期输出：`from ch1: hello`
