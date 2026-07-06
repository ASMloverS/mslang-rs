# 标准库 - os/string/time/path 模块

## 所属阶段
Phase 6.2c - 标准库

## 前置任务
45-module-system

## 目标
实现 `os`、`string`、`time`、`path` 标准库模块，提供操作系统接口、字符串工具、时间函数和路径操作。

## 设计规格

参照 [10-builtins](../10-builtins.md) § os / string / time / path：

### os 模块

| 函数/属性 | 签名 | 说明 |
|---|---|---|
| `os.getenv(key)` | `(string) -> string?` | 获取环境变量 |
| `os.setenv(key, val)` | `(string, string) -> nil` | 设置环境变量 |
| `os.getcwd()` | `() -> string` | 当前工作目录 |
| `os.chdir(path)` | `(string) -> nil` | 改变工作目录 |
| `os.exec(cmd)` | `(string) -> string` | 经 shell 执行命令并返回 stdout（有注入风险，见 §2） |
| `os.exit(code)` | `(int) -> !` | 抛 `ExitUnwind` 异常触发 VM 清理（defer/finalizer）后退出 |
| `os.args` | 属性 | 命令行参数列表 |

### string 模块

| 函数 | 签名 | 说明 |
|---|---|---|
| `string.format(template, *args)` | `(string, ...) -> string` | 格式化字符串 |
| `string.repeat(s, n)` | `(string, int) -> string` | 重复字符串 |
| `string.reverse(s)` | `(string) -> string` | 反转字符串 |
| `string.is_alpha(s)` | `(string) -> bool` | 是否全为字母 |
| `string.is_digit(s)` | `(string) -> bool` | 是否全为数字 |

### time 模块

| 函数 | 签名 | 说明 |
|---|---|---|
| `time.now()` | `() -> float` | 当前时间戳（秒） |
| `time.sleep(secs)` | `(int/float) -> nil` | 休眠指定秒（与 `10-builtins.md:326` 一致；`async.sleep` 用毫秒，两模块语义不同） |
| `time.format(ts)` | `(float) -> string` | 格式化时间戳 |

### path 模块

| 函数 | 签名 | 说明 |
|---|---|---|
| `path.join(*parts)` | `(...string) -> string` | 连接路径 |
| `path.ext(p)` | `(string) -> string` | 获取扩展名 |
| `path.base(p)` | `(string) -> string` | 获取文件名 |
| `path.dir(p)` | `(string) -> string` | 获取目录部分 |

## 实现细节

> **对象模型约束**（task 20/25/46/47）：Object 枚举严格为 `{Nil, Bool, Int, Float, Ref}`，**无 `NativeFn` 变体**。原生函数经 `alloc_native_function(NativeFunction{name, func})` 包装为 `Object::Ref` + `TypeTag::FUNCTION`。`NativeFn` 签名为 `fn(&mut VM, &[Object]) -> Result<Object, String>`（切片，非 Vec）。Module 经 `alloc_module(name)` + `read_module_mut` 构造（无 `Module::new`）。字符串参数校验复用 task 46 的 `expect_string`（`src/vm/stdlib.rs:242-254`），**必须用 `args.get(N)`** 而非 `args[N]`。所有 `std::io`/`std::env`/`std::process` 调用统一 `.map_err(|e| format!("IOError: {}", e))?`。

### 1. 原生模块注册（4 模块）

4 个模块的 `register_*_module` 签名统一为 `pub fn register_*_module() -> *mut MsObjHeader`（无 vm 参数，参照 task 46/47）。以 os 为例（string/time/path 同模式）：

```rust
pub fn register_os_module() -> *mut MsObjHeader {
    let mut exports = std::collections::HashMap::new();
    exports.insert("getenv".to_string(), alloc_native_function(NativeFunction{ name: "getenv".to_string(), func: native_os_getenv }));
    exports.insert("setenv".to_string(), alloc_native_function(NativeFunction{ name: "setenv".to_string(), func: native_os_setenv }));
    exports.insert("getcwd".to_string(), alloc_native_function(NativeFunction{ name: "getcwd".to_string(), func: native_os_getcwd }));
    exports.insert("chdir".to_string(),  alloc_native_function(NativeFunction{ name: "chdir".to_string(),  func: native_os_chdir }));
    exports.insert("exec".to_string(),   alloc_native_function(NativeFunction{ name: "exec".to_string(),   func: native_os_exec }));
    exports.insert("exit".to_string(),   alloc_native_function(NativeFunction{ name: "exit".to_string(),   func: native_os_exit }));
    exports.insert("args".to_string(),   build_args_list());  // List 属性（非函数）
    let m = alloc_module("os");
    match m { Object::Ref(p) => { unsafe { read_module_mut(p).exports = exports; } p } _ => unreachable!() }
}
```

### 1b. ModuleResolver 集成

复用 task 46/47 建立的 `native_modules` 注册表。`VM::new`（紧随 math 注册）注册 4 模块：

```rust
// VM::new（src/vm/mod.rs，紧随 task 47 的 math 注册）
for (name, ptr) in [
    ("os", stdlib::register_os_module()),
    ("string", stdlib::register_string_module()),
    ("time", stdlib::register_time_module()),
    ("path", stdlib::register_path_module()),
] {
    vm.module_resolver.native_modules.insert(name.to_string(), ptr);
}
```

`import os` / `import @std time` / `from string import format` 均经 `VM::load_module` 顶部查 `native_modules` 命中（task 46 §1b 路径）。

### 1c. native_arities 注册

```rust
// VM::new（紧随 4 模块注册）
vm.native_arities.insert("getenv".to_string(), 1);
vm.native_arities.insert("setenv".to_string(), 2);
vm.native_arities.insert("getcwd".to_string(), 0);
vm.native_arities.insert("chdir".to_string(), 1);
vm.native_arities.insert("exec".to_string(), 1);
vm.native_arities.insert("exit".to_string(), 1);
vm.native_arities.insert("repeat".to_string(), 2);
vm.native_arities.insert("reverse".to_string(), 1);
vm.native_arities.insert("is_alpha".to_string(), 1);
vm.native_arities.insert("is_digit".to_string(), 1);
vm.native_arities.insert("now".to_string(), 0);
vm.native_arities.insert("sleep".to_string(), 1);
vm.native_arities.insert("format".to_string(), usize::MAX);  // 可变参（string.format + time.format 同名）
vm.native_arities.insert("ext".to_string(), 1);
vm.native_arities.insert("base".to_string(), 1);
vm.native_arities.insert("dir".to_string(), 1);
vm.native_arities.insert("join".to_string(), usize::MAX);     // 可变参（path.join）
```

> **同名冲突**：`string.format` 与 `time.format` 共享 `native_arities["format"]`（按名查询）。两者均为可变参（`usize::MAX`），无冲突。`format` 若已由全局注册则不覆盖。

### 2. os 模块函数实现

```rust
/// 构建 os.args 列表：std::env::args() → alloc_string → alloc_list。
/// 在 register_os_module 时调用一次，结果存入 exports（不需 vm）。
fn build_args_list() -> Object {
    let items: Vec<Object> = std::env::args().map(|a| alloc_string(&a)).collect();
    alloc_list(items)
}

fn native_os_getenv(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let key = expect_string(args.get(0), "getenv(key)")?;
    match std::env::var(&key) {
        Ok(val) => Ok(alloc_string(&val)),
        Err(_) => Ok(Object::Nil),  // 不存在返回 nil（非异常）
    }
}

fn native_os_setenv(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let key = expect_string(args.get(0), "setenv(key, val)")?;
    let val = expect_string(args.get(1), "setenv(key, val)")?;
    std::env::set_var(&key, &val);  // 安全：Rust 2024 需 unsafe，MVP 用 std::env::set_var
    Ok(Object::Nil)
}

fn native_os_getcwd(_vm: &mut VM, _args: &[Object]) -> Result<Object, String> {
    let dir = std::env::current_dir().map_err(|e| format!("IOError: {}", e))?;
    Ok(alloc_string(&dir.to_string_lossy()))
}

fn native_os_chdir(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let path = expect_string(args.get(0), "chdir(path)")?;
    std::env::set_current_dir(&path).map_err(|e| format!("IOError: {}", e))?;
    Ok(Object::Nil)
}
```

**`os.exec` 执行模型与安全警告**（V1/R1/B6）：

```rust
/// os.exec(cmd) → 经 shell 执行，返回 stdout。
/// ⚠️ 安全警告：cmd 经 shell（Windows cmd /C、Unix sh -c）执行，
/// 用户可控输入直接拼入 → 命令注入风险（10-builtins.md:303）。
/// 调用者须自行消毒输入。MVP 不提供 exec_split 安全变体。
fn native_os_exec(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let cmd = expect_string(args.get(0), "exec(cmd)")?;
    #[cfg(windows)]
    let output = std::process::Command::new("cmd").args(["/C", &cmd]).output();
    #[cfg(not(windows))]
    let output = std::process::Command::new("sh").args(["-c", &cmd]).output();
    let output = output.map_err(|e| format!("IOError: exec failed: {}", e))?;
    if !output.status.success() {
        return Err(format!("IOError: command failed (exit code {:?})", output.status.code()));
    }
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    Ok(alloc_string(&stdout))
}
```

**`os.exit` 语义**（V4/B7）：不直接调 `std::process::exit`（绕过 defer/GC）。改为抛特殊异常 `ExitUnwind`，VM `run()` 捕获后执行 defer 栈 + `run_finalizers`，再 `std::process::exit(code)`：

```rust
fn native_os_exit(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let code = match args.get(0) {
        Some(Object::Int(n)) => *n as i32,
        other => return Err(format!("TypeError: exit(code) expects int, got {}",
            other.map(|o| o.type_name()).unwrap_or("missing"))),
    };
    Err(format!("__EXIT__{}", code))  // 特殊标记，VM run loop 检测后清理+退出
}
```

### 3. string 模块函数实现

注册见 §1（`register_string_module()`）。函数实现：

```rust
/// string.format(template, *args) → 替换 {} 占位符。
/// 非 string 参数经 object_to_display_string 转换（与 print/str 一致）：
///   Int→"42", Float→"3.14", Bool→"true"/"false", Nil→"nil"。
fn native_string_format(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let template = expect_string(args.get(0), "format(template, ...)")?;
    let mut result = String::new();
    let mut arg_idx = 1;
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '{' && chars.peek() == Some(&'}') {
            chars.next();  // 消费 '}'
            let val = args.get(arg_idx).ok_or_else(|| format!(
                "ValueError: format: not enough arguments for placeholder #{}", arg_idx))?;
            result.push_str(&object_to_display_string(val));
            arg_idx += 1;
        } else {
            result.push(c);
        }
    }
    Ok(alloc_string(&result))
}

/// string.repeat(s, n) → s 重复 n 次。负数 / 超大 n → ValueError。
fn native_string_repeat(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let s = expect_string(args.get(0), "repeat(s, n)")?;
    let n = match args.get(1) {
        Some(Object::Int(n)) if *n >= 0 && *n <= 1_000_000 => *n as usize,
        Some(Object::Int(n)) if *n < 0 => return Err("ValueError: repeat count cannot be negative".into()),
        Some(Object::Int(_)) => return Err("ValueError: repeat count too large (max 1000000)".into()),
        other => return Err(format!("TypeError: repeat(s, n) expects int, got {}",
            other.map(|o| o.type_name()).unwrap_or("missing"))),
    };
    Ok(alloc_string(&s.repeat(n)))
}

fn native_string_reverse(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let s = expect_string(args.get(0), "reverse(s)")?;
    Ok(alloc_string(&s.chars().rev().collect::<String>()))
}

fn native_string_is_alpha(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let s = expect_string(args.get(0), "is_alpha(s)")?;
    Ok(Object::Bool(!s.is_empty() && s.chars().all(|c| c.is_alphabetic())))
}

fn native_string_is_digit(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let s = expect_string(args.get(0), "is_digit(s)")?;
    Ok(Object::Bool(!s.is_empty() && s.chars().all(|c| c.is_ascii_digit())))
}
```

> **`object_to_display_string`**：复用 Object 的 Display 实现（task 20，与 `print`/`str` 一致）。若该辅助未提取为公共函数，参照 `src/vm/object.rs` 的 `impl fmt::Display for Object`。
> **空字符串**：`is_alpha("")` / `is_digit("")` → `false`（空串不满足「全为字母/数字」）。

### 4. time 模块函数实现

注册见 §1（`register_time_module()`）。函数实现：

```rust
use std::time::{SystemTime, UNIX_EPOCH, Duration};

/// time.now() → 当前 Unix 时间戳（秒，f64）。
/// 不使用 .unwrap()：系统时间早于 epoch 时返回 Err 而非 panic（V2 修复）。
fn native_time_now(_vm: &mut VM, _args: &[Object]) -> Result<Object, String> {
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("ClockError: system time before epoch: {}", e))?;
    Ok(Object::Float(dur.as_secs_f64()))
}

/// time.sleep(secs) → 阻塞指定秒数（int 或 float）。
/// 单位为秒（与 10-builtins.md:326 一致，非毫秒）。
fn native_time_sleep(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let secs = match args.get(0) {
        Some(Object::Int(n)) => *n as f64,
        Some(Object::Float(x)) => *x,
        other => return Err(format!("TypeError: sleep(secs) expects number, got {}",
            other.map(|o| o.type_name()).unwrap_or("missing"))),
    };
    if secs < 0.0 {
        return Err("ValueError: sleep duration cannot be negative".into());
    }
    std::thread::sleep(Duration::from_secs_f64(secs));
    Ok(Object::Nil)
}

/// time.format(ts) → 将 Unix 时间戳格式化为 UTC 字符串 "YYYY-MM-DD HH:MM:SS"。
/// MVP 手动格式化（不引入 chrono 依赖，B8/R3 决策）。时区为 UTC。
fn native_time_format(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ts = match args.get(0) {
        Some(Object::Int(n)) => *n as f64,
        Some(Object::Float(x)) => *x,
        other => return Err(format!("TypeError: format(ts) expects number, got {}",
            other.map(|o| o.type_name()).unwrap_or("missing"))),
    };
    if ts < 0.0 {
        return Err("ValueError: timestamp cannot be negative".into());
    }
    let secs = ts as u64;
    let (year, month, day, hour, min, sec) = unix_to_ymdhms(secs);
    Ok(alloc_string(&format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        year, month, day, hour, min, sec)))
}
```

> **`unix_to_ymdhms`**：手动实现 Unix 时间戳 → UTC 年月日时分秒（民用历法算法，约 30 行）。不引入 chrono（避免增加依赖与编译时间）。Phase 8 工具链阶段若需时区/本地化支持再评估。
> **`time.sleep` 与 `async.sleep` 的单位差异**：`time.sleep(secs)` 用秒，`async.sleep(ms)`（task 61）用毫秒。两者独立，不共享 native_arities（不同模块名前缀）。

### 5. path 模块函数实现

注册见 §1（`register_path_module()`）。使用 `std::path::Path` / `PathBuf`。

> **跨平台分隔符**（R2）：`std::path` 在 Windows 产生反斜杠（`\`），Unix 产生正斜杠（`/`）。`path.join` 输出保留平台分隔符（**不归一化为 `/`**）。10-builtins.md:344 的 `"a/b/c"` 是 Unix 示例。mslang 字符串中反斜杠需转义（`"a\\b\\c"`），建议测试用正斜杠输入（`path.join("a", "b")`），`std::path` 在 Windows 也接受正斜杠输入。

```rust
/// path.join(*parts) → 连接路径段。可变参（arity = usize::MAX，见 §1c）。
fn native_path_join(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    if args.is_empty() {
        return Err("ValueError: path.join requires at least one argument".into());
    }
    let mut result = std::path::PathBuf::new();
    for (i, arg) in args.iter().enumerate() {
        let part = expect_string(Some(arg), &format!("path.join part #{}", i))?;
        result.push(&part);
    }
    Ok(alloc_string(&result.to_string_lossy()))
}

/// path.ext(p) → 扩展名（含 "."），无扩展名返回 ""。
fn native_path_ext(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let p = expect_string(args.get(0), "ext(p)")?;
    let ext = std::path::Path::new(&p).extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    Ok(alloc_string(&ext))
}

/// path.base(p) → 文件名部分，无文件名返回 ""。
fn native_path_base(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let p = expect_string(args.get(0), "base(p)")?;
    let base = std::path::Path::new(&p).file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(alloc_string(&base))
}

/// path.dir(p) → 目录部分，无父目录返回 ""。
fn native_path_dir(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let p = expect_string(args.get(0), "dir(p)")?;
    let dir = std::path::Path::new(&p).parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(alloc_string(&dir))
}
```

> **边界**：`path.ext("file")` → `""`（无扩展名）；`path.base("/")` → `""`（根目录无文件名）；`path.dir("file.txt")` → `""`（无目录部分）。`unwrap_or_default` 统一处理 `None` → 空串。

## 验证标准

1. `os.getcwd()` 返回当前目录
2. `os.getenv` 正确读取环境变量（不存在返回 nil）
3. `os.exec` 执行命令返回 stdout；失败返回 IOError（V1）
4. `os.exit(code)` 触发 defer 栈后退出（V4）
5. `string.format` 正确替换占位符；非 string 参数（Int/Float/Bool/Nil）正确转换（B9）
6. `string.repeat` / `string.reverse` 结果正确；负数 n → ValueError（V3）
7. `time.now()` 返回合理的时间戳（不 panic）
8. `time.sleep(secs)` 以**秒**为单位阻塞；负数 → ValueError（A4）
9. `time.format(ts)` 返回 `"YYYY-MM-DD HH:MM:SS"` UTC 格式（B5）
10. `path.join` 正确连接路径（输出含平台分隔符，见 §5）
11. `path.ext/base/dir` 正确解析路径各部分；边界（无扩展名/根目录）返回 `""`

## 测试用例

### test_os_string_time.ms

```ms
import os
import time
import path
import string

print(os.getcwd())
print(time.now())
print(path.join("a", "b", "c"))
print(path.ext("file.txt"))
print(path.base("a/b/c.txt"))
print(path.dir("a/b/c.txt"))
print(string.format("{} + {} = {}", 1, 2, 3))
print(string.repeat("ab", 3))
print(string.reverse("hello"))
print(string.is_alpha("abc"))
print(string.is_digit("123"))
```

预期输出：
```
<当前工作目录>
<当前时间戳>
a\b\c    （Windows）或 a/b/c （Unix）
.txt
c.txt
a\b      （Windows）或 a/b （Unix）
1 + 2 = 3
ababab
olleh
true
true
```
