use clap::{Parser, Subcommand};
// mslang::parser::Parser 与 clap 的 Parser trait 同名，别名区分
use mslang::parser::Parser as MsParser;
use mslang::error::format_compile_error;
use mslang::lexer::Lexer;
use mslang::repl::Repl;
use mslang::vm::VM;

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

/// task 58：运行脚本文件或模块（§3）。
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

    let source = std::fs::read_to_string(&path).map_err(|e| format!("IO error: {}", e))?;

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

/// task 58：求值表达式并打印结果（§4）。
fn cmd_eval(expr: &str) -> Result<(), String> {
    let mut vm = VM::new();
    let result = vm.eval_expression(expr)?;
    println!("{}", result.display());
    Ok(())
}

/// task 58：启动交互式 REPL（§5）。
fn cmd_repl() -> Result<(), String> {
    let mut repl = Repl::new()?;
    repl.run()
}

/// task 58：仅词法 + 语法检查，不编译不执行（§6）。
fn cmd_check(file: &str) -> Result<(), String> {
    let source = std::fs::read_to_string(file).map_err(|e| format!("IO error: {}", e))?;

    // 只做词法分析 + 语法分析
    let tokens = Lexer::new(&source)
        .tokenize_all()
        .map_err(|e| format_compile_error(file, &source, &e))?;
    let _ast = MsParser::new(tokens)
        .parse()
        .map_err(|e| format_compile_error(file, &source, &e))?;

    println!("{}: syntax OK", file);
    Ok(())
}

/// task 58：打印版本号（§7）。
fn cmd_version() -> Result<(), String> {
    println!("mslang {}", env!("CARGO_PKG_VERSION"));
    Ok(())
}
