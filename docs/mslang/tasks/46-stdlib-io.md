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

`src/vm/stdlib.rs` 中注册 `io` 模块。

> **对象模型约束**（task 20/25）：Object 枚举严格为 `{Nil, Bool, Int, Float, Ref}`，**无 `NativeFn` 变体**。原生函数经 `alloc_native_function(NativeFunction{name, func})` 包装为 `Object::Ref` + `TypeTag::FUNCTION`。`NativeFn` 签名为 `fn(&mut VM, &[Object]) -> Result<Object, String>`（切片，非 Vec；task 25:102）。Module 经 task 45 的 `alloc_module(name)` 构造（无 `Module::new(name, exports)`）。

```rust
fn register_io_module(vm: &mut VM) -> *mut MsObjHeader {
    let mut exports = HashMap::new();
    // alloc_native_function 返回 Object::Ref（TypeTag::FUNCTION），存入 Module 的 exports
    exports.insert("open".to_string(),      alloc_native_function(NativeFunction{ name: "open".to_string(),      func: native_io_open }));
    exports.insert("read_file".to_string(),  alloc_native_function(NativeFunction{ name: "read_file".to_string(),  func: native_io_read_file }));
    exports.insert("write_file".to_string(), alloc_native_function(NativeFunction{ name: "write_file".to_string(), func: native_io_write_file }));
    exports.insert("exists".to_string(),     alloc_native_function(NativeFunction{ name: "exists".to_string(),     func: native_io_exists }));
    let m = alloc_module("io");  // task 45：返回空壳 MsModule 的 Object::Ref
    if let Object::Ref(p) = m {
        unsafe { read_module_mut(p).exports = exports; }
        p
    } else { unreachable!("alloc_module must return Ref") }
}
```

- 使用 Rust `std::fs` 实现文件操作
- native 函数的 arity 校验：`io.open`（2，可变）、`io.read_file`（1）、`io.write_file`（2）、`io.exists`（1）须在 VM 的 `native_arities` 表登记（参照 task 25 CALL 扩展）。由于经 `module.fn(...)` 调用走 GET_ATTR→CALL，arity 校验在 CALL BOUND_METHOD/native 路径生效（见 §7 方法分派）。

### 1b. 原生模块与 ModuleResolver 集成

task 45 的 `ModuleResolver`（`src/module/resolver.rs`）仅搜 `.ms` 文件，**无原生模块注册表**。本 task 须扩展：

1. `ModuleResolver` 新增字段 `native_modules: HashMap<String, *mut MsObjHeader>`（键为规范模块名，如 `"io"`）。
2. `VM::load_module`（`src/vm/mod.rs:2808`）在**解析路径前**先查 `module_resolver.native_modules`：命中则直接返回缓存的 MsModule 指针，跳过磁盘搜索与执行。
3. `VM::new` 初始化时调用 `register_io_module` 并 `module_resolver.native_modules.insert("io".to_string(), ptr)`。
4. `@std` 交互：`import @std io` 经 `parse_std_prefix` 剥离前缀后得 `"io"`，同样命中 `native_modules`（原生模块不区分 @std，注册表查找在 `@std:` 剥离之后）。

```rust
// VM::load_module 顶部（src/vm/mod.rs，resolve() 之前）
let (stdlib_only, mod_name) = module::parse_std_prefix(name);
if let Some(ptr) = self.module_resolver.native_modules.get(mod_name) {
    return Ok(*ptr);  // 命中原生模块，跳过磁盘
}
// ... 原 safe_mode / depth / resolve 流程 ...
```

> 同时更新全局 `builtin_open`（task 25 占位 `src/vm/builtins.rs:685`，返 Err task 46）：本 task 将其改为委托 `native_io_open`（`10-builtins.md:65`：全局 `open()` 是 `io.open()` 的快捷方式）。

### 2. FileHandle 对象

> **TypeTag 新增**（`src/vm/object.rs`，task 20 TypeTag 枚举）：`FILE_HANDLE = 20`。须同步在 `src/vm/gc.rs` 的 TypeDescriptor 表注册（见 §8 GC 集成）。

`src/vm/object.rs` 新增（参照 task 20 MsStr / task 22 集合的 `{ header, data_ptr }` 二级分配模式）：

```rust
#[repr(C)]
pub struct MsFileHandle {
    pub header:    MsObjHeader,           // type_tag = TypeTag::FILE_HANDLE (20)
    pub path_ptr:  *const u8,             // 路径字符串（二级堆分配，Box::into_raw）
    pub path_len:  u32,
    pub mode_ptr:  *const u8,             // mode 字符串（"r"/"w"/"a"）
    pub mode_len:  u32,
    pub file_ptr:  *mut Option<std::fs::File>,  // 二级堆分配：File 不实现 Clone，
                                                // 经间接持有，GC 复制仅复制指针（见 §8）
}
```

> **为何不用 `RefCell<Option<File>>` 内联**：File 不实现 `Clone`，若内联于对象，Minor GC 的 copy_for_gc 无法复制 → 双重关闭/use-after-close。改用 `file_ptr: *mut Option<File>`（`Box::into_raw(Box::new(Some(file)))`），对象内仅存指针：GC 复制对象时复制指针、不复制 File；finalizer 释放二级分配并关闭 File。

**辅助函数**（task 20/22 的 `alloc_*`/`read_*` 模式）：

```rust
/// 分配 FileHandle 堆对象，返回 Object::Ref。
/// path/mode/file 各自独立二级分配；MVP 用 Box::into_raw（task 52 GC 上线后由 §8 finalizer 回收）。
pub fn alloc_file_handle(path: &str, mode: &str, file: std::fs::File) -> Object {
    let path_box: Box<[u8]> = Box::from(path.as_bytes());
    let mode_box: Box<[u8]> = Box::from(mode.as_bytes());
    let file_box: Box<Option<std::fs::File>> = Box::new(Some(file));
    let h = Box::new(MsFileHandle {
        header: MsObjHeader { gc_meta: 0, type_tag: TypeTag::FILE_HANDLE as u8,
            size: size_of::<MsFileHandle>() as u16, _padding: 0, class_ptr: 0 },
        path_ptr: Box::into_raw(path_box) as *const u8, path_len: path.len() as u32,
        mode_ptr: Box::into_raw(mode_box) as *const u8, mode_len: mode.len() as u32,
        file_ptr: Box::into_raw(file_box),
    });
    Object::Ref(Box::into_raw(h) as *mut MsObjHeader)
}

/// # Safety: ptr 须指向 MsFileHandle 且在调用期间有效。
pub unsafe fn read_file_handle<'a>(ptr: *mut MsObjHeader) -> &'a MsFileHandle { &*(ptr as *mut MsFileHandle) }
pub unsafe fn read_file_handle_mut<'a>(ptr: *mut MsObjHeader) -> &'a mut MsFileHandle { &mut *(ptr as *mut MsFileHandle) }
```

**方法实现**（`src/vm/stdlib.rs`，签名 `fn(&mut VM, &[Object]) -> Result<Object, String>`）：

- `f.read()`：`read_to_string` 读全部内容（若 `*file_ptr` 为 None → `Err("IOError: file already closed")`）
- `f.write(content)`：`write_all`（None 同上）
- `f.close()`：`*file_ptr = None`（幂等：已关闭再调不报错）
- `f.lines()`：`read_to_string` 后按 `'\n'` 分割，返回 **List**（`alloc_list`）；末尾空行不产生空元素（`lines()` 语义：非空行列表，末尾换行不额外产空行）

> **关闭后调用**（漏洞 D5）：`read`/`write`/`lines` 检测 `(*file_ptr).is_none()` 时返 `Err("IOError: file already closed")`。

### 3. 上下文管理器支持

FileHandle 实现 `__enter__` / `__exit__`（参照 `06-oop.md:251-252`、`tasks/38-with-statement.md:26`）：

- `__enter__(self)`：返回 self（CALL 1）
- `__exit__(self, err_type, err_msg, traceback)`：忽略三个异常参数，调用 `self.close()`，返回 `nil`（异常**继续传播**，不抑制）。CALL 4（self + err_type + err_msg + tb，task 38:93）

这样 `with io.open("file.txt") as f { ... }` 可正常工作。FileHandle 的 `__exit__` 为 native 实现（经 §7 方法分派），签名须接收 4 参数以满足 with 编译器固定的 CALL 4 约定与 task 25 的 arity 校验。

### 4. io.open 实现

```rust
fn native_io_open(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let path = expect_string(args.get(0), "open(path, mode?)")?;
    let mode = if args.len() > 1 { expect_string(args.get(1), "open(path, mode?)")? } else { "r".to_string() };

    // mode 合法值：r/w/a（暂不支持二进制 b 后缀，10-builtins.md 未定义）
    let mut opts = std::fs::OpenOptions::new();
    match mode.as_str() {
        "r" => { opts.read(true); }
        "w" => { opts.write(true).create(true).truncate(true); }
        "a" => { opts.append(true).create(true); }
        _ => return Err(format!("ValueError: unknown mode '{}'", mode)),
    }
    let file = opts.open(&path)
        .map_err(|e| format!("IOError: cannot open '{}': {}", path, e))?;
    Ok(alloc_file_handle(&path, &mode, file))
}
```

> **map_err 必要**（C4）：NativeFn 返 `Result<Object, String>`，而 `OpenOptions::open` 返 `std::io::Error`（不实现 `Into<String>`），裸 `?` 无法编译。所有 std::fs/std::io 调用统一 `.map_err(|e| format!("IOError: {}", e))?`。

**`expect_string` 辅助**（`src/vm/stdlib.rs`，task 20 read_str 包装）：

```rust
fn expect_string(arg: Option<&Object>, who: &str) -> Result<String, String> {
    match arg {
        Some(Object::Ref(p)) if unsafe { (**p).type_tag } == TypeTag::STRING as u8 =>
            Ok(unsafe { read_str(*p) }.to_owned()),
        other => Err(format!("TypeError: {} expects string, got {}", who, other.map(|o| o.type_name()).unwrap_or("missing"))),
    }
}
```

### 5. io.read_file / io.write_file 实现

```rust
fn native_io_read_file(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let path = expect_string(args.get(0), "read_file(path)")?;
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("IOError: cannot read '{}': {}", path, e))?;
    Ok(alloc_string(&content))
}

fn native_io_write_file(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let path = expect_string(args.get(0), "write_file(path, content)")?;
    let content = expect_string(args.get(1), "write_file(path, content)")?;
    std::fs::write(&path, content)
        .map_err(|e| format!("IOError: cannot write '{}': {}", path, e))?;
    Ok(Object::Nil)
}
```

### 6. io.exists 实现

```rust
fn native_io_exists(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let path = expect_string(args.get(0), "exists(path)")?;
    Ok(Object::Bool(std::path::Path::new(&path).exists()))
}
```

### 7. FileHandle 方法分派

FileHandle 的方法（`read`/`write`/`close`/`lines`/`__enter__`/`__exit__`）经 `f.method(...)` 调用，须扩展 GET_ATTR 与 CALL。

**GET_ATTR 新增 FILE_HANDLE 分支**（`src/vm/mod.rs` GET_ATTR handler，参照 task 41 INSTANCE 分支）：

```rust
Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::FILE_HANDLE as u8 => {
    let native = match name {
        "read"  | "write" | "close" | "lines"
        | "__enter__" | "__exit__" => lookup_file_method(name),  // 返回 NativeFunction 对象
        _ => return Err(format!("AttributeError: FileHandle has no attribute '{}'", name)),
    };
    // 包装为 BoundMethod（task 41，TypeTag::BOUND_METHOD=15）：绑定 receiver=ptr + method=native
    Ok(alloc_bound_method(Object::Ref(*ptr), native))
}
```

**CALL on BOUND_METHOD**（task 41 已实现 receiver 绑定）：bound method 调用时把 receiver 压入参数前部，再调底层 NativeFunction。本 task 须确认 task 41 的 CALL BOUND_METHOD 路径支持 **native underlying**（非仅 closure）；若仅支持 closure，本 task 在该分支追加 `TypeTag::FUNCTION`（native）子分支：`args = [receiver, ...user_args]` → `(native.func)(vm, &args)`。

**`lookup_file_method`**（`src/vm/stdlib.rs`）：静态表映射方法名→NativeFunction 对象（`native_fh_read`/`native_fh_write`/`native_fh_close`/`native_fh_lines`/`native_fh_enter`/`native_fh_exit`）。各函数首位参数为 receiver（FileHandle Ref）。

### 8. FileHandle 的 GC 集成

参照 [14-gc](../14-gc.md) § 类型描述表。FileHandle 持 Rust 资源（`std::fs::File`），须注册 TypeDescriptor（`src/vm/gc.rs`，`20 => &FILE_HANDLE_DESC`）。

**关键约束**：`std::fs::File` 不实现 `Clone`。为避免 Minor GC 半空间复制破坏 File 所有权，FileHandle 经 `file_ptr` 二级间接持有 File（§2）。GC 处理：

- **trace**：空（MsFileHandle 内无 `Object::Ref`，path/mode/file 均非 GC 对象）。
- **copy_for_gc**：**不应被调用**——`alloc_file_handle` 时将 `gc_meta` 的 gen 位设为 `Immortal`（`14-gc.md:82`，代数=2），使对象不进入 Young 代半空间复制。若防御性调用，则 panic（表明 FileHandle 误入 Young）。
- **finalize**：关闭并释放——`if let Some(f) = *file_ptr.take() { drop(f); }`（关闭 fd），回收 path/mode/file 三个二级 `Box` 分配，最后回收 MsFileHandle 主体。`has_finalizer = true`，由 task 52 的 `run_finalizers` 调用（`14-gc.md:474-494`）。

```rust
// alloc_file_handle 设 finalizer 标志 + Immortal 代（不在 Young 复制）
header: MsObjHeader {
    gc_meta: GEN_IMMORTAL | HAS_FINALIZER,  // 14-gc.md 位域：gen=2(Immortal), has_finalizer=1
    type_tag: TypeTag::FILE_HANDLE as u8, ...
}

fn finalize_file_handle(obj: *mut MsObjHeader) {
    unsafe {
        let h = Box::from_raw(obj as *mut MsFileHandle);          // 回收主体
        if let Some(f) = (*h.file_ptr).take() { drop(f); }        // 关闭 fd
        drop(Box::from_raw(h.file_ptr as *mut Option<std::fs::File>));  // 回收 file 二级分配
        drop(Box::from_raw(h.path_ptr as *mut [u8])); *h.path_len as usize;  // 回收 path（实际需记录长度，此处示意）
        // mode 同理
    }
}
```

> **fd 泄漏缓解**（C1）：即使用户不用 `with`、不调 `close()`，FileHandle 不可达 → GC 回收 → finalize 关闭 fd。`__exit__` 是显式清理路径，finalize 是兜底。

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
