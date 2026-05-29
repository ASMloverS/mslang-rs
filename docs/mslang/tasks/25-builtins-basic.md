# 基础内置函数

## 所属阶段
Phase 2.5a - 字节码编译 + VM 核心

## 前置任务
- 24-vm-arithmetic-control

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

VM 需在 `register_builtins` 中注册以下类型对象到全局变量表：

```rust
// 类型对象注册
self.globals.insert("int".to_string(), Object::Type(TypeObj::Int));
self.globals.insert("float".to_string(), Object::Type(TypeObj::Float));
self.globals.insert("bool".to_string(), Object::Type(TypeObj::Bool));
self.globals.insert("string".to_string(), Object::Type(TypeObj::String));
self.globals.insert("nil".to_string(), Object::Type(TypeObj::Nil));
self.globals.insert("list".to_string(), Object::Type(TypeObj::List));
self.globals.insert("dict".to_string(), Object::Type(TypeObj::Dict));
self.globals.insert("tuple".to_string(), Object::Type(TypeObj::Tuple));
self.globals.insert("set".to_string(), Object::Type(TypeObj::Set));
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
        ];

        for (name, _arity, func) in builtins {
            self.globals.insert(
                name.to_string(),
                Object::NativeFunction(NativeFunction {
                    name: name.to_string(),
                    func,
                }),
            );
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
    Ok(Object::String(Gc::new(arg.type_name().to_string())))
}
```

### len

```rust
fn builtin_len(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let arg = args.get(0).ok_or("len() requires 1 argument")?;
    match arg {
        Object::String(s) => Ok(Object::Int(s.borrow().data.len() as i64)),
        Object::List(items) => Ok(Object::Int(items.borrow().data.len() as i64)),
        Object::Dict(map) => Ok(Object::Int(map.borrow().data.len() as i64)),
        Object::Tuple(items) => Ok(Object::Int(items.borrow().data.len() as i64)),
        Object::Set(items) => Ok(Object::Int(items.borrow().data.inner.len() as i64)),
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
    Ok(arg.to_str())
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
        Object::List(items) => {
            let mut total = Object::Int(0);
            for item in &items.borrow().data {
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
    let expected_type = match type_obj {
        Object::Type(t) => t.type_name(),
        _ => return Err("isinstance(): second argument must be a type".to_string()),
    };
    Ok(Object::Bool(val.type_name() == expected_type))
}
```

### Object 枚举扩展

需要在 Object 中添加 NativeFunction 变体：

```rust
#[derive(Clone)]
pub enum Object {
    // ... 已有变体 ...
    NativeFunction(NativeFunction),
}
```

### CALL 指令扩展

在 VM 的 CALL 指令处理中增加原生函数调用分支：

```rust
OpCode::Call => {
    let argc = self.read_u8() as usize;
    let callee = self.stack[self.stack.len() - argc - 1].clone();
    match &callee {
        Object::NativeFunction(native) => {
            let args = self.stack[self.stack.len() - argc..].to_vec();
            self.stack.truncate(self.stack.len() - argc - 1);
            let result = (native.func)(self, &args)?;
            self.push(result);
        }
        // ... 闭包调用在 Phase 3 实现 ...
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
