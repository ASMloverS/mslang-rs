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

在每个算术指令 handler 中，先检查左操作数是否为 Instance 并有对应魔术方法。若有，调用 `invoke_method`（已存在于 `src/vm/mod.rs:821`）分派。

> **GC 安全**：检查到 magic method 后，在调用 `invoke_method` 前将 `a`/`b` 重新 push 到值栈（栈是 GC 根集），避免局部 Object 中的 Ref 指针在嵌套执行期间因 Minor GC 悬垂。magic method 分支用 `return` 提前退出 handler，不 fallback 到内置类型。

> **无反射运算符**：mslang 不支持 `__radd__` 等反射运算符（`06-oop.md` 未定义）。`Int + Instance` 中左操作数 Int 无 `__add__`，fallback 到内置 `(Int, Ref)` 匹配失败 → TypeError。此为设计决策，非 bug。

```rust
OpCode::Add => {
    let b = self.pop()?;
    let a = self.pop()?;

    // 检查 a 是否为 Instance 且有 __add__
    if let Object::Ref(ptr) = &a {
        if unsafe { (**ptr).type_tag } == TypeTag::INSTANCE as u8 {
            let class_ptr = unsafe { read_instance(*ptr) }.class;
            if let Some(method_ptr) = unsafe { read_class(class_ptr) }.find_method("__add__") {
                // invoke_method 内部创建 BoundMethod + push args + call_value + run_loop + pop result
                let result = self.invoke_method(method_ptr, a, &[b])?;
                self.push(result)?;
                return Ok(());
            }
        }
    }

    // 内置类型加法（Object::add，参照 task 21）
    let result = a.add(&b)?;
    self.push(result)?;
}
```

同理应用于 Subtract、Multiply、Divide、FloorDiv、Modulo、Power，分别查找 `__sub__`/`__mul__`/`__div__`/`__floordiv__`/`__mod__`/`__pow__`。

> **分派函数表**：为避免 7 个 handler 各写一遍相同的 Instance 检查逻辑，可抽取辅助函数 `try_binary_magic(&mut self, a: &Object, b: Object, method_name: &str) -> Option<Result<Object>>`，返回 `Some(result)` 表示命中 magic method，`None` 表示 fallback 到内置。

### 2. 比较运算魔术方法

> **仅检查左操作数**：与算术运算一致，比较运算仅检查左操作数 `a` 的魔术方法。`1 == Vector(1,2)` 中 `a=Int` 无 `__eq__`，fallback 到内置比较返回 false。此为设计决策（无反射比较方法）。

```rust
OpCode::Equal => {
    let b = self.pop()?;
    let a = self.pop()?;

    // 检查 a 是否为 Instance 且有 __eq__
    if let Object::Ref(ptr) = &a {
        if unsafe { (**ptr).type_tag } == TypeTag::INSTANCE as u8 {
            let class_ptr = unsafe { read_instance(*ptr) }.class;
            if let Some(method_ptr) = unsafe { read_class(class_ptr) }.find_method("__eq__") {
                let result = self.invoke_method(method_ptr, a, &[b])?;
                self.push(result)?;
                return Ok(());
            }
        }
    }

    // 内置比较（Object::PartialEq，task 20）
    self.push(Object::Bool(a == b))?;
}
```

同理 NotEqual→`__ne__`、Less→`__lt__`、LessEqual→`__le__`、Greater→`__gt__`、GreaterEqual→`__ge__`。

> **`__ne__` 自动派生**：用户类只定义 `__eq__` 未定义 `__ne__` 时，`!=` 运算 fallback 到内置比较（Object PartialEq，对 Instance 比较指针身份）。若需 `not __eq__` 语义，用户须显式定义 `__ne__`。Object 基类（task 42）的 `__ne__` 为 `not (self == other)`，通过继承链自动可用。

### 3. 字符串表示魔术方法

扩展现有 `object_to_string`（`src/vm/builtins.rs:240`，task 40 实现），增加 `__str__` 优先级。task 40 已实现 `__repr__` fallback；本 task 补 `__str__` 优先于 `__repr__`。

```rust
/// 将 Object 转为显示字符串。Instance：优先 __str__，次 __repr__，最后默认。
/// 扩展 task 40 的 object_to_string（src/vm/builtins.rs:240）。
pub fn object_to_string(vm: &mut VM, obj: &Object) -> Result<String, String> {
    if let Object::Ref(ptr) = obj {
        if unsafe { (**ptr).type_tag } == TypeTag::INSTANCE as u8 {
            let inst_ptr = *ptr;
            let class_ptr = unsafe { read_instance(inst_ptr) }.class;

            // 1. 优先 __str__
            if let Some(str_ptr) = unsafe { read_class(class_ptr) }.find_method("__str__") {
                let result = vm.invoke_method(str_ptr, obj.clone(), &[])?;
                return object_to_rust_string(&result)?;  // 校验返回值为 String
            }
            // 2. 其次 __repr__
            if let Some(repr_ptr) = unsafe { read_class(class_ptr) }.find_method("__repr__") {
                let result = vm.invoke_method(repr_ptr, obj.clone(), &[])?;
                return object_to_rust_string(&result)?;
            }
            // 3. 默认
            let cls_name = unsafe { read_class(class_ptr) }.name.clone();
            return Ok(format!("<{} instance>", cls_name));
        }
    }
    // 内置类型：用 Object 的 Display impl（task 20）
    Ok(format!("{}", obj))
}

/// 从 Object（预期为 String）提取 Rust String。
fn object_to_rust_string(obj: &Object) -> Result<String, String> {
    match obj {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::STRING as u8 => {
            Ok(unsafe { read_str(*ptr) }.to_string())
        }
        _ => Err("magic method must return a string".into()),
    }
}
```

> **返回值类型校验**：`__str__` / `__repr__` 返回非 String 类型时报错（`object_to_rust_string` 校验 `type_tag == STRING`）。

### 4. 容器协议魔术方法

GET_INDEX handler（`src/vm/mod.rs:2544`）当前调用 `get_item(obj, key)`。扩展：先检查 Instance `__getitem__`，再 fallback 到内置 `get_item`。

```rust
OpCode::GetIndex => {
    let key = self.pop()?;
    let obj = self.pop()?;

    // Instance __getitem__ 分派
    if let Object::Ref(ptr) = &obj {
        if unsafe { (**ptr).type_tag } == TypeTag::INSTANCE as u8 {
            let class_ptr = unsafe { read_instance(*ptr) }.class;
            if let Some(method_ptr) = unsafe { read_class(class_ptr) }.find_method("__getitem__") {
                let result = self.invoke_method(method_ptr, obj, &[key])?;
                self.push(result)?;
                return Ok(());
            }
            return Err("'instance' object is not subscriptable".into());
        }
    }

    // 内置类型下标（task 22/24 get_item）
    self.push(get_item(obj, key)?)?;
}
```

SET_INDEX handler（`src/vm/mod.rs:2553`）当前调用 `set_item(obj, key, val)`。扩展：先检查 Instance `__setitem__`。

```rust
OpCode::SetIndex => {
    let key = self.pop()?;
    let obj = self.pop()?;
    let val = self.pop()?;

    // Instance __setitem__ 分派
    if let Object::Ref(ptr) = &obj {
        if unsafe { (**ptr).type_tag } == TypeTag::INSTANCE as u8 {
            let class_ptr = unsafe { read_instance(*ptr) }.class;
            if let Some(method_ptr) = unsafe { read_class(class_ptr) }.find_method("__setitem__") {
                self.invoke_method(method_ptr, obj, &[key, val])?;
                return Ok(());
            }
            return Err("'instance' object does not support item assignment".into());
        }
    }

    // 内置类型 set_item（task 22/24）
    set_item(obj, key, val)?;
}
```

> **`__len__` 在 `builtin_len` 中分派**：扩展 `src/vm/builtins.rs:290` 的 `builtin_len`——在 INSTANCE 分支调用 `__len__` 方法并期望返回 Int。验证标准 #5 覆盖。

### 5. __call__ 魔术方法

CALL handler（`src/vm/mod.rs` `call_value`，已有 CLASS/CLOSURE/BOUND_METHOD/NATIVE 分支）新增 INSTANCE 分支：当 callee 是 Instance 且类有 `__call__` 方法时，替换栈上 callee 为 BoundMethod，走已有 BOUND_METHOD 调用路径。

```rust
// 在 call_value（src/vm/mod.rs:549）中，检查 argc+1 位置的 callee：
fn call_value(&mut self, argc: usize) -> Result<(), String> {
    // ... 现有代码 peek callee ...
    let callee_idx = self.stack.len() - argc - 1;
    let callee = self.peek(0)?; // 或 peek(argc)

    // INSTANCE __call__ 分派
    if let Object::Ref(ptr) = callee {
        if unsafe { (**ptr).type_tag } == TypeTag::INSTANCE as u8 {
            let class_ptr = unsafe { read_instance(*ptr) }.class;
            if let Some(method_ptr) = unsafe { read_class(class_ptr) }.find_method("__call__") {
                let bound = alloc_bound_method(self.stack[callee_idx].clone(), method_ptr);
                self.stack[callee_idx] = bound;
                // 重新走 call_value（此时 callee 为 BOUND_METHOD）
                return self.call_value(argc);
            }
            return Err("object is not callable".into());
        }
    }
    // ... 现有 CLASS/CLOSURE/BOUND_METHOD/NATIVE 分支 ...
}
```

### 6. __contains__ 与 in 运算符

IN handler（`src/vm/mod.rs:1515`）当前仅支持 String 子串检查。扩展：先检查 Instance `__contains__`，再 fallback 到内置。

```rust
OpCode::In => {
    let container = self.pop()?;
    let item = self.pop()?;

    // Instance __contains__ 分派
    if let Object::Ref(ptr) = &container {
        if unsafe { (**ptr).type_tag } == TypeTag::INSTANCE as u8 {
            let class_ptr = unsafe { read_instance(*ptr) }.class;
            if let Some(method_ptr) = unsafe { read_class(class_ptr) }.find_method("__contains__") {
                let result = self.invoke_method(method_ptr, container, &[item])?;
                self.push(result)?;
                return Ok(());
            }
            // 无 __contains__：报错（不回退到迭代，迭代器协议由 task 32 处理）
            return Err("argument of type 'instance' is not iterable".into());
        }
    }

    // 内置类型 in 检查（String 子串、List/Set/Dict 成员 — task 22/24）
    let result = container.contains_str(&item)?;
    self.push(result)?;
}
```

> **无 `__contains__` 时不回退迭代**：Python 中无 `__contains__` 时回退到 `__iter__` 迭代检查。mslang MVP 不实现此回退（迭代器协议属 task 32/43 §10）；若需回退，由后续 task 补全。当前直接报错。

### 7. INVOKE 优化指令

INVOKE（`OpCode::Invoke`，操作数 `name_idx(2), argc(1)`）合并 GET_ATTR + CALL，避免创建 BoundMethod 中间对象。编译器在 `obj.method(args)` 模式时可选用 INVOKE 代替 GET_ATTR + CALL。

> **本 task 可选实现**：INVOKE 是性能优化，不影响正确性。若实现复杂度过高，可先落地 GET_ATTR + CALL 路径（已由 task 41 实现），INVOKE 留作后续优化。验证标准 #8 标注为"可选"。

```rust
OpCode::Invoke => {
    let name_idx = self.read_u16()? as usize;
    let argc = self.read_byte()? as usize;

    // bounds-checked 常量读取（task 40 模式）
    let name_obj = self.constants.get(name_idx)
        .ok_or_else(|| format!("constant index {} out of range", name_idx))?;
    let name = match name_obj {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::STRING as u8 => {
            unsafe { read_str(*ptr) }.to_owned()
        }
        _ => return Err("INVOKE expects a string constant".into()),
    };

    let receiver = self.peek(argc)?;

    // 直接在 receiver 上查找方法并调用（不创建 BoundMethod）
    if let Object::Ref(ptr) = receiver {
        if unsafe { (**ptr).type_tag } == TypeTag::INSTANCE as u8 {
            let class_ptr = unsafe { read_instance(*ptr) }.class;
            if let Some(method_ptr) = unsafe { read_class(class_ptr) }.find_method(&name) {
                // 直接设置栈基址为 receiver slot（self），call_value 处理参数
                let callee_idx = self.stack.len() - argc - 1;
                self.stack[callee_idx] = Object::Ref(method_ptr); // closure 替换
                // 注：此方案需配合 call_value 的 CLOSURE 路径 + receiver 写入 slot 0
                // 或使用 invoke_method 简化（但创建 BoundMethod，失去优化意义）
                return self.call_value(argc);
            }
            return Err(format!("no method '{}'", name));
        }
    }
    Err("INVOKE target is not an Instance".into())
}
```

### 8. 复用现有 invoke_method

**不新建 `invoke_magic`**。`src/vm/mod.rs:821-840` 已有 `invoke_method`：

```rust
fn invoke_method(
    &mut self,
    closure_ptr: *mut MsObjHeader,
    receiver: Object,
    extra_args: &[Object],
) -> Result<Object, String> {
    let bound = alloc_bound_method(receiver, closure_ptr);
    self.push(bound)?;
    let mut argc = 0usize;
    for a in extra_args {
        self.push(a.clone())?;
        argc += 1;
    }
    let caller_depth = self.call_stack.len();
    self.call_value(argc)?;
    if self.call_stack.len() > caller_depth {
        self.run_loop(Some(caller_depth))?;
    }
    self.pop()
}
```

本 task 全文 `invoke_method(method_ptr, receiver, &[args...])` 调用此函数。它内部完成 BoundMethod 创建、参数压栈、call_value、嵌套 run_loop、结果弹出——即原 §8 `invoke_magic` 想做的全部工作。

> **GC 安全**：`invoke_method` 在调用前将 receiver 和 args push 到值栈（栈是 GC 根集，`14-gc.md:608`），嵌套 `run_loop` 期间 GC 安全点触发时这些值作为根被扫描，不会悬垂。opcode handler 中 `invoke_method` 返回后用 `return Ok(())` 提前退出，不继续使用已弹出的局部 `a`/`b`。

> **`run_loop(Some(caller_depth))`** 是已有的嵌套执行机制（task 39 Generator、task 38 with 均使用），不是新发明的 `run_until_return`。

### 9. 与既有 task 的集成点

#### `__del__`（已由 task 40/52 覆盖）

`__del__` 的注册（`has_finalizer` 标志）由 [40-class-definition](./40-class-definition.md) §13 实现；`run_finalizers` 调用 `__del__(self)` 由 [52-gc](./52-gc.md) 落地。**本 task 无需额外实现 `__del__`**。

#### `__enter__` / `__exit__`（with 语句，task 38 协作）

[38-with-statement](./38-with-statement.md) §1 用 dict 模拟上下文管理器。本 task 落地后，`with` 的 GET_ATTR `__enter__`/`__exit__` 走 Instance 路径（task 41 GET_ATTR + task 42 继承链）——返回 BoundMethod → CALL 自动绑定 self。**无需修改 with handler**。task 38 的临时 DICT 分支可删除。

#### `__iter__` / `__next__`（for-in，task 32 协作）

`FOR_ITER` handler 检查目标为 Instance 时调用 `__next__`；StopIteration（task 37）结束迭代。`__iter__` 在 for-in 初始化时调用，返回 self 即可。

> **范围说明**：`__iter__`/`__next__` 完整 for-in 集成可作为本 task 一部分或拆分子 task。

#### `len()` builtin 扩展

`src/vm/builtins.rs:290` `builtin_len` 当前不处理 INSTANCE。扩展 INSTANCE 分支调用 `__len__`，校验返回 Int。

## 验证标准

1. `__repr__` / `__str__` 在 print/str 时自动调用（`__str__` 优先）
2. 算术运算符自动调用对应魔术方法（`+`→`__add__` 等 7 种）
3. 比较运算符自动调用对应魔术方法（`==`→`__eq__` 等 6 种）
4. `__call__` 使实例可调用（`obj(args)` 路径）
5. `__len__` 使 len() 工作（`builtin_len` INSTANCE 分派，§9）
6. `__getitem__` / `__setitem__` 使下标访问工作（GET_INDEX + SET_INDEX，§4）
7. `__contains__` 使 in 运算符工作（IN handler，§6）
8. **（可选）** INVOKE 优化正确工作（§7，不影响正确性）
9. **`__str__` 返回非 String 报错**（§3 `object_to_rust_string`）
10. **`__enter__`/`__exit__` 经 Instance 路径工作**（§9，task 38 临时分支可删除）
11. **二元运算仅检查左操作数**：`Int + Instance` 报 TypeError（无反射运算符，§1 注）

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
        return len(self.items)
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

## 设计规格回写（spec writeback）

- **`11-bytecode-vm.md`**：无需改动（INVOKE 操作数编码 `name_idx(2), argc(1)` 已在标准中定义，`:155`）。
- **`06-oop.md`**：无需改动（魔术方法语义未变）。
- **`10-builtins.md`**：`len()` 对 INSTANCE 有 `__len__` 时分派（§9 扩展 `builtin_len`）。
- **`tasks/38-with-statement.md`**：临时 DICT GET_ATTR 分支（`:72`）可标注"task 43 落地后删除"。

## 与后续 task 的协作约定

- **task 32（for-in）**：`__iter__`/`__next__` 的 FOR_ITER 集成可由本 task 或独立子 task 实现。最小方案：FOR_ITER 检测 Instance → `__next__` → StopIteration 结束。
- **task 38（with）**：本 task 落地后 `with` 自动走 Instance `__enter__`/`__exit__` 路径。task 38 的临时 DICT 分支可清理。
- **task 40/52（__del__）**：`__del__` 已由 task 40（注册 has_finalizer）+ task 52（run_finalizers）覆盖，本 task 无额外工作。
- **task 50/51（内置方法）**：String/List 等内置类型的魔术方法（如 `str.__add__`）由 task 50/51 在内置层实现，不走 Instance magic method 分派。本 task 的 magic method 机制仅针对用户定义的 Instance。
