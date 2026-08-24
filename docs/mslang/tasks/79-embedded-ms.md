# 标准库基础设施 - 嵌入式 .ms 模块

## 所属阶段
Phase 9 - 标准库扩展（M1）

## 前置任务
45-module-system, 78-stdlib-split

> **依赖说明**：复用 task 45 的 `ModuleResolver` 搜索/缓存/加载编排与
> `compile_module_source`。本 task 仅插入「嵌入式源码」这一新的模块来源层。
> 设计总纲见 [16-stdlib-expansion](../16-stdlib-expansion.md) §3.2。

## 目标

新增嵌入式 `.ms` 标准库机制：源码以 `include_str!` 编入二进制，
单二进制自足发行（无需部署 `stdlib/` 目录）。
为 task 84（collections/itertools/functools/test 四个 .ms 模块）提供载入基础。

## 设计规格

参照 [16-stdlib-expansion](../16-stdlib-expansion.md) §3.2：

### 解析顺序（扩展后）

```
native_modules（Rust 原生注册表）
  → 磁盘（当前目录 → stdlib/ → MS_PATH）     ← 既有顺序不变
  → embedded_modules（include_str! 嵌入源码）  ← 新增兜底层
```

用户当前目录同名 `.ms` 文件可覆盖嵌入版（便于调试与热修）。

### 嵌入源码位置

```
src/vm/stdlib/ms/
├── collections.ms   （task 84 填充）
├── itertools.ms     （task 84 填充）
├── functools.ms     （task 84 填充）
└── test.ms          （task 84 填充）
```

本 task 先建目录与占位模块（每个 `*.ms` 仅含 `const VERSION = "0.1.0"` 占位常量）
验证机制，实际内容由 task 84 实现。

### 关键机制

| 项 | 设计 |
|---|---|
| 注册表 | `ModuleResolver.embedded_modules: HashMap<String, &'static str>`（模块名 → 源码） |
| 缓存键 | 伪路径 `@embedded/<name>.ms`（PathBuf），与磁盘模块共用 `cache: HashMap<PathBuf, *mut MsObjHeader>` |
| 加载 | 命中 embedded 时走 `compile_module_source` + `VM::execute_module` 既有阶段化编排（resolve/登记与执行串行，借用模型不变） |
| safe_mode | embedded 视同 `@std` 来源，`MS_SAFE=1` 下允许导入；safe_mode 下非 `@std` import 的解析链为 native → stdlib_dir → embedded，**跳过用户可写根（当前目录、MS_PATH）**，防止恶意磁盘同名文件借嵌入名旁路（见实现细节 § safe_mode 编排） |
| `@std:` 前缀 | 剥离前缀后同样依次查 native → 磁盘 stdlib/ → embedded |

## 实现细节

### 文件位置

- `src/module/resolver.rs` — `ModuleResolver` 结构体加 `embedded_modules` 字段
  （`new()` / `with_config()` 初始化为空表）；`resolve()` 返回类型不变（磁盘路径），
  嵌入命中由 `VM::load_module` 编排层处理（新增 `resolve_embedded(name) -> Option<&'static str>`）
- `src/vm/stdlib/mod.rs` — 提供 `embedded_sources() -> HashMap<String, &'static str>`
  （`include_str!("ms/collections.ms")` 等，路径以本文件为基准）
- `src/vm/mod.rs` — `VM::new` 调用 `embedded_sources()` 填充
  `module_resolver.embedded_modules`；`load_module` 编排顺序调整
- `src/vm/stdlib/ms/*.ms` — 占位模块源码

### 与既有编排的衔接

task 45 将加载拆为「解析/缓存登记（持 `&mut self.module_resolver`）」与
「模块执行（持 `&mut self`）」两个串行阶段。嵌入模块沿用同一拆分：

1. `native_modules` 命中 → 直接返回缓存指针（现状）
2. 磁盘 `resolve()` 命中 → 既有路径（现状）。resolve 失败分支：**先查
   `resolve_embedded()`**（嵌入版优先于 task 72 的 capi 动态库兜底），再走 capi
3. `resolve_embedded()` 命中 → 计算伪路径 `@embedded/<name>.ms` → **cache 命中
   直接返回**（模块只执行一次）；未命中 → 登记 cache 空壳 → `execute_module`
   执行 → 成功保留 / 失败清理（与磁盘模块一致）

### safe_mode 编排

safe_mode 下非 `@std` import 不再无条件拒绝（现状 `vm/mod.rs` 安全检查在任何
解析之前拒绝一切非 `@std` 导入），改为**按来源放行**：解析链为 native →
stdlib_dir → embedded，**跳过用户可写根（当前目录、MS_PATH）**。即
`safe_mode && !stdlib_only` 时，`resolve()` 仅搜索 `stdlib_dir`（等价
`stdlib_only` 路径），未命中再查 `resolve_embedded()`；两者皆未命中 →
ImportError。如此 `MS_SAFE=1` 下 `import collections` 可用，且工作目录伪造的
`collections.ms` 无法借嵌入名旁路 safe_mode（磁盘用户根被跳过）。

### include_str 路径

`include_str!` 宏位于 `src/vm/stdlib/mod.rs`，路径以**宏所在文件**为基准
（cargo include 语义）：`ms/<name>.ms`（即 `src/vm/stdlib/ms/<name>.ms`）。
不得写 `stdlib/ms/<name>.ms`——在该位置会解析为 `src/vm/stdlib/stdlib/ms/...`。

## GC 安全

- 嵌入模块执行后产生的 Module 对象进入模块缓存（既有根集覆盖），无新增根集来源。
- `embedded_modules` 持 `&'static str`（编译期字符串），与 GC 无关。

## 验证标准

1. 占位模块 `import collections` 可用（导出占位常量可访问）
2. 同名磁盘模块优先于嵌入版：当前目录放置 `collections.ms` 覆盖后 import 到磁盘版
3. 嵌入模块缓存命中：重复 import 返回同一 Module 对象（`id()` 相同）
4. 循环导入 / 加载失败清理行为与磁盘模块一致（空壳移除、ImportError）
5. `MS_SAFE=1` 下 `import collections` 成功（经 native → stdlib_dir → embedded 链，
   跳过当前目录与 MS_PATH）；`import 用户模块` 仍被拒；当前目录放置伪造
   `collections.ms` 时 `import collections` **不得**加载磁盘版
6. `cargo test` 全绿
7. 同步更新 [09-modules](../09-modules.md) § 模块解析（补嵌入层解析顺序与
   `@embedded/` 缓存键说明，并澄清 safe_mode 放行集合：`@std` / native / embedded）
   与 [12-implementation-plan](../12-implementation-plan.md) 项目结构树（`ms/` 目录）

## 测试用例

### Rust 单测（resolver.rs / mod.rs）

- `test_embedded_basic` — 注册临时嵌入模块，import 并访问导出值
- `test_disk_overrides_embedded` — 临时目录同名 .ms 覆盖嵌入版
- `test_embedded_cache_hit` — 二次 import 返回同指针
- `test_embedded_safe_mode` — safe_mode 下嵌入可导入；另在临时目录放置伪造同名
  `collections.ms`，验证 safe_mode 下仍取嵌入版、非白名单用户模块被拒

### tests/ms/stdlib/test_embedded.ms

```ms
import collections   # 占位模块导出 VERSION 占位常量
assert(type(collections) == "module", "embedded module loadable")
assert(type(collections.VERSION) == "string", "placeholder constant accessible")
print("ALL PASSED")
```
