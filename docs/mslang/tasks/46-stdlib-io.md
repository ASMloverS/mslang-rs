# 标准库 - io 模块

## 所属阶段
Phase 6.2a - 标准库

## 前置任务
45-module-system

## 目标
实现 `io` 标准库模块，提供文件读写、文件句柄对象和 `with` 上下文管理器支持。

## 设计规格

参照 [10-builtins](../10-builtins.md) § io：

### io 模块 API

| 函数 | 签名 | 说明 |
|---|---|---|
| `io.open(path, mode?)` | `(string, string?) -> FileHandle` | 打开文件，默认 "r" |
| `io.read_file(path)` | `(string) -> string` | 一次性读取整个文件 |
| `io.write_file(path, content)` | `(string, string) -> nil` | 一次性写入文件 |
| `io.exists(path)` | `(string) -> bool` | 检查文件是否存在 |

### FileHandle 对象

| 方法 | 说明 |
|---|---|
| `f.read()` | 读取全部内容 |
| `f.write(content)` | 写入内容 |
| `f.close()` | 关闭文件句柄 |
| `f.lines()` | 按行读取，返回迭代器 |

FileHandle 需实现 `__enter__` / `__exit__` 魔术方法以支持 `with` 语句。

## 实现细节

### 1. 原生 Rust 模块注册

`src/vm/stdlib.rs` 中注册 `io` 模块：

```rust
fn register_io_module(vm: &mut VM) -> *mut MsObjHeader {  // 返回指向 MsModule 的指针
    let mut exports = HashMap::new();
    exports.insert("open".into(), Object::NativeFn(native_io_open));
    exports.insert("read_file".into(), Object::NativeFn(native_io_read_file));
    exports.insert("write_file".into(), Object::NativeFn(native_io_write_file));
    exports.insert("exists".into(), Object::NativeFn(native_io_exists));
    Module::new("io", exports)
}
```

- 使用 Rust `std::fs` 实现文件操作
- `NativeFn` 为 Rust 原生函数类型，签名 `fn(&mut VM, Vec<Object>) -> Result<Object>`

### 2. FileHandle 对象

`src/vm/object.rs` 新增：

```rust
struct FileHandle {
    path: String,
    mode: String,
    file: RefCell<Option<std::fs::File>>,
}
```

- `read()`：调用 `std::io::Read::read_to_string`
- `write(content)`：调用 `std::io::Write::write_all`
- `close()`：将 `file` 设为 `None`
- `lines()`：按行分割内容，返回列表或迭代器

### 3. 上下文管理器支持

FileHandle 实现 `__enter__` / `__exit__`：

- `__enter__(self)`：返回 self
- `__exit__(self, err)`：调用 `self.close()`

这样 `with io.open("file.txt") as f { ... }` 可正常工作。

### 4. io.open 实现

```rust
fn native_io_open(vm: &mut VM, args: Vec<Object>) -> Result<Object> {
    let path = expect_string(&args[0])?;
    let mode = if args.len() > 1 { expect_string(&args[1])? } else { "r" };
    let file = std::fs::File::open(&path)?;  // 或 OpenOptions
    let handle = FileHandle::new(path, mode, file);
    Ok(alloc_file_handle(handle))  // 返回 Object::Ref，type_tag 为自定义扩展标签
}
```

### 5. io.read_file / io.write_file 实现

```rust
fn native_io_read_file(vm: &mut VM, args: Vec<Object>) -> Result<Object> {
    let path = expect_string(&args[0])?;
    let content = std::fs::read_to_string(&path)?;
    Ok(alloc_string(&content))
}

fn native_io_write_file(vm: &mut VM, args: Vec<Object>) -> Result<Object> {
    let path = expect_string(&args[0])?;
    let content = expect_string(&args[1])?;
    std::fs::write(&path, content)?;
    Ok(Object::Nil)
}
```

### 6. io.exists 实现

```rust
fn native_io_exists(vm: &mut VM, args: Vec<Object>) -> Result<Object> {
    let path = expect_string(&args[0])?;
    Ok(Object::Bool(std::path::Path::new(&path).exists()))
}
```

## 验证标准

1. `io.write_file` 正确写入文件内容
2. `io.read_file` 正确读取文件内容
3. `io.exists` 正确判断文件是否存在
4. `io.open` 返回 FileHandle 对象
5. FileHandle 的 `read/write/close/lines` 方法正确工作
6. `with io.open(...) as f { ... }` 自动关闭文件
7. 对不存在文件的读取操作返回合理错误

## 测试用例

### test_io.ms

```ms
import io

io.write_file("test_io.txt", "hello\nworld\n")
print(io.exists("test_io.txt"))

with io.open("test_io.txt") as f {
    print(f.read())
}

content = io.read_file("test_io.txt")
print(content)
```

预期输出：
```
true
hello
world

hello
world

```

### test_io_lines.ms

```ms
import io

io.write_file("test_lines.txt", "line1\nline2\nline3\n")

with io.open("test_lines.txt") as f {
    for line in f.lines() {
        print(">> " + line)
    }
}
```

预期输出：
```
>> line1
>> line2
>> line3
```
