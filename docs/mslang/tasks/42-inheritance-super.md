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
| `GET_SUPER` | `class_idx(2), name_idx(2)` | 获取父类方法（双操作数：当前类 + 方法名；回写见 §设计规格回写） |

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
OpCode::Inherit => {
    let parent_obj = self.pop()?;
    let child_obj = self.peek_mut(0)?;

    let (parent_ptr, child_ptr) = match (&parent_obj, child_obj) {
        (Object::Ref(p), Object::Ref(c))
            if unsafe { (**p).type_tag } == TypeTag::CLASS as u8
            && unsafe { (**c).type_tag } == TypeTag::CLASS as u8 => (*p, *c),
        _ => return Err("parent must be a class".into()),
    };

    // 分阶段访问，避免两个 &mut MsClass 同时存活（task 40 V5 模式）
    unsafe { read_class(child_ptr) }.parent = Some(parent_ptr);

    // 不复制 class_attrs：继承链查找由 find_class_attr 递归处理（task 41 已实现）。
    // 复制会创建陈旧快照——父类属性在 INHERIT 后修改时子类看不到更新。
}
```

> **class_attrs 继承由 find_class_attr 递归处理**：[41-self-instance-attributes](./41-self-instance-attributes.md) §3 的 `find_class_attr` 已沿 parent 链递归查找。INHERIT 只需设置 `parent` 指针，不复制属性。

### 2. 隐式 Object 基类

VM 初始化时创建 Object 基类：

```rust
fn init_object_class(&mut self) {
    let object_class = alloc_class("Object".to_string());
    let Object::Ref(object_ptr) = object_class else { unreachable!() };

    // Object 方法注入：以 Rust 原生函数实现，包装为 MsClosure 注入 methods。
    // 不从 mslang 源码编译——VM 初始化时编译器尚未就绪，且 Object.__repr__
    // 依赖 type(self) 返回类名（本 task §8 补丁），Object.__eq__ 依赖 IS 指令。
    //
    // 方案：复用 task 25 的 native function 注册机制（builtin_type 等模式），
    // 为 __repr__/__eq__/__ne__ 各创建一个 native closure：
    //   __repr__(self) → alloc_string(format!("{} instance", class_name_of(self)))
    //   __eq__(self, other) → self is other（复用 OpCode::Is 的 is_identity）
    //   __ne__(self, other) → not (self is other)
    // 详见 task 25 register_builtins 的 native function 包装模式。
    unsafe { read_class(object_ptr) }.methods.insert("__repr__", /* native closure */);
    unsafe { read_class(object_ptr) }.methods.insert("__eq__",    /* native closure */);
    unsafe { read_class(object_ptr) }.methods.insert("__ne__",    /* native closure */);

    // GC 保护：Object 类须在整个 VM 生命周期存活。
    // 方案一：设为 Immortal 代（gc_meta |= GEN_IMMORTAL）
    // 方案二：加入 c_roots（14-gc.md 根集）
    unsafe { (*object_ptr).gc_meta |= 0x0C; }  // gen = Immortal(2) << 2
    self.object_class = object_ptr;
}
```

> **VM struct 扩展**：在 `src/vm/mod.rs` 的 `VM` struct 新增 `object_class: *mut MsObjHeader` 字段。回写 `11-bytecode-vm.md` § VM 核心结构（见设计规格回写）。

> **Object 方法注入与 task 43 的关系**：本 task 仅注入 Object 基类方法并建立继承链。`==`/`!=` 运算符分派到 `__eq__`/`__ne__`、`print`/`str` 分派到 `__str__`/`__repr__` 由 [43-magic-methods](./43-magic-methods.md) 落地。本 task 的验证仅覆盖 `s.__repr__()`（显式调用），不覆盖 `==` 自动分派。

编译 class 定义时，无显式父类的 class 在运行时自动链接 Object。采用 **运行时方案**（非编译期）——CLASS handler 创建 MsClass 后，若字节码中无后续 INHERIT 指令（即无显式父类），VM 自动设置 parent：

```rust
// OpCode::Class handler 内，创建 MsClass 后：
let cls_ptr = match alloc_class(name) {
    Object::Ref(p) => p,
    _ => unreachable!(),
};
// 若编译器未发射 INHERIT（无显式父类），运行时自动链接 Object
if !has_explicit_parent {
    unsafe { read_class(cls_ptr) }.parent = Some(self.object_class);
}
self.push(Object::Ref(cls_ptr))?;
```

> **编译器配合**：`compile_class_decl`（`src/compiler/statement.rs:354`）当前对 `parent.is_some()` 返回 `Err("inheritance not yet supported (task 42)")`。本 task 移除此检查；parent 非空时 emit `LOAD_GLOBAL parent_name; INHERIT`。parent 为空时不 emit INHERIT，由 CLASS handler 自动链接 Object。`has_explicit_parent` 对应 `Stmt::ClassDecl { parent: Some(_), .. }`。

### 3. 方法查找（含继承链）

**复用 [41-self-instance-attributes](./41-self-instance-attributes.md) §3 的成果**：`find_method` / `find_class_attr` 已在 task 41 实现（`src/vm/object.rs:1005-1027`），本 task 不重写。task 42 仅通过 INHERIT 使 `MsClass.parent` 非 None，继承链递归自然生效。

```rust
// task 41 已实现（src/vm/object.rs:1005-1027），本 task 无需改动：
// impl MsClass {
//     pub unsafe fn find_method(&self, name: &str) -> Option<*mut MsObjHeader> { ... }
//     pub unsafe fn find_class_attr(&self, name: &str) -> Option<Object> { ... }
// }
```

### 4. 方法覆盖

子类方法覆盖父类同名方法不需要特殊处理——`find_method` 先查找子类 methods，找到即返回，父类方法自然被覆盖。

### 5. super 编译

`src/compiler/expression.rs`：当前 `Expr::SuperAccess { .. }` 在 `:90` 返回 `Err("super compilation not yet implemented (task 42)")`。本 task 替换为正式编译。

`super.method(args)` 编译为：

```
1. emit GET_SUPER class_idx(2), name_idx(2)   → 获取父类方法，返回 BoundMethod 压栈
2. 编译 args                                  → 每个 arg 压栈
3. emit CALL argc
```

`class_idx` 为当前类的名字常量索引（编译期从 `current_class` 获取，§7）。运行时通过 `LOAD_GLOBAL class_name` 找到当前类对象，取其 `parent`，在 parent 链中 `find_method(name)`。

```
// super.__init__(name) 在 Dog.__init__ 内：
emit GET_SUPER  class_idx="Dog"  name_idx="__init__"
emit LOAD_LOCAL name              // 编译参数
emit CALL 1
```

### 6. GET_SUPER 指令实现

GET_SUPER 采用双操作数编码：`class_idx(2), name_idx(2)`。

> **操作数编码变更**：`11-bytecode-vm.md:154` 原规定 `GET_SUPER | name_idx(2)`（单操作数）。运行时需要知道"当前类"才能定位 parent 链，单操作数无法满足。本 task 扩展为 `class_idx(2), name_idx(2)`，须回写标准（见设计规格回写）。

```rust
OpCode::GetSuper => {
    let class_idx = self.read_u16()? as usize;
    let name_idx = self.read_u16()? as usize;

    // 取方法名（bounds-checked，task 40 模式）
    let name_obj = self.constants.get(name_idx)
        .ok_or_else(|| format!("constant index {} out of range", name_idx))?;
    let name = match name_obj {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::STRING as u8 => {
            unsafe { read_str(*ptr) }.to_owned()
        }
        _ => return Err("GET_SUPER expects a string constant for method name".into()),
    };

    // 取当前类名 → LOAD_GLOBAL 查找类对象
    let class_name_obj = self.constants.get(class_idx)
        .ok_or_else(|| format!("constant index {} out of range", class_idx))?;
    let class_name = match class_name_obj {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::STRING as u8 => {
            unsafe { read_str(*ptr) }.to_owned()
        }
        _ => return Err("GET_SUPER expects a string constant for class name".into()),
    };
    let current_cls_obj = self.globals.get(&class_name)
        .ok_or_else(|| format!("class '{}' not found", class_name))?;
    let current_cls_ptr = match current_cls_obj {
        Object::Ref(p) if unsafe { (**p).type_tag } == TypeTag::CLASS as u8 => *p,
        _ => return Err(format!("'{}' is not a class", class_name)),
    };

    // 取 parent
    let parent_ptr = unsafe { read_class(current_cls_ptr) }.parent
        .ok_or_else(|| format!("class '{}' has no parent", class_name))?;

    // 在 parent 链中查找方法
    let method_ptr = unsafe { read_class(parent_ptr) }.find_method(&name)
        .ok_or_else(|| format!("parent class has no method '{}'", name))?;

    // receiver = 当前方法的 self（slot 0 = stack_base）
    let frame = self.call_stack.last()
        .ok_or("GET_SUPER outside method call")?;
    let receiver = self.stack[frame.stack_base].clone();

    let bound = alloc_bound_method(receiver, method_ptr);
    self.push(bound)?;
}
```

### 7. super 上下文传递（编译器侧）

编译器在编译 class 定义时记录当前类名，供 `Expr::SuperAccess` 编译 GET_SUPER 的 `class_idx` 使用：

```rust
struct Compiler {
    // ...
    current_class: Option<String>,  // 当前编译的类名（None = 不在类方法内）
}
```

`compile_class_decl` 进入方法编译前设置 `current_class = Some(name.clone())`，结束后恢复。`Expr::SuperAccess { name }` 编译时：

```rust
Expr::SuperAccess { name } => {
    let class_name = self.current_class.as_ref()
        .ok_or_else(|| "'super' used outside of class method".to_string())?;
    let class_idx = self.add_constant(alloc_string(class_name));
    let class_idx = u16::try_from(class_idx)
        .map_err(|_| "constant pool overflow".to_string())?;
    let name_idx = self.add_constant(alloc_string(name));
    let name_idx = u16::try_from(name_idx)
        .map_err(|_| "constant pool overflow".to_string())?;
    self.emit_byte(OpCode::GetSuper as u8, line);
    self.emit_bytes(&class_idx.to_be_bytes(), line);
    self.emit_bytes(&name_idx.to_be_bytes(), line);
}
```

> **`super` 在非方法上下文**：若 `current_class` 为 None（顶层代码或普通函数内使用 `super.x()`），编译期返回 `Err("'super' used outside of class method")`（06-oop.md:168 "在子类方法中使用 super"）。

### 8. type() 对 INSTANCE 返回类名（补丁）

Object.__repr__ 为 `type(self) + " instance"`（[06-oop](../06-oop.md) § Object 基类 `:343`），测试预期 `s.__repr__()` → `"Simple instance"`（§测试用例）。但当前 `type_name()`（`src/vm/object.rs:1093-1094`）对 INSTANCE 一律返回 `"instance"`，会使 `type(self) + " instance"` 得到 `"instance instance"`。

本 task 须扩展 `builtin_type`（`src/vm/builtins.rs`）对 INSTANCE 返回类名：

```rust
fn builtin_type(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let arg = args.get(0).ok_or("type() requires 1 argument")?;
    // INSTANCE 特判：返回类名（非 "instance"）
    if let Object::Ref(ptr) = arg {
        if unsafe { (**ptr).type_tag } == TypeTag::INSTANCE as u8 {
            let class_ptr = unsafe { read_instance(*ptr) }.class;
            let name = unsafe { read_class(class_ptr) }.name.clone();
            return Ok(alloc_string(&name));
        }
    }
    Ok(alloc_string(arg.type_name()))
}
```

> **`type_name()` 不变**：`Object::type_name()` 仍返回 `&'static str`（对 INSTANCE 返回 `"instance"`），用于 TypeError 等错误信息。`type()` builtin 对 INSTANCE 特判返回动态类名，仅影响 `type(x)` 表达式的返回值。

## 验证标准

1. 子类继承父类的方法和属性
2. 子类方法正确覆盖父类方法
3. `super.method()` 正确调用父类方法
4. `super.__init__()` 正确调用父类构造器
5. 隐式 Object 基类提供默认方法（`s.__repr__()` 返回 `"Simple instance"`）
6. 属性查找沿继承链正确进行
7. 无显式父类的类自动继承 Object
8. **`type(instance)` 返回类名**：`type(s)` 返回 `"Simple"`（§8 补丁；覆盖 `10-builtins.md:35` 与 task 25 对 INSTANCE 的处理差异）
9. **`super` 在非方法上下文报错**：顶层 `super.x()` 编译期返回明确错误（§7）
10. **GET_SUPER 操作数编码**：`class_idx(2), name_idx(2)` 双操作数（§6；回写 `11-bytecode-vm.md:154`）
11. **Object 基类 GC 存活**：Object 类标为 Immortal 代，GC 不回收（§2）

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

## 设计规格回写（spec writeback）

本任务对设计文档的扩展（参照 task 40/41 §"设计规格回写" 惯例）：

- **`11-bytecode-vm.md:154` GET_SUPER 操作数**：从 `name_idx(2)`（单操作数）扩展为 `class_idx(2), name_idx(2)`（双操作数）。运行时需要 class_idx 定位当前类以取 parent 链。
- **`11-bytecode-vm.md` § VM 核心结构**：VM struct 新增 `object_class: *mut MsObjHeader` 字段（隐式 Object 基类引用）。
- **`14-gc.md` § 根集**：根集列表新增 `object_class`（Immortal 代，或加入 c_roots）。
- **`10-builtins.md:35` / task 25 type()**：`type(instance)` 对 INSTANCE 返回类名（非 `"instance"`）。`Object::type_name()` 不变（仍返回 `"instance"` 用于 TypeError 信息），仅 `builtin_type()` 特判。
- **`06-oop.md`**：无需改动（继承 / super / Object 语义未变）。

## 与后续 task 的协作约定

- **task 41（self 绑定 / 属性查找）**：本 task 复用 task 41 的 `find_method` / `find_class_attr`（`object.rs:1005-1027`）与 `alloc_bound_method` / BoundMethod CALL 机制。本 task 仅使 `parent` 非 None + 落地 GET_SUPER。
- **task 43（魔术方法）**：本 task 注入 Object.__repr__/__eq__/__ne__ 并建立继承链，但 `==`/`!=` 运算符分派到 `__eq__`/`__ne__`、`print`/`str` 分派到 `__str__`/`__repr__` 由 task 43 落地。本 task 验证仅覆盖显式调用 `s.__repr__()`，不覆盖 `==` 自动分派。
- **task 52（GC）**：Object 基类标为 Immortal 代，GC 不回收。若未来支持用户重定义 Object 类，须重新评估。
- **task 50/51（内置方法）**：String/List 等内置类型的方法解析若需继承自 Object（如 `str.__repr__` fallback），由 task 50/51 对接。
