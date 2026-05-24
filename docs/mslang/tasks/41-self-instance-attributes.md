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
    receiver: Object,     // self 绑定的实例
    method: Gc<Closure>,  // 方法闭包
}
```

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

### 1. BoundMethod 对象

`src/vm/object.rs`：

```rust
struct BoundMethod {
    receiver: Object,       // 绑定的实例
    method: Gc<Closure>,    // 方法闭包
}
```

在 Object 枚举中已包含：

```rust
enum Object {
    // ...
    BoundMethod(Gc<BoundMethod>),
}
```

### 2. BoundMethod 调用

当 `CALL` 目标是 `BoundMethod` 时：

```rust
fn call_bound_method(&mut self, bound: &BoundMethod, argc: u8) -> Result<()> {
    // 将 receiver（self）作为第一个参数
    // 然后传入用户提供的 argc 个参数
    
    // 设置新 CallFrame
    let frame = CallFrame {
        closure: bound.method.clone(),
        ip: 0,
        stack_base: self.stack.len() - argc as usize - 1, // 包含 callee
        defer_stack_base: self.defer_stack.len(),
    };
    
    // 栈布局已正确：[bound_method, arg0, arg1, ...]
    // 方法的第 0 个局部变量（self）绑定到 receiver
    // 这在方法编译时 self 已作为第一个参数声明
    
    self.call_stack.push(frame);
    Ok(())
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

需要在 CALL 中替换 BoundMethod 为其 receiver：

```rust
// 在设置新帧前
let receiver = bound.receiver.clone();
// 替换栈底（callee 位置）为 receiver
self.stack[frame_base] = receiver;
```

### 3. 属性查找实现

`src/vm/object.rs` 或 `src/vm/attrs.rs`：

```rust
impl Class {
    fn find_method(&self, name: &str) -> Option<Gc<Closure>> {
        if let Some(method) = self.methods.get(name) {
            return Some(method.clone());
        }
        if let Some(parent) = &self.parent {
            return parent.find_method(name);
        }
        None
    }
}
```

完整的属性查找：

```rust
fn get_attribute(obj: &Object, name: &str) -> Result<AttrResult> {
    match obj {
        Object::Instance(inst) => {
            // 1. 实例字段
            if let Some(val) = inst.fields.get(name) {
                return Ok(AttrResult::Value(val.clone()));
            }
            // 2. 类方法（含继承链）
            if let Some(method) = inst.class.find_method(name) {
                return Ok(AttrResult::Method(obj.clone(), method));
            }
            // 3. 类属性
            if let Some(val) = find_class_attr(&inst.class, name) {
                return Ok(AttrResult::Value(val.clone()));
            }
            Err(...)
        }
        // 其他类型...
    }
}

enum AttrResult {
    Value(Object),
    Method(Object, Gc<Closure>),  // receiver, method → BoundMethod
}
```

### 4. self 编译处理

在方法编译时，`self` 被声明为第 0 个局部变量：

```rust
fn compile_method(&mut self, class_name: &str, method: &MethodDef) -> Result<()> {
    self.begin_scope();
    
    // self 作为第一个参数（slot 0）
    self.declare_local("self");
    
    // 声明其他参数
    for param in &method.params {
        self.declare_local(&param.name);
    }
    
    // 编译方法体
    for stmt in &method.body {
        self.compile_stmt(stmt)?;
    }
    
    // 隐式 return nil
    self.emit(OpCode::NIL);
    self.emit(OpCode::RETURN);
    
    self.end_scope();
}
```

### 5. 实例属性存储

`src/vm/object.rs`：

```rust
struct Instance {
    class: Gc<Class>,
    fields: RefCell<HashMap<String, Object>>,  // 使用 RefCell 允许在 Gc 内修改
}
```

`SET_ATTR` 对 Instance 操作：

```rust
OpCode::SET_ATTR => {
    let name_idx = self.read_u16();
    let name = &self.constants[name_idx as usize].to_string();
    
    let value = self.stack_pop();
    let obj = self.stack_pop();
    
    match obj {
        Object::Instance(ref inst) => {
            inst.fields.borrow_mut().insert(name.clone(), value);
        }
        Object::Class(ref cls) => {
            cls.set_class_attr(name.clone(), value);
        }
        _ => return Err(runtime_error("cannot set attribute on this type")),
    }
}
```

## 验证标准

1. `self` 在方法内正确引用当前实例
2. `obj.method(args)` 自动绑定 self
3. 实例属性读写正确
4. 动态属性添加正确
5. 属性查找遵循 实例字段 → 类方法 → 类属性 → 父类 的顺序
6. 不同实例的字段互不干扰

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
