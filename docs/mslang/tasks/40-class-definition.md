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
    methods: HashMap<String, Gc<Closure>>
    parent: Option<Gc<Class>>
}

Instance {
    class: Gc<Class>
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

### 3. 编译 class 定义

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

### 4. CLASS 指令实现

```rust
OpCode::CLASS => {
    let name_idx = self.read_u16();
    let name = &self.constants[name_idx as usize];
    
    let class = Class {
        name: name.clone(),
        methods: HashMap::new(),
        parent: None,
    };
    
    self.stack_push(Object::Class(Gc::new(class)));
}
```

### 5. METHOD 指令实现

```rust
OpCode::METHOD => {
    let name_idx = self.read_u16();
    let name = &self.constants[name_idx as usize];
    
    let method = self.stack_pop(); // Closure
    let class = self.stack_peek_mut(0); // Class
    
    if let Object::Class(cls) = class {
        cls.methods.insert(name.clone(), method);
    }
}
```

### 6. 类实例化（调用 Class）

当 `CALL` 的目标是 `Object::Class` 时：

```rust
fn call_class(&mut self, class: Gc<Class>, argc: u8) -> Result<()> {
    // 创建实例
    let instance = Instance {
        class: class.clone(),
        fields: HashMap::new(),
    };
    let inst_obj = Object::Instance(Gc::new(instance));
    
    // 弹出 callee（class）和参数
    let args: Vec<Object> = (0..argc).rev()
        .map(|_| self.stack_pop())
        .collect();
    self.stack_pop(); // 弹出 class
    
    // 压入实例
    self.stack_push(inst_obj.clone());
    
    // 调用 __init__（如果存在）
    if let Some(init) = class.methods.get("__init__") {
        // 绑定 self
        let bound = BoundMethod {
            receiver: inst_obj,
            method: init.clone(),
        };
        self.stack_push(Object::BoundMethod(Gc::new(bound)));
        for arg in args {
            self.stack_push(arg);
        }
        self.call(argc + 1); // self + args
    }
    
    // 返回实例（__init__ 执行完毕后实例在栈上）
    Ok(())
}
```

### 7. GET_ATTR / SET_ATTR 实现

```rust
OpCode::GET_ATTR => {
    let name_idx = self.read_u16();
    let name = &self.constants[name_idx as usize];
    
    let obj = self.stack_pop();
    match &obj {
        Object::Instance(inst) => {
            // 先查实例字段
            if let Some(val) = inst.fields.get(name) {
                self.stack_push(val.clone());
            }
            // 再查类方法
            else if let Some(method) = inst.class.find_method(name) {
                // 返回 BoundMethod
                let bound = BoundMethod {
                    receiver: obj,
                    method: method.clone(),
                };
                self.stack_push(Object::BoundMethod(Gc::new(bound)));
            }
            // 最后查类属性
            else if let Some(val) = inst.class.get_class_attr(name) {
                self.stack_push(val.clone());
            }
            else {
                return Err(runtime_error(format!(
                    "'{}' has no attribute '{}'", inst.class.name, name
                )));
            }
        }
        Object::Class(cls) => {
            // 访问类方法或类属性
            if let Some(method) = cls.methods.get(name) {
                self.stack_push(Object::Closure(method.clone()));
            } else if let Some(val) = cls.get_class_attr(name) {
                self.stack_push(val.clone());
            } else {
                return Err(runtime_error(format!(
                    "class '{}' has no attribute '{}'", cls.name, name
                )));
            }
        }
        // 其他类型...
        _ => { /* 内置类型的属性 */ }
    }
}

OpCode::SET_ATTR => {
    let name_idx = self.read_u16();
    let name = &self.constants[name_idx as usize];
    
    let value = self.stack_pop();
    let obj = self.stack_pop();
    
    if let Object::Instance(inst) = obj {
        inst.fields.insert(name.clone(), value);
        self.stack_push(Object::Nil);
    } else if let Object::Class(cls) = obj {
        cls.set_class_attr(name.clone(), value);
        self.stack_push(Object::Nil);
    } else {
        return Err(runtime_error("cannot set attribute"));
    }
}
```

### 8. print() 与 __repr__

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
