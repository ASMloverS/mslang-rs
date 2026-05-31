# Class 定义与实例化

## 所属阶段
Phase 5.1 - 类 + OOP

## 前置任务
39-generator-yield

## 目标
实现 class 定义、实例化、实例方法、类属性、`__init__` 构造方法、`__repr__` 字符串表示。

## 设计规格

参照 [06-oop](../06-oop.md) § 类定义 / 实例化：

### 语法

```
class_def  = "class" IDENTIFIER ("<" IDENTIFIER)? "{" class_body "}"
class_body = (method_def | class_var)*
method_def = "fn" IDENTIFIER "(" param_list? ")" block
class_var  = "var"? IDENTIFIER "=" expression
```

### 字节码指令

参照 [11-bytecode-vm](../11-bytecode-vm.md) § 类与实例：

| OpCode | 操作数 | 说明 |
|---|---|---|
| `CLASS` | `name_idx(2)` | 创建类 |
| `METHOD` | `name_idx(2)` | 定义方法 |

### 运行时对象

```
Class {
    name: String
    methods: HashMap<String, *mut MsObjHeader>  // 每项指向 MsClosure
    parent: Option<*mut MsObjHeader>             // 指向父类 MsClass，或 null
}

Instance {
    class: *mut MsObjHeader   // 指向 MsClass
    fields: HashMap<String, Object>
}
```

### 实例化语义

`ClassName(args)` 等价于：
1. 创建新的 Instance 对象（fields 为空）
2. 查找 `__init__` 方法
3. 如果存在，将 Instance 和 args 传入调用
4. 返回该 Instance

### 类属性

在 class 体内（不在方法内）定义的变量为类属性，所有实例共享。通过 `ClassName.attr` 或 `self.attr`（实例无同名属性时）访问。

## 实现细节

### 1. 解析 class 定义

`src/parser/statement.rs`：

```rust
fn parse_class(&mut self) -> Result<Stmt> {
    self.consume(TokenKind::Class)?;
    let name = self.consume_identifier()?;
    
    let parent = if self.match_token(TokenKind::Less)? {
        Some(self.consume_identifier()?)
    } else {
        None
    };
    
    self.consume(TokenKind::LeftBrace)?;
    
    let mut methods = Vec::new();
    let mut class_vars = Vec::new();
    
    while !self.check(TokenKind::RightBrace)? {
        if self.check(TokenKind::Fn)? {
            methods.push(self.parse_method()?);
        } else {
            class_vars.push(self.parse_class_var()?);
        }
    }
    
    self.consume(TokenKind::RightBrace)?;
    
    Ok(Stmt::ClassDef {
        name,
        parent,
        methods,
        class_vars,
    })
}
```

### 2. AST 节点

```rust
struct ClassDef {
    name: String,
    parent: Option<String>,
    methods: Vec<MethodDef>,
    class_vars: Vec<ClassVar>,
}

struct MethodDef {
    name: String,
    params: Vec<Param>,
    body: Vec<Stmt>,
}

struct ClassVar {
    name: String,
    value: Expr,
}
```

### 3. 堆对象布局

引用 [20-object-system-basic](./20-object-system-basic.md) 的 `MsObjHeader` 和 `TypeTag`。本任务新增：

```rust
/// Class 堆对象（TypeTag::CLASS = 8）
#[repr(C)]
pub struct MsClass {
    pub header:      MsObjHeader,
    pub name:        String,
    pub methods:     HashMap<String, *mut MsObjHeader>,  // 指向 MsClosure
    pub parent:      Option<*mut MsObjHeader>,           // 指向 MsClass
    pub class_attrs: HashMap<String, Object>,
}

/// Instance 堆对象（TypeTag::INSTANCE = 9）
#[repr(C)]
pub struct MsInstance {
    pub header: MsObjHeader,
    pub class:  *mut MsObjHeader,        // 指向 MsClass
    pub fields: HashMap<String, Object>,
}
```

### 4. 堆分配辅助函数

```rust
/// 分配 Class 堆对象，返回 Object::Ref。
pub fn alloc_class(name: String) -> Object {
    let obj = Box::new(MsClass {
        header: MsObjHeader {
            gc_meta:   0,
            type_tag:  TypeTag::CLASS as u8,
            size:      std::mem::size_of::<MsClass>() as u16,
            _padding:  0,
            class_ptr: 0,
        },
        name,
        methods: HashMap::new(),
        parent: None,
        class_attrs: HashMap::new(),
    });
    Object::Ref(Box::into_raw(obj) as *mut MsObjHeader)
}

/// 读取 MsClass 内容。
///
/// # Safety
/// `ptr` 必须指向由 `alloc_class` 分配的有效 `MsClass`。
pub unsafe fn read_class(ptr: *mut MsObjHeader) -> &'static mut MsClass {
    &mut *(ptr as *mut MsClass)
}

/// 分配 Instance 堆对象，返回 Object::Ref。
pub fn alloc_instance(class_ptr: *mut MsObjHeader) -> Object {
    let obj = Box::new(MsInstance {
        header: MsObjHeader {
            gc_meta:   0,
            type_tag:  TypeTag::INSTANCE as u8,
            size:      std::mem::size_of::<MsInstance>() as u16,
            _padding:  0,
            class_ptr: class_ptr as u64,
        },
        class: class_ptr,
        fields: HashMap::new(),
    });
    Object::Ref(Box::into_raw(obj) as *mut MsObjHeader)
}

/// 读取 MsInstance 内容。
///
/// # Safety
/// `ptr` 必须指向由 `alloc_instance` 分配的有效 `MsInstance`。
pub unsafe fn read_instance(ptr: *mut MsObjHeader) -> &'static mut MsInstance {
    &mut *(ptr as *mut MsInstance)
}
```

### 5. 编译 class 定义

`src/compiler/statement.rs`：

```
编译 class Animal { kingdom = "Animalia"; fn __init__(...) {...}; fn speak(...) {...} }:

1. emit CLASS "Animal"           → 创建类对象，压栈
2. [如果有父类] emit LOAD_GLOBAL parent; emit INHERIT
3. 编译类属性:
   编译 value 表达式
   emit SET_ATTR "kingdom"       → 设置在类对象上
4. 编译方法:
   对每个方法:
   a. 创建新的编译单元
   b. 编译方法体
   c. emit CLOSURE method_func
   d. emit METHOD "method_name"  → 添加到类
5. emit STORE_GLOBAL "Animal"    → 存为全局变量
```

### 6. CLASS 指令实现

```rust
OpCode::CLASS => {
    let name_idx = self.read_u16();
    let name_obj = &self.constants[name_idx as usize];
    let name = match name_obj {
        Object::Ref(ptr) if unsafe { (*(*ptr)).type_tag } == TypeTag::STRING as u8 => {
            unsafe { read_str(*ptr) }.to_owned()
        }
        _ => return self.runtime_error("CLASS expects a string name"),
    };
    self.stack_push(alloc_class(name));
}
```

### 7. METHOD 指令实现

```rust
OpCode::METHOD => {
    let name_idx = self.read_u16();
    let name = match &self.constants[name_idx as usize] {
        Object::Ref(ptr) if unsafe { (*(*ptr)).type_tag } == TypeTag::STRING as u8 => {
            unsafe { read_str(*ptr) }.to_owned()
        }
        _ => return self.runtime_error("METHOD expects a string name"),
    };
    
    let method = self.stack_pop(); // Object::Ref → MsClosure
    let class_obj = self.stack_peek_mut(0);
    
    if let Object::Ref(cls_ptr) = class_obj {
        if unsafe { (*(*cls_ptr)).type_tag } == TypeTag::CLASS as u8 {
            let class = unsafe { read_class(*cls_ptr) };
            if let Object::Ref(method_ptr) = method {
                class.methods.insert(name, method_ptr);
            }
        }
    }
}
```

### 8. 类实例化（调用 Class）

当 `CALL` 的目标是 `Object::Ref` 且 type_tag 为 `TypeTag::CLASS` 时：

```rust
fn call_class(&mut self, cls_ptr: *mut MsObjHeader, argc: u8) -> Result<()> {
    // 创建实例
    let inst_obj = alloc_instance(cls_ptr);
    
    // 弹出 callee（class）和参数
    let args: Vec<Object> = (0..argc).rev()
        .map(|_| self.stack_pop())
        .collect();
    self.stack_pop(); // 弹出 class
    
    // 压入实例
    self.stack_push(inst_obj.clone());
    
    // 调用 __init__（如果存在）
    let class = unsafe { read_class(cls_ptr) };
    if let Some(init_ptr) = class.methods.get("__init__").copied() {
        // 绑定 self，构造 BoundMethod（task 41 定义 alloc_bound_method）
        let bound = alloc_bound_method(inst_obj, init_ptr);
        self.stack_push(bound);
        for arg in args {
            self.stack_push(arg);
        }
        self.call(argc + 1); // self + args
    }
    
    Ok(())
}
```

### 9. GET_ATTR / SET_ATTR 实现

```rust
OpCode::GET_ATTR => {
    let name_idx = self.read_u16();
    let name = match &self.constants[name_idx as usize] {
        Object::Ref(ptr) if unsafe { (*(*ptr)).type_tag } == TypeTag::STRING as u8 => {
            unsafe { read_str(*ptr) }.to_owned()
        }
        _ => return self.runtime_error("GET_ATTR expects a string name"),
    };
    
    let obj = self.stack_pop();
    match &obj {
        Object::Ref(ptr) if unsafe { (*(*ptr)).type_tag } == TypeTag::INSTANCE as u8 => {
            let inst = unsafe { read_instance(*ptr) };
            // 先查实例字段
            if let Some(val) = inst.fields.get(&name) {
                self.stack_push(val.clone());
            }
            // 再查类方法
            else if let Some(method_ptr) = unsafe { read_class(inst.class) }.methods.get(&name).copied() {
                let bound = alloc_bound_method(obj, method_ptr);
                self.stack_push(bound);
            }
            // 最后查类属性
            else if let Some(val) = unsafe { read_class(inst.class) }.class_attrs.get(&name) {
                self.stack_push(val.clone());
            }
            else {
                let cls_name = &unsafe { read_class(inst.class) }.name;
                return Err(runtime_error(format!(
                    "'{}' has no attribute '{}'", cls_name, name
                )));
            }
        }
        Object::Ref(ptr) if unsafe { (*(*ptr)).type_tag } == TypeTag::CLASS as u8 => {
            let cls = unsafe { read_class(*ptr) };
            if let Some(method_ptr) = cls.methods.get(&name).copied() {
                let closure_obj = Object::Ref(method_ptr);
                self.stack_push(closure_obj);
            } else if let Some(val) = cls.class_attrs.get(&name) {
                self.stack_push(val.clone());
            } else {
                return Err(runtime_error(format!(
                    "class '{}' has no attribute '{}'", cls.name, name
                )));
            }
        }
        _ => { /* 内置类型的属性 */ }
    }
}

OpCode::SET_ATTR => {
    let name_idx = self.read_u16();
    let name = match &self.constants[name_idx as usize] {
        Object::Ref(ptr) if unsafe { (*(*ptr)).type_tag } == TypeTag::STRING as u8 => {
            unsafe { read_str(*ptr) }.to_owned()
        }
        _ => return self.runtime_error("SET_ATTR expects a string name"),
    };
    
    let value = self.stack_pop();
    let obj = self.stack_pop();
    
    match obj {
        Object::Ref(ptr) if unsafe { (*ptr).type_tag } == TypeTag::INSTANCE as u8 => {
            unsafe { read_instance(ptr) }.fields.insert(name, value);
            self.stack_push(Object::Nil);
        }
        Object::Ref(ptr) if unsafe { (*ptr).type_tag } == TypeTag::CLASS as u8 => {
            unsafe { read_class(ptr) }.class_attrs.insert(name, value);
            self.stack_push(Object::Nil);
        }
        _ => return Err(runtime_error("cannot set attribute")),
    }
}
```

### 10. print() 与 __repr__

当 `print(obj)` 时，检查 obj 是否为 Instance 且其 class 有 `__repr__` 方法。如果有，调用 `__repr__` 获取字符串；否则输出默认格式如 `<ClassName instance>`。

## 验证标准

1. class 定义创建类对象并存入全局变量
2. 类属性在所有实例间共享
3. `ClassName(args)` 创建实例并调用 `__init__`
4. 实例方法通过 `obj.method(args)` 调用，self 自动绑定
5. `__repr__` 被 print/str 调用
6. `self.attr` 正确读写实例字段
7. 动态属性赋值（`obj.new_attr = val`）正确

## 测试用例

```ms
// test_class.ms — Class 定义与实例化

class Animal {
    kingdom = "Animalia"
    
    fn __init__(self, name, sound) {
        self.name = name
        self.sound = sound
    }
    
    fn speak(self) {
        return self.name + " says " + self.sound
    }
    
    fn __repr__(self) {
        return "Animal(" + self.name + ")"
    }
}

dog = Animal("Dog", "Woof")
print(dog.speak())
print(dog)
print(dog.kingdom)

// 动态属性
dog.age = 3
print(dog.age)

// 类属性共享
cat = Animal("Cat", "Meow")
print(cat.kingdom)
print(Animal.kingdom)

// 多个实例互不干扰
dog2 = Animal("Another Dog", "Bark")
print(dog2.speak())
print(dog.speak())

// __repr__ 用于 print
class Point {
    fn __init__(self, x, y) {
        self.x = x
        self.y = y
    }
    
    fn __repr__(self) {
        return "Point(" + str(self.x) + ", " + str(self.y) + ")"
    }
}

p = Point(3, 4)
print(p)
```

预期输出：

```
Dog says Woof
Animal(Dog)
Animalia
3
Animalia
Animalia
Another Dog says Bark
Dog says Woof
Point(3, 4)
```
