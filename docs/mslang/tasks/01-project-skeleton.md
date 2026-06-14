# 项目骨架搭建

## 所属阶段
Phase 1.1 - 基础设施

## 前置任务
无

## 目标
初始化 Cargo 项目，建立模块结构，定义统一错误类型，搭建 CLI 入口，确保项目可编译。

## 设计规格

参照 [12-implementation-plan](../12-implementation-plan.md) § 项目结构：

```
mslang-rs/
├── Cargo.toml
├── src/
│   ├── main.rs                 # CLI 入口
│   ├── lib.rs                  # 库入口，声明所有子模块
│   ├── error.rs                # 统一错误类型
│   ├── lexer/
│   │   ├── mod.rs
│   │   └── token.rs
│   ├── ast/
│   │   ├── mod.rs
│   │   └── node.rs
│   ├── parser/
│   │   ├── mod.rs
│   │   ├── expression.rs
│   │   └── statement.rs
│   ├── compiler/
│   │   ├── mod.rs
│   │   └── opcode.rs
│   ├── vm/
│   │   ├── mod.rs
│   │   ├── object.rs
│   │   ├── frame.rs
│   │   ├── builtins.rs
│   │   └── stdlib.rs
│   ├── gc/
│   │   └── mod.rs
│   ├── module/
│   │   ├── mod.rs
│   │   └── resolver.rs
│   ├── async_runtime/
│   │   ├── mod.rs
│   │   └── channel.rs
│   └── repl/
│       └── mod.rs
```

## 实现细节

### 1. Cargo.toml

```toml
[package]
name = "mslang"
version = "0.1.0"
edition = "2021"
description = "mslang scripting language implementation"

[dependencies]
clap = { version = "4", features = ["derive"] }
thiserror = "1"

[[bin]]
name = "ms"
path = "src/main.rs"
```

- 使用 `clap` derive 模式处理 CLI 参数
- 使用 `thiserror` 派生错误类型

### 2. src/lib.rs

声明所有子模块（Phase 1 只需要 `lexer`、`ast`、`parser`，其余先占位）：

```rust
pub mod lexer;
pub mod ast;
pub mod parser;
pub mod compiler;
pub mod vm;
pub mod gc;
pub mod module;
pub mod async_runtime;
pub mod repl;

pub mod error;
```

### 3. src/error.rs — 统一错误类型

```rust
use thiserror::Error;

#[derive(Error, Debug)]
#[non_exhaustive]
pub enum MspError {
    #[error("lexer error at {line}:{column}: {message}")]
    LexError { line: usize, column: usize, message: String },

    #[error("parse error at {line}:{column}: {message}")]
    ParseError { line: usize, column: usize, message: String },

    #[error("compile error: {message}")]
    CompileError { message: String },

    #[error("runtime error: {0}")]
    RuntimeError(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, MspError>;
```

- `LexError`：词法分析阶段错误，包含行列号和消息
- `ParseError`：语法分析阶段错误，包含行列号和消息
- `CompileError`：编译阶段错误
- `RuntimeError`：VM 运行时错误
- `IoError`：IO 错误，自动从 `std::io::Error` 转换

### 4. src/main.rs — CLI 入口

```rust
use clap::Parser;

#[derive(Parser)]
#[command(name = "ms", about = "mslang scripting language")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    #[arg(long)]
    version: bool,
}

#[derive(clap::Subcommand)]
enum Commands {
    Run { file: String },
    Eval { expr: String },
    Repl,
    Check { file: String },
}

fn main() {
    let cli = Cli::parse();
    if cli.version {
        println!("mslang 0.1.0");
        return;
    }
    match cli.command {
        Some(Commands::Run { file }) => { /* Phase 2+ */ }
        Some(Commands::Eval { expr }) => { /* Phase 2+ */ }
        Some(Commands::Repl) => { /* Phase 8 */ }
        Some(Commands::Check { file }) => { /* Phase 2+ */ }
        None => {
            use clap::CommandFactory;
            Cli::command().print_help().ok();
        }
    }
}
```

### 5. 占位模块

仅创建各一级模块目录下的 `mod.rs`（初始为空或仅含 `// TODO: implement` 注释），确保编译通过。结构树中展示的子文件（如 `token.rs`、`node.rs` 等）在后续对应 task 中创建。

## 验证标准

1. `cargo build` 编译无错误、无警告
2. `cargo test` 通过（包含 error 类型的基础测试，应输出 2 passed）
3. `cargo run -- --version` 输出 `mslang 0.1.0`
4. `cargo run -- --help` 输出帮助信息
5. 核心模块结构（`src/` 下的一级模块）与设计文档一致（`include/`、`src/capi/` 由 task 65 创建；`src/gc/` 子模块由 task 40+ 逐步添加；`stdlib/`、`tests/` 由后续 Phase 添加）

## 测试用例

本任务无 `.ms` 测试。仅 Rust 级别验证：

```rust
#[cfg(test)]
mod tests {
    use crate::error::{MspError, Result};

    #[test]
    fn test_error_display() {
        let err = MspError::LexError {
            line: 1,
            column: 5,
            message: "unexpected character".into(),
        };
        assert_eq!(
            format!("{}", err),
            "lexer error at 1:5: unexpected character"
        );
    }

    #[test]
    fn test_result_alias() {
        fn sample() -> Result<i32> { Ok(42) }
        assert_eq!(sample().unwrap(), 42);
    }
}
```
