# 标准库 - fs 模块 / os 扩充 / sys 模块

## 所属阶段
Phase 9 - 标准库扩展（M4）

## 前置任务
78-stdlib-split

> **依赖说明**：全部基于 std 标准库，零新依赖。fs 与既有 io 模块分工：
> io 保 read_file/write_file/exists/open（内容读写），fs 管结构与元数据。
> 设计总纲见 [16-stdlib-expansion](../16-stdlib-expansion.md) §4.7-4.9。

## 目标

1. 新增 `fs` 模块（17 个函数：目录/文件结构操作与元数据）。
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
- `abs` 用 `std::path::absolute`：不解析符号链接；Unix 保留 `..`，但 Windows 经
  GetFullPathNameW 会**词法归一 `..`**（平台差异，测试仅断言 `is_absolute` 不变量，
  10-builtins.md 注明）

### os 扩充

参照 [16-stdlib-expansion](../16-stdlib-expansion.md) §4.8：

| 函数 | 签名 | 说明 |
|---|---|---|
| getpid | () -> Int | |
| hostname | () -> string | env COMPUTERNAME/HOSTNAME；缺失 → IOError（Linux 非交互 shell/CI 下 HOSTNAME 常未导出，已知限制，10-builtins.md 注明） |
| environ | () -> dict | 全量环境变量快照；经 `vars_os` + `to_string_lossy` 构建（无效 Unicode 项不 panic） |
| unsetenv | (key) -> nil | |
| run | (argv) -> dict | `{"status","stdout","stderr"}`；argv 为 string list **不经 shell** |

- os.run 语义：空列表 / 非 string 元素 → TypeError；启动失败（可执行不存在）→ IOError；
  返回 status 为 Int（Unix 信号情形 platform 特定，统一映射为负值或 128+n，实现时以
  `ExitStatus` 序列化为准并在 10-builtins.md 注明）。
- os.exec（shell 字符串）保留不动；文档引导结构化场景用 os.run。
- os.run 同步阻塞（与 os.exec 一致，单线程协作事件循环；长命令饿死其他协程，
  10-builtins.md 注明）。

### sys

参照 [16-stdlib-expansion](../16-stdlib-expansion.md) §4.9：

| 函数 | 说明 |
|---|---|
| platform() | "windows" / "linux" / "macos"（cfg! 映射） |
| version() | "mslang 0.1.0"（env!("CARGO_PKG_VERSION")，与 Cargo.toml 自动同步） |
| executable() | current_exe 绝对路径；失败（二进制已删等）→ IOError |
| stdin_read_all() | 读 stdin 至 EOF（管道/重定向场景） |

## 实现细节

### 文件位置

- `src/vm/stdlib/fs.rs` — `register_fs_module` + 17 个 native 函数
- `src/vm/stdlib/os.rs` — 追加 5 个函数到既有 `register_os_module` exports
- `src/vm/stdlib/sys.rs` — `register_sys_module` + 4 个 native 函数
- `src/vm/stdlib/mod.rs` — `pub use` 转发
- `src/vm/mod.rs` — 注册 + `native_arities` 登记（os 侧 getenv 等既有不变；
  `run → 1`；`abs → 1` 与全局内置 abs（builtins 已注册 1）同名同 arity，无冲突；
  `size → 1`、`platform → 0` 唯一；**`copy` 与全局内置 `copy(val)`（已注册 1）同名
  不同 arity**，按总纲 §2.2 必须 `copy → usize::MAX`，fs.copy 自校验恰 2 参、
  全局 builtin copy 自校验 1 参，并补同名交叉调用回归用例）
- `docs/mslang/10-builtins.md` — 新增 fs/sys 章节、os 章节扩表（5 函数）与
  exec 注入警示引导至 os.run
- `docs/mslang/tasks/README.md` — task 82 状态 ⬜ → ✅

### walk 实现

显式栈迭代（避免递归深目录栈溢出），**pop 时输出**的 DFS 先序，输出**含 root 自身**
（首元素）：

```
out = []
stack = [root]
while let Some(p) = stack.pop():
    out.push(p)
    if p.is_dir() && !p.is_symlink():
        for e in read_dir(p).sorted().rev():
            stack.push(join(p, e))
```

- 排序 + 逆序压栈保证字典序最小子项先展开：输出为严格递归先序（父目录条目先于
  其子目录内容，且后继兄弟排在先前兄弟的子树之后，与 Go filepath.Walk 同序）。
- 与 list_dir 同一排序函数；以实现期单测锁定确切顺序。

### copy 实现

- `std::fs::copy`（Windows CopyFileExW / Unix copy_file_range 由 std 内部处理）；
  dst 为目录时 → IOError（不自动拼接文件名，显式优于隐式）。

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
8. `echo hello | ms run t.ms` 中 stdin_read_all() 去除行尾后 == "hello"
   （echo 必附 `\r\n`/`\n`，断言用 trim/包含，不得裸相等）
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

- `echo` 管道驱动 `ms run`，验证 stdin_read_all 输出（验证标准 8；断言去除行尾后
  相等——echo 必附 `\r\n`/`\n`）。
- 同名交叉调用回归：全局 `copy([1,2])` 与 `fs.copy(src, dst)` 并存均可用
  （§2.2 治理，MAX 下各自自校验）。

### 文件内 Rust 单元测试（fs.rs / os.rs / sys.rs 各含 `#[cfg(test)]`）

- 按总纲 §2.4 第 1 条：各模块文件内单测（fs round-trip、copy 同名治理、
  os.run 参数校验、sys.platform/version 等）；文件系统用例各用
  `std::env::temp_dir()` 下唯一子目录，测试后清理。
