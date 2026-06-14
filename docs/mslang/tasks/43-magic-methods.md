# 魔术方法

## 所属阶段
Phase 5.4 - 类 + OOP

## 前置任务
42-inheritance-super

## 目标
实现魔术方法的自动调用机制，覆盖构造/析构、字符串表示、比较运算、算术运算、容器协议、上下文管理器、可调用对象、迭代器协议。

## 设计规格

参照 [06-oop](../06-oop.md) § 魔术方法：

### 魔术方法分类

#### 构造与析构

| 方法 | 触发时机 |
|---|---|
| `__init__(self, ...)` | 实例创建时 |
| `__del__(self)` | 实例被 GC 回收时 |

#### 字符串表示

| 方法 | 触发时机 |
|---|---|
| `__repr__(self)` | `print(obj)`, `str(obj)` |
| `__str__(self)` | 字符串转换（优先于 `__repr__`） |

#### 比较运算

| 方法 | 对应运算符 |
|---|---|
| `__eq__(self, other)` | `==` |
| `__ne__(self, other)` | `!=` |
| `__lt__(self, other)` | `<` |
| `__le__(self, other)` | `<=` |
| `__gt__(self, other)` | `>` |
| `__ge__(self, other)` | `>=` |

#### 算术运算

| 方法 | 对应运算符 |
|---|---|
| `__add__(self, other)` | `+` |
| `__sub__(self, other)` | `-` |
| `__mul__(self, other)` | `*` |
| `__div__(self, other)` | `/` |（注：mslang 有意采用 `__div__` 而非 Python 的 `__truediv__`，简化运算符协议）
| `__floordiv__(self, other)` | `//` |
| `__mod__(self, other)` | `%` |
| `__pow__(self, other)` | `**` |

#### 容器协议

| 方法 | 触发时机 |
|---|---|
| `__len__(self)` | `len(obj)` |
| `__getitem__(self, key)` | `obj[key]` |
| `__setitem__(self, key, val)` | `obj[key] = val` |
| `__contains__(self, item)` | `item in obj` |
| `__iter__(self)` | `for x in obj` |

#### 上下文管理器 / 可调用 / 迭代器

| 方法 | 触发时机 |
|---|---|
| `__enter__(self)` | 进入 with 块 |
| `__exit__(self, err, msg, tb)` | 离开 with 块 |
| `__call__(self, ...)` | `obj(args)` |
| `__iter__(self)` | 迭代器协议 |
| `__next__(self)` | 获取下一个元素 |

### INVOKE 优化指令

参照 [11-bytecode-vm](../11-bytecode-vm.md) § 类与实例：

| OpCode | 操作数 | 说明 |
|---|---|---|
| `INVOKE` | `name_idx(2), argc(1)` | 合并 GET_ATTR + CALL 的优化 |

当编译器检测到 `obj.method(args)` 模式时，可使用 INVOKE 代替 GET_ATTR + CALL，避免创建 BoundMethod 的中间对象。

## 实现细节

### 1. 算术运算魔术方法

在 VM 的算术指令中，检查操作数是否为 Instance 并有对应的魔术方法：

```rust
OpCode::ADD => {
    let b = self.stack_pop();
    let a = self.stack_pop();
    
    // 检查 a 是否有 __add__
    if let Object::Ref(ptr) = &a {
        if unsafe { (*(*ptr)).type_tag } == TypeTag::INSTANCE as u8 {
            let inst = unsafe { read_instance(*ptr) };
            if let Some(method_ptr) = unsafe { read_class(inst.class).find_method("__add__") } {
                let bound = alloc_bound_method(a.clone(), method_ptr);
                self.stack_push(b);
                self.stack_push(bound);
                return self.call_bound_method(1); // 1 arg: b
            }
        }
    }
    
    // 内置类型加法（通过 Ref + TypeTag 识别，参照 Task 20 对象模型）
    match (a, b) {
        (Object::Int(x), Object::Int(y)) => self.stack_push(Object::Int(x + y)),
        (Object::Float(x), Object::Float(y)) => self.stack_push(Object::Float(x + y)),
        (Object::Ref(x), Object::Ref(y))
            if unsafe { (**x).type_tag } == TypeTag::STRING as u8
            && unsafe { (**y).type_tag } == TypeTag::STRING as u8 =>
        {
            let result = format!("{}{}", unsafe { read_str(*x) }, unsafe { read_str(*y) });
            self.stack_push(alloc_string(&result));
        }
        // ...
    }
}
```

同理应用于 SUBTRACT、MULTIPLY、DIVIDE、FLOOR_DIV、MODULO、POWER。

### 2. 比较运算魔术方法

```rust
OpCode::EQUAL => {
    let b = self.stack_pop();
    let a = self.stack_pop();
    
    // 检查 __eq__
    if let Object::Ref(ptr) = &a {
        if unsafe { (*(*ptr)).type_tag } == TypeTag::INSTANCE as u8 {
            let inst = unsafe { read_instance(*ptr) };
            if let Some(method_ptr) = unsafe { read_class(inst.class).find_method("__eq__") } {
                let result = self.invoke_magic(&a, method_ptr, &[b])?;
                self.stack_push(result);
                return Ok(());
            }
        }
    }
    
    // 内置比较
    self.stack_push(Object::Bool(a == b));
}
```

### 3. 字符串表示魔术方法

```rust
fn object_to_string(&mut self, obj: &Object) -> Result<String> {
    match obj {
        Object::Ref(ptr) if unsafe { (*(*ptr)).type_tag } == TypeTag::INSTANCE as u8 => {
            let inst = unsafe { read_instance(*ptr) };
            // 优先 __str__
            if let Some(method_ptr) = unsafe { read_class(inst.class).find_method("__str__") } {
                let result = self.invoke_magic(obj, method_ptr, &[])?;
                return Ok(result.as_string().clone());
            }
            // 其次 __repr__
            if let Some(method_ptr) = unsafe { read_class(inst.class).find_method("__repr__") } {
                let result = self.invoke_magic(obj, method_ptr, &[])?;
                return Ok(result.as_string().clone());
            }
            let cls_name = &unsafe { read_class(inst.class) }.name;
            Ok(format!("<{} instance>", cls_name))
        }
        // 内置类型的字符串表示
        _ => Ok(format_builtin(obj)),
    }
}
```

### 4. 容器协议魔术方法

```rust
OpCode::GET_INDEX => {
    let key = self.stack_pop();
    let obj = self.stack_pop();
    
    match &obj {
        Object::Ref(ptr) if unsafe { (*(*ptr)).type_tag } == TypeTag::INSTANCE as u8 => {
            let inst = unsafe { read_instance(*ptr) };
            if let Some(method_ptr) = unsafe { read_class(inst.class).find_method("__getitem__") } {
                let result = self.invoke_magic(&obj, method_ptr, &[key])?;
                self.stack_push(result);
            } else {
                return Err(runtime_error("object is not subscriptable"));
            }
        }
        Object::Ref(ptr) if unsafe { (*(*ptr)).type_tag } == TypeTag::LIST as u8 => { /* 内置 list 下标 */ }
        Object::Ref(ptr) if unsafe { (*(*ptr)).type_tag } == TypeTag::DICT as u8 => { /* 内置 dict 下标 */ }
        _ => return Err(runtime_error("object is not subscriptable")),
    }
}
```

### 5. __call__ 魔术方法

当 CALL 目标是 Instance 且其类有 `__call__` 方法时：

```rust
OpCode::CALL => {
    let argc = self.read_byte();
    let callee = self.stack_peek(argc);
    
    match callee {
        Object::Ref(ptr) if unsafe { (*(*ptr)).type_tag } == TypeTag::INSTANCE as u8 => {
            let inst = unsafe { read_instance(*ptr) };
            if let Some(method_ptr) = unsafe { read_class(inst.class).find_method("__call__") } {
                let bound = alloc_bound_method(callee, method_ptr);
                // 替换栈上 callee 为 BoundMethod
                self.stack[self.stack.len() - argc as usize - 1] = bound;
                return self.call_bound_method(argc);
            }
            return Err(runtime_error("object is not callable"));
        }
        Object::Ref(ptr) if unsafe { (*(*ptr)).type_tag } == TypeTag::CLASS as u8 => { /* 类实例化 */ }
        Object::Ref(ptr) if unsafe { (*(*ptr)).type_tag } == TypeTag::CLOSURE as u8 => { /* 普通函数调用 */ }
        Object::Ref(ptr) if unsafe { (*(*ptr)).type_tag } == TypeTag::BOUND_METHOD as u8 => { /* 方法调用 */ }
        // ...
    }
}
```

### 6. __contains__ 与 in 运算符

```rust
OpCode::IN => {
    let container = self.stack_pop();
    let item = self.stack_pop();
    
    match &container {
        Object::Ref(ptr) if unsafe { (*(*ptr)).type_tag } == TypeTag::INSTANCE as u8 => {
            let inst = unsafe { read_instance(*ptr) };
            if let Some(method_ptr) = unsafe { read_class(inst.class).find_method("__contains__") } {
                let result = self.invoke_magic(&container, method_ptr, &[item])?;
                self.stack_push(result);
            } else {
                // 回退到迭代检查
                let found = self.iter_contains(&container, &item)?;
                self.stack_push(Object::Bool(found));
            }
        }
        Object::Ref(ptr) if unsafe { (*(*ptr)).type_tag } == TypeTag::LIST as u8 => { /* 内置 list in */ }
        Object::Ref(ptr) if unsafe { (*(*ptr)).type_tag } == TypeTag::SET as u8 => { /* 内置 set in */ }
        Object::Ref(ptr) if unsafe { (*(*ptr)).type_tag } == TypeTag::DICT as u8 => { /* 内置 dict in */ }
        _ => return Err(runtime_error("type does not support 'in'")),
    }
}
```

### 7. INVOKE 优化指令

```rust
OpCode::INVOKE => {
    let name_idx = self.read_u16();
    let argc = self.read_byte();
    let name = &self.constants[name_idx as usize].to_string();
    
    let receiver = self.stack_peek(argc);
    
    // 直接在 receiver 上查找方法并调用
    // 不创建 BoundMethod 中间对象
    match receiver {
        Object::Ref(ptr) if unsafe { (*(*ptr)).type_tag } == TypeTag::INSTANCE as u8 => {
            let inst = unsafe { read_instance(*ptr) };
            if let Some(method_ptr) = unsafe { read_class(inst.class).find_method(name) } {
                // 直接调用，self 绑定在方法内部处理
                self.invoke_direct(receiver, method_ptr, argc)?;
            } else {
                return Err(runtime_error(format!("no method '{}'", name)));
            }
        }
        _ => { /* 其他类型 */ }
    }
}
```

### 8. invoke_magic 辅助方法

```rust
fn invoke_magic(&mut self, receiver: &Object, method_ptr: *mut MsObjHeader, args: &[Object]) -> Result<Object> {
    let bound = alloc_bound_method(receiver.clone(), method_ptr);
    
    self.stack_push(bound);
    for arg in args {
        self.stack_push(arg.clone());
    }
    
    self.call_bound_method(args.len() as u8)?;
    self.run_until_return();
    
    let result = self.stack_pop();
    Ok(result)
}
```

## 验证标准

1. `__repr__` / `__str__` 在 print/str 时自动调用
2. 算术运算符自动调用对应魔术方法
3. 比较运算符自动调用对应魔术方法
4. `__call__` 使实例可调用
5. `__len__` 使 len() 工作
6. `__getitem__` / `__setitem__` 使下标访问工作
7. `__contains__` 使 in 运算符工作
8. INVOKE 优化正确工作

## 测试用例

```ms
// test_magic_methods.ms — 魔术方法

// 算术运算
class Vector {
    fn __init__(self, x, y) {
        self.x = x
        self.y = y
    }
    
    fn __add__(self, other) {
        return Vector(self.x + other.x, self.y + other.y)
    }
    
    fn __repr__(self) {
        return "Vector(" + str(self.x) + ", " + str(self.y) + ")"
    }
    
    fn __eq__(self, other) {
        return self.x == other.x and self.y == other.y
    }
}

v1 = Vector(1, 2)
v2 = Vector(3, 4)
v3 = v1 + v2
print(v3)
print(v1 == Vector(1, 2))
print(v1 == v2)

// __call__
class Multiplier {
    fn __init__(self, factor) {
        self.factor = factor
    }
    
    fn __call__(self, x) {
        return x * self.factor
    }
}

double = Multiplier(2)
print(double(5))

// __len__ 和 __getitem__
class MyList {
    fn __init__(self, items) {
        self.items = items
    }
    
    fn __len__(self) {
        return self.items.length()
    }
    
    fn __getitem__(self, idx) {
        return self.items[idx]
    }
    
    fn __repr__(self) {
        return "MyList(" + str(self.items) + ")"
    }
}

ml = MyList([10, 20, 30])
print(len(ml))
print(ml[1])

// __contains__
class Range10 {
    fn __contains__(self, item) {
        return item >= 0 and item < 10
    }
}

r = Range10()
print(5 in r)
print(15 in r)

// __str__ 优先于 __repr__
class Named {
    fn __str__(self) {
        return "str form"
    }
    fn __repr__(self) {
        return "repr form"
    }
}

n = Named()
print(n)
print(str(n))
```

预期输出：

```
Vector(4, 6)
true
false
10
3
20
true
false
str form
str form
```
