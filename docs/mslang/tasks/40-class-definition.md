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

### 1. 解析（已在 task 15 完成）

class 定义解析已在 [15-parser-advanced-statements](./15-parser-advanced-statements.md) 落地于 `src/parser/statement.rs:450`（`parse_class`）与 `:493`（`parse_class_method`）。**本任务不重写解析器**，仅消费其产出的 AST。

实际 AST 节点（`src/ast/node.rs:129`）：

```rust
Stmt::ClassDecl {
    name: String,
    parent: Option<String>,
    methods: Vec<Stmt>,           // 每项为 Stmt::FnDecl（与普通函数同）
    class_vars: Vec<(String, Expr)>,  // 元组而非结构体
}
```

> **注意**：方法节点复用 `Stmt::FnDecl { name, params, body, is_async }`，无独立 `MethodDef` 类型；类变量为 `(String, Expr)` 元组，无独立 `ClassVar` 类型。task 实现须按此消费。

### 2. AST 节点

见 §1。task 40 仅消费 `Stmt::ClassDecl`，不新增 AST 类型。

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

> **`class_attrs` 字段为标准扩展**：`11-bytecode-vm.md:283-290` 的 Class 结构仅列 `{ header, name, methods, parent }`，未含 `class_attrs`。本任务新增该字段以支持 [06-oop 类属性语义](../06-oop.md)，须回写标准（见 §设计规格回写）。

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
    debug_assert!(std::mem::size_of::<MsClass>() <= LARGE_OBJ_THRESHOLD,
                  "MsClass too large, use LOS");
    Object::Ref(Box::into_raw(obj) as *mut MsObjHeader)
}

/// 读取 MsClass 内容。
///
/// # Safety
/// `ptr` 必须指向由 `alloc_class` 分配的有效 `MsClass`。
pub unsafe fn read_class<'a>(ptr: *mut MsObjHeader) -> &'a mut MsClass {
    debug_assert_eq!((*ptr).type_tag, TypeTag::CLASS as u8, "read_class on non-CLASS");
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
    debug_assert!(std::mem::size_of::<MsInstance>() <= LARGE_OBJ_THRESHOLD,
                  "MsInstance too large, use LOS");
    Object::Ref(Box::into_raw(obj) as *mut MsObjHeader)
}

/// 读取 MsInstance 内容。
///
/// # Safety
/// `ptr` 必须指向由 `alloc_instance` 分配的有效 `MsInstance`。
pub unsafe fn read_instance<'a>(ptr: *mut MsObjHeader) -> &'a mut MsInstance {
    debug_assert_eq!((*ptr).type_tag, TypeTag::INSTANCE as u8, "read_instance on non-INSTANCE");
    &mut *(ptr as *mut MsInstance)
}
```

> **命名约定（task 25-28 / 39 既定 API）**：本任务下文统一使用 `self.push(v)?` / `self.pop()?` / `self.peek(n)?` / `self.peek_mut(n)?` / `self.read_byte()?` / `self.read_u16()?`。**不得**使用 `stack_push` / `stack_pop` / `stack_peek` / `runtime_error` 等非标准名（前者不存在，后者统一为 `return Err("...".into())`）。

### 5. 编译 class 定义

`src/compiler/statement.rs`：

```
编译 class Animal { kingdom = "Animalia"; fn __init__(...) {...}; fn speak(...) {...} }:

1. emit CLASS "Animal"           → 创建类对象，压栈
2. [如果有父类] emit LOAD_GLOBAL parent; emit INHERIT
   ⚠ INHERIT opcode handler 由 task 42 实现。本任务验证用例**不得**使用继承语法；
     task 40 编译器遇到 parent 非空时，应在编译期 `Err("inheritance not yet supported (task 42)")`
     或在 VM 启动时校验字节码不含 INHERIT（避免运行期遇未注册 opcode 而 panic）。
     parser 层（src/parser/statement.rs:454）已接受 `< Parent` 语法，不可改；
     本任务的编译期检查是防止语义未实现时静默崩盘的兜底。
3. 编译类属性:
   编译 value 表达式
   emit SET_ATTR "kingdom"       → 设置在类对象上
4. 编译方法:
   对 methods: Vec<Stmt> 中每个 Stmt::FnDecl:
   a. 创建新的编译单元（task 17 compile_fn_decl 等价流程）
   b. 编译方法体（slot 0 = self，由 task 41 完善 self 关键字绑定；本任务按普通函数编译，
      方法定义须含 self 形参，调用方按位置传参）
   c. emit CLOSURE method_func
   d. emit METHOD "method_name"  → 添加到类
5. emit STORE_GLOBAL "Animal"    → 存为全局变量
   注：本任务仅支持顶层 class 定义（task 17 局部变量规则未覆盖 class）；
   函数内定义 class 暂不支持，编译期 `Err("class definition inside function not supported")`。
```

### 6. CLASS 指令实现

```rust
OpCode::CLASS => {
    let name_idx = self.read_u16()? as usize;
    let name_obj = self.constants.get(name_idx)
        .ok_or_else(|| format!("constant index {} out of range", name_idx))?;
    let name = match name_obj {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::STRING as u8 => {
            unsafe { read_str(*ptr) }.to_owned()
        }
        _ => return Err("CLASS expects a string constant".into()),
    };
    self.push(alloc_class(name))?;
}
```

### 7. METHOD 指令实现

```rust
OpCode::METHOD => {
    let name_idx = self.read_u16()? as usize;
    let name_obj = self.constants.get(name_idx)
        .ok_or_else(|| format!("constant index {} out of range", name_idx))?;
    let name = match name_obj {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::STRING as u8 => {
            unsafe { read_str(*ptr) }.to_owned()
        }
        _ => return Err("METHOD expects a string constant".into()),
    };

    // 弹出栈顶方法（必须为 Ref → MsClosure），再读下面的 class
    let method_obj = self.pop()?;
    let method_ptr = match method_obj {
        Object::Ref(p) if unsafe { (*p).type_tag } == TypeTag::CLOSURE as u8 => p,
        _ => return Err("METHOD expects a closure on stack".into()),
    };
    let cls_obj = self.peek_mut(0)?;

    if let Object::Ref(cls_ptr) = cls_obj {
        if unsafe { (**cls_ptr).type_tag } == TypeTag::CLASS as u8 {
            // TODO task 62: 并发 GC 启用后，下方 methods.insert 须经 write_barrier
            unsafe { read_class(*cls_ptr) }.methods.insert(name, method_ptr);
        } else {
            return Err("METHOD target is not a Class".into());
        }
    } else {
        return Err("METHOD target is not a heap object".into());
    }
}
```

> **修复说明**：原方案的 `if let ... { if let ... { if let ... }}` 嵌套在 method 非 Ref 时静默失败（无错误反馈），已改为显式 `match` + `Err`。

### 8. 类实例化（调用 Class）

当 `CALL` 的目标是 `Object::Ref` 且 type_tag 为 `TypeTag::CLASS` 时：

```rust
fn call_class(&mut self, cls_ptr: *mut MsObjHeader, argc: u8) -> Result<(), String> {
    let argc_usize = argc as usize;

    // V1/R1 修复：防御性栈下溢校验
    if argc_usize + 1 > self.stack.len() {
        return Err("stack underflow in call_class".into());
    }

    // 弹出 callee（class）和参数
    let args: Vec<Object> = (0..argc_usize).rev()
        .map(|_| self.pop())
        .collect::<Result<_, _>>()?;
    self.pop()?; // 弹出 class

    // 创建实例
    let inst_obj = alloc_instance(cls_ptr);

    // 查找 __init__
    let init_ptr_opt = unsafe { read_class(cls_ptr) }.methods.get("__init__").copied();

    match init_ptr_opt {
        Some(init_ptr) => {
            // task 41 落地前不使用 BoundMethod（前向依赖）。
            // 直接构造栈布局：[closure, inst(self), args...]
            // task 41 切换为 BoundMethod 后，改为 [bound, args...] + CALL argc。
            self.push(Object::Ref(init_ptr))?;
            self.push(inst_obj)?;        // slot 0 = self
            for arg in args {
                self.push(arg)?;
            }
            self.call(argc + 1)          // argc + 1 = self + args
        }
        None => {
            // R4 修复：无 __init__ 且 argc > 0 时报错（与 Python 一致）
            if argc_usize > 0 {
                let cls_name = unsafe { read_class(cls_ptr) }.name.clone();
                return Err(format!(
                    "'{}' takes no arguments (got {})", cls_name, argc_usize
                ));
            }
            // 无 __init__ 且无参数：直接返回实例
            self.push(inst_obj)?;
            Ok(())
        }
    }
}
```

> **V1 修复说明**：原方案同时 `push(inst_obj.clone())` 与 `push(bound)`（含同一 receiver），再 `call(argc + 1)`，导致 callee 位置错乱（CALL 把 inst_obj 当 callee）。本任务在 task 41 落地前采用直接 push closure + self + args 的简化方案，匹配 `src/compiler/expression.rs` callee-below-args 约定。task 41 落地 BoundMethod 后切换为 bound 方法（参见 [41-self-instance-attributes](./41-self-instance-attributes.md) §2）。

### 9. GET_ATTR / SET_ATTR 实现

> **V5 修复（别名可变借用）**：原方案在 `read_instance(*ptr)` 后嵌套 `read_class(inst.class)`，两个 `&mut` 同时存活（虽然指向不同对象，但 `'static` 生命周期让借用检查器失明，模式脆弱）。下文先 copy 出所需裸指针（`*mut` is `Copy`），再分阶段查找。

```rust
OpCode::GET_ATTR => {
    let name_idx = self.read_u16()? as usize;
    let name_obj = self.constants.get(name_idx)
        .ok_or_else(|| format!("constant index {} out of range", name_idx))?;
    let name = match name_obj {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::STRING as u8 => {
            unsafe { read_str(*ptr) }.to_owned()
        }
        _ => return Err("GET_ATTR expects a string constant".into()),
    };

    let obj = self.pop()?;
    match &obj {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::INSTANCE as u8 => {
            let inst_ptr = *ptr;
            let class_ptr = unsafe { read_instance(inst_ptr) }.class;  // copy 出 *mut

            // 1. 实例字段
            let field_val = unsafe { read_instance(inst_ptr) }
                .fields.get(&name).cloned();
            if let Some(v) = field_val {
                self.push(v)?;
                return Ok(());
            }
            // 2. 类方法（直接类，不含父类链 — task 41 完善）
            // TODO task 41: 改用 find_method(name) 遍历 parent 链
            let method_opt = unsafe { read_class(class_ptr) }
                .methods.get(&name).copied();
            if let Some(m) = method_opt {
                // task 41 落地前：直接返回 closure，调用者需显式传 self
                // task 41 后：alloc_bound_method(obj.clone(), m) 返回 BoundMethod
                self.push(Object::Ref(m))?;
                return Ok(());
            }
            // 3. 类属性（直接类，不含父类链 — task 41 完善）
            let attr_val = unsafe { read_class(class_ptr) }
                .class_attrs.get(&name).cloned();
            if let Some(v) = attr_val {
                self.push(v)?;
                return Ok(());
            }
            // 4. __name__ 内置属性（本任务 §12）
            if name == "__name__" {
                let n = unsafe { read_class(class_ptr) }.name.clone();
                self.push(alloc_string(n))?;
                return Ok(());
            }
            let cls_name = unsafe { read_class(class_ptr) }.name.clone();
            return Err(format!("'{}' instance has no attribute '{}'", cls_name, name));
        }
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::CLASS as u8 => {
            let cls_ptr = *ptr;
            let method_opt = unsafe { read_class(cls_ptr) }
                .methods.get(&name).copied();
            if let Some(m) = method_opt {
                self.push(Object::Ref(m))?;
                return Ok(());
            }
            let attr_val = unsafe { read_class(cls_ptr) }
                .class_attrs.get(&name).cloned();
            if let Some(v) = attr_val {
                self.push(v)?;
                return Ok(());
            }
            if name == "__name__" {
                let n = unsafe { read_class(cls_ptr) }.name.clone();
                self.push(alloc_string(n))?;
                return Ok(());
            }
            let cls_name = unsafe { read_class(cls_ptr) }.name.clone();
            return Err(format!("class '{}' has no attribute '{}'", cls_name, name));
        }
        _ => {
            // 内置类型属性（如 String/List 方法）— 由 task 50/51 接管
            return Err(format!("unsupported attribute access on this type"));
        }
    }
}

OpCode::SET_ATTR => {
    let name_idx = self.read_u16()? as usize;
    let name_obj = self.constants.get(name_idx)
        .ok_or_else(|| format!("constant index {} out of range", name_idx))?;
    let name = match name_obj {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::STRING as u8 => {
            unsafe { read_str(*ptr) }.to_owned()
        }
        _ => return Err("SET_ATTR expects a string constant".into()),
    };

    let value = self.pop()?;
    let obj = self.pop()?;

    match obj {
        Object::Ref(ptr) if unsafe { (*ptr).type_tag } == TypeTag::INSTANCE as u8 => {
            // TODO task 62: 并发 GC 启用后须经 write_barrier
            unsafe { read_instance(ptr) }.fields.insert(name, value);
            self.push(Object::Nil)?;
        }
        Object::Ref(ptr) if unsafe { (*ptr).type_tag } == TypeTag::CLASS as u8 => {
            // TODO task 62: 并发 GC 启用后须经 write_barrier
            unsafe { read_class(ptr) }.class_attrs.insert(name, value);
            self.push(Object::Nil)?;
        }
        _ => return Err("cannot set attribute on this type".into()),
    }
}
```

> **task 41 协作约定**：本任务的 GET_ATTR 仅查直接类的 `methods` / `class_attrs`，**不递归 parent 链**。task 41 §3 引入 `find_method(name)` 与 `find_class_attr(class, name)` 后替换为递归查找。task 40 验证用例不得依赖继承的方法/属性访问。

### 10. print() 与 __repr__ / __str__

`src/vm/builtins.rs` 的 `print` 实现：

```rust
fn builtin_print(args: Vec<Object>) -> Result<Object, String> {
    if args.is_empty() {
        println!();
        return Ok(Object::Nil);
    }
    let s = object_to_string(&args[0])?;
    println!("{}", s);
    Ok(Object::Nil)
}

/// 将任意 Object 转为显示字符串。
/// Instance：优先 __str__（task 43），次 __repr__，最后默认 `<ClassName instance>`。
pub fn object_to_string(obj: &Object) -> Result<String, String> {
    if let Object::Ref(ptr) = obj {
        if unsafe { (**ptr).type_tag } == TypeTag::INSTANCE as u8 {
            let inst_ptr = *ptr;
            let class_ptr = unsafe { read_instance(inst_ptr) }.class;
            let cls = unsafe { read_class(class_ptr) };
            // TODO task 43: 优先调用 __str__
            if let Some(str_ptr) = cls.methods.get("__str__").copied() {
                let _ = str_ptr;  // task 43 实现
            }
            if let Some(repr_ptr) = cls.methods.get("__repr__").copied() {
                // 调用 __repr__(self) 返回字符串
                return invoke_repr(inst_ptr, repr_ptr);
            }
            return Ok(format!("<{} instance>", cls.name));
        }
    }
    Ok(default_display(obj))  // 其他类型走既有显示路径
}
```

> **`__str__` 优先级（task 43 完善）**：本任务仅实现 `__repr__` 触发；`__str__` 优先于 `__repr__` 的逻辑由 [43-magic-methods](./43-magic-methods.md) §1 落地。本任务的 print 实现须保留 `// TODO task 43: __str__` 钩子。

### 11. GC 集成（关键：避免 Minor GC 后引用悬垂）

参照 [14-gc](../14-gc.md) § 类型描述表、task 39 §9 模板。**替换** `src/vm/gc.rs:472` 的 `TODO task 40/41: CLASS/INSTANCE/BOUND_METHOD`（本任务覆盖 CLASS/INSTANCE；BOUND_METHOD 仍由 task 41 处理）。

```rust
/// 遍历 MsClass 内所有 Ref 槽：methods 值 + parent + class_attrs 值。
fn trace_class(obj: *mut MsObjHeader, callback: &mut dyn FnMut(*mut MsObjHeader)) {
    let c = unsafe { read_class(obj) };
    for m in c.methods.values() { callback(*m); }
    if let Some(p) = c.parent { callback(p); }
    for v in c.class_attrs.values() {
        if let Object::Ref(r) = v { callback(*r); }
    }
}

/// 遍历 MsInstance 内所有 Ref 槽：class + fields 值。
fn trace_instance(obj: *mut MsObjHeader, callback: &mut dyn FnMut(*mut MsObjHeader)) {
    let i = unsafe { read_instance(obj) };
    callback(i.class);
    for v in i.fields.values() {
        if let Object::Ref(r) = v { callback(*r); }
    }
}

/// Cheney 复制时修正 MsClass 内的 Ref 槽（Minor GC）。
fn forward_fields_class(obj: *mut MsObjHeader, forwarder: &mut dyn FnMut(&mut Object)) {
    let c = unsafe { read_class(obj) };
    // methods 的 *mut MsObjHeader 裸指针
    for m in c.methods.values_mut() {
        let mut tmp = Object::Ref(*m);
        forwarder(&mut tmp);
        if let Object::Ref(new) = tmp { *m = new; }
    }
    // parent 裸指针
    if let Some(p) = c.parent {
        let mut tmp = Object::Ref(p);
        forwarder(&mut tmp);
        if let Object::Ref(new) = tmp { c.parent = Some(new); }
    }
    // class_attrs Object 槽
    for v in c.class_attrs.values_mut() { forwarder(v); }
}

fn forward_fields_instance(obj: *mut MsObjHeader, forwarder: &mut dyn FnMut(&mut Object)) {
    let i = unsafe { read_instance(obj) };
    let mut class_obj = Object::Ref(i.class);
    forwarder(&mut class_obj);
    if let Object::Ref(new) = class_obj { i.class = new; }
    for v in i.fields.values_mut() { forwarder(v); }
}

/// Minor GC 复制：MsClass/MsInstance 含 HashMap（堆分配的独立缓冲），不可盲字节拷贝。
fn copy_for_gc_class(src: *mut MsObjHeader, dst: *mut MsObjHeader) -> usize {
    let s = unsafe { read_class(src) };
    let d = unsafe { read_class(dst) };
    d.name = s.name.clone();
    d.methods = s.methods.clone();          // HashMap 深拷贝
    d.parent = s.parent;
    d.class_attrs = s.class_attrs.clone();  // HashMap 深拷贝
    std::mem::size_of::<MsClass>()
}

fn copy_for_gc_instance(src: *mut MsObjHeader, dst: *mut MsObjHeader) -> usize {
    let s = unsafe { read_instance(src) };
    let d = unsafe { read_instance(dst) };
    d.class = s.class;
    d.fields = s.fields.clone();            // HashMap 深拷贝
    std::mem::size_of::<MsInstance>()
}
```

`src/vm/gc.rs` 的 `type_descriptor` match（tag 8/9）注册：

```rust
8 => &TypeDescriptor {
    type_tag: TypeTag::CLASS,
    name: "CLASS",
    trace: trace_class,
    finalize: None,
    size_base: std::mem::size_of::<MsClass>(),
},
9 => &TypeDescriptor {
    type_tag: TypeTag::INSTANCE,
    name: "INSTANCE",
    trace: trace_instance,
    finalize: None,                       // __del__ 触发由 run_finalizers 处理（见 §13）
    size_base: std::mem::size_of::<MsInstance>(),
},
```

### 12. `__name__` 内置类属性

参照 [06-oop](../06-oop.md) § Object 基类 § 类内置属性。GET_ATTR handler 已在 §9 内联 `__name__` 分支：访问类名时返回 `alloc_string(class.name.clone())`。无需在 `class_attrs` 中预存——`__name__` 由类型描述符合成（类似 String 的 `.length`），与类属性命名空间隔离。

> **实现要点**：GET_ATTR 末尾的 `__name__` 分支同时适用于 Instance 与 Class 访问（`inst.__name__` 返回类名，`Cls.__name__` 同）。

### 13. `__del__` finalizer 注册（与 task 52 协同）

参照 [14-gc](../14-gc.md) § 阶段 5 Finalize、[tasks/52-gc](./52-gc.md) § run_finalizers。

Instance 是否有 `__del__` 在**编译期已知**（class.methods 含 `__del__`）。task 40 的编译器在 emit CLOSURE method 后，可由 `MsClass` 在运行时第一次注册 `__del__` 方法时（METHOD handler 内）设置 `has_finalizer` 标志：

```rust
OpCode::METHOD => {
    // ... 见 §7 ...
    if name == "__del__" {
        // 标记所有 instance 应被 GC finalizer 关注。
        // 实际实现：在 alloc_instance 时根据 class.methods.contains_key("__del__")
        // 设置 header.gc_meta |= MsObjHeader::HAS_FINALIZER。
        // 此处无需额外动作，§4 alloc_instance 已知 class_ptr，
        // 但 alloc_instance 不知道 method 注册顺序——故应在 call_class 创建 instance 时判断。
    }
}
```

修改 `call_class`（§8）在 `alloc_instance` 后立即检查并设标志：

```rust
let inst_ptr = match inst_obj {
    Object::Ref(p) => p,
    _ => unreachable!(),
};
let has_del = unsafe { read_class(cls_ptr) }.methods.contains_key("__del__");
if has_del {
    unsafe { (*inst_ptr).gc_meta |= MsObjHeader::HAS_FINALIZER; }
}
```

task 52 `run_finalizers`（`52-gc.md:586` `// Instance：调用用户定义的 __del__`）已预留 INSTANCE 分支：

```rust
// run_finalizers 内，对每个 obj：
if tag == TypeTag::INSTANCE as u8 {
    let inst = unsafe { read_instance(obj) };
    let class_ptr = inst.class;
    if let Some(del_ptr) = unsafe { read_class(class_ptr) }.methods.get("__del__").copied() {
        // 调用 __del__(self)，失败静默
        vm.invoke_del(obj, del_ptr).ok();
    }
    header.gc_meta &= !MsObjHeader::HAS_FINALIZER;
    header.set_color(Color::White);
    continue;
}
```

> **task 52 集成**：本任务仅注册 `has_finalizer` 标志；实际 `__del__` 调用由 task 52 的 `run_finalizers` 在 mutator 线程执行（与 task 39 Generator finalizer 同路径，`gc_disabled = true` 防重入）。

## 验证标准

1. class 定义创建类对象并存入全局变量
2. 类属性在所有实例间共享
3. `ClassName(args)` 创建实例并调用 `__init__`
4. 实例方法通过 `obj.method(args)` 调用（task 41 完善 self 自动绑定；本任务方法须显式声明 self 形参）
5. `__repr__` 被 print/str 调用（`__str__` 优先级由 task 43 落地）
6. `self.attr` 正确读写实例字段
7. 动态属性赋值（`obj.new_attr = val`）正确
8. **`__name__` 内置属性**：`Cls.__name__` 与 `inst.__name__` 返回类名字符串（覆盖 §12）
9. **GC 存活性**：在频繁 Minor GC 下，Instance 的 `class` 指针与 `fields` 值仍可正确解引用（覆盖 §11 trace/forward/copy）
10. **无 `__init__` + 有参数**：`Foo(1, 2)` 在 Foo 无 `__init__` 时抛出 `'<name>' takes no arguments`（R4 修复）
11. **常量池越界保护**：损坏字节码触发的 `name_idx >= constants.len()` 不 panic，返回 Err（V3 修复）
12. **METHOD 错误反馈**：栈顶非 closure 时返回明确错误（V4 修复）
13. **`__del__` finalizer 注册**：含 `__del__` 的类的 instance 在 alloc 时 `gc_meta & HAS_FINALIZER != 0`（§13，配合 task 52 run_finalizers 验证）
14. **不支持继承语法（task 42 前）**：编译 `class Dog < Animal {}` 时返回明确错误"inheritance not yet supported (task 42)"（R2/B4 修复）

## 设计规格回写（spec writeback）

本任务对设计文档的扩展（参照 task 28 / 37 / 39 的回写惯例）：

- **`11-bytecode-vm.md` Class 结构**：扩展为 `{ header, name, methods, parent, class_attrs }`（新增 `class_attrs: HashMap<String, Object>` 字段，task 40 类属性语义）。
- **`14-gc.md` TypeDescriptor 表（CLASS=8 / INSTANCE=9 行）**：填充 trace / forward_fields / copy_for_gc 三字段实际实现（替换 task 52 占位 noop；finalize 由 task 52 run_finalizers 调用 `__del__`，本任务只设 `has_finalizer` 标志）。
- **`14-gc.md` has_finalizer 注册**：含 `__del__` 方法的类，其 instance 在 `alloc_instance` 后（call_class 内）置 `gc_meta |= HAS_FINALIZER`。
- **`14-gc.md` finalizer 队列**：明确 INSTANCE 类型的 finalizer 在 mutator 线程、GC 结束后经 `run_finalizers(&mut VM)` 调用 `__del__(self)`（与 task 39 Generator close_generator 同路径，`gc_disabled = true` 防重入）。
- **`06-oop.md`**：无需改动（语义未变）。
- **`07-advanced.md`**：无需改动。

## 与后续 task 的协作约定

- **task 41（self 绑定 / BoundMethod）**：本任务 GET_ATTR 找到方法时**直接返回 closure**（不构造 BoundMethod），调用者须显式传 self。task 41 §2 落地 BoundMethod 后，GET_ATTR 改为 `alloc_bound_method(obj, method_ptr)`；§8 call_class 可切换为 bound 方案。
- **task 41（属性查找链）**：本任务 GET_ATTR 仅查直接类的 methods/class_attrs，**不递归 parent**。task 41 §3 引入 `find_method` / `find_class_attr` 后替换。
- **task 42（继承 / super）**：本任务编译器对 `parent` 非空的 ClassDecl 返回编译期错误（R2）；INHERIT/GET_SUPER opcode handler 由 task 42 实现。
- **task 43（魔术方法）**：本任务仅实现 `__repr__`；`__str__` 优先级、`__add__` 等运算符分派、INVOKE 优化指令由 task 43 实现。
- **task 52（GC）**：本任务实现 CLASS/INSTANCE 的 trace/forward/copy + has_finalizer 注册；run_finalizers 调用 `__del__` 由 task 52 落地。

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
