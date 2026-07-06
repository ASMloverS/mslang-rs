//! task 45：模块解析器。
//!
//! 实现 `import` 的模块搜索、缓存与加载编排。参照
//! [45-module-system](../../docs/mslang/tasks/45-module-system.md) §2/§5/§6。
//!
//! # 加载编排与借用
//!
//! `ModuleResolver::load` 在 spec 中持 `&mut self` 跨 `vm.execute_module` 调用，
//! 与 `&mut VM` 构成重叠借用（§7 借用说明）。为规避该冲突且保持嵌套 IMPORT 可重入，
//! 本实现将加载流程拆为**阶段化**步骤，由 `VM::load_module` 编排：解析/缓存登记
//! （持 `&mut self.module_resolver`）与模块执行（持 `&mut self`）严格串行，无重叠
//! 借用。功能与 spec 等价（缓存命中返回、空壳登记、失败清理、成功保留）。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::compiler::{Chunk, Compiler};
use crate::lexer::Lexer;
use crate::parser::Parser;
use crate::vm::object::MsObjHeader;

/// 导入递归深度上限：防 N 层线性依赖链栈溢出（§5）。
pub const MAX_IMPORT_DEPTH: usize = 200;

/// 模块解析器。持有搜索路径、标准库目录、已加载模块缓存与加载链状态。
///
/// 字段公开供 `VM::load_module` 编排访问（同 crate 内）。
pub struct ModuleResolver {
    /// 搜索根（按优先级）：当前目录 < stdlib < MS_PATH。
    pub search_paths: Vec<PathBuf>,
    /// 标准库目录，`@std` 前缀专用搜索根。
    pub stdlib_dir: PathBuf,
    /// 已加载模块缓存，键为规范化绝对路径。循环导入下空壳 Module 亦暂存于此。
    pub cache: HashMap<PathBuf, *mut MsObjHeader>,
    /// 正在加载中的模块（规范化路径），用于深度限制与循环诊断。
    pub loading_stack: HashSet<PathBuf>,
    /// 安全模式（`MS_SAFE=1` 或 `ms run --safe`）：为真时拒绝非 `@std` 的 import。
    pub safe_mode: bool,
    /// task 46：原生模块注册表（键为规范模块名，如 "io"）。命中则直接返回缓存指针，
    /// 跳过磁盘搜索与执行。注册表查找在 `@std:` 前缀剥离之后。
    pub native_modules: HashMap<String, *mut MsObjHeader>,
}

impl ModuleResolver {
    /// 默认构造：按 当前目录 → stdlib → MS_PATH 填入 search_paths，读 MS_SAFE。
    /// stdlib 目录取自环境变量 `MS_STDLIB`，缺省为可执行文件同级的 `stdlib/`。
    pub fn new() -> Self {
        let stdlib_dir = std::env::var("MS_STDLIB")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                // 默认：可执行文件同级 stdlib/。不存在时 @std import 会失败（task 46+ 实装）。
                std::env::current_exe()
                    .ok()
                    .and_then(|p| p.parent().map(|d| d.join("stdlib")))
                    .unwrap_or_else(|| PathBuf::from("stdlib"))
            });

        let mut search_paths = vec![PathBuf::from(".")];
        search_paths.push(stdlib_dir.clone());
        // MS_PATH：";" 分隔的额外搜索路径。
        if let Ok(ms_path) = std::env::var("MS_PATH") {
            for p in ms_path.split(';') {
                if !p.is_empty() {
                    search_paths.push(PathBuf::from(p));
                }
            }
        }

        let safe_mode = std::env::var("MS_SAFE").map(|v| v == "1").unwrap_or(false);

        ModuleResolver {
            search_paths,
            stdlib_dir,
            cache: HashMap::new(),
            loading_stack: HashSet::new(),
            safe_mode,
            native_modules: HashMap::new(),
        }
    }

    /// 测试/注入用构造：指定 stdlib 目录与安全模式。
    pub fn with_config(stdlib_dir: PathBuf, safe_mode: bool) -> Self {
        let mut search_paths = vec![PathBuf::from(".")];
        search_paths.push(stdlib_dir.clone());
        ModuleResolver {
            search_paths,
            stdlib_dir,
            cache: HashMap::new(),
            loading_stack: HashSet::new(),
            safe_mode,
            native_modules: HashMap::new(),
        }
    }

    /// 加入额外搜索根（测试用，如把临时模块目录置于搜索首位）。
    pub fn add_search_path(&mut self, path: PathBuf) {
        self.search_paths.insert(0, path);
    }

    /// 按搜索规则查找模块文件，返回**规范化绝对路径**（兼作缓存键，§6）。
    ///
    /// - dotted path：`os.path` → 段 `["os","path"]`，对应 `os/path.ms`。
    /// - 候选 1：`root/<seg0>/.../<segN>.ms`（文件模块）。
    /// - 候选 2：`root/<seg0>/.../<segN>/index.ms`（包模块）。
    /// - `stdlib_only`：仅搜索标准库目录（`@std` 前缀）。
    pub fn resolve(&self, name: &str, stdlib_only: bool) -> Result<PathBuf, String> {
        let segments: Vec<&str> = name.split('.').collect();

        // 标识符受限（[a-zA-Z_][a-zA-Z0-9_]*），模块名段不含 ".." 或绝对路径，
        // 故路径拼接天然免疫目录穿越；canonicalize 进一步消除符号链接别名（§6）。
        let roots: Vec<PathBuf> = if stdlib_only {
            vec![self.stdlib_dir.clone()]
        } else {
            self.search_paths.clone()
        };

        for root in &roots {
            let joined = segments.join("/");
            // 候选 1：文件模块
            let file = root.join(&joined).with_extension("ms");
            if file.is_file() {
                return canonicalize_or_err(&file, name);
            }
            // 候选 2：包模块（目录 + index.ms）
            let pkg = root.join(&joined).join("index.ms");
            if pkg.is_file() {
                return canonicalize_or_err(&pkg, name);
            }
        }
        Err(format!("ImportError: 找不到模块 '{}'", name))
    }
}

impl Default for ModuleResolver {
    fn default() -> Self {
        Self::new()
    }
}

/// 解析 `@std:` 前缀（编译期由编译器写入常量池），返回 (是否仅标准库, 真实模块名)。
pub fn parse_std_prefix(name: &str) -> (bool, &str) {
    if let Some(rest) = name.strip_prefix("@std:") {
        (true, rest)
    } else {
        (false, name)
    }
}

fn canonicalize_or_err(p: &Path, name: &str) -> Result<PathBuf, String> {
    p.canonicalize()
        .map_err(|e| format!("ImportError: 解析 '{}' 失败: {}", name, e))
}

/// 编译模块源码为字节码，返回 (Chunk, 导出名集, 私有顶层名集)。
///
/// 模块以 module_mode 编译：顶层 const/var 走 STORE_GLOBAL，使 execute_module 可经
/// globals 捕获并拆分导出/私有（§7）。导出名（fn/class/const）与私有名（var/:=/=）
/// 由 Compiler 记录，经 `take_module_kinds` 取出。
pub fn compile_module_source(
    source: &str,
    name: &str,
) -> Result<(Chunk, Vec<String>, Vec<String>), String> {
    let tokens = Lexer::new(source)
        .tokenize_all()
        .map_err(|e| format!("{}", e))?;
    let program = Parser::new(tokens).parse().map_err(|e| format!("{}", e))?;
    let mut compiler = Compiler::new();
    compiler.set_source_file(name.to_string());
    compiler.set_module_mode(true);
    let chunk = compiler.compile(&program)?;
    let (exports, private) = compiler.take_module_kinds();
    Ok((chunk, exports, private))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_std_prefix() {
        assert_eq!(parse_std_prefix("@std:math"), (true, "math"));
        assert_eq!(parse_std_prefix("@std:os.path"), (true, "os.path"));
        assert_eq!(parse_std_prefix("math"), (false, "math"));
        assert_eq!(parse_std_prefix(""), (false, ""));
    }

    #[test]
    fn test_resolve_missing_module() {
        let r = ModuleResolver::with_config(PathBuf::from("nonexistent_stdlib"), false);
        let err = r.resolve("definitely_no_such_module_xyz", false).unwrap_err();
        assert!(err.contains("ImportError"));
    }

    #[test]
    fn test_resolve_file_module() {
        // 临时目录写入模块文件，验证 resolve 返回规范化绝对路径。
        let dir = std::env::temp_dir().join("mslang_resolve_test_file");
        std::fs::create_dir_all(&dir).unwrap();
        let mod_path = dir.join("mymod.ms");
        std::fs::write(&mod_path, "const X = 1").unwrap();
        let mut r = ModuleResolver::with_config(PathBuf::from("nonexistent_stdlib"), false);
        r.add_search_path(dir.clone());
        let resolved = r.resolve("mymod", false).unwrap();
        assert_eq!(resolved, mod_path.canonicalize().unwrap());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_resolve_package_module() {
        // 目录 + index.ms 包模块。
        let dir = std::env::temp_dir().join("mslang_resolve_test_pkg");
        let pkg = dir.join("mylib");
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(pkg.join("index.ms"), "const X = 1").unwrap();
        let mut r = ModuleResolver::with_config(PathBuf::from("nonexistent_stdlib"), false);
        r.add_search_path(dir.clone());
        let resolved = r.resolve("mylib", false).unwrap();
        assert_eq!(resolved, pkg.join("index.ms").canonicalize().unwrap());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_resolve_dotted_path() {
        let dir = std::env::temp_dir().join("mslang_resolve_test_dotted");
        std::fs::create_dir_all(dir.join("os")).unwrap();
        let target = dir.join("os").join("path.ms");
        std::fs::write(&target, "const X = 1").unwrap();
        let mut r = ModuleResolver::with_config(PathBuf::from("nonexistent_stdlib"), false);
        r.add_search_path(dir.clone());
        let resolved = r.resolve("os.path", false).unwrap();
        assert_eq!(resolved, target.canonicalize().unwrap());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_resolve_stdlib_only() {
        // stdlib_only=true 仅搜索 stdlib_dir，忽略当前目录/MS_PATH。
        let dir = std::env::temp_dir().join("mslang_resolve_test_std");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("math.ms"), "const X = 1").unwrap();
        let r = ModuleResolver::with_config(dir.clone(), false);
        let resolved = r.resolve("math", true).unwrap();
        assert_eq!(resolved, dir.join("math.ms").canonicalize().unwrap());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_compile_module_source_records_kinds() {
        let src = "const VERSION = \"1.0\"\nvar private_var = 1\nfn add(a, b) { return a + b }";
        let (chunk, exports, private) = compile_module_source(src, "test_mod").unwrap();
        assert!(exports.contains(&"VERSION".to_string()));
        assert!(exports.contains(&"add".to_string()));
        assert!(private.contains(&"private_var".to_string()));
        // 非 module_mode 的 STORE_GLOBAL 行为：验证常量池含模块名串（编译成功即可）。
        assert!(!chunk.code.is_empty());
    }
}
