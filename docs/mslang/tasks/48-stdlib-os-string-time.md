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
| `os.exec(cmd)` | `(string) -> string` | 执行命令并返回输出 |
| `os.exit(code)` | `(int) -> !` | 退出程序 |
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
| `time.sleep(ms)` | `(int) -> nil` | 休眠指定毫秒 |
| `time.format(ts)` | `(float) -> string` | 格式化时间戳 |

### path 模块

| 函数 | 签名 | 说明 |
|---|---|---|
| `path.join(*parts)` | `(...string) -> string` | 连接路径 |
| `path.ext(p)` | `(string) -> string` | 获取扩展名 |
| `path.base(p)` | `(string) -> string` | 获取文件名 |
| `path.dir(p)` | `(string) -> string` | 获取目录部分 |

## 实现细节

### 1. os 模块

`src/vm/stdlib.rs`：

```rust
fn register_os_module(vm: &mut VM) -> Gc<Module> {
    let mut exports = HashMap::new();
    exports.insert("getenv".into(), Object::NativeFn(native_os_getenv));
    exports.insert("setenv".into(), Object::NativeFn(native_os_setenv));
    exports.insert("getcwd".into(), Object::NativeFn(native_os_getcwd));
    exports.insert("chdir".into(), Object::NativeFn(native_os_chdir));
    exports.insert("exec".into(), Object::NativeFn(native_os_exec));
    exports.insert("exit".into(), Object::NativeFn(native_os_exit));
    exports.insert("args".into(), build_args_list(vm));
    Module::new("os", exports)
}
```

- `getenv`：`std::env::var`，不存在返回 nil
- `setenv`：`std::env::set_var`
- `getcwd`：`std::env::current_dir` → 转为字符串
- `chdir`：`std::env::set_current_dir`
- `exec`：`std::process::Command::new("cmd").args(...).output()`，Windows 使用 `cmd /C`
- `exit`：`std::process::exit`
- `args`：`std::env::args().collect()` → 转为 mslang List

### 2. string 模块

```rust
fn register_string_module(vm: &mut VM) -> Gc<Module> {
    let mut exports = HashMap::new();
    exports.insert("format".into(), Object::NativeFn(native_string_format));
    exports.insert("repeat".into(), Object::NativeFn(native_string_repeat));
    exports.insert("reverse".into(), Object::NativeFn(native_string_reverse));
    exports.insert("is_alpha".into(), Object::NativeFn(native_string_is_alpha));
    exports.insert("is_digit".into(), Object::NativeFn(native_string_is_digit));
    Module::new("string", exports)
}
```

- `format`：将 `{}` 占位符替换为参数字符串
- `repeat`：`str.repeat(n)`
- `reverse`：`str.chars().rev().collect()`
- `is_alpha`：`str.chars().all(|c| c.is_alphabetic())`
- `is_digit`：`str.chars().all(|c| c.is_ascii_digit())`

### 3. time 模块

```rust
fn register_time_module(vm: &mut VM) -> Gc<Module> {
    let mut exports = HashMap::new();
    exports.insert("now".into(), Object::NativeFn(native_time_now));
    exports.insert("sleep".into(), Object::NativeFn(native_time_sleep));
    exports.insert("format".into(), Object::NativeFn(native_time_format));
    Module::new("time", exports)
}
```

- `now`：`std::time::SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs_f64()`
- `sleep`：`std::thread::sleep(Duration::from_millis(ms))`
- `format`：使用 `chrono` crate 或手动格式化（MVP 可用简单格式）

### 4. path 模块

```rust
fn register_path_module(vm: &mut VM) -> Gc<Module> {
    let mut exports = HashMap::new();
    exports.insert("join".into(), Object::NativeFn(native_path_join));
    exports.insert("ext".into(), Object::NativeFn(native_path_ext));
    exports.insert("base".into(), Object::NativeFn(native_path_base));
    exports.insert("dir".into(), Object::NativeFn(native_path_dir));
    Module::new("path", exports)
}
```

- 使用 `std::path::PathBuf` 和 `std::path::Path`：
- `join`：`parts.iter().fold(PathBuf::new(), |p, s| p.join(s))`
- `ext`：`Path::new(p).extension().unwrap_or("").to_string()`
- `base`：`Path::new(p).file_name().unwrap_or("").to_string()`
- `dir`：`Path::new(p).parent().unwrap_or(Path::new("")).to_string()`

## 验证标准

1. `os.getcwd()` 返回当前目录
2. `os.getenv` 正确读取环境变量
3. `string.format` 正确替换占位符
4. `string.repeat` / `string.reverse` 结果正确
5. `time.now()` 返回合理的时间戳
6. `time.sleep` 正确阻塞指定时间
7. `path.join` 正确连接路径（注意跨平台分隔符）
8. `path.ext/base/dir` 正确解析路径各部分

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
