//! .ms 测试语料库 harness。
//!
//! 规则：
//! - 遍历 `tests/ms/**/*.ms`（跳过 `fixtures/` 目录），子进程执行
//!   `ms run <脚本>`（二进制经 `CARGO_BIN_EXE_ms` 定位）。
//! - 普通脚本：退出码 0 通过；若存在同名 `.expected`，stdout 须与其
//!   精确匹配（仅 trim_end 容差，避免行尾换行差异）。
//! - `tests/ms/negative/` 下脚本：退出码须非 0，且 stderr 须包含
//!   `.expected` 中的每一行非注释子串（`#` 开头与空行忽略）。
//! - 单脚本超时 30 秒（防并发用例死锁），超时杀进程记失败。
//! - 顺序执行，失败汇总报告（脚本名 + 实际/期望差异）。

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// 单脚本执行超时。
const TIMEOUT: Duration = Duration::from_secs(30);

/// corpus 根目录（本文件位于 tests/ 下）。
const ROOT: &str = "ms";

/// fixtures 目录名（被 import 的辅助模块，不作为独立用例执行）。
const FIXTURES: &str = "fixtures";

/// negative 子目录相对路径（相对 tests/ms/）。
const NEGATIVE_DIR: &str = "negative";

struct Case {
    /// .ms 脚本绝对路径。
    script: PathBuf,
    /// 同名 .expected 路径（存在才需要校验输出）。
    expected: Option<PathBuf>,
    /// negative 用例：期望失败。
    negative: bool,
}

#[test]
fn ms_corpus() {
    let mut failed: Vec<String> = Vec::new();
    let mut cases = Vec::new();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join(ROOT);
    collect_cases(&root, &root, &mut cases);
    cases.sort_by(|a, b| a.script.cmp(&b.script));

    assert!(!cases.is_empty(), "no .ms cases found under {}", root.display());

    for case in &cases {
        let rel = case.script.strip_prefix(root.parent().unwrap()).unwrap().display().to_string();
        match run_case(case) {
            Ok(()) => eprintln!("ok   {}", rel),
            Err(msg) => {
                eprintln!("FAIL {}", rel);
                failed.push(format!("--- {} ---\n{}", rel, msg));
            }
        }
    }

    if !failed.is_empty() {
        panic!(
            "\n{} corpus case(s) failed:\n\n{}",
            failed.len(),
            failed.join("\n")
        );
    }
}

/// 递归收集用例：fixtures/ 跳过；negative/ 标记期望失败。
fn collect_cases(root: &Path, dir: &Path, out: &mut Vec<Case>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(err) => panic!("read_dir {}: {}", dir.display(), err),
    };
    for entry in entries {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            if entry.file_name().to_string_lossy() == FIXTURES {
                continue;
            }
            collect_cases(root, &path, out);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("ms") {
            continue;
        }
        let expected = path.with_extension("expected");
        let expected = if expected.is_file() {
            Some(expected)
        } else {
            None
        };
        // negative/ 顶层子目录（tests/ms/negative/**）下的用例期望失败
        let negative = path
            .strip_prefix(root)
            .ok()
            .and_then(|rel| rel.components().next())
            .map(|c| c.as_os_str().to_string_lossy() == NEGATIVE_DIR)
            .unwrap_or(false);
        out.push(Case {
            script: path,
            expected,
            negative,
        });
    }
}

/// 执行单个用例并校验。失败返回描述信息。
fn run_case(case: &Case) -> Result<(), String> {
    let start = Instant::now();
    let mut child = Command::new(env!("CARGO_BIN_EXE_ms"))
        .arg("run")
        .arg(&case.script)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn ms failed: {}", e))?;

    // 轮询等待，超时杀进程
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                let out = child.wait_with_output().expect("wait after exit");
                return verify(case, &out.status, &out.stdout, &out.stderr);
            }
            Ok(None) => {
                if start.elapsed() > TIMEOUT {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("timeout after {:?}", TIMEOUT));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => return Err(format!("wait failed: {}", e)),
        }
    }
}

fn verify(
    case: &Case,
    status: &std::process::ExitStatus,
    stdout: &[u8],
    stderr: &[u8],
) -> Result<(), String> {
    let stdout = String::from_utf8_lossy(stdout);
    let stderr = String::from_utf8_lossy(stderr);
    let code = status.code().unwrap_or(-1);

    if case.negative {
        if code == 0 {
            return Err(format!(
                "expected failure but exited 0\nstdout:\n{}",
                indent(&stdout)
            ));
        }
        return match &case.expected {
            Some(path) => {
                let text = std::fs::read_to_string(path)
                    .map_err(|e| format!("read {}: {}", path.display(), e))?;
                for line in text.lines() {
                    let line = line.trim();
                    if line.is_empty() || line.starts_with('#') {
                        continue;
                    }
                    if !stderr.contains(line) {
                        return Err(format!(
                            "stderr missing expected substring {:?}\nstderr:\n{}",
                            line,
                            indent(&stderr)
                        ));
                    }
                }
                Ok(())
            }
            None => Err("negative case requires .expected file".to_string()),
        };
    }

    // 普通用例：须成功
    if code != 0 {
        return Err(format!(
            "expected success but exited {}\nstderr:\n{}",
            code,
            indent(&stderr)
        ));
    }
    match &case.expected {
        Some(path) => {
            let expected = std::fs::read_to_string(path)
                .map_err(|e| format!("read {}: {}", path.display(), e))?;
            if stdout.trim_end() != expected.trim_end() {
                return Err(format!(
                    "stdout mismatch\n--- expected ---\n{}--- actual ---\n{}",
                    indent(&expected),
                    indent(&stdout)
                ));
            }
            Ok(())
        }
        None => Ok(()),
    }
}

fn indent(s: &str) -> String {
    s.lines()
        .map(|l| format!("  {}", l))
        .collect::<Vec<_>>()
        .join("\n")
}
