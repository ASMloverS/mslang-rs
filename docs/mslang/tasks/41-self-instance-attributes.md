# self 绑定与实例属性

## 所属阶段
Phase 5.2 - 类 + OOP

## 前置任务
40-class-definition

## 目标
完善 self 绑定机制（BoundMethod）、实例属性存储与查找、动态属性添加、属性查找链（实例字段 → 类方法 → 父类）。

## 设计规格

参照 [06-oop](../06-oop.md) § self / 实例属性 / 类属性：

### self 绑定

- `self` 是实例方法的第一个参数，引用当前实例
- `self` 不需要在调用时显式传入（编译器自动绑定）
- 调用 `obj.method(args)` 时，VM 创建 BoundMethod，将 `obj` 绑定为 `self`

### BoundMethod

参照 [11-bytecode-vm](../11-bytecode-vm.md) § 对象系统：

```
BoundMethod {
    receiver: Object,          // self 绑定的实例
    method: *mut MsObjHeader,  // 指向 MsClosure
}
```

BoundMethod 以 `Object::Ref(*mut MsObjHeader)` 存储，type_tag 为 `TypeTag::BOUND_METHOD`。

### 属性查找顺序

1. 实例字段（`instance.fields`）
2. 类方法（`class.methods`）→ 返回 BoundMethod
3. 类属性（class 体内定义的变量）
4. 父类方法 → 父类属性 → ... → Object

### 动态属性

```ms
obj.new_attr = value  // 运行时动态添加到 instance.fields
```

## 实现细节

### 1. BoundMethod 堆对象

`src/vm/object.rs`，引用 [20-object-system-basic](./20-object-system-basic.md) 的 `MsObjHeader`：

```rust
/// BoundMethod 堆对象（TypeTag::BOUND_METHOD = 15）
#[repr(C)]
pub struct MsBoundMethod {
    pub header:   MsObjHeader,
    pub receiver: Object,             // 绑定的实例
    pub method:   *mut MsObjHeader,   // 指向 MsClosure
}
```

```rust
/// 分配 BoundMethod 堆对象，返回 Object::Ref。
/// MVP：Box 分配；task 52-gc 替换为 TLAB bump 分配。
pub fn alloc_bound_method(receiver: Object, method: *mut MsObjHeader) -> Object {
    let obj = Box::new(MsBoundMethod {
        header: MsObjHeader {
            gc_meta:   0,
            type_tag:  TypeTag::BOUND_METHOD as u8,
            size:      std::mem::size_of::<MsBoundMethod>() as u16,
            _padding:  0,
            class_ptr: 0,
        },
        receiver,
        method,
    });
    debug_assert!(std::mem::size_of::<MsBoundMethod>() <= LARGE_OBJ_THRESHOLD,
                  "MsBoundMethod too large, use LOS");
    Object::Ref(Box::into_raw(obj) as *mut MsObjHeader)
}

/// 读取 MsBoundMethod 内容。
///
/// 返回值生命周期由调用方约束（`'a`），**不可**用 `'static`——
/// 数据来自堆分配，task 52 GC 上线后会被回收（参见 task 20 read_str 约定）。
///
/// # Safety
/// `ptr` 必须指向由 `alloc_bound_method` 分配的、在 `'a` 期间保持有效的 `MsBoundMethod`。
pub unsafe fn read_bound_method<'a>(ptr: *mut MsObjHeader) -> &'a mut MsBoundMethod {
    debug_assert_eq!((*ptr).type_tag, TypeTag::BOUND_METHOD as u8,
                     "read_bound_method on non-BOUND_METHOD");
    &mut *(ptr as *mut MsBoundMethod)
}
```

### 2. BoundMethod 调用

当 `CALL` 目标是 BoundMethod 时：

```rust
Object::Ref(ptr) if unsafe { (*ptr).type_tag } == TypeTag::BOUND_METHOD as u8 => {
    let bound = unsafe { read_bound_method(ptr) };
    let closure_ptr = bound.method;
    debug_assert!(!closure_ptr.is_null(), "bound method closure pointer is null");
    debug_assert_eq!(unsafe { (*closure_ptr).type_tag }, TypeTag::CLOSURE as u8,
                     "bound method not pointing to closure");
    let receiver = bound.receiver.clone();

    // 栈基址 = callee(BoundMethod) 所在 slot
    let frame_base = self.stack.len() - argc as usize - 1;

    // 将 receiver（self）替换到栈上 callee 的位置（slot 0）
    self.stack[frame_base] = receiver;

    // 设置新 CallFrame，closure 指向 MsClosure
    let frame = CallFrame {
        closure: closure_ptr,
        ip: 0,
        stack_base: frame_base,
        defer_stack_base: self.defer_stack.len(),
    };
    self.call_stack.push(frame);
}
```

关键：当 `GET_ATTR` 找到方法时，创建 BoundMethod 并压栈。调用时，BoundMethod 的 receiver 被放在方法局部变量 slot 0 的位置（即 self）。

实际栈布局：

```
调用 obj.method(a, b):

1. GET_ATTR "method" → 栈: [BoundMethod{obj, method}]
2. PUSH a            → 栈: [BoundMethod, a]
3. PUSH b            → 栈: [BoundMethod, a, b]
4. CALL 2

CALL 处理:
- argc = 2
- callee = BoundMethod
- 新帧栈布局: [BoundMethod, a, b]
  - slot 0 = BoundMethod.receiver (self)
  - slot 1 = a
  - slot 2 = b
```

> **call_class 切换**：[40-class-definition](./40-class-definition.md) §8 的 `call_class` 在本 task 落地后须切换为 BoundMethod 方案。原方案（task 40）push `[closure, inst(self), args...]` 再 `call(argc + 1)`；本 task 改为：找到 `__init__` closure 后，`let bound = alloc_bound_method(inst_obj.clone(), init_ptr); push(bound); push(args...); call(argc)`。即 callee 为 BoundMethod，self 由 CALL handler 内部写入 slot 0，调用方不再显式 push self。无 `__init__` 时同理：若类有其他构造路径需绑 self，也走 BoundMethod。

> **GET_ATTR 切换**：[40-class-definition](./40-class-definition.md) §9 GET_ATTR 的 Instance 分支找到方法时，原返回 `Object::Ref(m)`（裸 closure），本 task 改为 `self.push(alloc_bound_method(obj.clone(), m))?`，使后续 CALL 自动绑定 self。

### 3. 属性查找实现

`src/vm/object.rs` 或 `src/vm/attrs.rs`：

```rust
impl MsClass {
    /// 沿继承链查找方法。单继承下链路线性，深度有限。
    pub unsafe fn find_method(&self, name: &str) -> Option<*mut MsObjHeader> {
        if let Some(&ptr) = self.methods.get(name) {
            return Some(ptr);
        }
        if let Some(parent_ptr) = self.parent {
            return read_class(parent_ptr).find_method(name);
        }
        None
    }

    /// 沿继承链查找类属性。
    pub unsafe fn find_class_attr(&self, name: &str) -> Option<Object> {
        if let Some(val) = self.class_attrs.get(name) {
            return Some(val.clone());
        }
        if let Some(parent_ptr) = self.parent {
            return read_class(parent_ptr).find_class_attr(name);
        }
        None
    }
}
```

> **free function 形式**：`get_attribute` 调用的是 `find_class_attr(inst.class, name)`，等价于 `unsafe { read_class(inst.class).find_class_attr(name) }`。若 `read_class` 借用冲突，先 copy 出 `class_ptr: *mut MsObjHeader`（`*mut` is `Copy`），再 `read_class(class_ptr).find_class_attr(name)`（参照 task 40 §9 V5 修复的别名可变借用规避）。

完整的属性查找（仅替换 [40-class-definition](./40-class-definition.md) §9 GET_ATTR 的 **Instance 分支**；Class 分支与内置类型分支保持 task 40/50/51 既有实现不动）：

```rust
/// Instance 属性查找：实例字段 → 类方法(含继承链) → 类属性(含继承链) → __name__ → Err。
/// 仅处理 Instance；Class / String / List 等由 GET_ATTR 其他分支处理（task 40/50/51）。
fn get_instance_attribute(obj: &Object, name: &str) -> Result<AttrResult> {
    match obj {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::INSTANCE as u8 => {
            let inst_ptr = *ptr;
            // 先 copy 出 class 指针，避免 read_instance 与 read_class 别名可变借用（task 40 V5）
            let class_ptr = unsafe { read_instance(inst_ptr) }.class;

            // 1. 实例字段
            if let Some(val) = unsafe { read_instance(inst_ptr) }.fields.get(name) {
                return Ok(AttrResult::Value(val.clone()));
            }
            // 2. 类方法（含继承链）→ 返回 BoundMethod 材料
            if let Some(method_ptr) = unsafe { read_class(class_ptr) }.find_method(name) {
                return Ok(AttrResult::Method(obj.clone(), method_ptr));
            }
            // 3. 类属性（含继承链）
            if let Some(val) = unsafe { read_class(class_ptr) }.find_class_attr(name) {
                return Ok(AttrResult::Value(val));
            }
            // 4. __name__ 内置属性（task 40 §12）
            if name == "__name__" {
                let n = unsafe { read_class(class_ptr) }.name.clone();
                return Ok(AttrResult::Value(alloc_string(n)));
            }
            let cls_name = unsafe { read_class(class_ptr) }.name.clone();
            Err(format!("'{}' instance has no attribute '{}'", cls_name, name))
        }
        _ => Err("get_instance_attribute called on non-Instance".into()),
    }
}

enum AttrResult {
    Value(Object),
    Method(Object, *mut MsObjHeader),  // receiver, method_ptr → 用于构造 BoundMethod
}
```

### 4. self 编译处理

在方法编译时，`self` 被声明为第 0 个局部变量。本 task 完善 task 40 §5 step 4b 的 "self 关键字绑定"。

> **AST 节点**：方法复用 `Stmt::FnDecl { name, params, body, is_async }`（[40-class-definition](./40-class-definition.md) §1），**无独立 `MethodDef` 类型**。

> **self 关键字语义**（[06-oop](../06-oop.md) § self 解析规则）：`self` 是词法层关键字（[01-lexical](../01-lexical.md) 关键字表），仅允许在方法参数列表首位作标识符。方法内闭包可通过 upvalue 捕获 `self`（`fn method(self) { return fn() { self.x } }`），捕获路径走既有 `LOAD_UPVALUE` / `STORE_UPVALUE`（task 28），本 task 不改 upvalue 机制，仅验证对 `self` 局部生效。

```rust
/// 编译方法体。补充 task 40 §5 step 4b 的 self 关键字绑定。
/// method 为 Stmt::FnDecl（无独立 MethodDef 类型 — 见 task 40 §1）。
fn compile_method(&mut self, method: &Stmt::FnDecl) -> Result<()> {
    let (mname, params, body) = match method {
        Stmt::FnDecl { name, params, body, .. } => (name, params, body),
        _ => return Err("compile_method expects FnDecl".into()),
    };

    // self 关键字校验：方法首参数必须为 self
    // （self 在词法层是关键字，仅此位置可作标识符 — 06-oop.md:56）
    if params.is_empty() || params[0].name != "self" {
        return Err(format!(
            "method '{}' must have 'self' as first parameter", mname
        ));
    }

    self.begin_scope();

    // self 作为第一个参数（slot 0），由 CALL BoundMethod handler 写入
    self.declare_local("self");

    // 声明其他参数（跳过已声明的 self）
    for param in params.iter().skip(1) {
        self.declare_local(&param.name);
    }

    // 编译方法体
    for stmt in body {
        self.compile_stmt(stmt)?;
    }

    // 隐式 return nil
    self.emit(OpCode::NIL);
    self.emit(OpCode::RETURN);

    self.end_scope();
    // 注：CLOSURE 发射由 task 40 §5 step 4c 负责，本函数不重复
    Ok(())
}
```

> **self 关键字词法限制归属**：`self` 不可作赋值目标、for 循环变量等，由 lexer/parser（task 01-02 / 15）在词法层拒绝（`self` 是关键字，不可解析为 IDENTIFIER）。本 task 的编译期校验仅负责 "方法首参数必须为 self" 这条语义规则；若方法体内出现 `self = ...` 等非法用法，应在 parser 层报错，本 task 不处理。

> **upvalue 捕获验证**：方法内闭包捕获 `self` 时，`self` 局部被标记 `is_captured = true`，CLOSURE 指令发射对应 Upvalue 项。此路径与普通局部一致（task 28），本 task 验证标准 #7 覆盖。

### 5. 实例属性存储

实例属性写入 `MsInstance.fields`，由 `SET_ATTR` 指令处理（task 40 已实现）。

`SET_ATTR` 对 Instance 操作：

```rust
Object::Ref(ptr) if unsafe { (*ptr).type_tag } == TypeTag::INSTANCE as u8 => {
    unsafe { read_instance(ptr) }.fields.insert(name.clone(), value);
}
```

## 6. GC 集成（关键：避免 Minor GC 后 method/receiver 悬垂）

参照 [14-gc](../14-gc.md) § 类型描述表、[40-class-definition](./40-class-definition.md) §11 模板、[52-gc](./52-gc.md) `:229`（`trace_bound_method // TODO task 41`）。**替换** `src/vm/gc.rs` 中 tag=15 的占位空 trace（task 52 已留 `// TODO task 41`）。

MsBoundMethod 同时持有 `receiver: Object`（可能为 `Object::Ref`）与 `method: *mut MsObjHeader`（指向 MsClosure），二者都是堆引用，必须经 trace / forward 处理，否则 Minor GC 复制 Closure / receiver 后悬垂。

```rust
/// 遍历 MsBoundMethod 内所有 Ref 槽：receiver（若为 Ref）+ method 指针。
/// 用于 Major GC 三色标记。
fn trace_bound_method(obj: *mut MsObjHeader, callback: &mut dyn FnMut(*mut MsObjHeader)) {
    let b = unsafe { read_bound_method(obj) };
    if let Object::Ref(r) = &b.receiver { callback(*r); }
    callback(b.method);
}

/// Cheney 复制时修正 MsBoundMethod 内的 Ref 槽（Minor GC）。
/// receiver 经 forwarder 修正；method 裸指针包成 Object::Ref 修正后写回。
fn forward_fields_bound_method(obj: *mut MsObjHeader, forwarder: &mut dyn FnMut(&mut Object)) {
    let b = unsafe { read_bound_method(obj) };
    forwarder(&mut b.receiver);
    let mut method_tmp = Object::Ref(b.method);
    forwarder(&mut method_tmp);
    if let Object::Ref(new) = method_tmp { b.method = new; }
}

/// Minor GC 复制：MsBoundMethod 无 HashMap 载荷，直接字段拷贝。
fn copy_for_gc_bound_method(src: *mut MsObjHeader, dst: *mut MsObjHeader) -> usize {
    let s = unsafe { read_bound_method(src) };
    let d = unsafe { read_bound_method(dst) };
    d.receiver = s.receiver.clone();
    d.method = s.method;
    std::mem::size_of::<MsBoundMethod>()
}
```

`src/vm/gc.rs` 的 `type_descriptor` match（tag 15）注册，替换 task 52 的占位：

```rust
15 => &TypeDescriptor {
    type_tag: TypeTag::BOUND_METHOD,
    name: "BOUND_METHOD",
    trace: trace_bound_method,
    finalize: None,
    size_base: std::mem::size_of::<MsBoundMethod>(),
},
```

> **alloc 方式**：MVP 用 `Box::new` + `Box::into_raw`（与 task 20/40 一致）。task 52 GC 上线后替换为 TLAB bump 分配。本 task 的 trace/forward/copy 函数在 task 52 落地后即被 GC 调用，须在 task 52 集成时验证（本 task 的验证标准 #8 用手动触发 Minor GC 的方式覆盖）。

## 验证标准

1. `self` 在方法内正确引用当前实例
2. `obj.method(args)` 自动绑定 self（BoundMethod 路径）
3. 实例属性读写正确
4. 动态属性添加正确
5. 属性查找遵循 实例字段 → 类方法 → 类属性 → 父类 的顺序
6. 不同实例的字段互不干扰
7. **方法首参数校验**：方法首参数非 `self` 时编译期报错（§4，覆盖 06-oop.md:56 关键字语义）
8. **GC 存活性**：在频繁 Minor GC 下，BoundMethod 的 `method` 与 `receiver` 指针仍可正确解引用（§6 trace/forward/copy 生效；参照 task 40 §11 验证 #9）
9. **`__name__` 不回归**：`inst.__name__` 与 `Cls.__name__` 仍返回类名（§3 `get_instance_attribute` 第 4 步，task 40 §12 能力保留）
10. **call_class 切换**：`ClassName(args)` 经 BoundMethod 路径调用 `__init__`，self 自动绑定（§2 call_class 切换说明）

## 测试用例

```ms
// test_self_attrs.ms — self 绑定与实例属性

// self 绑定与方法调用
class Point {
    fn __init__(self, x, y) {
        self.x = x
        self.y = y
    }
    
    fn distance_to(self, other) {
        dx = self.x - other.x
        dy = self.y - other.y
        return (dx * dx + dy * dy) ** 0.5
    }
    
    fn __repr__(self) {
        return "Point(" + str(self.x) + ", " + str(self.y) + ")"
    }
}

p1 = Point(3, 4)
p2 = Point(0, 0)
print(p1.distance_to(p2))
print(p1)

// 动态属性
p1.z = 10
print(p1.z)

// 属性查找顺序：实例字段优先于类属性
class Config {
    value = "class"
    
    fn __init__(self) {
        self.value = "instance"
    }
}

c = Config()
print(c.value)

// 不同实例互不干扰
a = Point(1, 0)
b = Point(0, 1)
a.label = "A"
b.label = "B"
print(a.label)
print(b.label)
print(a.x)
print(b.x)

// 方法链
class Builder {
    fn __init__(self) {
        self.parts = []
    }
    
    fn add(self, part) {
        self.parts.push(part)
        return self
    }
    
    fn build(self) {
        return self.parts.join(", ")
    }
}

result = Builder().add("a").add("b").add("c").build()
print(result)
```

预期输出：

```
5.0
Point(3, 4)
10
instance
A
B
1
0
a, b, c
```

## 设计规格回写（spec writeback）

本任务对设计文档的扩展（参照 task 40 §"设计规格回写" 惯例）：

- **`11-bytecode-vm.md` § 对象系统**：新增 BoundMethod 堆对象结构（`{ header, receiver: Object, method: *mut MsObjHeader }`，TypeTag::BOUND_METHOD = 15）。原小节仅列 Function/Closure/Class/Instance，本 task 补 BoundMethod。
- **`14-gc.md` TypeDescriptor 表（BOUND_METHOD=15 行）**：填充 trace / forward_fields / copy_for_gc 三字段实际实现（替换 task 52 占位 noop）。
- **`06-oop.md`**：无需改动（self / 实例属性 / 类属性语义未变）。

## 与后续 task 的协作约定

- **task 42（继承 / super）**：本 task 的 `find_method` / `find_class_attr` 已递归 parent 链，但 parent 在 task 42 INHERIT 落地前始终为 None。继承链查找的端到端 `.ms` 验证延后至 task 42；本 task 对 `find_method` / `find_class_attr` 的递归路径用 Rust 单测（手动构造 `class.parent = Some(...)`）覆盖。
- **task 43（魔术方法）**：本 task 仅落地 BoundMethod 调用机制；`__add__` 等运算符分派、`__str__` 优先级、INVOKE 优化指令由 task 43 实现。
- **task 52（GC）**：本 task 实现 BOUND_METHOD 的 trace/forward/copy + TypeDescriptor 注册；task 52 GC 上线后由 GC 调用这些函数，届时做端到端 GC 存活性验证。
- **task 50/51（内置方法）**：String/List 等内置类型的 BoundMethod 由 task 50/51 复用本 task 的 `MsBoundMethod` 结构与 CALL 分支。
