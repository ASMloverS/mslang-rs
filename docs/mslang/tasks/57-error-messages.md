# 友好错误信息与堆栈跟踪

## 所属阶段
Phase 8.2 - REPL + 工具链

## 前置任务
37-try-except-finally

## 目标
实现清晰友好的错误信息输出，包括行号标注、源码行显示、错误高亮和格式化堆栈跟踪。

## 设计规格

参照 [11-bytecode-vm](../11-bytecode-vm.md) § 调试信息 / 堆栈跟踪格式：

### 行号表

编译单元中维护行号映射：

```
lines: Vec<(instruction_offset, source_line)>
```

### 堆栈跟踪格式

```
Error: division by zero
    at divmod (math.ms:5)
    at calculate (main.ms:12)
    at <main> (main.ms:20)
```

### 错误类型

#### 编译时错误

- `unexpected token`：意外的词法单元
- `undefined variable`：未定义的变量
- 语法错误：缺少括号、分号等

#### 运行时错误

- 类型错误：操作数类型不匹配
- 索引越界：list/dict 索引超出范围
- 除零错误：除数为零
- 属性错误：对象没有该属性
- 未定义变量：运行时访问不存在的变量

## 实现细节

### 1. 行号表构建

`src/compiler/mod.rs`：

编译时在每个指令前记录行号：

```rust
fn emit_byte(&mut self, byte: u8) {
    let line = self.current_line();
    self.unit.code.push(byte);
    if self.unit.lines.last().map(|&(off, _)| off) != Some(self.unit.code.len() - 1) {
        self.unit.lines.push((self.unit.code.len() - 1, line));
    }
}
```

行号表格式优化：使用 RLE（行程编码）压缩连续相同行号。

### 2. 行号查询

```rust
impl CompilationUnit {
    fn get_line(&self, offset: usize) -> usize {
        for i in (0..self.lines.len()).rev() {
            if self.lines[i].0 <= offset {
                return self.lines[i].1;
            }
        }
        0
    }
}
```

### 3. 错误对象增强

`src/error.rs` 新增 `SourceLocation` 和 `StackTrace`：

```rust
struct SourceLocation {
    file: String,
    line: usize,
    column: Option<usize>,
}

struct StackTrace {
    frames: Vec<StackTraceFrame>,
}

struct StackTraceFrame {
    function_name: String,
    location: SourceLocation,
}
```

### 4. RuntimeError 增强

```rust
pub struct RuntimeError {
    message: String,
    stack_trace: StackTrace,
    source_line: Option<String>,
    location: Option<SourceLocation>,
}
```

- `message`：错误描述
- `stack_trace`：调用堆栈
- `source_line`：出错的源码行内容
- `location`：文件名和行号

### 5. 堆栈跟踪构建

在 VM 抛出运行时错误时，遍历 `call_stack` 构建堆栈跟踪：

```rust
fn build_stack_trace(&self) -> StackTrace {
    let mut frames = Vec::new();

    for frame in self.call_stack.iter().rev() {
        let ip = frame.ip;
        let line = frame.closure.function.unit.get_line(ip);
        let name = frame.closure.function.name.clone();
        let file = frame.closure.function.source_file.clone()
            .unwrap_or_else(|| "<script>".into());
        frames.push(StackTraceFrame {
            function_name: name,
            location: SourceLocation {
                file,
                line,
                column: None,
            },
        });
    }

    StackTrace { frames }
}
```

### 6. 错误显示格式化

```rust
impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        // 错误标题
        writeln!(f, "Error: {}", self.message)?;

        // 源码行和错误指示
        if let Some(ref loc) = self.location {
            if let Some(ref source_line) = self.source_line {
                writeln!(f, "  --> {}:{}:{}", loc.file, loc.line, loc.column.unwrap_or(0))?;
                writeln!(f, "   |")?;
                writeln!(f, "{:3}| {}", loc.line, source_line)?;
                writeln!(f, "   | {}^", " ".repeat(loc.column.unwrap_or(0)))?;
            }
        }

        // 堆栈跟踪
        if !self.stack_trace.frames.is_empty() {
            writeln!(f, "Stack trace:")?;
            for frame in &self.stack_trace.frames {
                writeln!(
                    f,
                    "    at {} ({}:{})",
                    frame.function_name,
                    frame.location.file,
                    frame.location.line
                )?;
            }
        }

        Ok(())
    }
}
```

### 7. 编译时错误格式化

```rust
impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        writeln!(f, "Compile error: {}", self.message)?;
        if let Some(ref loc) = self.location {
            writeln!(f, "  --> {}:{}:{}", loc.file, loc.line, loc.column)?;
            if let Some(ref source_line) = self.source_line {
                writeln!(f, "   |")?;
                writeln!(f, "{:3}| {}", loc.line, source_line)?;
                writeln!(f, "   | {}^--- here", " ".repeat(loc.column.saturating_sub(1)))?;
            }
        }
        Ok(())
    }
}
```

### 8. 颜色输出（可选）

使用 `termcolor` 或 `colored` crate：

```rust
// Cargo.toml 可选依赖
[dependencies]
colored = { version = "2", optional = true }

[features]
color = ["colored"]
```

- 红色：错误信息
- 黄色：警告
- 蓝色：文件名和行号
- 绿色：行号标尺

### 9. 源码行读取

运行时需要能读取源文件来显示出错行：

```rust
fn read_source_line(file: &str, line: usize) -> Option<String> {
    let content = std::fs::read_to_string(file).ok()?;
    content.lines().nth(line - 1).map(|s| s.to_string())
}
```

- 缓存已读取的源文件，避免重复 IO
- 对于 REPL 输入，直接从输入历史中获取

## 验证标准

1. 运行时错误显示文件名和行号
2. 堆栈跟踪正确显示调用链
3. 源码行在错误信息中正确显示
4. 编译时错误和运行时错误格式一致
5. 多文件错误正确显示各文件中的位置
6. 错误信息不包含内部实现细节（如指令偏移量）

## 测试用例

### test_error_divzero.ms

```ms
x = 10 / 0
```

预期错误输出：
```
Error: division by zero
  --> test_error_divzero.ms:1:9
   |
  1| x = 10 / 0
   |         ^
```

### test_error_stack_trace.ms

```ms
fn foo() {
    bar()
}

fn bar() {
    throw RuntimeError("oops")
}

foo()
```

预期错误输出：
```
Error: oops
Stack trace:
    at bar (test_error_stack_trace.ms:6)
    at foo (test_error_stack_trace.ms:2)
    at <main> (test_error_stack_trace.ms:9)
```

### test_error_type.ms

```ms
x = "hello" + 42
```

预期错误输出：
```
Error: cannot add string and int
  --> test_error_type.ms:1:5
   |
  1| x = "hello" + 42
   |     ^^^^^^^^^^^^^
```

### test_error_index.ms

```ms
lst = [1, 2, 3]
print(lst[10])
```

预期错误输出：
```
Error: index 10 out of bounds for list of length 3
  --> test_error_index.ms:2:7
   |
  2| print(lst[10])
   |        ^^^^^^^
Stack trace:
    at <main> (test_error_index.ms:2)
```

### test_error_undefined.ms

```ms
print(undefined_var)
```

预期错误输出：
```
Error: undefined variable 'undefined_var'
  --> test_error_undefined.ms:1:7
   |
  1| print(undefined_var)
   |        ^^^^^^^^^^^^^
Stack trace:
    at <main> (test_error_undefined.ms:1)
```
