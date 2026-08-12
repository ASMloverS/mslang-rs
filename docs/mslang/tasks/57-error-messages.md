# 友好错误信息与堆栈跟踪

## 所属阶段
Phase 8.2 - REPL + 工具链

## 前置任务
37-try-except-finally

## 目标
实现清晰友好的错误信息输出，包括行号标注、源码行显示、错误高亮和格式化堆栈跟踪。

## 与设计标准的关系

### §0.1 错误来源统一

本 task 的 Display 路径区分两类输入：

1. **`MsException` 对象**（task 37，`TypeTag::EXCEPTION`）—— Display 输出 `Error: <message>`，其中 `<message>` 取自 `MsException.message` 字段（string），**不**重复输出 `class_name`（class_name 仅出现在栈跟踪的栈帧或异常属性中）。
2. **VM 内部 `Result<_, String>` 错误**（形如 `"IndexError: list index 10 out of range"`，见 `src/vm/object.rs:1926` 等）—— Display 直接输出该字符串，**不**改写消息内容。本 task 不负责统一两类错误（属跨 task 集成，见 task 37 §验证标准末注）；仅在 `RuntimeError` 结构中容纳二者（`message: String` 字段）。

测试用例的预期文本与现有 VM 错误字符串逐字对齐，不发明新措辞。

### §0.2 与 task 37 的关系

本 task 重写 task 37 引入的 `format_uncaught_error`（`src/vm/mod.rs:2770-2784`），把它从单行 `<ClassName>: <msg>` 升级为调用 `RuntimeError::Display`，输出多行 traceback。原 `last_uncaught_exception: Option<Object>` 字段保留（供 C API `ms_last_error` 与测试断言），不与新 `RuntimeError` 冲突。

### §0.3 与 spec 格式的差异

本 task 在 spec `11-bytecode-vm.md:416-420` 格式（`Error: <msg>` 紧接 `    at ...`）基础上加 `Stack trace:` 标题行作为可读性增强（spec 回写见 §10）。

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

行号表格式优化：使用 RLE（行程编码）压缩连续相同行号（**可选优化，非本 task 必需**；当前 `emit_byte` 已逐字节 push 一条 `(offset, line)`，运行可用）。

### 1.1 行号表迁移到运行时（必需前置）

当前 `Function` 构造时（`src/compiler/expression.rs:699-713` 与 `src/compiler/statement.rs:411-427`）只迁移 `chunk.code` 与 `chunk.constants`，**`chunk.lines` 被丢弃**，VM 运行时无法做 ip→line 反查。本 task 必须先把 lines 迁入运行时：

1. `Function` struct（`src/vm/object.rs:506-527`）新增字段：
   ```rust
   pub lines: Vec<(usize, usize)>,  // (instruction_offset, source_line)
   ```
2. `Function::new`（`src/vm/object.rs:530`）初始化为 `Vec::new()`。
3. `compile_function_closure` 在构造 `Function` 时（两处：`expression.rs:699` 与 `statement.rs:411`）按现有 `code`/`constants` 的迁移模式加一行：
   ```rust
   lines: func_unit.chunk.lines,  // 编译单元 chunk.lines 移入 Function（不再回写）
   ```
4. 脚本顶层入口的 lines 由 `Compiler::compile`（顶层 Chunk → 常量池 `MsFunction`）同样携带。

> **GC 注**：`Vec<(usize, usize)>` 不持 `Object` 引用，trace 函数无需扫描；`MsFunction` 体积变化由 `alloc_function` 的 `size: std::mem::size_of::<MsFunction>()` 自动反映（`src/vm/object.rs:555`），无需手工调整。

### 1.2 列号（column）获取

`emit_byte(byte, line)` 签名仅含 line，无 column（`src/compiler/mod.rs:203`）。本 task 不改 `emit_byte` 签名（避免大规模调用点变更），列号按以下策略获取：

- **运行时错误**：`SourceLocation.column` 恒为 `None`。Display 输出 `--> <file>:<line>`（不显示 `:col`），不画 `^` 指示行（见 §6）。
- **编译时错误**（lexer/parser 已有 column，见 `src/error.rs:8-18` 的 `LexError.column` / `ParseError.column`）：`SourceLocation.column = Some(col)`，Display 输出 `--> <file>:<line>:<col>` 并画 `^` 指示行。

本 task 不实现「运行时错误回追 token 列号」——该工作需扩展 `lines` 表为 `(offset, line, col)` 或维护独立的 ip→token 索引，留作未来增强。

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

在 VM 抛出运行时错误时，遍历 `call_stack` 构建堆栈跟踪。**注意字段访问路径**：`CallFrame.closure` 是 `*mut MsObjHeader` 指向 `MsClosure`，`MsClosure.function` 是 `*mut MsObjHeader` 指向 `MsFunction`，`MsFunction.function` 才是 `Function` struct（含 `name`/`source_file`/`lines` 等字段，见 `src/vm/object.rs:500-527`）。不能直接 `frame.closure.function.unit.<field>` 三层点访问。

```rust
fn build_stack_trace(&self) -> StackTrace {
    let mut frames = Vec::new();
    for frame in self.call_stack.iter().rev() {
        // SAFETY: CallFrame.closure 由 alloc_closure 分配；帧存活期间 closure 与
        // 其 function 均被根集（call_stack）持有，GC 不会回收。本函数不分配 Object
        // （仅 String::clone / Vec::push，String 在栈上不进 GC 堆），不触发 GC。
        let closure = unsafe { read_closure(frame.closure) };
        let function = unsafe { read_function(closure.function) };
        let func = &function.function;
        let line = get_line(&func.lines, frame.ip);
        let name = func.name.clone();
        let file = func.source_file.clone().unwrap_or_else(|| "<script>".into());
        frames.push(StackTraceFrame {
            function_name: name,
            location: SourceLocation { file, line, column: None },
        });
    }
    StackTrace { frames }
}

/// ip → source line 反查。lines 表按 instruction_offset 升序；从尾向前找到第一个
/// offset <= ip 的条目即对应行号；空表或 ip 早于首条 → 0（表示「未知行」）。
fn get_line(lines: &[(usize, usize)], ip: usize) -> usize {
    for (off, line) in lines.iter().rev() {
        if *off <= ip {
            return *line;
        }
    }
    0
}
```

> **GC 安全**：本函数仅在持 `frame.closure`/`closure.function` 原始指针期间做 `String::clone`（堆分配走 Rust 全局分配器，不进 mslang GC 堆），不触发 GC safepoint。若未来扩展为分配 `Object`（如缓存 `func.source_file` 为 mslang String），须先把所需字段 clone 到本地 Vec 再放下 raw 指针借用。

### 5.1 生成器与协程帧

`call_stack` 中的生成器帧（`CallFrame.gen_owner: Some(_)`，task 39）按普通帧显示，函数名取 `Function.name`；`PausedCoroutine`（task 53）的暂停帧不计入（异步错误经 event_loop 调度恢复时已重新压入 `call_stack`）。本 task 不实现跨协程栈串联（即不显示 `await` 处的调用方协程链），留待后续。

### 6. 错误显示格式化

```rust
impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        // 错误标题（消息内容不改写，见 §0.1）
        writeln!(f, "Error: {}", self.message)?;

        // 源码行和错误指示（仅当 location 与 source_line 均可用时才输出）
        if let Some(ref loc) = self.location {
            // 列号缺失（运行时错误，见 §1.2）→ 不显示 :col，不画 ^ 行
            if let Some(col) = loc.column {
                if let Some(ref source_line) = self.source_line {
                    let line_str = loc.line.to_string();
                    let width = line_str.len().max(3);
                    writeln!(f, "{:>width$} --> {}:{}:{}", "", loc.file, loc.line, col, width = width)?;
                    writeln!(f, "{:>width$} |", "", width = width)?;
                    writeln!(f, "{:>width$} | {}", line_str, source_line, width = width)?;
                    let caret_pad = col.saturating_sub(1).min(source_line.chars().count());
                    writeln!(f, "{:>width$} | {}^", "", " ".repeat(caret_pad), width = width)?;
                }
            } else {
                // 列号缺失：仅显示文件名:行号，不画 ^ 行
                if let Some(ref source_line) = self.source_line {
                    let line_str = loc.line.to_string();
                    let width = line_str.len().max(3);
                    writeln!(f, "{:>width$} --> {}:{}", "", loc.file, loc.line, width = width)?;
                    writeln!(f, "{:>width$} |", "", width = width)?;
                    writeln!(f, "{:>width$} | {}", line_str, source_line, width = width)?;
                } else {
                    let width = loc.line.to_string().len().max(3);
                    writeln!(f, "{:>width$} --> {}:{}", "", loc.file, loc.line, width = width)?;
                }
            }
        }

        // 堆栈跟踪（spec 格式扩展，加 "Stack trace:" 标题，见 §0.3）
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

> **行号宽度对齐**：用动态宽度 `width = line.to_string().len().max(3)` 替代固定的 `{:3}`，避免 ≥ 1000 行的源文件破坏 `|` 列对齐。
> **caret 上界**：`col.saturating_sub(1).min(source_line.chars().count())` 防止 col 超出源码行字符数时 `^` 远离错误位置（V3）。
> **源码行不可读时**（§9 read_source_line 返回 None）：`source_line = None`，仅输出 `--> file:line`，省略源码与 `^` 行（R7）。

### 7. 编译时错误格式化

```rust
impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        writeln!(f, "Compile error: {}", self.message)?;
        if let Some(ref loc) = self.location {
            let line_str = loc.line.to_string();
            let width = line_str.len().max(3);
            // 编译时错误 column 来自 lexer/parser（见 §1.2），恒为 Some
            if let Some(col) = loc.column {
                writeln!(f, "{:>width$} --> {}:{}:{}", "", loc.file, loc.line, col, width = width)?;
                if let Some(ref source_line) = self.source_line {
                    writeln!(f, "{:>width$} |", "", width = width)?;
                    writeln!(f, "{} | {}", line_str, source_line)?;
                    let caret_pad = col.saturating_sub(1).min(source_line.chars().count());
                    writeln!(f, "{:>width$} | {}^--- here", "", " ".repeat(caret_pad), width = width)?;
                }
            } else {
                writeln!(f, "{:>width$} --> {}:{}", "", loc.file, loc.line, width = width)?;
            }
        }
        Ok(())
    }
}
```

> **统一规则**：行号动态宽度、`^` 上界保护、源码不可读时回退均与 §6 RuntimeError 一致。

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

运行时需要能读取源文件来显示出错行。须做四项防护：line==0 下溢、文件大小无界、源文件不可读回退、缓存策略。

```rust
/// 源文件大小上限。超过此大小的文件视为「源码不可读」，避免 OOM。
/// 1 MiB 对脚本文件足够；嵌入宿主可通过 VM 配置覆盖。
const MAX_SOURCE_FILE_BYTES: u64 = 1024 * 1024;

fn read_source_line(file: &str, line: usize) -> Option<String> {
    // (V2) line==0 时 line-1 下溢 → 提前返回 None
    if line == 0 { return None; }

    // (V4) 大文件防护：metadata 校验大小
    let meta = std::fs::metadata(file).ok()?;
    if meta.len() > MAX_SOURCE_FILE_BYTES { return None; }

    let content = std::fs::read_to_string(file).ok()?;
    content.lines().nth(line - 1).map(|s| s.to_string())
}
```

- **缓存策略**（R5/V5）：VM 持有 `source_cache: HashMap<String, Arc<Vec<String>>>`，键为 `file` 路径，值为按行切分的源码。首次读取后缓存，重复命中走缓存（避免深栈多帧同文件的 O(n²) IO）。**不设容量上限**——单次运行内导入的源文件数量有限；REPL 重启时 `source_cache.clear()`。未来若嵌入宿主长生命周期场景需要，再加 LRU 上限。
- **REPL 输入**（task 56 集成）：REPL 源码无文件路径，编译时 `source_file` 设为字面量 `"<repl>"`；`read_source_line("<repl>", n)` 走特殊路径——从 VM 持有的 REPL 输入历史 `Vec<String>` 取第 n 行（task 56 已有 buffer 历史）。
- **TOCTOU 局限**（V6）：运行时读取的源码可能因文件被编辑而与编译时不一致；本 task 不做快照，接受此局限（一致性需求留给未来 source map 持久化）。
- **回退显示**（R7）：以上任何一步失败 → `read_source_line` 返回 None → Display 仅输出 `--> <file>:<line>`，省略源码与 `^` 行（见 §6）。

### 10. 设计规格回写（spec writeback）

本 task 对设计文档的扩展（参照 task 37 §11 的回写惯例）：

- **`11-bytecode-vm.md` §堆栈跟踪格式**（行 416-420）：在 `Error: <message>` 与 `    at <fn> (...)` 之间插入 `Stack trace:` 标题行（见 §0.3）。最终格式：
  ```
  Error: <message>
  Stack trace:
      at <fn> (<file>:<line>)
      ...
  ```
- **`11-bytecode-vm.md` §CompilationUnit / §Function**：`Function` struct（行 273-281）新增 `lines: Vec<(usize, usize)>` 字段（见 §1.1）。
- **`12-implementation-plan.md` Phase 8.2**：行号表生成（行 183）与行号标注（行 603）合并到本 task；本 task 同时完成 lines 运行时迁移。

> spec `11-bytecode-vm.md` 行 213 的 `CompilationUnit.lines: Vec<(usize, usize)>` 不变（编译单元保留 lines）；本 task 仅扩展运行时 `Function` 携带一份 lines 副本。

## 验证标准

1. 运行时错误显示文件名和行号
2. 堆栈跟踪正确显示调用链
3. 源码行在错误信息中正确显示
4. 编译时错误和运行时错误格式一致
5. 多文件错误正确显示各文件中的位置
6. 错误信息不包含内部实现细节（如指令偏移量）

## 测试用例

> **格式说明**：运行时错误 `SourceLocation.column` 恒为 `None`（§1.2），故 `--> file:line`（无 `:col`），不画 `^` 行；编译时错误 column 来自 lexer/parser，画 `:col` 与 `^` 行。VM 内部 `Result<_, String>` 错误按 §0.1 case 2 **逐字输出**——预期文本与 `src/vm/object.rs`、`src/vm/stdlib.rs` 中的字符串字面对齐，不发明新措辞。

### test_error_divzero.ms

```ms
x = 10 / 0
```

预期错误输出（VM 字符串 `ZeroDivisionError: division by zero` 来自 `src/vm/object.rs:1440`，逐字输出；运行时错误无 column）：
```
ZeroDivisionError: division by zero
    --> test_error_divzero.ms:1
    |
  1 | x = 10 / 0
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

预期错误输出（MsException 路径，§0.1 case 1：`Error: <message>`）：
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

预期错误输出（VM 字符串 `TypeError: unsupported operand type(s) for +: 'string' and 'int'` 来自 `src/vm/object.rs:1375-1379`，逐字输出；运行时错误无 column）：
```
TypeError: unsupported operand type(s) for +: 'string' and 'int'
    --> test_error_type.ms:1
    |
  1 | x = "hello" + 42
```

### test_error_index.ms

```ms
lst = [1, 2, 3]
print(lst[10])
```

预期错误输出（VM 字符串 `IndexError: list index 10 out of range` 来自 `src/vm/object.rs:1926`，逐字输出）：
```
IndexError: list index 10 out of range
    --> test_error_index.ms:2
    |
  2 | print(lst[10])
Stack trace:
    at <main> (test_error_index.ms:2)
```

### test_error_undefined.ms

> 当前 `OpCode::LoadGlobal`（`src/vm/mod.rs:3020`）对未定义全局静默返回 `Object::Nil`，**不**抛 NameError。本测试用 `throw NameError(...)` 显式构造异常对象，验证 task 37 异常机制 + task 57 Display 路径协作（顶层全局访问的 NameError 接线属未来 task）。

```ms
throw NameError("undefined variable 'undefined_var'")
```

预期错误输出（MsException 路径，§0.1 case 1：`Error: <message>`）：
```
Error: undefined variable 'undefined_var'
Stack trace:
    at <main> (test_error_undefined.ms:1)
```

### test_compile_error.ms（编译时错误，验证 column 与 `^` 行）

```ms
x = 1 + 
```

预期错误输出（parser 在 `+` 后报「unexpected token / expected expression」，column 来自 lexer）：
```
Compile error: expected expression after '+'
    --> test_compile_error.ms:1:7
    |
  1 | x = 1 + 
    |       ^--- here
```
