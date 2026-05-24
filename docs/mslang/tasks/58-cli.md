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

### 退出码

| 退出码 | 说明 |
|---|---|
| 0 | 成功 |
| 1 | 错误（编译错误、运行时错误等） |

## 实现细节

### 1. CLI 定义（clap derive）

`src/main.rs`：

```rust
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "ms", about = "mslang scripting language", version)]
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
        /// Arguments passed to the script
        #[arg(trailing_var_arg = true)]
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

### 2. 命令处理

```rust
fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Run { target, args } => cmd_run(&target, &args),
        Commands::Eval { expr } => cmd_eval(&expr),
        Commands::Repl => cmd_repl(),
        Commands::Check { file } => cmd_check(&file),
        Commands::Version => cmd_version(),
    };

    if let Err(e) = result {
        eprintln!("{}", e);
        std::process::exit(1);
    }
}
```

### 3. cmd_run 实现

```rust
fn cmd_run(target: &str, args: &[String]) -> Result<()> {
    let mut vm = VM::new();

    // 设置命令行参数
    vm.set_args(args);

    // 判断是文件路径还是模块路径
    let path = if target.ends_with(".ms") || std::path::Path::new(target).exists() {
        target.to_string()
    } else {
        // 模块路径：mylib.utils -> mylib/utils.ms
        target.replace(".", &std::path::MAIN_SEPARATOR.to_string()) + ".ms"
    };

    let source = std::fs::read_to_string(&path)
        .map_err(|e| MspError::IoError(e))?;

    // 设置脚本目录为模块搜索路径
    if let Some(dir) = std::path::Path::new(&path).parent() {
        vm.add_search_path(dir.to_path_buf());
    }

    vm.exec(&source)?;
    Ok(())
}
```

### 4. cmd_eval 实现

```rust
fn cmd_eval(expr: &str) -> Result<()> {
    let mut vm = VM::new();
    let result = vm.eval_expression(expr)?;
    println!("{}", result.display());
    Ok(())
}
```

### 5. cmd_repl 实现

```rust
fn cmd_repl() -> Result<()> {
    let mut repl = Repl::new()?;
    repl.run()
}
```

### 6. cmd_check 实现

```rust
fn cmd_check(file: &str) -> Result<()> {
    let source = std::fs::read_to_string(file)
        .map_err(|e| MspError::IoError(e))?;

    // 只做词法分析 + 语法分析
    let tokens = Lexer::new(&source).collect::<Result<Vec<_>>>()?;
    let _ast = Parser::new(tokens).parse()?;

    println!("{}: syntax OK", file);
    Ok(())
}
```

- 不编译为字节码，不执行
- 检查通过打印 "syntax OK"
- 检查失败显示语法错误

### 7. cmd_version 实现

```rust
fn cmd_version() -> Result<()> {
    println!("mslang {}", env!("CARGO_PKG_VERSION"));
    Ok(())
}
```

### 8. 错误处理

所有命令共享统一的错误处理：

```rust
fn main() {
    // ... 命令分派 ...

    if let Err(e) = result {
        match e {
            MspError::RuntimeError { .. } => {
                // 已由 RuntimeError Display 格式化
                eprintln!("{}", e);
            }
            MspError::CompileError { .. } => {
                eprintln!("{}", e);
            }
            MspError::IoError(e) => {
                eprintln!("IO error: {}", e);
            }
            _ => {
                eprintln!("Error: {}", e);
            }
        }
        std::process::exit(1);
    }
}
```

### 9. Cargo.toml 更新

```toml
[dependencies]
clap = { version = "4", features = ["derive"] }
thiserror = "1"
rustyline = "14"

[package.metadata]
version = "0.1.0"
```

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
预期输出：语法错误信息

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
预期输出包含命令行参数列表
