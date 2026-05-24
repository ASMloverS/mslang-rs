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
            let line = self.editor.readline(prompt)?;

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
    let source = &self.buffer;

    // 尝试解析
    match Parser::parse(source) {
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
    let source = &self.buffer;

    // 尝试作为表达式求值
    if self.is_expression(source) {
        match self.vm.eval_expression(source) {
            Ok(val) => {
                println!("{}", val.display());
                self.editor.add_history_entry(source);
            }
            Err(e) => self.print_error(&e),
        }
    } else {
        // 作为语句执行
        match self.vm.exec(source) {
            Ok(_) => {
                self.editor.add_history_entry(source);
            }
            Err(e) => self.print_error(&e),
        }
    }
    Ok(())
}
```

表达式 vs 语句判断：
- 如果顶层节点是表达式（不是 var/const/fn/class/if/while/for/import 等），按表达式处理
- 表达式结果使用 `display()` 格式化输出（字符串带引号）

### 4. 上下文持久化

REPL 使用持久化的 VM 实例：

- `self.vm` 在整个 REPL 生命周期内保持
- 全局变量、函数、类定义在多次输入间共享
- import 的模块被缓存，后续输入可直接使用

```rust
fn evaluate_buffer(&mut self) -> Result<()> {
    // vm.globals 和 vm.module_resolver.cache 持久化
    // 每次输入在同一个 VM 上执行
}
```

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
use rustyline::hint::HistoryHinter;

struct ReplHelper;

impl Hinter for ReplHelper {
    type Hint = String;
    fn hint(&self, line: &str, pos: usize, ctx: &Context) -> Option<String> {
        HistoryHinter.hint(line, pos, ctx)
    }
}
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
        let input = "fn add(a, b) {\n    return a + b\n}";
        assert!(repl.is_complete(input));
    }
}
```
