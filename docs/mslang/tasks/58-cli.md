# CLI 工具链

## 所属阶段
Phase 8.3 - REPL + 工具链

## 前置任务
56-repl, 57-error-messages

## 目标
完善 CLI 工具链，实现所有命令行子命令，包括 `run`、`eval`、`repl`、`check`、`version`，提供统一的 mslang 开发体验。

## 设计规格

参照 [09-modules](../09-modules.md) § CLI 与模块、[12-implementation-plan](../12-implementation-plan.md) § 8.3 CLI：

### CLI 命令

| 命令 | 说明 |
|---|---|
| `ms run script.ms` | 运行脚本文件 |
| `ms run module.path` | 运行模块（等价于 `module/path.ms`） |
| `ms eval "expression"` | 求值表达式 |
| `ms repl` | 启动 REPL |
| `ms check script.ms` | 仅语法检查（不执行） |
| `ms version` | 打印版本号 |
| `ms fmt script.ms` | 格式化源码（后续版本） |

> `ms fmt` 为后续版本（09-modules.md § 格式化标注"后续版本"），本 task 不实现，clap 枚举中不定义。

### 安全模式

参照 [09-modules](../09-modules.md) § 安全提示（`ms run --safe script.ms`：CLI 安全模式标志）：

- CLI 参数 `ms run --safe` 启用安全模式（仅允许 `import @std`，见 [45-module-system](./45-module-system.md) § 安全模式）
- 环境变量 `MS_SAFE=1` 已由 `ModuleResolver::new` 自动读取（`src/module/resolver.rs:69`），CLI 无需重复接线

### 退出码

| 退出码 | 说明 |
|---|---|
| 0 | 成功 |
| 1 | 错误（编译错误、运行时错误等） |
| 2 | 用法错误（clap 参数解析失败的默认退出码） |

> `os.exit(code)`（[48-stdlib-os-string-time](./48-stdlib-os-string-time.md) §2）以 `__EXIT__{code}` 标记传播，main 检测后**静默**以用户指定的任意码退出（见 §8），不受上表约束。

## 实现细节

### 1. CLI 定义（clap derive）

`src/main.rs`：

```rust
use clap::{Parser, Subcommand};

#[derive(Parser)]
// arg_required_else_help：无参数时打印帮助而非报用法错误
// （clap 将帮助输出到 stderr 并以退出码 2 结束，见验证标准 15/16）
#[command(name = "ms", about = "mslang scripting language", version, arg_required_else_help = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run a script file or module
    Run {
        /// Script file path or module path (e.g. mylib.utils)
        target: String,
        /// Enable safe mode (only `import @std` allowed)
        #[arg(long)]
        safe: bool,
        /// Arguments passed to the script (captured into os.args via process argv)
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Evaluate an expression
    Eval {
        /// Expression to evaluate
        expr: String,
    },
    /// Start interactive REPL
    Repl,
    /// Check syntax without executing
    Check {
        /// Script file to check
        file: String,
    },
    /// Print version information
    Version,
}
```

> `--safe` 须置于目标之前（`ms run --safe script.ms a b`）；`trailing_var_arg` 会把目标之后的所有 token（含 `-` 开头者）归入脚本参数。

### 2. 命令处理

命令分派与统一错误处理（含 `os.exit` 的 `__EXIT__` 标记）合并在 §8 的 `main` 中实现，本节不单独定义 `main`：

```rust
let result = match cli.command {
    Commands::Run { target, args, safe } => cmd_run(&target, &args, safe),
    Commands::Eval { expr } => cmd_eval(&expr),
    Commands::Repl => cmd_repl(),
    Commands::Check { file } => cmd_check(&file),
    Commands::Version => cmd_version(),
};
```

> **错误类型约定**：各 `cmd_*` 函数返回 `Result<(), String>`。VM 侧 `exec`/`exec_file`/`eval_expression`（task 56/57 实装）与 `Repl::new/run` 均返回 `Result<_, String>`，且 String 内容已是 task 57 格式化后的多行错误文本（含 traceback / `--> file:line` / `^` 指示行）——不得再包装 `MspError::RuntimeError`（其 Display 会加 `runtime error: ` 前缀，破坏 task 57 输出格式）。

### 3. cmd_run 实现

```rust
fn cmd_run(target: &str, _args: &[String], safe: bool) -> Result<(), String> {
    let mut vm = VM::new();

    // CLI --safe 标志接线（MS_SAFE 环境变量已由 ModuleResolver::new 读取，无需重复）
    if safe {
        vm.set_module_safe_mode(true);
    }

    // os.args 无需 CLI 注入：task 48 的 build_args_list 在 VM::new 时一次性快照
    // std::env::args()（全量 argv，含 "run"、脚本路径与脚本参数，见
    // 48-stdlib-os-string-time.md §2 与 src/vm/stdlib.rs:593）

    // 显式文件路径直读；模块路径复用 import 的解析规则（见 §3.1 resolve_run_target）
    let path = vm.resolve_run_target(target)?;

    let source = std::fs::read_to_string(&path)
        .map_err(|e| format!("IO error: {}", e))?;

    // 设置脚本目录为模块搜索路径（09-modules.md § CLI 与模块）。
    // 跳过空 parent（裸文件名 "hello.ms" 的 parent 为 ""，resolver 已含 "." 根）
    if let Some(dir) = std::path::Path::new(&path).parent() {
        if !dir.as_os_str().is_empty() {
            vm.add_module_search_path(dir.to_path_buf());
        }
    }

    vm.exec_file(&source, &path.to_string_lossy())?;
    Ok(())
}
```

### 3.1 VM 适配（exec_file / resolve_run_target / API 转正）

**`VM::exec_file`**：`exec`（task 56）内部走 `interpret(chunk)`，不携带源文件名——堆栈跟踪将显示 `<script>` 且 task 57 的 `read_source_line` 无法显示源码行。新增带文件名变体（复用已实装的 `interpret_named`，`src/vm/mod.rs:522`）：

```rust
/// task 58：带源文件名的 exec（与 exec 同路径，仅 interpret → interpret_named）。
pub fn exec_file(&mut self, source: &str, file: &str) -> Result<(), String> {
    self.reset_execution_state();
    let program = parse_source(source)?;
    let mut compiler = crate::compiler::Compiler::new();
    compiler.set_module_mode(true);
    let chunk = compiler.compile(&program)?;
    self.interpret_named(chunk, Some(file.to_string())).map(|_| ())
}
```

**`VM::resolve_run_target`**：模块路径解析必须与 import 同规则（包模块 `index.ms`、stdlib、MS_PATH 搜索、规范化），不自造第二套 `replace('.', sep)` 逻辑：

```rust
/// task 58：解析 CLI run 目标为脚本文件路径。显式文件路径（含路径分隔符 /
/// 以 .ms 结尾 / 目标文件已存在）直接返回；否则按模块路径经
/// ModuleResolver::resolve 解析（与 import 同一规则）。
pub fn resolve_run_target(&self, target: &str) -> Result<PathBuf, String> {
    let p = std::path::Path::new(target);
    if target.ends_with(".ms")
        || target.contains('/')
        || target.contains(std::path::MAIN_SEPARATOR)
        || p.exists()
    {
        return Ok(p.to_path_buf());
    }
    self.module_resolver.resolve(target, false)
}
```

> `module_resolver` 为 `pub(crate)` 字段，`main.rs`（独立 bin crate）不可直接访问，故经 VM 公开方法包装。

**API 转正**：`VM::add_module_search_path`（`src/vm/mod.rs:5360`）与 `VM::set_module_safe_mode`（`:5366`）当前标 `#[doc(hidden)]` 且注释"测试用"，本 task 将二者转正为公开正式 API（移除 `#[doc(hidden)]` 与"测试用"注释）。

### 4. cmd_eval 实现

```rust
fn cmd_eval(expr: &str) -> Result<(), String> {
    let mut vm = VM::new();
    let result = vm.eval_expression(expr)?;
    println!("{}", result.display());
    Ok(())
}
```

### 5. cmd_repl 实现

```rust
fn cmd_repl() -> Result<(), String> {
    let mut repl = Repl::new()?;
    repl.run()
}
```

### 6. cmd_check 实现

```rust
fn cmd_check(file: &str) -> Result<(), String> {
    let source = std::fs::read_to_string(file)
        .map_err(|e| format!("IO error: {}", e))?;

    // 只做词法分析 + 语法分析
    let tokens = Lexer::new(&source).tokenize_all()
        .map_err(|e| format_compile_error(file, &source, &e))?;
    let _ast = Parser::new(tokens).parse()
        .map_err(|e| format_compile_error(file, &source, &e))?;

    println!("{}: syntax OK", file);
    Ok(())
}
```

- 不编译为字节码，不执行
- 词法/解析错误经 `format_compile_error`（`src/error.rs:262`，task 57 实装）输出 `--> file:line:col` + `^--- here` 指示行格式，与运行时错误格式一致（task 57 验证标准 4）
- 检查通过打印 "syntax OK"
- 检查失败显示语法错误

### 7. cmd_version 实现

```rust
fn cmd_version() -> Result<(), String> {
    println!("mslang {}", env!("CARGO_PKG_VERSION"));
    Ok(())
}
```

### 8. 错误处理

所有命令共享统一的错误处理（`cmd_*` 返回 `Result<(), String>`，错误文本已是 task 57 格式化输出，见 §2 错误类型约定）：

```rust
fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Run { target, args, safe } => cmd_run(&target, &args, safe),
        Commands::Eval { expr } => cmd_eval(&expr),
        Commands::Repl => cmd_repl(),
        Commands::Check { file } => cmd_check(&file),
        Commands::Version => cmd_version(),
    };

    if let Err(msg) = result {
        // os.exit(code) 的特殊标记（task 48 §2，src/vm/stdlib.rs:676）：
        // defer/finally 已在异常传播途中执行，此处静默以用户码退出（不打印错误）
        if let Some(code) = msg.strip_prefix("__EXIT__") {
            std::process::exit(code.parse::<i32>().unwrap_or(1));
        }
        eprintln!("{}", msg);
        std::process::exit(1);
    }
}
```

### 9. Cargo.toml 更新

本 task **无需新增依赖**：`clap`（derive）、`thiserror`、`rustyline` 已分别由 task 01/56 加入（见 `Cargo.toml:7-13`）。版本号取 `[package] version`（task 01 已定义），`env!("CARGO_PKG_VERSION")` 自动读取。

`src/main.rs` 重写时保留 task 01 的 `[[bin]] name = "ms"` 入口不变。

## 验证标准

1. `ms run script.ms` 正确执行脚本文件
2. `ms run module.path` 正确解析并运行模块
3. `ms eval "1 + 2"` 输出 `3`
4. `ms repl` 启动交互式 REPL
5. `ms check` 正确检测语法错误
6. `ms check` 对正确脚本输出 "syntax OK"
7. `ms version` 输出正确版本号
8. 错误情况下退出码为 1
9. 成功情况下退出码为 0
10. `ms --help` 显示帮助信息
11. `ms run --safe script.ms` 拒绝非 `@std` 的 import（ImportError，退出码 1）
12. `os.exit(n)` 静默以退出码 n 退出（不打印错误）
13. `ms run` 的运行时错误输出携带真实文件名与源码行（task 57 格式，非 `<script>`）
14. `ms check` 的语法错误输出 `--> file:line:col` + `^--- here` 指示行格式
15. `ms` 无参数打印帮助（`arg_required_else_help`，clap 输出到 stderr 并以退出码 2 结束）
16. 用法错误（未知子命令/标志）退出码为 2（clap 默认）

## 测试用例

### 运行脚本

创建 `hello.ms`：
```ms
print("Hello from mslang!")
```

```
ms run hello.ms
```
预期输出：`Hello from mslang!`

### 求值表达式

```
ms eval "1 + 2"
```
预期输出：`3`

```
ms eval "[1, 2, 3].length()"
```
预期输出：`3`

### 语法检查

创建 `good.ms`：
```ms
fn add(a, b) {
    return a + b
}
print(add(1, 2))
```

```
ms check good.ms
```
预期输出：`good.ms: syntax OK`

创建 `bad.ms`：
```ms
fn add(a, b {
    return a + b
}
```

```
ms check bad.ms
```
预期输出（`format_compile_error` 格式，task 57；具体错误消息与列号以 parser 实际输出为准，措辞不在本 task 发明）：

```
Compile error: <parser 实际消息>
    --> bad.ms:1:<col>
     |
   1 | fn add(a, b {
     |   <按 col 缩进的 ^>--- here
```

### 版本号

```
ms version
```
预期输出：`mslang 0.1.0`

### 退出码

```
ms eval "1 + 2"
echo $?
```
预期输出：`0`（成功）

```
ms eval "1 / 0"
echo $?
```
预期输出：`1`（错误）

### 模块路径运行

创建目录结构：
```
mylib/
  utils.ms
```

`mylib/utils.ms`：
```ms
print("utils loaded")
```

```
ms run mylib.utils
```
预期输出：`utils loaded`

### 命令行参数

`args.ms`：
```ms
import os
print(os.args)
```

```
ms run args.ms hello world
```
预期输出：os.args 为 task 48 语义的全量 `std::env::args()` 快照——列表含可执行文件路径、`"run"`、`"args.ms"`、`"hello"`、`"world"`（长度 ≥ 4，末两项为 `hello`/`world`）。

### 安全模式（验证 #11）

`unsafe_import.ms`：
```ms
import math_utils
```

```
ms run --safe unsafe_import.ms
echo $?
```
预期输出：`ImportError: 安全模式下仅允许 import @std`（退出码 1，脚本不执行）。

对照：`import @std math` 在 `--safe` 下正常加载；`MS_SAFE=1 ms run unsafe_import.ms` 行为相同（环境变量路径，无需 --safe）。

### os.exit 退出码（验证 #12）

`exit_test.ms`：
```ms
import os
os.exit(3)
```

```
ms run exit_test.ms
echo $?
```
预期：无错误输出（`__EXIT__3` 标记被 main 静默消费），退出码 `3`。

### 运行时错误格式（验证 #13）

`err.ms`：
```ms
x = 10 / 0
```

```
ms run err.ms
```
预期输出（task 57 格式，文件名为脚本真实路径而非 `<script>`）：

```
ZeroDivisionError: division by zero
    --> err.ms:1
    |
  1 | x = 10 / 0
```
