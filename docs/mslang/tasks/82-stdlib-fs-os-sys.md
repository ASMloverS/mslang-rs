# 标准库 - fs 模块 / os 扩充 / sys 模块

## 所属阶段
Phase 9 - 标准库扩展（M4）

## 前置任务
78-stdlib-split

> **依赖说明**：全部基于 std 标准库，零新依赖。fs 与既有 io 模块分工：
> io 保 read_file/write_file/exists/open（内容读写），fs 管结构与元数据。
> 设计总纲见 [16-stdlib-expansion](../16-stdlib-expansion.md) §4.7-4.9。

## 目标

1. 新增 `fs` 模块（15 个函数：目录/文件结构操作与元数据）。
2. `os` 扩充 5 个函数（getpid/hostname/environ/unsetenv/run）。
3. 新增 `sys` 模块（4 个函数）。

## 设计规格

### fs

参照 [16-stdlib-expansion](../16-stdlib-expansion.md) §4.7（错误一律 IOError 前缀）：

mkdir/mkdirs/rmdir/remove/remove_all/rename/copy/list_dir/walk/is_dir/is_file/is_abs/
abs/size/mtime/temp_dir/home_dir。

- `mkdir` 已存在 → IOError；`mkdirs` 幂等（已存在目录成功）
- `remove_all` 路径不存在返回 nil（幂等，Go RemoveAll）
- `list_dir` 返回**排序后**子项文件名列表（跨平台确定性，不含 `.`/`..`）
- `walk` 递归先序返回全路径扁平 list（目录+文件），不跟随符号链接
- `temp_dir()` / `home_dir()` 无参；home 缺失（env USERPROFILE/HOME 均无）→ IOError

### os 扩充

参照 [16-stdlib-expansion](../16-stdlib-expansion.md) §4.8：

| 函数 | 签名 | 说明 |
|---|---|---|
| getpid | () -> Int | |
| hostname | () -> string | env COMPUTERNAME/HOSTNAME；缺失 → IOError |
| environ | () -> dict | 全量环境变量快照 |
| unsetenv | (key) -> nil | |
| run | (argv) -> dict | `{"status","stdout","stderr"}`；argv 为 string list **不经 shell** |

- os.run 语义：空列表 / 非 string 元素 → TypeError；启动失败（可执行不存在）→ IOError；
  返回 status 为 Int（Unix 信号情形 platform 特定，统一映射为负值或 128+n，实现时以
  `ExitStatus` 序列化为准并在 10-builtins.md 注明）。
- os.exec（shell 字符串）保留不动；文档引导结构化场景用 os.run。

### sys

参照 [16-stdlib-expansion](../16-stdlib-expansion.md) §4.9：

| 函数 | 说明 |
|---|---|
| platform() | "windows" / "linux" / "macos"（cfg! 映射） |
| version() | "mslang 0.1.0"（env!("CARGO_PKG_VERSION")，与 Cargo.toml 自动同步） |
| executable() | current_exe 绝对路径 |
| stdin_read_all() | 读 stdin 至 EOF（管道/重定向场景） |

## 实现细节

### 文件位置

- `src/vm/stdlib/fs.rs` — `register_fs_module` + 15 个 native 函数
- `src/vm/stdlib/os.rs` — 追加 5 个函数到既有 `register_os_module` exports
- `src/vm/stdlib/sys.rs` — `register_sys_module` + 4 个 native 函数
- `src/vm/stdlib/mod.rs` — `pub use` 转发
- `src/vm/mod.rs` — 注册 + `native_arities` 登记（fs/sys 新函数逐个；
  os 侧 getenv 等既有不变；`run → 1`；注意 `abs → 1` 与 math 无同名冲突、
  `size → 1` 唯一、`platform → 0` 唯一）

### walk 实现

显式栈迭代（避免递归深目录栈溢出）：

```
stack = [root]
while let Some(dir) = stack.pop():
    entries = read_dir(dir) 排序
    for e in entries:
        path = dir + e
        out.push(path)
        if e.is_dir() && !e.is_symlink(): stack.push(path)
```

- 排序保证输出顺序确定（与 list_dir 同一排序函数）。
- 先序语义：父目录条目先于其子目录内容（栈实现时注意压入顺序反转，
  保证字典序小的子目录先展开；以实现期单测锁定确切顺序）。

### copy 实现

- `std::fs::copy`（12.8 万字节缓冲由 std 内部处理）；dst 为目录时 → IOError
  （不自动拼接文件名，显式优于隐式）。

### os.run 实现

- `std::process::Command::new(argv[0]).args(&argv[1..]).output()`
- stdout/stderr `String::from_utf8_lossy`（与 os.exec 一致）

### stdin_read_all

- `std::io::Read::read_to_string`；非 UTF-8 → IOError（lossy 不可逆，宁可报错）
- 注意：交互 REPL 场景会阻塞至 EOF；仅面向 `ms run script.ms < input` 管道用例

## GC 安全

- 全部函数返回值经 `alloc_string`/`alloc_list`/`alloc_dict` 既有路径分配；无新根集。
- environ() 逐条 `alloc_string` 后 `alloc_dict`，一次构建。

## 验证标准

1. mkdir/mkdirs/rmdir/remove/rename/copy round-trip（临时目录内完成）
2. mkdir 已存在 → IOError；mkdirs 已存在成功；remove_all 不存在返回 nil
3. list_dir 排序确定；walk 输出先序且与 list_dir 顺序规则一致
4. is_dir/is_file/is_abs/abs/size/mtime 语义抽查（size 与写入字节数一致）
5. os.getpid() > 0；os.environ() 含 PATH 键；unsetenv 后 getenv 返回 nil
6. os.run(["cmd","/C","echo","hi"])（Windows）/ os.run(["sh","-c","echo hi"])（Unix）
   status==0 且 stdout 含 hi；os.run([]) → TypeError；os.run(["no_such_exe_xyz"]) → IOError
7. sys.platform() ∈ {windows, linux, macos}；sys.version() 以 "mslang" 开头；
   sys.executable() 非空
8. `echo hello | ms run t.ms` 中 stdin_read_all() == "hello"
9. 全部文件系统用例使用 `std::env::temp_dir()` 下唯一子目录，测试后清理（Rust 侧
   与 .ms 侧均遵守；ms 语料清理失败不判失败——CI 临时目录容错）
10. `cargo test` 全绿

## 测试用例

### tests/ms/stdlib/test_fs.ms

验证标准 1-4（assert + ALL PASSED；临时目录经 `fs.temp_dir()` + uuid4 后缀构造唯一路径）。

### tests/ms/stdlib/test_os_ext.ms

验证标准 5-6（os.run 用条件分支按 `sys.platform()` 分平台选命令）。

### tests/ms/stdlib/test_sys.ms

验证标准 7（stdin_read_all 的管道用例放 Rust 集成测试，见下）。

### Rust 集成测试（tests/ 内新增 `sys_stdin.rs` 或并入 ms_corpus 辅助）

- `echo` 管道驱动 `ms run`，验证 stdin_read_all 输出（验证标准 8）。
