# 继承与 super

## 所属阶段
Phase 5.3 - 类 + OOP

## 前置任务
41-self-instance-attributes

## 目标
实现单继承、方法覆盖、super 关键字、方法解析顺序（MRO）、隐式 Object 基类。

## 设计规格

参照 [06-oop](../06-oop.md) § 继承：

### 语法

```
class_def = "class" IDENTIFIER ("<" IDENTIFIER)? "{" class_body "}"
```

使用 `<` 表示继承（单继承）。

### 字节码指令

参照 [11-bytecode-vm](../11-bytecode-vm.md) § 类与实例：

| OpCode | 操作数 | 说明 |
|---|---|---|
| `INHERIT` | — | 继承父类 |
| `GET_SUPER` | `name_idx(2)` | 获取父类方法 |

### 继承规则

- 仅支持单继承
- 子类继承父类的所有属性和方法
- 子类可以覆盖父类方法
- `super` 关键字引用父类
- 每个类隐式继承自 `Object`（不显式指定时）

### MRO（方法解析顺序）

单继承场景下为线性链：

```
Dog -> Animal -> Object
```

### Object 基类

参照 [06-oop](../06-oop.md) § Object 基类：

```ms
class Object {
    fn __repr__(self) {
        return type(self) + " instance"
    }
    
    fn __eq__(self, other) {
        return self is other
    }
    
    fn __ne__(self, other) {
        return not (self == other)
    }
}
```

## 实现细节

### 1. INHERIT 指令

编译 `class Dog < Animal { ... }`：

```
1. emit CLASS "Dog"
2. emit LOAD_GLOBAL "Animal"       → 压入父类
3. emit INHERIT                     → 设置继承关系
4. 编译方法...
```

```rust
OpCode::INHERIT => {
    let parent = self.stack_pop(); // 父类
    let child = self.stack_peek_mut(0); // 子类
    
    match (&parent, child) {
        (Object::Ref(parent_ptr), Object::Ref(child_ptr))
            if unsafe { (*(*parent_ptr)).type_tag } == TypeTag::CLASS as u8
            && unsafe { (*(*child_ptr)).type_tag } == TypeTag::CLASS as u8 =>
        {
            let parent_cls = unsafe { read_class(*parent_ptr) };
            let child_cls = unsafe { read_class(*child_ptr) };
            child_cls.parent = Some(*parent_ptr);
            // 继承父类属性
            for (name, value) in &parent_cls.class_attrs {
                if !child_cls.class_attrs.contains_key(name) {
                    child_cls.class_attrs.insert(name.clone(), value.clone());
                }
            }
        }
        _ => return Err(runtime_error("parent must be a class")),
    }
}
```

### 2. 隐式 Object 基类

VM 初始化时创建 Object 基类：

```rust
fn init_object_class(&mut self) {
    let object_class = alloc_class("Object".to_string());
    
    // Object.__repr__、Object.__eq__、Object.__ne__ 编译为内置闭包后
    // 通过 read_class + methods.insert 注入
    
    let Object::Ref(object_ptr) = object_class else { unreachable!() };
    self.object_class = object_ptr;
}
```

编译 class 定义时，如果未指定父类，自动设置 parent 为 Object：

```rust
// 在 CLASS 指令后
if !has_explicit_parent {
    // 自动继承 Object
    self.emit_load_object_class();
    self.emit(OpCode::INHERIT);
}
```

### 3. 方法查找（含继承链）

```rust
impl MsClass {
    pub unsafe fn find_method(&self, name: &str) -> Option<*mut MsObjHeader> {
        if let Some(&ptr) = self.methods.get(name) {
            return Some(ptr);
        }
        if let Some(parent_ptr) = self.parent {
            return read_class(parent_ptr).find_method(name);
        }
        None
    }
    
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

### 4. 方法覆盖

子类方法覆盖父类同名方法不需要特殊处理——`find_method` 先查找子类 methods，找到即返回，父类方法自然被覆盖。

### 5. super 编译

`src/compiler/expression.rs`：

`super.method(args)` 编译为：

```
1. emit GET_SUPER "method"     → 获取父类方法，返回 BoundMethod
2. 编译 args
3. emit CALL argc
```

`super.__init__(args)` 编译为：

```
1. emit GET_SUPER "__init__"
2. 编译 args
3. emit CALL argc
```

### 6. GET_SUPER 指令实现

```rust
OpCode::GET_SUPER => {
    let name_idx = self.read_u16();
    let name = &self.constants[name_idx as usize].to_string();
    
    // 获取当前实例的类的父类
    let current_class = self.get_current_class(); // 从编译上下文获取
    let parent = current_class.parent.as_ref()
        .ok_or_else(|| runtime_error("no parent class"))?;
    
    let method = parent.find_method(name)
        .ok_or_else(|| runtime_error(format!(
            "parent class has no method '{}'", name
        )))?;
    
    // 获取 self（当前实例）
    let receiver = self.get_self();
    
    let bound = alloc_bound_method(receiver, method_ptr);
    self.stack_push(bound);
}
```

### 7. super 上下文传递

编译器需要知道当前编译的方法所在的类，以便 GET_SUPER 查找父类。在编译 class 定义时，将当前类名存入编译器状态：

```rust
struct Compiler {
    // ...
    current_class: Option<String>,  // 当前编译的类名
}
```

运行时也需要知道当前类。方法一：GET_SUPER 的操作数包含类名常量索引。方法二：在 CallFrame 中记录当前类。

推荐方案：GET_SUPER 操作数包含 `class_idx`（当前类）和 `name_idx`（方法名），运行时通过 class_idx 找到类，再取其 parent。

```
GET_SUPER class_idx(2), name_idx(2)
```

## 验证标准

1. 子类继承父类的方法和属性
2. 子类方法正确覆盖父类方法
3. `super.method()` 正确调用父类方法
4. `super.__init__()` 正确调用父类构造器
5. 隐式 Object 基类提供默认方法
6. 属性查找沿继承链正确进行
7. 无显式父类的类自动继承 Object

## 测试用例

```ms
// test_inheritance.ms — 继承与 super

// 基本继承
class Animal {
    fn __init__(self, name) {
        self.name = name
    }
    
    fn speak(self) {
        return self.name + " speaks"
    }
}

class Dog < Animal {
    fn __init__(self, name, breed) {
        super.__init__(name)
        self.breed = breed
    }
    
    fn speak(self) {
        return self.name + " barks"
    }
}

d = Dog("Rex", "Shepherd")
print(d.speak())
print(d.name)
print(d.breed)

// 方法覆盖与 super 调用
class Base {
    fn greet(self) {
        return "hello from Base"
    }
}

class Child < Base {
    fn greet(self) {
        return super.greet() + " and Child"
    }
}

c = Child()
print(c.greet())

// 继承链
class A {
    fn who(self) {
        return "A"
    }
}

class B < A {
    fn who(self) {
        return "B+" + super.who()
    }
}

class C < B {
    fn who(self) {
        return "C+" + super.who()
    }
}

obj = C()
print(obj.who())

// 隐式 Object 基类
class Simple {
    fn __init__(self) {
        self.x = 1
    }
}

s = Simple()
print(s.__repr__())
```

预期输出：

```
Rex barks
Rex
Shepherd
hello from Base and Child
C+B+A
Simple instance
```
