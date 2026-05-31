# 闭包与上值机制

## 所属阶段
Phase 3.2 - 函数 + 闭包

## 前置任务
- 27-call-frame（调用帧与函数调用）

## 目标
实现闭包对象、上值（upvalue）机制、相关指令，使函数能正确捕获并修改外层作用域的变量，闭包语义符合引用捕获要求。

## 设计规格

### Closure 对象

参照 [11-bytecode-vm](../11-bytecode-vm.md) § Closure：

```
Closure {
    function: *mut MsObjHeader   // 指向 MsFunction
    upvalues: Vec<*mut MsObjHeader>  // 每项指向 MsUpvalue
}
```

每个函数在运行时都包装为 Closure，即使不捕获任何上值（空 upvalues 数组）。

### Upvalue 机制

参照 [11-bytecode-vm](../11-bytecode-vm.md) § 编译单元 Upvalue：

```
Upvalue {
    index: usize          // 外层局部变量索引
    is_local: bool        // 是直接的外层局部变量，还是外层的上值
}
```

运行时存在两种状态的 upvalue：

- **开放上值（Open Upvalue）**：指向栈上的局部变量槽位。当变量仍在作用域内时使用。
- **关闭上值（Closed Upvalue）**：变量离开作用域时，将值从栈拷贝到堆分配的 `closed` 字段中，后续访问改为读写 `closed`。

```
RuntimeUpvalue {
    location: usize            // 栈位置（开放时）
    closed: Option<Object>     // 堆存储（关闭后）
    is_open: bool
}
```

### 指令

参照 [11-bytecode-vm](../11-bytecode-vm.md) § 闭包：

| OpCode | 操作数 | 说明 |
|---|---|---|
| `CLOSURE` | `func_idx(2)` | 创建闭包：从常量池取出 Function，创建 Closure 并捕获上值 |
| `LOAD_UPVALUE` | `idx(1)` | 将当前闭包的上值[idx]压栈 |
| `STORE_UPVALUE` | `idx(1)` | 将栈顶存入当前闭包的上值[idx] |
| `CLOSE_UPVALUE` | — | 关闭栈顶位置对应的所有开放上值 |

### 闭包语义

参照 [04-functions](../04-functions.md) § 闭包语义：

- 内层函数捕获外层变量的**引用**（不是值）
- 多个闭包可以共享同一个外层变量
- 外层函数返回后，被捕获的变量仍然存活（由 GC 管理）

## 实现细节

### 1. 堆对象布局

引用 [20-object-system-basic](./20-object-system-basic.md) 的 `MsObjHeader` 和 `TypeTag`。本任务新增以下堆对象：

```rust
/// Function 堆对象（TypeTag::FUNCTION = 6）
#[repr(C)]
pub struct MsFunction {
    pub header:        MsObjHeader,
    pub name:          String,
    pub arity:         usize,
    pub code:          Vec<u8>,
    pub constants:     Vec<Object>,
    pub upvalue_count: usize,
    pub is_generator:  bool,
    pub source_file:   Option<String>,
}

/// RuntimeUpvalue 堆对象（作为 MsUpvalue，TypeTag::ITERATOR 暂借；
/// MVP 阶段直接用 Box<RuntimeUpvalue> 管理，GC 替换后迁移）
pub struct RuntimeUpvalue {
    pub location: usize,
    pub closed: Option<Object>,
}

impl RuntimeUpvalue {
    pub fn new(location: usize) -> Self {
        Self {
            location,
            closed: None,
        }
    }

    pub fn get(&self, stack: &[Object]) -> Object {
        match &self.closed {
            Some(val) => val.clone(),
            None => stack[self.location].clone(),
        }
    }

    pub fn set(&mut self, stack: &mut [Object], value: Object) {
        if self.closed.is_some() {
            self.closed = Some(value);
        } else {
            stack[self.location] = value;
        }
    }

    pub fn close(&mut self, stack: &[Object]) {
        if self.closed.is_none() {
            self.closed = Some(stack[self.location].clone());
        }
    }
}

/// Closure 堆对象（TypeTag::CLOSURE = 7）
#[repr(C)]
pub struct MsClosure {
    pub header:   MsObjHeader,
    pub function: *mut MsObjHeader,          // 指向 MsFunction
    pub upvalues: Vec<*mut RuntimeUpvalue>,  // MVP：裸指针；GC 阶段迁移为 MsUpvalue 头
}
```

### 2. 堆分配辅助函数

```rust
/// 分配 Function 堆对象，返回 Object::Ref。
pub fn alloc_function(f: MsFunction) -> Object {
    let obj = Box::new(f);
    // 修改 header 的 type_tag（MsFunction 的 header 是第一个字段）
    let ptr = Box::into_raw(obj) as *mut MsObjHeader;
    unsafe {
        (*ptr).type_tag = TypeTag::FUNCTION as u8;
        (*ptr).gc_meta = 0;
        (*ptr).size = std::mem::size_of::<MsFunction>() as u16;
        (*ptr).class_ptr = 0;
    }
    Object::Ref(ptr)
}

/// 读取 MsFunction 内容。
///
/// # Safety
/// `ptr` 必须指向由 `alloc_function` 分配的有效 `MsFunction`。
pub unsafe fn read_function(ptr: *mut MsObjHeader) -> &'static MsFunction {
    &*(ptr as *const MsFunction)
}

/// 分配 Closure 堆对象，返回 Object::Ref。
pub fn alloc_closure(function: *mut MsObjHeader, upvalues: Vec<*mut RuntimeUpvalue>) -> Object {
    let obj = Box::new(MsClosure {
        header: MsObjHeader {
            gc_meta:   0,
            type_tag:  TypeTag::CLOSURE as u8,
            size:      std::mem::size_of::<MsClosure>() as u16,
            _padding:  0,
            class_ptr: 0,
        },
        function,
        upvalues,
    });
    Object::Ref(Box::into_raw(obj) as *mut MsObjHeader)
}

/// 读取 MsClosure 内容。
///
/// # Safety
/// `ptr` 必须指向由 `alloc_closure` 分配的有效 `MsClosure`。
pub unsafe fn read_closure(ptr: *mut MsObjHeader) -> &'static mut MsClosure {
    &mut *(ptr as *mut MsClosure)
}
```

### 3. src/compiler/mod.rs — 编译单元上值追踪

扩展 `CompilationUnit` 增加上值解析：

```rust
impl CompilationUnit {
    pub fn resolve_upvalue(&mut self, name: &str, parent: &mut CompilationUnit) -> Option<usize> {
        if let Some(idx) = parent.resolve_local(name) {
            parent.locals[idx].is_captured = true;
            let upvalue = Upvalue { index: idx, is_local: true };
            return Some(self.add_upvalue(upvalue));
        }

        if let Some(idx) = parent.resolve_upvalue_from_parent(name) {
            let upvalue = Upvalue { index: idx, is_local: false };
            return Some(self.add_upvalue(upvalue));
        }

        None
    }

    fn add_upvalue(&mut self, upvalue: Upvalue) -> usize {
        for (i, existing) in self.upvalues.iter().enumerate() {
            if existing.index == upvalue.index && existing.is_local == upvalue.is_local {
                return i;
            }
        }
        self.upvalues.push(upvalue);
        self.upvalues.len() - 1
    }
}
```

### 4. 编译闭包捕获

变量解析优先级调整：

1. 当前编译单元的局部变量 → `LOAD_LOCAL`
2. 外层编译单元的局部变量或上值 → 标记为上值，生成 `LOAD_UPVALUE`
3. 全局变量 → `LOAD_GLOBAL`

编译函数声明/匿名函数时：
1. 创建子编译单元
2. 编译函数体
3. 设置 `upvalue_count = sub_unit.upvalues.len()`
4. 在父编译单元生成 `CLOSURE(func_idx)`，紧跟 `upvalue_count` 个上值操作数

### 5. src/vm/mod.rs — CLOSURE 指令

```rust
OpCode::CLOSURE => {
    let func_idx = self.read_u16();
    let func_obj = self.current_frame_constants()[func_idx as usize].clone();

    let func_ptr = match func_obj {
        Object::Ref(ptr) if unsafe { (*ptr).type_tag } == TypeTag::FUNCTION as u8 => ptr,
        _ => return self.runtime_error("CLOSURE expects a Function"),
    };

    let upvalue_count = unsafe { read_function(func_ptr) }.upvalue_count;
    let mut upvalues = Vec::with_capacity(upvalue_count);

    for _ in 0..upvalue_count {
        let is_local = self.read_byte() == 1;
        let index = self.read_byte() as usize;

        if is_local {
            let stack_base = self.current_frame().stack_base;
            let location = stack_base + index;
            let upvalue = self.capture_upvalue(location);
            upvalues.push(upvalue);
        } else {
            let closure_ptr = self.current_frame().closure;
            let parent_closure = unsafe { read_closure(closure_ptr) };
            upvalues.push(parent_closure.upvalues[index]);
        }
    }

    let closure_obj = alloc_closure(func_ptr, upvalues);
    self.stack.push(closure_obj);
}
```

### 6. src/vm/mod.rs — 上值捕获

`open_upvalues` 存储 `*mut RuntimeUpvalue`（MVP 阶段，GC 阶段迁移为 `*mut MsObjHeader`）：

```rust
impl VM {
    fn capture_upvalue(&mut self, location: usize) -> *mut RuntimeUpvalue {
        for upvalue in &self.open_upvalues {
            if unsafe { (**upvalue).location } == location {
                return *upvalue;
            }
        }

        let upvalue = Box::into_raw(Box::new(RuntimeUpvalue::new(location)));
        self.open_upvalues.push(upvalue);
        upvalue
    }
}
```

### 7. LOAD_UPVALUE / STORE_UPVALUE

```rust
OpCode::LOAD_UPVALUE => {
    let idx = self.read_byte() as usize;
    let closure_ptr = self.current_frame().closure;
    let closure = unsafe { read_closure(closure_ptr) };
    let upvalue = unsafe { &*closure.upvalues[idx] };
    let value = upvalue.get(&self.stack);
    self.stack.push(value);
}

OpCode::STORE_UPVALUE => {
    let idx = self.read_byte() as usize;
    let value = self.stack.last().unwrap().clone();
    let closure_ptr = self.current_frame().closure;
    let closure = unsafe { read_closure(closure_ptr) };
    let upvalue = unsafe { &mut *closure.upvalues[idx] };
    upvalue.set(&mut self.stack, value);
}
```

### 8. CLOSE_UPVALUE

```rust
OpCode::CLOSE_UPVALUE => {
    let stack_top = self.stack.len() - 1;
    self.close_upvalues_from(stack_top);
    self.stack.pop();
}
```

在作用域结束（block 退出）时，编译器对每个被捕获的局部变量生成 `CLOSE_UPVALUE`。

在 RETURN 时，也需要关闭当前帧所有开放上值：

```rust
fn close_upvalues_from(&mut self, last: usize) {
    let mut i = self.open_upvalues.len();
    while i > 0 {
        i -= 1;
        let upvalue = unsafe { &mut *self.open_upvalues[i] };
        if upvalue.location < last {
            break;
        }
        upvalue.close(&self.stack);
        self.open_upvalues.remove(i);
    }
}
```

### 9. CALL 指令适配闭包

修改 Task 27 的 CALL 指令，增加 Closure 分支：

```rust
Object::Ref(ptr) if unsafe { (*ptr).type_tag } == TypeTag::CLOSURE as u8 => {
    let closure = unsafe { read_closure(ptr) };
    let func = unsafe { read_function(closure.function) };
    if argc != func.arity {
        return self.runtime_error(
            &format!("expected {} arguments, got {}", func.arity, argc)
        );
    }

    if self.call_stack.len() >= MAX_CALL_DEPTH {
        return self.runtime_error("stack overflow");
    }

    let stack_base = callee_idx;
    self.call_stack.push(CallFrame::new(
        ptr,  // *mut MsObjHeader 指向 MsClosure
        stack_base,
    ));
}
```

同时修改 `CallFrame`，将 `closure` 字段类型固定为 `*mut MsObjHeader`（已在 task 27 完成）。读取字节码和常量池改为从 `closure.function`（MsFunction）获取。顶层脚本也包装为无上值的 Closure。

## 验证标准

1. 内层函数能正确捕获外层局部变量（引用捕获，非值捕获）
2. 闭包能修改外层变量，修改对其他共享同一变量的闭包可见
3. 外层函数返回后，被捕获变量仍然存活
4. 多个闭包共享同一变量时，修改互相可见
5. 嵌套多层闭包时上值链正确解析
6. 开放上值在变量离开作用域时正确关闭
7. 所有函数调用统一使用 Closure 对象（包括无上值函数）

## 测试用例

```ms
fn make_counter() {
    count = 0
    return fn() {
        count += 1
        return count
    }
}

counter = make_counter()
print(counter())
print(counter())
print(counter())

fn make_pair() {
    x = 10
    getter = fn() { return x }
    setter = fn(v) { x = v }
    return getter, setter
}

get, set = make_pair()
print(get())
set(42)
print(get())
```

预期输出：

```
1
2
3
10
42
```
