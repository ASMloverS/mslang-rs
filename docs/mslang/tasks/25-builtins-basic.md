# 基础内置函数

## 所属阶段
Phase 2.5a - 字节码编译 + VM 核心

## 前置任务
- 21-object-system-operations, 22-object-system-collections, 24-vm-arithmetic-control

## 目标

实现 mslang 的基础内置函数，使其作为原生函数（native function）注册在 VM 中，无需 import 即可全局调用。

## 设计规格

引用 [10-builtins.md](../10-builtins.md) 内置函数定义：

### I/O 函数

| 函数 | 签名 | 说明 |
|---|---|---|
| `print` | `print(*args)` | 打印到标准输出（空格分隔，追加换行） |
| `println` | `println(*args)` | 等价于 `print`，别名（两者行为完全一致） |

### 类型检查

| 函数 | 签名 | 说明 |
|---|---|---|
| `type` | `type(val) -> string` | 返回类型名称 |

### 容器函数

| 函数 | 签名 | 说明 |
|---|---|---|
| `len` | `len(val) -> int` | 返回长度 |

### 类型转换

| 函数 | 签名 | 说明 |
|---|---|---|
| `int` | `int(val) -> int` | 转换为整数 |
| `float` | `float(val) -> float` | 转换为浮点数 |
| `str` | `str(val) -> string` | 转换为字符串 |
| `bool` | `bool(val) -> bool` | 转换为布尔值 |

### 数学函数

| 函数 | 签名 | 说明 |
|---|---|---|
| `abs` | `abs(n) -> number` | 绝对值 |
| `max` | `max(*args) -> number` | 最大值 |
| `min` | `min(*args) -> number` | 最小值 |
| `sum` | `sum(iterable) -> number` | 求和 |
| `ceil` | `ceil(n) -> int` | 向上取整 |
| `floor` | `floor(n) -> int` | 向下取整 |
| `round` | `round(n, digits?) -> number` | 四舍五入 |

### 类型检查函数

| 函数 | 签名 | 说明 |
|---|---|---|
| `isinstance` | `isinstance(val, type) -> bool` | 检查是否为指定类型 |

`isinstance` 第二参数为类型对象（如 `int`、`string`、`list`）。类型对象是内置全局常量，VM 初始化时注册。用法：`isinstance(42, int)` → `true`。

VM 需在 `register_builtins` 中注册以下类型常量到全局变量表。类型常量是 Class 对象（`Object::Ref` + `TypeTag::CLASS`），与用户自定义类统一表示：

```rust
// 类型常量注册（Class 对象，参照 11-bytecode-vm.md Object 模型）
// 内置类型 Class 在 VM::new() 中创建，存入 globals
self.globals.insert("int".to_string(), self.builtin_type_class("int"));
self.globals.insert("float".to_string(), self.builtin_type_class("float"));
self.globals.insert("bool".to_string(), self.builtin_type_class("bool"));
self.globals.insert("string".to_string(), self.builtin_type_class("string"));
self.globals.insert("nil".to_string(), self.builtin_type_class("nil"));
self.globals.insert("list".to_string(), self.builtin_type_class("list"));
self.globals.insert("dict".to_string(), self.builtin_type_class("dict"));
self.globals.insert("tuple".to_string(), self.builtin_type_class("tuple"));
self.globals.insert("set".to_string(), self.builtin_type_class("set"));
```

### 断言

| 函数 | 签名 | 说明 |
|---|---|---|
| `assert` | `assert(cond, msg?)` | 断言 |

## 实现细节

### 文件位置

`src/vm/builtins.rs`

### NativeFunction 类型

```rust
pub type NativeFn = fn(&mut VM, &[Object]) -> Result<Object, String>;

pub struct NativeFunction {
    pub name: String,
    pub arity: usize,  // usize::MAX 表示可变参数
    pub func: NativeFn,
}
```

### 内置函数注册

原生函数通过堆分配的 Function 对象表示（`Object::Ref` + `TypeTag::FUNCTION`），不新增 Object 变体。

```rust
impl VM {
    pub fn register_builtins(&mut self) {
        let builtins: Vec<(&str, usize, NativeFn)> = vec![
            ("print", usize::MAX, builtin_print),
            ("println", usize::MAX, builtin_println),
            ("type", 1, builtin_type),
            ("len", 1, builtin_len),
            ("int", 1, builtin_int),
            ("float", 1, builtin_float),
            ("str", 1, builtin_str),
            ("bool", 1, builtin_bool),
            ("abs", 1, builtin_abs),
            ("max", usize::MAX, builtin_max),
            ("min", usize::MAX, builtin_min),
            ("sum", 1, builtin_sum),
            ("ceil", 1, builtin_ceil),
            ("floor", 1, builtin_floor),
            ("round", usize::MAX, builtin_round),
            ("isinstance", 2, builtin_isinstance),
            ("assert", usize::MAX, builtin_assert),
            // 补充标准要求的全局内置（参照 10-builtins.md）
            ("open", usize::MAX, builtin_open),
            ("input", usize::MAX, builtin_input),
            ("id", 1, builtin_id),
            ("hash", 1, builtin_hash),
            ("copy", 1, builtin_copy),
            ("deepcopy", 1, builtin_deepcopy),
            ("range", usize::MAX, builtin_range),
        ];

        for (name, _arity, func) in builtins {
            let native_fn = NativeFunction {
                name: name.to_string(),
                func,
            };
            // 通过 alloc_native_function 创建堆对象，返回 Object::Ref
            self.globals.insert(name.to_string(), alloc_native_function(native_fn));
        }
    }
}
```

### print / println

```rust
fn builtin_print(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let output: Vec<String> = args.iter().map(|a| format!("{}", a)).collect();
    println!("{}", output.join(" "));
    Ok(Object::Nil)
}

fn builtin_println(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    builtin_print(_vm, args)
}
```

### type

```rust
fn builtin_type(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let arg = args.get(0).ok_or("type() requires 1 argument")?;
    Ok(alloc_string(arg.type_name()))
}
```

### len

```rust
fn builtin_len(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let arg = args.get(0).ok_or("len() requires 1 argument")?;
    match arg {
        Object::Ref(ptr) => {
            let tag = unsafe { (*(*ptr)).type_tag };
            let len = if tag == TypeTag::STRING as u8 {
                unsafe { read_str(*ptr) }.len()
            } else if tag == TypeTag::LIST as u8 {
                unsafe { read_list(*ptr) }.len()
            } else if tag == TypeTag::DICT as u8 {
                unsafe { read_dict(*ptr) }.len()
            } else if tag == TypeTag::TUPLE as u8 {
                unsafe { read_tuple(*ptr) }.len()
            } else if tag == TypeTag::SET as u8 {
                unsafe { read_set(*ptr) }.len()
            } else {
                return Err(format!("TypeError: object of type '{}' has no len()", arg.type_name()));
            };
            Ok(Object::Int(len as i64))
        }
        _ => Err(format!("TypeError: object of type '{}' has no len()", arg.type_name())),
    }
}
```

### 类型转换函数

```rust
fn builtin_int(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let arg = args.get(0).ok_or("int() requires 1 argument")?;
    arg.to_int()
}

fn builtin_float(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let arg = args.get(0).ok_or("float() requires 1 argument")?;
    arg.to_float()
}

fn builtin_str(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let arg = args.get(0).ok_or("str() requires 1 argument")?;
    Ok(arg.to_str())  // to_str() 内部调用 alloc_string（参照 task 21）
}

fn builtin_bool(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let arg = args.get(0).ok_or("bool() requires 1 argument")?;
    Ok(arg.to_bool())
}
```

### 数学函数

```rust
fn builtin_abs(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let arg = args.get(0).ok_or("abs() requires 1 argument")?;
    match arg {
        Object::Int(n) => Ok(Object::Int(n.abs())),
        Object::Float(n) => Ok(Object::Float(n.abs())),
        _ => Err(format!("TypeError: bad operand type for abs(): '{}'", arg.type_name())),
    }
}

fn builtin_max(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    if args.is_empty() {
        return Err("max() requires at least 1 argument".to_string());
    }
    let mut result = args[0].clone();
    for arg in &args[1..] {
        match result.compare(arg, &OpCode::Less)? {
            Object::Bool(true) => result = arg.clone(),
            _ => {}
        }
    }
    Ok(result)
}

fn builtin_min(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    if args.is_empty() {
        return Err("min() requires at least 1 argument".to_string());
    }
    let mut result = args[0].clone();
    for arg in &args[1..] {
        match result.compare(arg, &OpCode::Greater)? {
            Object::Bool(true) => result = arg.clone(),
            _ => {}
        }
    }
    Ok(result)
}

fn builtin_sum(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let arg = args.get(0).ok_or("sum() requires 1 argument")?;
    match arg {
        Object::Ref(ptr) if unsafe { (*(*ptr)).type_tag } == TypeTag::LIST as u8 => {
            let mut total = Object::Int(0);
            for item in unsafe { read_list(*ptr) }.iter() {
                total = total.add(item)?;
            }
            Ok(total)
        }
        _ => Err(format!("TypeError: '{}' object is not iterable", arg.type_name())),
    }
}
```

### assert

```rust
fn builtin_assert(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let cond = args.get(0).ok_or("assert() requires at least 1 argument")?;
    if !cond.is_truthy() {
        let msg = if args.len() > 1 {
            format!("{}", args[1])
        } else {
            "AssertionError".to_string()
        };
        return Err(format!("AssertionError: {}", msg));
    }
    Ok(Object::Nil)
}

fn builtin_ceil(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let arg = args.get(0).ok_or("ceil() requires 1 argument")?;
    match arg {
        Object::Int(_) => Ok(arg.clone()),
        Object::Float(n) => Ok(Object::Int(n.ceil() as i64)),
        _ => Err(format!("TypeError: bad operand type for ceil(): '{}'", arg.type_name())),
    }
}

fn builtin_floor(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let arg = args.get(0).ok_or("floor() requires 1 argument")?;
    match arg {
        Object::Int(_) => Ok(arg.clone()),
        Object::Float(n) => Ok(Object::Int(n.floor() as i64)),
        _ => Err(format!("TypeError: bad operand type for floor(): '{}'", arg.type_name())),
    }
}

fn builtin_round(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let arg = args.get(0).ok_or("round() requires at least 1 argument")?;
    let digits = if args.len() > 1 {
        match &args[1] {
            Object::Int(d) => *d as i32,
            _ => return Err("round(): digits must be int".to_string()),
        }
    } else {
        0
    };
    match arg {
        Object::Int(_) => Ok(arg.clone()),
        Object::Float(n) => {
            let factor = 10f64.powi(digits);
            Ok(Object::Float((n * factor).round() / factor))
        }
        _ => Err(format!("TypeError: bad operand type for round(): '{}'", arg.type_name())),
    }
}

fn builtin_isinstance(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let val = args.get(0).ok_or("isinstance() requires 2 arguments")?;
    let type_obj = args.get(1).ok_or("isinstance() requires 2 arguments")?;

    // type_obj 可以是内置类型 Class（Ref + TypeTag::CLASS）或用户自定义 Class
    let expected_type_name = match type_obj {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::CLASS as u8 => {
            unsafe { read_class(*ptr) }.name.clone()
        }
        _ => return Err("isinstance(): second argument must be a type".to_string()),
    };

    // 检查 val 的类型是否匹配（含继承链，参照 02-types.md § isinstance）
    let matches = match val {
        // 基本类型直接比较 type_name
        Object::Nil => expected_type_name == "nil",
        Object::Bool(_) => expected_type_name == "bool",
        Object::Int(_) => expected_type_name == "int",
        Object::Float(_) => expected_type_name == "float",
        // 引用类型：检查 TypeTag 或遍历 Class 继承链
        Object::Ref(ptr) => {
            let tag = unsafe { (**ptr).type_tag };
            if tag == TypeTag::INSTANCE as u8 {
                // 用户实例：遍历继承链（参照 02-types.md: 所有实例隐式继承 Object）
                let inst = unsafe { read_instance(*ptr) };
                let mut class = inst.class.clone();
                loop {
                    if class.name == expected_type_name || expected_type_name == "object" {
                        break true;
                    }
                    match &class.parent {
                        Some(parent) => class = parent.clone(),
                        None => break false,
                    }
                }
            } else {
                // 内置引用类型：按 TypeTag 映射到类型名
                let type_name = match tag {
                    t if t == TypeTag::STRING as u8 => "string",
                    t if t == TypeTag::LIST as u8 => "list",
                    t if t == TypeTag::DICT as u8 => "dict",
                    t if t == TypeTag::TUPLE as u8 => "tuple",
                    t if t == TypeTag::SET as u8 => "set",
                    t if t == TypeTag::FUNCTION as u8 => "function",
                    t if t == TypeTag::CLOSURE as u8 => "function",
                    _ => "object",
                };
                type_name == expected_type_name || expected_type_name == "object"
            }
        }
    };
    Ok(Object::Bool(matches))
}
```

### 补充内置函数（参照 10-builtins.md）

以下内置函数为标准要求但原任务遗漏的全局内置：

```rust
/// open(path, mode?) -> File（全局内置，无需 import）
/// 参照 10-builtins.md § open
fn builtin_open(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let path = args.get(0).ok_or("open() requires at least 1 argument")?;
    let mode = args.get(1);
    // 委托给 io 模块（Task 46），此处为全局快捷入口
    vm.open_file(path, mode)
}

/// input(prompt?) -> string
/// 参照 10-builtins.md § input
fn builtin_input(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    if let Some(prompt) = args.get(0) {
        print!("{}", prompt);
    }
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)
        .map_err(|e| format!("IOError: {}", e))?;
    let line = line.trim_end_matches('\n').trim_end_matches('\r');
    Ok(alloc_string(line))
}

/// id(val) -> int — 返回对象唯一标识（引用地址）
/// 参照 10-builtins.md § id
fn builtin_id(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let val = args.get(0).ok_or("id() requires 1 argument")?;
    match val {
        Object::Ref(ptr) => Ok(Object::Int(*ptr as i64)),
        // 内联值使用值本身的标识
        Object::Int(n) => Ok(Object::Int(*n)),
        Object::Float(f) => Ok(Object::Int(f.to_bits() as i64)),
        Object::Bool(b) => Ok(Object::Int(*b as i64)),
        Object::Nil => Ok(Object::Int(0)),
    }
}

/// hash(val) -> int — 返回对象哈希值
/// 参照 10-builtins.md § hash
fn builtin_hash(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    use std::hash::{Hash, Hasher};
    use std::collections::hash_map::DefaultHasher;
    let val = args.get(0).ok_or("hash() requires 1 argument")?;
    let mut hasher = DefaultHasher::new();
    val.hash(&mut hasher);
    Ok(Object::Int(hasher.finish() as i64))
}

/// copy(val) -> 浅拷贝
/// 参照 10-builtins.md § copy
fn builtin_copy(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let val = args.get(0).ok_or("copy() requires 1 argument")?;
    match val {
        Object::Ref(ptr) => {
            let tag = unsafe { (**ptr).type_tag };
            match tag {
                t if t == TypeTag::LIST as u8 => {
                    let items = unsafe { read_list(*ptr) }.clone();
                    Ok(alloc_list(&items))
                }
                t if t == TypeTag::DICT as u8 => {
                    let pairs = unsafe { read_dict(*ptr) }.clone();
                    Ok(alloc_dict(&pairs))
                }
                _ => Ok(val.clone()), // 不可变类型直接返回
            }
        }
        _ => Ok(val.clone()),
    }
}

/// deepcopy(val) -> 深拷贝
/// 参照 10-builtins.md § deepcopy
fn builtin_deepcopy(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let val = args.get(0).ok_or("deepcopy() requires 1 argument")?;
    vm.deep_copy(val)
}

/// range(start, stop?, step?) -> List
/// 参照 10-builtins.md § range
fn builtin_range(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let (start, stop, step) = match args.len() {
        1 => (0, require_int(&args[0])?, 1),
        2 => (require_int(&args[0])?, require_int(&args[1])?, 1),
        3 => (require_int(&args[0])?, require_int(&args[1])?, require_int(&args[2])?),
        _ => return Err("range() requires 1-3 arguments".to_string()),
    };
    if step == 0 { return Err("ValueError: range() step argument must not be zero".to_string()); }
    let mut items = Vec::new();
    let mut i = start;
    if step > 0 {
        while i < stop { items.push(Object::Int(i)); i += step; }
    } else {
        while i > stop { items.push(Object::Int(i)); i += step; }
    }
    Ok(alloc_list(&items))
}
```

> **注**：`alloc_list`、`alloc_dict` 等辅助函数由 Task 22（object-system-collections）定义。`vm.open_file`、`vm.deep_copy` 方法分别在 Task 46（stdlib-io）和 Task 22 中实现。MVP 阶段可先注册占位，后续任务填充实现。

### 原生函数堆分配（参照 Task 20 对象模型）

原生函数不新增 Object 变体，而是通过堆分配为 Function 对象（`TypeTag::FUNCTION`）：

```rust
/// 堆上 Native Function 对象布局
#[repr(C)]
pub struct MsNativeFunction {
    pub header: MsObjHeader,
    pub name_ptr: *const u8,
    pub name_len: u32,
    pub func: NativeFn,
}

/// 分配 NativeFunction 堆对象，返回 Object::Ref。
/// MVP：Box 分配；task 52-gc 替换为 TLAB bump 分配。
pub fn alloc_native_function(native: NativeFunction) -> Object {
    let name_bytes = native.name.as_bytes();
    let name_box: Box<[u8]> = Box::from(name_bytes);
    let name_len = name_box.len() as u32;
    let name_ptr = Box::into_raw(name_box) as *const u8;

    let ms_fn = Box::new(MsNativeFunction {
        header: MsObjHeader {
            gc_meta: 0,
            type_tag: TypeTag::FUNCTION as u8,
            size: std::mem::size_of::<MsNativeFunction>() as u16,
            _padding: 0,
            class_ptr: 0,
        },
        name_ptr,
        name_len,
        func: native.func,
    });
    Object::Ref(Box::into_raw(ms_fn) as *mut MsObjHeader)
}
```

### CALL 指令扩展

在 VM 的 CALL 指令处理中增加原生函数调用分支（通过 Ref + TypeTag::FUNCTION 识别）：

```rust
OpCode::Call => {
    let argc = self.read_u8() as usize;
    let callee = self.stack[self.stack.len() - argc - 1].clone();
    match &callee {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::FUNCTION as u8 => {
            let native = unsafe { read_native_function(*ptr) };
            let args = self.stack[self.stack.len() - argc..].to_vec();
            self.stack.truncate(self.stack.len() - argc - 1);
            let result = (native.func)(self, &args)?;
            self.push(result);
        }
        // ... 闭包调用（TypeTag::CLOSURE）在 Phase 3 实现 ...
        _ => return Err(format!("TypeError: '{}' object is not callable", callee.type_name())),
    }
}
```

## 验证标准

1. `print("hello")` 输出 `hello`
2. `type(42)` 返回 `"int"`
3. `len("hello")` 返回 `5`
4. `len([1, 2, 3])` 返回 `3`
5. `abs(-5)` 返回 `5`
6. `max(1, 2, 3)` 返回 `3`
7. `min(1, 2, 3)` 返回 `1`
8. `sum([1, 2, 3])` 返回 `6`
9. `int("42")` 返回 `42`
10. `float("3.14")` 返回 `3.14`
11. `str(42)` 返回 `"42"`
12. `bool(0)` 返回 `false`
13. `ceil(3.7)` 返回 `4`
14. `floor(3.7)` 返回 `3`
15. `round(3.5)` 返回 `4.0`
16. `isinstance(42, int)` 返回 `true`
17. `isinstance("hi", int)` 返回 `false`
18. `isinstance(dog_instance, Animal)` 返回 `true`（继承链匹配，参照 02-types.md）
19. `id(obj)` 返回唯一标识
20. `hash("key")` 返回稳定哈希值
21. `range(5)` 返回 `[0, 1, 2, 3, 4]`

## 测试用例

```ms
# test_builtins_basic.ms
print("hello")
print(type(42))
print(type("hello"))
print(len("hello"))
print(len([1, 2, 3]))
print(abs(-5))
print(max(1, 2, 3))
print(min(1, 2, 3))
print(sum([1, 2, 3]))
print(int("42"))
print(float("3.14"))
print(str(42))
print(bool(0))
print(ceil(3.7))
print(floor(3.7))
print(round(3.5))
print(isinstance(42, int))
print(isinstance("hi", int))
```

预期输出：
```
hello
int
string
5
3
5
3
1
6
42
3.14
42
false
4
3
4.0
true
false
```
