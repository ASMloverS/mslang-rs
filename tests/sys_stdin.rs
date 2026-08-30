//! task 82 Rust 集成测试：sys.stdin_read_all 管道用例 + copy 同名交叉调用回归。
//!
//! 参照 [82-stdlib-fs-os-sys](../docs/mslang/tasks/82-stdlib-fs-os-sys.md)
//! 验证标准 8 与 §2.2 同名冲突治理。

use std::io::Write;
use std::process::{Command, Stdio};

/// 在 temp_dir 下唯一子目录执行 `ms run script`，返回 (退出码, stdout, stderr)。
/// `case` 区分并行用例的子目录，避免 t.ms 相互覆盖。
fn ms_run(case: &str, script: &str, stdin_data: Option<&[u8]>) -> (Option<i32>, String, String) {
    let dir =
        std::env::temp_dir().join(format!("mslang_sys_stdin_{}_{}", case, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script_path = dir.join("t.ms");
    std::fs::write(&script_path, script).unwrap();
    let stdin_cfg = if stdin_data.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    };
    let mut child = Command::new(env!("CARGO_BIN_EXE_ms"))
        .arg("run")
        .arg(&script_path)
        .stdin(stdin_cfg)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn ms failed");
    if let Some(data) = stdin_data {
        let mut stdin = child.stdin.take().expect("stdin piped");
        stdin.write_all(data).expect("write stdin");
        // stdin 在块尾 drop → 管道关闭 → 子进程读到 EOF
    }
    let out = child.wait_with_output().expect("wait ms");
    std::fs::remove_dir_all(&dir).ok();
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// 验证标准 8：`echo hello | ms run t.ms` 中 stdin_read_all() 去除行尾后
/// == "hello"（echo 必附 `\r\n`/`\n`，断言去行尾，不裸相等）。
#[test]
fn stdin_read_all_pipe() {
    let script = "import sys\nprint(sys.stdin_read_all())\n";
    // 模拟 echo 管道行尾：Windows echo 附 "\r\n"，Unix echo 附 "\n"，两种均须通过。
    for probe in ["hello\r\n", "hello\n"] {
        let (code, stdout, stderr) = ms_run("pipe", script, Some(probe.as_bytes()));
        assert_eq!(code, Some(0), "exit 0, stderr: {}", stderr);
        assert_eq!(
            stdout.trim_end_matches(['\r', '\n']),
            "hello",
            "去行尾后相等（输入 {:?}），stdout: {:?}",
            probe,
            stdout
        );
    }
}

/// §2.2 同名交叉调用回归：全局 copy(val)（恰 1 参）与 fs.copy(src, dst)
/// （恰 2 参）并存均可用（native_arities copy → usize::MAX 下各自自校验）。
#[test]
fn copy_cross_call_global_and_fs() {
    let dir = std::env::temp_dir().join(format!("mslang_copy_cross_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("src.txt").to_string_lossy().replace('\\', "/");
    let dst = dir.join("dst.txt").to_string_lossy().replace('\\', "/");
    let script = format!(
        r#"import fs
import io
a = copy([1, 2])
assert(len(a) == 2 and a[0] == 1 and a[1] == 2, "全局 copy 浅拷贝 list")
d = copy({{"k": 1}})
assert(d["k"] == 1, "全局 copy 浅拷贝 dict")
io.write_file("{src}", "data")
fs.copy("{src}", "{dst}")
assert(io.read_file("{dst}") == "data", "fs.copy 二参文件复制")
"#,
        src = src,
        dst = dst
    );
    let (code, _stdout, stderr) = ms_run("copy_cross", &script, None);
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(code, Some(0), "copy 同名共存失败, stderr: {}", stderr);
}
