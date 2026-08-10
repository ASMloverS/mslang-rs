# REPL 交互式命令行

## 所属阶段
Phase 8.1 - REPL + 工具链

## 前置任务
55-go-concurrency

## 目标
实现交互式 REPL（Read-Eval-Print Loop），支持多行输入、表达式求值、上下文持久化和模块导入。

## 设计规格

参照 [09-modules](../09-modules.md) § REPL、[12-implementation-plan](../12-implementation-plan.md) § 8.1 REPL：

### REPL 功能

- 交互式 read-eval-print 循环
- 多行输入支持（检测不完整的块）
- 表达式求值：打印表达式结果
- 上下文持久化：变量和函数在多次输入间保持
- 模块导入支持
- 行编辑支持（历史记录、自动补全等）

## 实现细节

### 1. REPL 模块

`src/repl/mod.rs`：

```rust
pub struct Repl {
    vm: VM,
    editor: Editor<ReplHelper>,
    buffer: String,
}

impl Repl {
    pub fn new() -> Result<Self> {
        let vm = VM::new();
        let mut editor = Editor::new()?;
        editor.set_helper(Some(ReplHelper));
        Ok(Self { vm, editor, buffer: String::new() })
    }

    pub fn run(&mut self) -> Result<()> {
        println!("mslang 0.1.0 REPL");
        println!("Type :quit to exit");

        loop {
            let prompt = if self.buffer.is_empty() { "> " } else { ". " };
            let line = match self.editor.readline(prompt) {
                Ok(line) => line,
                Err(ReadlineError::Interrupted) => {
                    // Ctrl+C：取消当前输入，清空 buffer 继续读
                    self.buffer.clear();
                    continue;
                }
                Err(ReadlineError::Eof) => break,  // Ctrl+D：退出 REPL
                Err(e) => return Err(e),
            };

            if line == ":quit" { break; }

            self.buffer.push_str(&line);
            self.buffer.push('\n');

            if self.is_complete() {
                self.evaluate_buffer()?;
                self.buffer.clear();
            }
        }
        Ok(())
    }
}
```

### 2. 多行输入检测

检测输入是否为完整的语句块：

```rust
fn is_complete(&self) -> bool {
    // 防止无限追加（用户持续输入未闭合块）
    const MAX_BUFFER: usize = 64 * 1024;
    if self.buffer.len() > MAX_BUFFER {
        return true;  // 超限，交给执行阶段报告
    }

    let source = &self.buffer;

    // 先词法分析，再语法分析（Parser 接收 Vec<Token>，非原始字符串）
    let tokens = match Lexer::new(source).tokenize_all() {
        Ok(tokens) => tokens,
        Err(e) if e.is_unterminated() => return false,  // 未终结字符串/注释，继续读
        Err(_) => return true,  // 其他词法错误，交给执行阶段报告
    };

    match Parser::new(tokens).parse() {
        Ok(_) => true,
        Err(e) if e.is_unexpected_eof() => false,  // 不完整，继续读
        Err(_) => true,  // 其他语法错误，交给执行阶段报告
    }
}
```

判断规则：
- 花括号不匹配 → 继续读
- 末尾有 `\` 或 `{` → 继续读
- 解析器报告 unexpected EOF → 继续读
- 解析成功 → 执行

### 3. 表达式求值

区分语句和表达式：

```rust
fn evaluate_buffer(&mut self) -> Result<()> {
    let source = self.buffer.clone();  // clone 避免 &self.buffer 与 &mut self.vm 借用冲突

    // 尝试作为表达式求值（顶层为裸表达式的输入打印结果）
    if Self::is_expression(&source) {
        match self.vm.eval_expression(&source) {
            Ok(val) => {
                println!("{}", val.display());
                self.editor.add_history_entry(&source);
            }
            Err(e) => self.print_error(&e),
        }
    } else {
        // 作为语句执行（不打印返回值）
        match self.vm.exec(&source) {
            Ok(_) => {
                self.editor.add_history_entry(&source);
            }
            Err(e) => self.print_error(&e),
        }
    }
    Ok(())
}

/// 判断源码顶层是否为裸表达式（而非语句）。
/// 解析后若 Program 仅含单个 ExprStmt，则按表达式求值；
/// 否则按语句执行（var/const/赋值/fn/class/import/defer/try/with/
/// if/while/for/return/break/continue/throw/global/nonlocal/async 等）。
fn is_expression(source: &str) -> bool {
    let tokens = match Lexer::new(source).tokenize_all() {
        Ok(t) => t,
        Err(_) => return false,
    };
    match Parser::new(tokens).parse() {
        Ok(program) => program.statements.len() == 1
            && matches!(program.statements[0], Stmt::Expr(_)),
        Err(_) => false,
    }
}

/// 格式化输出错误，不退出 REPL。
/// 错误格式参照 tasks/57-error-messages.md（行号标注、高亮、堆栈跟踪）。
fn print_error(&self, e: &Error) {
    eprintln!("Error: {}", e);
}
```

表达式 vs 语句判断：
- 解析后若 Program 仅含单个 `ExprStmt`，按表达式求值并打印结果
- 其余一律按语句执行：`var/const/赋值/fn/class/import/defer/try/with/if/while/for/return/break/continue/throw/global/nonlocal/async` 等
- 表达式结果使用 `display()` 格式化输出（字符串带引号）

### 4. 上下文持久化

REPL 使用持久化的 VM 实例（`evaluate_buffer` 定义见上文 §3）：

- `self.vm` 在整个 REPL 生命周期内保持
- 全局变量、函数、类定义在多次输入间共享
- import 的模块被缓存，后续输入可直接使用
- `vm.globals` 和 `vm.module_resolver.cache` 持久化，每次输入在同一个 VM 上执行

### 5. 行编辑（rustyline）

`Cargo.toml` 添加依赖：

```toml
[dependencies]
rustyline = "14"
```

功能：
- 上下箭头浏览历史
- Tab 自动补全（变量名、关键字）
- 行内编辑
- `Ctrl+C` 取消当前输入
- `Ctrl+D` 退出 REPL

```rust
use rustyline::Editor;
use rustyline::error::ReadlineError;
use rustyline::hint::HistoryHinter;
use rustyline::completion::Completer;
use rustyline::highlight::Highlighter;
use rustyline::validate::Validator;
use rustyline::Helper;

struct ReplHelper;

// Helper trait 要求 Completer + Hinter + Highlighter + Validator 四者齐全
impl Completer for ReplHelper {
    type Candidate = String;
    // Tab 补全：补全 vm.globals 的键与关键字表（完整实现可在后续迭代扩展）
}

impl Hinter for ReplHelper {
    type Hint = String;
    fn hint(&self, line: &str, pos: usize, ctx: &Context) -> Option<String> {
        HistoryHinter.hint(line, pos, ctx)
    }
}

impl Highlighter for ReplHelper {}
impl Validator for ReplHelper {}
impl Helper for ReplHelper {}
```

### 6. VM 适配

VM 需要新增两个方法用于 REPL：

```rust
impl VM {
    /// 执行语句（不返回值）
    pub fn exec(&mut self, source: &str) -> Result<()>;

    /// 求值表达式（返回值）
    pub fn eval_expression(&mut self, source: &str) -> Result<Object>;
}
```

- `exec`：编译为顶层语句，执行但不打印
- `eval_expression`：编译为表达式，执行并返回结果

> **依赖说明**：`eval_expression` 要求 Compiler 支持"表达式编译模式"（将单个表达式编译为结果压栈的字节码，而非执行后丢弃结果）。若 Phase 2 的 Compiler 仅有语句编译入口，需额外新增 `Compiler::compile_expression`，此为隐性前置依赖。

## 验证标准

1. 单行表达式正确求值并打印结果
2. 多行块（fn、if、for）正确处理
3. 变量和函数在多次输入间保持
4. import 语句在 REPL 中正确工作
5. 语法错误不退出 REPL，显示错误后继续
6. `:quit` 命令正常退出
7. 历史记录可通过上下箭头浏览

## 测试用例

### 手动验证场景

```
> x = 10
> y = 20
> x + y
30
> fn greet(name) { return "Hello, " + name }
> greet("World")
"Hello, World"
```

### 多行输入

```
> fn add(a, b) {
.     return a + b
. }
> add(3, 4)
7
```

### 上下文持久化

```
> counter = 0
> fn inc() { counter += 1; return counter }
> inc()
1
> inc()
2
> counter
2
```

### 模块导入

```
> import math
> math.sqrt(16)
4.0
> math.pi
3.141592653589793
```

### 错误处理

```
> 10 / 0
Error: division by zero
> x = 10
> print(x)
10
```

### 自动化测试（Rust 级别）

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn test_repl_expression() {
        let mut repl = Repl::new();
        let result = repl.vm.eval_expression("1 + 2").unwrap();
        assert_eq!(result, Object::Int(3));
    }

    #[test]
    fn test_repl_persistence() {
        let mut repl = Repl::new();
        repl.vm.exec("x = 42").unwrap();
        let result = repl.vm.eval_expression("x").unwrap();
        assert_eq!(result, Object::Int(42));
    }

    #[test]
    fn test_repl_multiline() {
        let mut repl = Repl::new().unwrap();
        // 输入未闭合的 fn 块 → 不完整
        repl.buffer = "fn add(a, b) {".to_string();
        assert!(!repl.is_complete());
        // 闭合后 → 完整
        repl.buffer = "fn add(a, b) {\n    return a + b\n}".to_string();
        assert!(repl.is_complete());
    }
}
```
