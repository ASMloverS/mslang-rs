# 标准库重构 - stdlib.rs 目录拆分

## 所属阶段
Phase 9 - 标准库扩展（M0）

## 前置任务
49-stdlib-json, 60-stdlib-gc, 61-stdlib-async

> **依赖说明**：本 task 为纯移动重构，不新增任何功能。为后续 task 79-86 的实现铺路
> （`src/vm/stdlib.rs` 已 5312 行，继续追加将膨胀至 8000+ 行）。
> 设计总纲见 [16-stdlib-expansion](../16-stdlib-expansion.md) §3.1。

## 目标

将 `src/vm/stdlib.rs` 拆分为 `src/vm/stdlib/` 目录，每模块一文件，
**零行为变更**（编译产物语义等价，`cargo test` 全绿）。

## 设计规格

参照 [16-stdlib-expansion](../16-stdlib-expansion.md) §3.1：

```
src/vm/stdlib/
├── mod.rs        # 公共 helper + pub use 各子模块 register_*；对外引用路径不变
├── io.rs         # register_io_module / native_io_* / lookup_file_method / native_fh_*
├── math.rs       # register_math_module / native_math_* / expect_number / float_to_int
├── os.rs         # register_os_module / native_os_* / build_args_list
├── string.rs     # register_string_module / native_string_* / lookup_string_method
├── time.rs       # register_time_module / native_time_* / unix_to_ymdhms
├── path.rs       # register_path_module / native_path_*
├── json.rs       # register_json_module / native_json_* / 解析器与序列化器
├── gc.rs         # register_gc_module / native_gc_*
└── async.rs      # register_async_module / async_sleep / async_timeout / rejected_future
```

## 实现细节

### mod.rs 职责

- 汇集各子模块的 `pub fn register_*_module()`，以 `pub use` 转发；
  `src/vm/mod.rs` 中 `stdlib::register_io_module()` 等**调用点零改动**。
- 公共 helper（被 ≥2 个模块使用）上移至 mod.rs：
  - `expect_string` / `expect_number` / `float_to_int`（现为 stdlib.rs 私有 fn）
- 仅单模块使用的 helper（如 `unix_to_ymdhms`、`canonicalize` 类）留在各模块文件内私有。

### 可见性规则

- 跨文件引用的私有项提升为 `pub(super)`（crate 内部可见性最小化原则）。
- `alloc_dict` / `alloc_list` 等 object.rs 既有 pub API 引用方式不变。

### 测试迁移

- stdlib.rs:2496 起的 `#[cfg(test)] mod tests` 按被测函数归属拆入对应模块文件；
  测试公共 util（若有）置于 mod.rs 的 `#[cfg(test)]` 模块。

### 不变式

- 不修改任何函数体逻辑、不重排注册顺序、不改 `native_arities` 登记。
- `git diff` 审查时每个函数应为纯移动（允许 use 路径与可见性修饰调整）。

## 验证标准

1. `cargo build` 编译通过，无新 warning
2. `cargo test` 全绿（Rust 单测 + ms_corpus 全量回归 + gc/capi 相关测试）
3. `cargo build --features capi` 通过（capi 引用 stdlib 符号若有需同步调整）
4. 拆分后各文件行数 ≤ 1500（json.rs 预计最大 ~1000 行）
5. 无任何 `.ms` 语料测试结果变化（ms_corpus 输出零 diff）

## 测试用例

无新增测试用例；依赖既有测试全量回归作为等价性证明。
