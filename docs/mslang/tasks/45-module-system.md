# 模块系统（import）

## 所属阶段
Phase 6.1 - 模块系统 + 标准库

## 前置任务
44-decorators

## 目标
实现完整的模块系统，支持 `import`、`from...import`、`import as`，包括模块搜索、加载、缓存、作用域隔离和导出规则。

## 设计规格

参照 [09-modules](../09-modules.md) § 模块解析、[11-bytecode-vm](../11-bytecode-vm.md) § IMPORT 指令：

### 语法规则

```
import_stmt = "import" module_path ("as" IDENTIFIER)?
            | "from" module_path "import" import_targets

module_path = IDENTIFIER ("." IDENTIFIER)*

import_targets = import_target ("," import_target)*
import_target = IDENTIFIER ("as" IDENTIFIER)?
```

### IMPORT 指令

| OpCode | 操作数 | 说明 |
|---|---|---|
| `IMPORT` | `module_idx(2)` | 导入模块，将 Module 对象压栈 |

### 搜索路径

按以下顺序搜索模块：
1. **当前目录** — 脚本所在目录
2. **标准库目录** — mslang 安装目录下的 `stdlib/`
3. **MS_PATH** — 环境变量指定的路径（用 `;` 分隔）

### 安全模块加载（`import @std`）

参照 [09-modules](../09-modules.md) § 安全模块加载：

```
import @std math         # 强制从标准库目录加载，跳过当前目录搜索
from @std io import open
```

`@std` 前缀确保只从标准库目录搜索模块，避免当前目录下的同名文件被恶意替换。在安全模式（`MS_SAFE=1`）下，所有非 `@std` 的 import 被禁止。

搜索规则更新：

```
import foo -> foo.ms, foo/index.ms, stdlib/foo.ms
import @std foo -> stdlib/foo.ms（仅标准库目录）
```

### 安全模式

参照 [09-modules](../09-modules.md)：

- 环境变量 `MS_SAFE=1` 启用安全模式
- CLI 参数 `ms run --safe` 启用安全模式
- 安全模式下：
  - 禁止 `os.exec()`
  - 禁止 `import` 非标准库模块（只允许 `import @std xxx`）
  - 禁止文件写入操作
  - 限制网络访问

### 搜索规则

```
import foo -> foo.ms, foo/index.ms, stdlib/foo.ms
import os.path -> os/path.ms, os/path/index.ms
import @std math -> stdlib/math.ms（仅标准库）
```

### 导出规则

- `fn`、`class`、`const` 顶层定义默认可被外部访问
- `var`、`:=`、`=` 顶层变量为模块私有

### 模块缓存

- 模块在**首次被导入**时执行其顶层代码，然后缓存
- 后续再次 import 同一模块不重新执行，直接返回缓存

### 模块作用域

- 每个模块有独立的全局作用域
- 模块内顶层变量相互隔离

## 实现细节

### 1. Module 对象

`src/vm/object.rs` 中新增（参照 [20-object-system-basic](./20-object-system-basic.md) MsStr、[40-class-definition](./40-class-definition.md) MsClass 的 `{ header, ... }` 布局）：

```rust
#[repr(C)]
pub struct MsModule {
    pub header:  MsObjHeader,   // type_tag = TypeTag::MODULE (10)
    pub name:    String,         // 模块名（如 "math_utils"）
    pub exports: HashMap<String, Object>,  // 可被外部访问（fn/class/const）
    pub globals: HashMap<String, Object>,  // 模块私有全局作用域（var/:=/=）
}
```

- `name`：模块名（如 `"math_utils"`）
- `exports`：可被外部访问的名称（fn, class, const）
- `globals`：模块自己的全局作用域（包含私有变量）

**辅助函数**（参照 task 20/22 的 `alloc_*`/`read_*` 模式；MVP 用 `Box::into_raw` 泄漏分配，task 52 GC 上线后由 §9 的 TypeDescriptor 接管）：

```rust
/// 分配空壳 Module（exports/globals 为空），返回 Object::Ref。load() 先建空壳再填充。
pub fn alloc_module(name: &str) -> Object {
    let m = Box::new(MsModule {
        header: MsObjHeader { gc_meta: 0, type_tag: TypeTag::MODULE as u8,
            size: size_of::<MsModule>() as u16, _padding: 0, class_ptr: 0 },
        name: name.to_string(), exports: HashMap::new(), globals: HashMap::new(),
    });
    Object::Ref(Box::into_raw(m) as *mut MsObjHeader)
}

/// # Safety: ptr 须指向 MsModule 且在调用期间有效。
pub unsafe fn read_module(ptr: *mut MsObjHeader) -> &MsModule { &*(ptr as *mut MsModule) }
pub unsafe fn read_module_mut(ptr: *mut MsObjHeader) -> &mut MsModule { &mut *(ptr as *mut MsModule) }
```

### 2. ModuleResolver

`src/module/resolver.rs`：

```rust
struct ModuleResolver {
    search_paths: Vec<PathBuf>,                // 当前目录 + stdlib + MS_PATH（按优先级）
    stdlib_dir: PathBuf,                       // 标准库目录（@std 前缀专用）
    cache: HashMap<PathBuf, *mut MsObjHeader>, // 键=规范化绝对路径；值指向 MsModule（TypeTag::MODULE）
    loading_stack: HashSet<PathBuf>,           // 正在加载中的模块（递归深度计数 + 循环诊断）
    safe_mode: bool,                           // MS_SAFE=1 或 ms run --safe
}
```

- `search_paths`：初始化时按优先级填入当前目录、stdlib/、MS_PATH
- `stdlib_dir`：标准库目录的独立引用，供 `@std` 前缀搜索（§6 resolve）
- `cache`：已加载模块缓存，**键为规范化绝对路径**（见 §5）；循环导入下空壳 Module 也暂存于此
- `loading_stack`：当前加载链；用于 `MAX_IMPORT_DEPTH` 深度限制与诊断
- `safe_mode`：启动时由 CLI/环境变量置位；为 true 时 `load` 拒绝非 `@std` 的 import
- `resolve(name: &str, stdlib_only: bool) -> Result<PathBuf>`：按搜索规则查找模块文件，返回规范化绝对路径（见 §6 resolve 实现）
- `load(name: &str, vm: &mut VM) -> Result<*mut MsObjHeader>`：编译并执行 .ms 文件，构建 Module 对象（TypeTag::MODULE）。`name` 可能带 `@std:` 前缀

### 3. 编译器改动

`src/compiler/mod.rs`：

- `import foo`：编译为 `IMPORT module_idx`，模块名 `"foo"` 存入常量池
- `import foo as bar`：编译为 `IMPORT module_idx` + `STORE_GLOBAL "bar"`
- `from foo import a, b as c`：编译为 `IMPORT module_idx` + `GET_ATTR "a"` + `STORE_GLOBAL "a"` + `GET_ATTR "b"` + `STORE_GLOBAL "c"`
- `import @std foo` / `from @std io import open`：编译器检测 AST 中 `@std` 标志（由 [15-parser-advanced-statements](./15-parser-advanced-statements.md) 的 `parse_import` 设置），常量池存入带前缀的模块名 `"@std:foo"`；其余编译路径不变。VM 侧 `IMPORT` 透明地以该常量为 `load` 入参，前缀解析在 `load` 内完成（见 §5）。

> **前缀编码选择**：IMPORT 操作数为单条 `module_idx(2)`（[11-bytecode-vm](../11-bytecode-vm.md)），无空闲标志位。将 `@std` 标志折叠进常量池字符串前缀（`"@std:"`）无需新增 opcode，且前缀经 `parse_std_prefix` 在 `load` 入口剥离，对搜索逻辑透明。

### 4. VM IMPORT 指令处理

`src/vm/mod.rs`：

```rust
OpCode::IMPORT => {
    let module_name = self.read_constant(idx);  // 可能含 "@std:" 前缀
    let name = match module_name { Object::Ref(p) => unsafe { read_str(p) }, _ => "" };
    // 安全点（14-gc.md § 安全点位置：IMPORT 可能触发 IO）
    self.check_safepoint();
    match self.module_resolver.load(name, self) {
        Ok(module_ptr) => self.stack.push(Object::Ref(module_ptr)),
        Err(msg) => {
            // Result<String> → 抛出 mslang 异常对象（ImportError，见 §异常注册）
            let exc = alloc_exception("ImportError", &msg, "", Object::Nil);
            return self.throw(exc);
        }
    }
}
```

> `load` 返回 `Result<_, String>`（字符串错误便于跨 resolver 边界传播）；IMPORT handler 统一将 `Err(msg)` 经 `alloc_exception` 构造 `ImportError` 实例并 `throw`，与 [37-try-except-finally](./37-try-except-finally.md) 的异常系统集成。`throw` 返回错误供 VM 执行循环进入 except 分派器。`module_resolver` 与 VM 的可变借用冲突经 §execute_module 全局隔离所述的 `execute_module` 设计规避（resolver 不在执行期持有 VM）。

### 5. 模块加载流程

1. **解析 `@std:` 前缀**：拆出 `stdlib_only` 标志与真实模块名
2. **安全模式检查**：`safe_mode` 为真且非 `@std` → 返回 `ImportError`
3. **深度限制**：`loading_stack.len() >= MAX_IMPORT_DEPTH` → 返回 `ImportError`（防线性依赖链栈溢出）
4. **解析路径**：`resolve(mod_name, stdlib_only)` 查找 .ms 文件并规范化绝对路径
5. **缓存命中 → 直接返回**：已加载完成返回完整 Module；循环导入下返回已预留的部分初始化空壳 Module（访问未初始化名称由 GET_ATTR 触发 `NameError`，参照 [09-modules](../09-modules.md) § 循环导入）
6. **预分配空 Module**：`alloc_module` 创建空壳，连同规范化路径加入 `cache` 与 `loading_stack`（支持循环导入部分访问）
7. **挂载清理 guard**：`LoadingGuard` 析构时总是移除 loading_stack 项；若失败（未 dismiss）额外移除 cache 条目，避免残留破损 Module
8. **读取源码**：`fs::read_to_string`
9. **编译**：Lexer → Parser → Compiler，生成字节码
10. **执行**：在新的隔离全局作用域中执行字节码（见 §execute_module 全局隔离）
11. **构建导出表**：扫描顶层定义，fn/class/const 加入 exports
12. **填充 Module**：写入 exports 与 globals（Phase 7.5 须经写屏障）
13. **卸载 guard**：`dismiss()` 标记成功；guard 析构时移除 loading_stack 项（不再「加载中」），保留 cache 条目（完整 Module 供后续 import 命中）

> 缓存键为 `resolve()` 返回的**规范化绝对路径**（`PathBuf`），而非 import 名——避免大小写不敏感文件系统（Windows/macOS）上 `import Foo` 与 `import foo` 解析到同一文件却分占两个条目，导致模块顶层代码执行两次（违反「模块只执行一次」）。循环导入下，被引用方在执行前即以空壳入 cache，因此**缓存命中**自然返回部分初始化的 Module（访问未初始化名称由 GET_ATTR 触发 `NameError`），无需额外的 loading_stack 查询分支。

```rust
const MAX_IMPORT_DEPTH: usize = 200;  // loading_stack 深度上限：防 N 层线性依赖链栈溢出

/// 解析 `@std:` 前缀（编译期由编译器写入常量池），返回 (是否仅标准库, 真实模块名)。
fn parse_std_prefix(name: &str) -> (bool, &str) {
    if let Some(rest) = name.strip_prefix("@std:") { (true, rest) }
    else { (false, name) }
}

fn load(&mut self, name: &str, vm: &mut VM) -> Result<*mut MsObjHeader, String> {
    let (stdlib_only, mod_name) = parse_std_prefix(name);

    // 安全模式（MS_SAFE=1 或 --safe）：仅允许 @std import
    if self.safe_mode && !stdlib_only {
        return Err(format!("ImportError: 安全模式下仅允许 import @std（拒绝 {}）", mod_name));
    }

    // 递归深度限制：load → execute_module → IMPORT → load 的递归链
    if self.loading_stack.len() >= MAX_IMPORT_DEPTH {
        return Err(format!("ImportError: 导入深度超过 {} 层，疑似无限递归", MAX_IMPORT_DEPTH));
    }

    // 解析为规范化绝对路径（兼作缓存键；处理 dotted path、包模块、@std 分支）
    let canon = self.resolve(mod_name, stdlib_only)?;

    // 缓存命中：已加载完成，或循环导入下尚未填充的空壳 Module
    if let Some(ptr) = self.cache.get(&canon) {
        return Ok(*ptr);
    }

    // 预分配空壳 Module 并登记。出错时由 LoadingGuard 的 Drop 清理，避免残留破损条目。
    let partial = alloc_module(mod_name);
    let partial_ptr = match partial {
        Object::Ref(p) => p,
        other => return Err(format!("ImportError: alloc_module 返回非 Ref（{:?}）", other)),
    };
    self.cache.insert(canon.clone(), partial_ptr);
    self.loading_stack.insert(canon.clone());
    let mut guard = LoadingGuard { resolver: self, key: canon.clone(), active: true };

    // 读取、编译、在隔离全局作用域中执行（任一步失败 → guard 清理后传播 Err）
    let source = std::fs::read_to_string(&canon)
        .map_err(|e| format!("ImportError: 无法加载 '{}': {}", mod_name, e))?;
    let unit = compile(&source, mod_name)
        .map_err(|e| format!("ImportError: 编译 '{}' 失败: {}", mod_name, e))?;
    let (exports, globals) = vm.execute_module(unit, mod_name)
        .map_err(|e| format!("ImportError: 执行 '{}' 失败: {}", mod_name, e))?;

    // 填充 Module（Phase 7.5 并发标记期间须改经写屏障，见 §GC 集成）
    unsafe {
        let module = read_module_mut(partial_ptr);
        module.exports = exports;
        module.globals = globals;
    }

    guard.dismiss();  // 成功：保留 cache 条目；loading_stack 经 guard.drop 移除
    Ok(partial_ptr)
}

/// RAII：load 返回时（成功或失败）总是从 loading_stack 移除（不再「加载中」）；
/// 仅在失败（未 dismiss）时额外移除 cache 中的破损空壳，避免后续 import 永久拿到空 Module。
struct LoadingGuard<'a> {
    resolver: &'a mut ModuleResolver,
    key: PathBuf,
    active: bool,  // true=未 dismiss（失败路径）
}
impl<'a> Drop for LoadingGuard<'a> {
    fn drop(&mut self) {
        self.resolver.loading_stack.remove(&self.key);   // 无论成败
        if self.active {                                  // 失败 → 清理破损 cache 条目
            self.resolver.cache.remove(&self.key);
        }
    }
}
impl<'a> LoadingGuard<'a> {
    fn dismiss(&mut self) { self.active = false; }
}
```

### 6. resolve 实现

`src/module/resolver.rs`。返回**规范化绝对路径**（`dunce::canonicalize` 或 `std::fs::canonicalize`，消除 `..`、符号链接、大小写歧义），兼作缓存键。

```rust
fn resolve(&self, name: &str, stdlib_only: bool) -> Result<PathBuf, String> {
    // dotted path: "os.path" → 段 ["os","path"]，对应 os/path.ms
    let segments: Vec<&str> = name.split('.').collect();

    // 候选搜索根：@std 仅标准库目录；否则 当前目录 → 标准库 → MS_PATH
    let roots: Vec<PathBuf> = if stdlib_only {
        vec![self.stdlib_dir.clone()]
    } else {
        self.search_paths.clone()  // 已按 当前目录 < stdlib < MS_PATH 优先级填入
    };

    for root in &roots {
        // 候选 1: root/<seg0>/<seg1>/.../<segN>.ms  （文件模块）
        let file = root.join(segments.join("/")).with_extension("ms");
        if file.is_file() {
            return canonicalize_or_err(&file, name);
        }
        // 候选 2: root/<seg0>/.../<segN>/index.ms   （包模块，仅当末段为目录）
        let pkg = root.join(segments.join("/")).join("index.ms");
        if pkg.is_file() {
            return canonicalize_or_err(&pkg, name);
        }
    }
    Err(format!("ImportError: 找不到模块 '{}'", name))
}

fn canonicalize_or_err(p: &Path, name: &str) -> Result<PathBuf, String> {
    p.canonicalize().map_err(|e| format!("ImportError: 解析 '{}' 失败: {}", name, e))
}
```

> 标识符受限（`[a-zA-Z_][a-zA-Z0-9_]*`，[01-lexical](../01-lexical.md)），模块名段不含 `..` 或绝对路径，故路径拼接天然免疫目录穿越。`canonicalize` 进一步消除符号链接别名。

### 7. execute_module 全局隔离

参照 [11-bytecode-vm](../11-bytecode-vm.md)，VM 仅有单一 `globals: HashMap<String, Object>`。模块要求独立全局作用域（[09-modules](../09-modules.md) § 模块作用域）。`execute_module(unit, name) -> Result<(exports, globals), String>` 实现：

```rust
fn execute_module(&mut self, unit: CompilationUnit, name: &str)
    -> Result<(HashMap<String, Object>, HashMap<String, Object>), String>
{
    // 1. 保存调用方 globals，切换为空表（隔离）
    let saved = std::mem::take(&mut self.globals);
    // 2. 在新 globals 中执行模块顶层字节码（IMPORT 会递归 load，但其 globals 切换嵌套保存）
    let result = self.run_unit(unit);  // 复用 run 循环；HALT 或顶层结束返回
    // 3. 无论成败恢复调用方 globals
    let module_globals = std::mem::replace(&mut self.globals, saved);
    let globals = module_globals?;
    // 4. 拆分导出：fn/class/const 入 exports，var/:=/= 留 globals（私有）
    let exports = split_exports(&globals, &unit.top_level_kinds);
    Ok((exports, globals))
}
```

`split_exports` 依编译单元记录的顶层定义种类（`fn`/`class`/`const` → 导出；`var`/`:=`/`=` → 私有）拆分。嵌套 import 的 globals 经 `saved` 栈式保存/恢复天然隔离。

> **借用说明**：`load` 持 `&mut ModuleResolver`、`execute_module` 持 `&mut VM`——二者为不同对象，不构成重叠借用。`load` 不在模块执行期访问 VM，故 §4 IMPORT handler 中 `self.module_resolver.load(name, self)` 的形式借用须经 `std::mem::take` 临时取出 resolver 或拆分 VM 字段以满足借用检查器（实现期决定，此处伪代码展示语义）。

### 8. GET_ATTR 对 MODULE 的处理

`from foo import a` 编译为 `GET_ATTR "a"`，需在 GET_ATTR handler 新增 MODULE 分支（参照 [37-try-except-finally](./37-try-except-finally.md) 为 EXCEPTION 加分支、[41-self-instance-attributes](./41-self-instance-attributes.md) 为 INSTANCE 加分支的同类扩展）：

```rust
Object::Ref(ptr) if (*ptr).type_tag == TypeTag::MODULE as u8 => {
    let module = unsafe { read_module(ptr) };
    if let Some(val) = module.exports.get(name) {
        self.stack.push(val.clone());
    } else {
        // 访问未导出或尚未初始化（循环导入）的名称 → NameError
        let exc = alloc_exception("NameError",
            &format!("模块 '{}' 没有 '{}'", module.name, name), "", Object::Nil);
        return self.throw(exc);
    }
}
```

### 9. GC 集成

参照 [14-gc](../14-gc.md) § 类型描述表与根集、[52-gc](./52-gc.md) `:227`（`fn trace_module(...) {} // TODO task 45`）与 `:11`（`module_cache` 根集增量扩展点）。本 task **必须**替换 task 52 留下的 noop 占位。

**Module 的 TypeDescriptor**（`src/vm/gc.rs`，替换 `10 => &MODULE_DESC` 占位）：

```rust
fn trace_module(obj: *mut MsObjHeader, cb: &mut dyn FnMut(*mut MsObjHeader)) {
    let m = unsafe { read_module(obj) };
    for v in m.exports.values().chain(m.globals.values()) {
        if let Object::Ref(child) = v { cb(*child); }
    }
}

fn forward_fields_module(obj: *mut MsObjHeader, forward: &dyn Fn(&mut *mut MsObjHeader)) {
    let m = unsafe { read_module_mut(obj) };
    for v in m.exports.values_mut().chain(m.globals.values_mut()) {
        if let Object::Ref(slot) = v { forward(slot); }
    }
}

fn copy_for_gc_module(src: *mut MsObjHeader, dst: *mut MsObjHeader) {
    unsafe {
        let s = read_module(src); let d = read_module_mut(dst);
        d.name = s.name.clone();
        d.exports = s.exports.clone();   // HashMap 深拷贝（值内 Ref 由 forward 修正）
        d.globals = s.globals.clone();
    }
}
```

> Module 无 `__del__`，`has_finalizer = false`，`finalize = None`。

**根集扫描**（`src/vm/gc.rs` 的 `scan_roots`，启用 task 52 feature-gated 的 `module_cache` 扫描块）：

```rust
// task 45：扫描模块缓存（此前为占位跳过）
for ptr in self.module_resolver.cache.values() {
    if !(*ptr).is_null() && (*ptr).color() == White {
        (*ptr).set_color(Gray);
        gray_queue.push(*ptr);
    }
}
```

**写屏障**：`load` 在 `read_module_mut` 填充 exports/globals 时（§5），若 GC 处于并发标记阶段，须改经 `write_barrier` 写入各 Ref 槽（[14-gc](../14-gc.md) § 混合写屏障）。MVP（STW）无此问题，Phase 7.5 实装时将该 `unsafe` 块替换为屏障包装。

### 10. 异常注册扩展

参照 [37-try-except-finally](./37-try-except-finally.md) § 异常层级注册（`init_exception_classes` 注册 12 个内置异常类）。本 task **扩展**异常层级，新增 `ImportError`（task 37 未含此类，但本 task 的 `load`/IMPORT handler 需抛出它）：

- 在 VM 初始化（或首次 IMPORT 前）调用 `alloc_exception_class("ImportError")`，父类为 `Error`（经 §5 静态 MRO 表登记 `("ImportError", "Error")`）。
- `globals.insert("ImportError", cls)` 使脚本可 `except ImportError`。

`NameError` 已由 task 37 注册，循环导入访问未初始化名称直接复用，无需新增。

### 11. MODULE 的 Display / type_name

扩展 [20-object-system-basic](./20-object-system-basic.md) 的 `Display` 与 `type_name` 的 `Ref` 分支（当前对非 String 的 Ref 返回 `<object:N>` / `"object"`）：

- `type(m)` → `"module"`
- `print(m)` / `str(m)` → `<module "math_utils">`（取 `Module.name`）

## 验证标准

1. `import module_name` 正确加载并执行模块文件
2. `module.name` 可访问模块导出的 fn/class/const
3. `from module import name` 正确提取指定名称
4. `import module as alias` 别名正常工作
5. 模块只执行一次（缓存生效）
6. 模块私有变量（var/:=/=）不可从外部访问
7. 包模块（目录 + index.ms）正确加载
8. 模块作用域相互隔离
9. `import @std math` 正确从标准库目录加载（跳过当前目录）
10. `from @std io import open` 正确加载标准库模块的指定名称
11. 安全模式（`MS_SAFE=1`）下，非 `@std` import 被拒绝
12. 循环导入（A imports B, B imports A）不导致死循环：部分初始化的模块可访问已定义名称，未定义名称抛出 NameError

## 测试用例

### math_utils.ms

```ms
const VERSION = "1.0"

fn add(a, b) {
    return a + b
}

fn multiply(a, b) {
    return a * b
}
```

### main.ms

```ms
import math_utils

print(math_utils.VERSION)
print(math_utils.add(3, 4))
print(math_utils.multiply(3, 4))
```

预期输出：
```
1.0
7
12
```

### from_import.ms

```ms
from math_utils import add, multiply as mul

print(add(1, 2))
print(mul(3, 4))
```

预期输出：
```
3
12
```

### 缓存测试 cache_test.ms

```ms
import math_utils
import math_utils

print("cache ok")
```

预期输出（math_utils 顶层代码只执行一次）：
```
cache ok
```

### 私有变量测试 private_test.ms

```ms
import math_utils

# VERSION 是 const，可访问
print(math_utils.VERSION)

# 以下应报错或返回 nil（私有变量不可访问）
# math_utils 内部如用 var 声明的变量，外部不应看到
```

### 包模块测试（验证 #7）

目录结构：

```
mylib/
├── index.ms        # fn lib_root() { return "root" }
└── utils.ms        # fn tool() { return "tool" }
```

```ms
# package_test.ms
import mylib
import mylib.utils

print(mylib.lib_root())      # root
print(mylib.utils.tool())    # tool
from mylib.utils import tool
print(tool())                # tool
```

### @std 加载测试（验证 #9、#10）

前置：当前目录存在恶意同名 `math.ms`（内容 `const FAKE = true`），stdlib/math.ms 为正式实现。

```ms
# std_import_test.ms
import @std math             # 强制标准库，跳过当前目录
print(type(math))            # module

from @std math import sqrt   # 从标准库模块导入指定名称
print(sqrt(16))              # 4（或 4.0，依 math 实现）
```

预期：`@std` 前缀确保不被当前目录 math.ms 覆盖（`math.FAKE` 不存在 → NameError）。

### 安全模式测试（验证 #11）

```ms
# safe_mode_test.ms
import math_utils            # 当前目录模块
```

运行：`MS_SAFE=1 ms run safe_mode_test.ms`

预期：抛出 `ImportError`（安全模式下仅允许 `import @std`），脚本不执行。

对照：`import @std math` 在 `MS_SAFE=1` 下正常加载。

### 循环导入测试（验证 #12）

```ms
# cycle_a.ms
import cycle_b
fn hello() { return "from a" }
print(cycle_b.world())       # from b（b 此时已加载完成）
```

```ms
# cycle_b.ms
import cycle_a
fn world() { return "from b" }
# 访问 cycle_a.hello()：a 尚未加载完成 → 部分初始化
try {
    cycle_a.hello()
} except NameError {
    print("a 未完成初始化")    # 预期：捕获 NameError
}
```

运行：`ms run cycle_a.ms`

预期输出：
```
a 未完成初始化
from b
```

> 循环导入不死循环；访问尚未初始化的导出名称抛出 `NameError`（经 §8 GET_ATTR MODULE 分支），已被 `try/except` 捕获。

### dotted path 测试

```ms
# dotted_test.ms
import os.path
print(type(os.path))         # module
```

预期：解析为 `os/path.ms`（或 `os/path/index.ms`），`os.path` 作为模块对象可访问其导出。

### 深度限制测试

构造 201 层线性依赖链（`d0` import `d1`，…，`d200` import `d201`），`ms run d0.ms` 预期抛出 `ImportError: 导入深度超过 200 层`，无栈溢出。
