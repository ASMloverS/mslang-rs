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
    header: MsObjHeader             // 统一对象头
    function: *mut MsObjHeader      // 指向 MsFunction
    upvalues: Vec<*mut MsObjHeader> // 每项指向 MsUpvalue
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

运行时上值为带 `MsObjHeader` 的堆对象（`MsUpvalue`，TypeTag::UPVALUE — 见 §1），存在两种状态：

- **开放上值（Open Upvalue）**：`location` 指向栈上的局部变量槽位。当变量仍在作用域内时使用。
- **关闭上值（Closed Upvalue）**：变量离开作用域时，将值从栈拷贝到堆分配的 `closed` 字段中，后续访问改为读写 `closed`。

```
MsUpvalue {
    header:   MsObjHeader        // 统一对象头（TypeTag::UPVALUE）
    location: usize              // 栈位置（开放时有效）
    closed:   Option<Object>     // 堆存储（关闭后有效）
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
- 闭包内修改外层变量**必须**使用 `nonlocal` 声明（否则 `=` 赋值会在闭包内创建新的局部变量）

## 实现细节

### 1. 堆对象布局

引用 [20-object-system-basic](./20-object-system-basic.md) 的 `MsObjHeader` 和 `TypeTag`。

> **对齐 task 27**：`MsFunction`（嵌套 `Function` 结构体）、`MsClosure`（`upvalues: Vec<*mut MsObjHeader>`）已在 [27-call-frame](./27-call-frame.md) §2 落地（`src/vm/object.rs:481-568`）。**本任务不重定义这两个结构**，仅在运行时填充 `MsClosure.upvalues` 与 `MsFunction.function.upvalue_count`。本任务唯一新增的堆对象是 `MsUpvalue`。

#### MsUpvalue（本任务新增）

上值在运行时存在两种状态：

- **开放上值（Open Upvalue）**：`location` 指向栈上的局部变量槽位（变量仍在作用域内）。
- **关闭上值（Closed Upvalue）**：变量离开作用域时，将值从栈拷贝到 `closed` 字段；此后 `location` 不再使用。

为与 GC 统一对象头模型对齐（`14-gc.md:58-68`：所有 GC 管理的引用类型带 `MsObjHeader`），上值实现为带头的堆对象：

```rust
/// 上值堆对象。开放时读 location 指向的栈槽；关闭后读 closed。
/// TypeTag::UPVALUE（= 17，本任务新增 — 见下方 TypeTag 扩展）。
#[repr(C)]
pub struct MsUpvalue {
    pub header:   MsObjHeader,
    pub location: usize,            // 栈位置（开放时有效）
    pub closed:   Option<Object>,   // 堆存储（关闭后有效）
}

impl MsUpvalue {
    pub fn new(location: usize) -> Self {
        Self {
            header: MsObjHeader {
                gc_meta: 0,
                type_tag: TypeTag::UPVALUE as u8,
                size: std::mem::size_of::<MsUpvalue>() as u16,
                _padding: 0,
                class_ptr: 0,
            },
            location,
            closed: None,
        }
    }

    /// 读取上值当前持有的值。开放时读栈槽，关闭时读 closed。
    /// 调用方须保证 stack 在开放态下长度 > location。
    pub fn get(&self, stack: &[Object]) -> Object {
        match &self.closed {
            Some(val) => val.clone(),
            None => stack[self.location].clone(),
        }
    }

    /// 写入上值。开放时写栈槽，关闭时写 closed。
    pub fn set(&mut self, stack: &mut [Object], value: Object) {
        if self.closed.is_some() {
            self.closed = Some(value);
        } else {
            stack[self.location] = value;
        }
    }

    /// 关闭上值：将栈槽当前值拷贝到 closed。已关闭则幂等（不覆盖）。
    /// 调用方须保证此调用发生在栈截断之前（见 §8 RETURN 改造）。
    pub fn close(&mut self, stack: &[Object]) {
        if self.closed.is_none() {
            self.closed = Some(stack[self.location].clone());
        }
    }
}
```

#### TypeTag 扩展

`14-gc.md:90-109` 的 `TypeTag` 枚举当前无 UPVALUE 值。本任务在 [20-object-system-basic](./20-object-system-basic.md) 定义的全局权威 `TypeTag`（`src/vm/object.rs:22-40`）中新增：

```rust
#[repr(u8)]
pub enum TypeTag {
    // ... 1-16 不变 ...
    UPVALUE      = 17,   // 本任务新增
    LARGE_OBJECT = 0xFF,
}
```

> **设计规格回写**：`14-gc.md` 的 TypeTag 表须同步补 `UPVALUE = 17`（由本任务驱动的规格扩展）。

> **GC 所有权契约**：`MsUpvalue` 由 `MsClosure.upvalues` 最终持有（GC 回收 closure 时连带释放其全部 upvalues）。`VM.open_upvalues` 在上值关闭后移除指针，不再持有。task 52 GC 须为 `TypeTag::UPVALUE` 注册 trace 函数，遍历 `closed` 中的 `Object::Ref`（开放态下值在栈上，由栈根集覆盖）。

### 2. 堆分配辅助函数

> **对齐 task 27**：`alloc_function`、`read_function`、`read_closure` 已在 [27-call-frame](./27-call-frame.md) §2 落地且签名正确（参数化生命周期 `'a`，`read_closure` 返回不可变 `&'a MsClosure`）。**本任务不改写这三者**。下方仅列出本任务**新增**与**扩展**的函数。

#### 新增：alloc_upvalue / read_upvalue

```rust
/// 分配 MsUpvalue 堆对象（TypeTag::UPVALUE），返回 Object::Ref。
/// MVP：Box 分配；task 52-gc 替换为 TLAB bump 分配。
pub fn alloc_upvalue(location: usize) -> Object {
    let obj = Box::new(MsUpvalue::new(location));
    Object::Ref(Box::into_raw(obj) as *mut MsObjHeader)
}

/// 读取 MsUpvalue（alloc_upvalue 的对偶）。
///
/// # Safety
/// `ptr` 必须指向由 `alloc_upvalue` 分配的、在 `'a` 期间有效的 `MsUpvalue`。
/// 生命周期由调用方约束（`'a`），**不得**用 `'static` — 遵循 task 20 read_* 约定。
pub unsafe fn read_upvalue<'a>(ptr: *mut MsObjHeader) -> &'a mut MsUpvalue {
    &mut *(ptr as *mut MsUpvalue)
}
```

#### 扩展：alloc_closure（接收 upvalues）

task 27 的 `alloc_closure(function: Object) -> Object` 以空 upvalues 构造闭包。本任务将其签名**扩展**为接收 upvalues 列表（修改 `src/vm/object.rs:544` 的现有函数）：

```rust
/// 分配 MsClosure（TypeTag::CLOSURE），包裹一个 MsFunction 与其上值列表。
/// task 28 扩展：新增 upvalues 参数（task 27 原签名为单参、upvalues 恒空）。
pub fn alloc_closure(function: Object, upvalues: Vec<*mut MsObjHeader>) -> Object {
    let Object::Ref(func_ptr) = function else {
        unreachable!("alloc_closure expects MsFunction Ref");
    };
    let cl = Box::new(MsClosure {
        header: MsObjHeader {
            gc_meta: 0,
            type_tag: TypeTag::CLOSURE as u8,
            size: std::mem::size_of::<MsClosure>() as u16,
            _padding: 0,
            class_ptr: 0,
        },
        function: func_ptr,
        upvalues,
    });
    Object::Ref(Box::into_raw(cl) as *mut MsObjHeader)
}
```

> **task 27 调用点适配**：task 27 中 `alloc_closure(function)` 的两处调用（`compile_fn_decl` 的 `statement.rs:224`、`VM::new` 顶层帧）须改为 `alloc_closure(function, Vec::new())`。本任务 §4 会将 `compile_fn_decl` 进一步改造为发射 `CLOSURE` 指令（不再在编译期包装闭包）。

> **read_closure 保持不可变**：task 28 所有指令（LOAD/STORE_UPVALUE、CLOSURE、CALL）经 `read_closure` 取 `&MsClosure`（不可变），仅在操作具体 upvalue 指针时用 `read_upvalue` 取局部 `&mut MsUpvalue`。这避免对同一 closure 产生重叠 `&mut`（别名 UB）。

### 3. src/compiler/mod.rs — 编译单元上值追踪（已存在，本任务启用）

> **已实现**：`CompilationUnit` 的上值追踪基础设施在 task 17/27 已落地，本任务**不新增解析逻辑**，仅修复一处遗留断链：
> - `CompilationUnit.upvalues: Vec<Upvalue>`（`src/compiler/mod.rs:52`）
> - `Local.is_captured: bool`（`src/compiler/mod.rs:67`）
> - `CompilationUnit.parent: Option<&'a CompilationUnit<'a>>`（`src/compiler/mod.rs:56`）
> - `Compiler::resolve_local`（`src/compiler/mod.rs:277`）
> - `Compiler::resolve_upvalue`（`src/compiler/mod.rs:282`，含去重）
> - `CompilationUnit::resolve_upvalue_recursive`（`src/compiler/mod.rs:304`，递归走 `parent` 链）

#### 关键修复：compile_fn_decl 须链接 parent

当前 `compile_fn_decl`（`src/compiler/statement.rs:187-200`）创建子编译单元时设 `parent: None`，导致 `resolve_upvalue_recursive` 无法攀爬父链、上值解析失效。本任务将其改为链接父单元：

```rust
// statement.rs compile_fn_decl 内 —— 替换原 parent: None
let mut func_unit = CompilationUnit {
    chunk: super::Chunk::new(),
    locals: vec![Local { /* slot 0 = <self> */ }],
    upvalues: Vec::new(),
    scope_depth: 0,
    parent: Some(&self.unit),   // ← 链接父单元，使 resolve_upvalue_recursive 可达外层
};
```

> **借用注意**：`parent: Some(&self.unit)` 借用 `self.unit` 的不可变引用。由于编译函数体期间通过 `std::mem::replace(&mut self.unit, func_unit)` 已将父单元换出（`statement.rs:209`），实际借用的是被换出的 `saved_unit`。须确保子单元存活期不超过 `saved_unit`（当前 `replace` 模式已满足：func_unit 在 replace 回父单元后即被消费构建 Function）。实现时若借用检查报错，可将所需父信息（locals 快照）显式传入，但优先尝试引用链方案。

#### 上值捕获标记

`resolve_upvalue_recursive`（`src/compiler/mod.rs:304-313`）在 parent.locals 命中时返回 `(idx, true)`，但**当前未设置 `parent.locals[idx].is_captured = true`**。本任务须补此副作用——`is_captured` 标志驱动 §4 在作用域退出时发射 `CLOSE_UPVALUE`：

```rust
// mod.rs resolve_upvalue_recursive —— 命中 parent 局部变量时标记捕获
fn resolve_upvalue_recursive(&self, name: &str) -> Option<(usize, bool)> {
    let parent = self.parent?;
    if let Some(idx) = parent.locals.iter().rposition(|l| l.name == name) {
        // SAFETY: parent 为 &'_ 不可变引用；is_captured 标记需 &mut。
        // 实现方案：将 resolve_upvalue_recursive 改为接收 &mut parent，
        // 或在 Compiler::resolve_upvalue 层（持有 self.unit 可变）回填标记。
        // 见下方 Compiler::resolve_upvalue 补丁。
        Some((idx, true))
    } else {
        parent.resolve_upvalue_recursive(name).map(|(idx, _)| (idx, false))
    }
}
```

由于 `parent` 是不可变引用，`is_captured` 回填须在 `Compiler::resolve_upvalue`（持有 `&mut self`）层完成。`resolve_upvalue_recursive` 已返回命中的 `(idx, is_local)`；当 `is_local == true` 时，Compiler 须定位到对应 parent 单元并置 `is_captured = true`。实现可选择：
1. 递归函数额外返回 parent 单元的可访问路径，或
2. 在 `resolve_upvalue` 成功后，遍历 `self.unit.parent` 链找到 `is_local` 对应层级回填。

> **实现提示**：方案 1 更直接——将 `resolve_upvalue_recursive` 签名改为 `&mut self` 并递归传递 `&mut parent`。由于 `CompilationUnit.parent: Option<&CompilationUnit>`（不可变），改为 `Option<&mut CompilationUnit>` 会引发借用冲突。推荐采用 clox 风格：Compiler 维护一个 `Vec<&mut CompilationUnit>` 编译单元栈（显式压栈/弹栈），避免 `self.unit` 单字段替换与 parent 引用的生命周期纠缠。本任务实现者须选定一种方案并在实现时验证借用合法性。

### 4. 编译闭包捕获

#### 变量读取解析优先级（已实现于 expression.rs:105-108）

1. 当前编译单元的局部变量 → `LOAD_LOCAL`
2. 外层编译单元的局部变量或上值（经 `resolve_upvalue`）→ `LOAD_UPVALUE`
3. 全局变量 → `LOAD_GLOBAL`

#### 变量写入与 `nonlocal` 语义（本任务补全）

参照 [04-functions](../04-functions.md) §闭包语义（"闭包内修改外层变量**必须**使用 `nonlocal` 声明"），写捕获须区分是否声明 `nonlocal`：

| 写入场景 | 条件 | 行为 |
|---|---|---|
| 赋值 `x = v`（无 nonlocal） | `resolve_local(x)` 命中 | `STORE_LOCAL`（写当前作用域局部） |
| 赋值 `x = v`（无 nonlocal） | `resolve_local(x)` 未命中 | **创建新局部** `STORE_LOCAL`（不穿透外层） |
| 赋值 `x = v`（有 `nonlocal x`） | `resolve_upvalue(x)` 命中 | `STORE_UPVALUE` |
| 赋值 `x = v`（有 `nonlocal x`） | `resolve_upvalue(x)` 未命中 | **编译错误** `"no binding for nonlocal 'x'"` |

> `nonlocal` 声明已由 task 15 解析、task 19 编译为 `compile_nonlocal`（`statement.rs:255-265`）标记 `self.nonlocal_names`。本任务在**赋值编译**（`statement.rs` 的 `compile_assign`）中据 `self.nonlocal_names` 分派：声明为 nonlocal 的名字强制走 upvalue 路径，否则按现有局部/全局逻辑（未命中局部时创建新局部，不自动穿透）。

> **复合赋值**（`x += 1` ≡ `x = x + 1`）：读取侧 `x` 按 `resolve_local → resolve_upvalue` 解析；写入侧按上表 nonlocal 规则分派。若 `x` 既未声明 nonlocal 又非当前局部，则读走 upvalue、写创建新局部——此时 `+=` 会"读取外层、写入新局部"，符合 Python 语义（`04-functions.md:175`）。

#### compile_fn_decl 改造（CLOSURE 指令发射）

task 27 的 `compile_fn_decl`（`statement.rs:180-232`）在编译期即 `alloc_closure` 包装并存 Closure 入常量池、发 `CONSTANT`。本任务改为**存 Function（非 Closure）入常量池**、发 `CLOSURE(func_idx)` 并跟逐上值操作数：

```rust
fn compile_fn_decl(&mut self, name: &str, params: &[Param], body: &[Stmt], line: usize)
    -> Result<(), String>
{
    let mut func_unit = CompilationUnit {
        chunk: Chunk::new(),
        locals: vec![Local { name: "<self>".into(), depth: 0, is_captured: false }],
        upvalues: Vec::new(),
        scope_depth: 0,
        parent: Some(&self.unit),   // §3：链接父单元
    };
    for param in params {
        func_unit.locals.push(Local { name: param.name.clone(), depth: 0, is_captured: false });
    }

    let saved_unit = std::mem::replace(&mut self.unit, func_unit);
    self.compile_block(body, line)?;
    self.emit_byte(OpCode::Nil as u8, line);
    self.emit_byte(OpCode::Return as u8, line);
    let func_unit = std::mem::replace(&mut self.unit, saved_unit);

    // 存 Function（非 Closure）入常量池 —— CLOSURE 指令运行期包装。
    let function = alloc_function(Function {
        name: name.to_string(),
        arity: params.len(),
        code: func_unit.chunk.code,
        constants: func_unit.chunk.constants,
        upvalue_count: func_unit.upvalues.len(),   // ← task 27 写 0，本任务写真值
        source_file: self.source_file.clone(),
    });
    let func_idx = self.add_constant(function);
    let func_idx = u16::try_from(func_idx)
        .map_err(|_| "constant pool overflow".to_string())?;

    // 发 CLOSURE(func_idx) + 逐上值操作数（is_local:1 + index:1 每上值）
    self.emit_byte(OpCode::Closure as u8, line);
    self.emit_bytes(&func_idx.to_be_bytes(), line);
    for uv in &func_unit.upvalues {
        self.emit_byte(if uv.is_local { 1 } else { 0 }, line);
        let idx = u8::try_from(uv.index).map_err(|_| {
            format!("upvalue index {} exceeds 255 (function too large)", uv.index)
        })?;
        self.emit_byte(idx, line);
    }

    // 绑定函数名到全局（与 task 27 一致）
    let name_idx = self.add_constant(alloc_string(name));
    let name_idx = u16::try_from(name_idx).map_err(|_| "constant pool overflow".to_string())?;
    self.emit_byte(OpCode::StoreGlobal as u8, line);
    self.emit_bytes(&name_idx.to_be_bytes(), line);
    Ok(())
}
```

> **操作数编码**：`CLOSURE` 后跟 `func_idx(2)` + 每上值 `(is_local:1, index:1)`。`11-bytecode-vm.md:118` 仅定义 `CLOSURE | func_idx(2)`，上值操作数序列为本任务扩展（须回写设计规格）。`index` 单字节 → 单函数局部变量 + 上值 ≤ 255（编译期断言，溢出报错，对齐 task 27 V3 对 argc 的处理）。

#### 作用域退出时发射 CLOSE_UPVALUE

block 退出（`end_scope`）时，编译器须对每个 `is_captured == true` 且 `depth > 当前作用域` 的局部变量发射 `CLOSE_UPVALUE` + `POP`：

```rust
fn end_scope(&mut self, line: usize) {
    while self.unit.locals.last().map_or(false, |l| l.depth > self.unit.scope_depth) {
        let local = self.unit.locals.pop().unwrap();
        if local.is_captured {
            self.emit_byte(OpCode::CloseUpvalue as u8, line);
        } else {
            self.emit_byte(OpCode::Pop as u8, line);
        }
    }
    self.unit.scope_depth -= 1;
}
```

> `CLOSE_UPVALUE` 关闭栈顶位置对应的开放上值并弹栈（§8）。故被捕获局部须按栈顶到栈底的逆序弹出（`locals.pop()` 自然满足：最深处局部在栈顶）。

### 5. src/vm/mod.rs — CLOSURE 指令

```rust
OpCode::CLOSURE => {
    let func_idx = self.read_u16() as usize;
    // current_frame_constants 经 closure.function.function.constants 读取（task 27 已接线）
    let func_obj = self.current_frame_constants()[func_idx].clone();

    let func_ptr = match func_obj {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::FUNCTION as u8 => ptr,
        _ => return self.runtime_error("CLOSURE expects a Function"),
    };

    // task 27 嵌套布局：read_function(ptr).function.upvalue_count
    let upvalue_count = unsafe { read_function(func_ptr) }.function.upvalue_count;
    let mut upvalues: Vec<*mut MsObjHeader> = Vec::with_capacity(upvalue_count);

    for _ in 0..upvalue_count {
        let is_local = self.read_byte() == 1;
        let index = self.read_byte() as usize;   // 编译期已断言 ≤ 255（§4）

        if is_local {
            let stack_base = self.current_frame().stack_base;
            let location = stack_base + index;
            upvalues.push(self.capture_upvalue(location));   // → *mut MsObjHeader (MsUpvalue)
        } else {
            // 复用当前闭包的上值（外层上值链）
            let closure_ptr = self.current_frame().closure;
            let closure = unsafe { read_closure(closure_ptr) };   // 不可变 &MsClosure
            upvalues.push(closure.upvalues[index]);
        }
    }

    // alloc_closure 扩展签名（§2）：接收 Object + upvalues
    let closure_obj = alloc_closure(Object::Ref(func_ptr), upvalues);
    self.stack.push(closure_obj);
}
```

### 6. src/vm/mod.rs — 上值捕获

`VM.open_upvalues: Vec<*mut MsObjHeader>`（task 27 已声明，`11-bytecode-vm.md:303`），每项指向 `MsUpvalue`（TypeTag::UPVALUE）。

> **排序不变量（关键）**：`open_upvalues` 须按 `location` **升序**维护（最小 location 在 `open_upvalues[0]`，最大在末尾）。`close_upvalues_from`（§8）**从末尾向前**扫描——升序下末尾即最大 location，逐个关闭 `location >= last` 的上值，遇 `location < last` 即 break。`capture_upvalue` 必须按升序插入，而非简单 push 末尾（否则 close 会提前中断或遗漏，遗留应关闭的上值 → 栈槽复用后读穿，见审核 V1）。

```rust
impl VM {
    /// 捕获（或复用）指向栈槽 location 的开放上值，返回 *mut MsObjHeader (MsUpvalue)。
    /// 插入时维持 open_upvalues 按 location 升序。
    fn capture_upvalue(&mut self, location: usize) -> *mut MsObjHeader {
        // 升序表中，第一个 location >= 新 location 的位置即插入点；
        // 若该处已有等 location 上值则复用。
        let insert_at = self.open_upvalues.iter().position(|&ptr| {
            // SAFETY: ptr 指向由 alloc_upvalue 分配的有效 MsUpvalue。
            let loc = unsafe { (*(ptr as *mut MsUpvalue)).location };
            loc >= location
        });

        if let Some(i) = insert_at {
            let existing = self.open_upvalues[i];
            let loc = unsafe { (*(existing as *mut MsUpvalue)).location };
            if loc == location {
                return existing;   // 复用已存在的开放上值
            }
            // 插入新上值于 i（保持升序：新 location < existing[i].location）
            let Object::Ref(ptr) = alloc_upvalue(location) else { unreachable!() };
            self.open_upvalues.insert(i, ptr);
            return ptr;
        }

        // 新 location 大于所有现存上值 → 追加末尾
        let Object::Ref(ptr) = alloc_upvalue(location) else { unreachable!() };
        self.open_upvalues.push(ptr);
        ptr
    }
}
```

> **升序约定说明**：`close_upvalues_from(last)` 从 `open_upvalues` 末尾（最大 location）向前扫描，关闭所有 `location >= last` 的上值——升序保证一旦遇到 `location < last`，其前（更小索引）的所有 location 均更小，确属作用域外，可安全 break。

### 7. LOAD_UPVALUE / STORE_UPVALUE

`closure.upvalues[idx]` 为 `*mut MsObjHeader`（指向 MsUpvalue）。经 `read_closure`（不可变）取 closure，再经 `read_upvalue` 取具体上值的可变引用——仅一个局部 `&mut MsUpvalue`，不与其它借用重叠：

```rust
OpCode::LOAD_UPVALUE => {
    let idx = self.read_byte() as usize;
    let closure_ptr = self.current_frame().closure;
    let closure = unsafe { read_closure(closure_ptr) };   // 不可变 &MsClosure
    let upvalue_ptr = closure.upvalues[idx];
    // SAFETY: upvalue_ptr 指向由 alloc_upvalue 分配的有效 MsUpvalue。
    let value = unsafe { read_upvalue(upvalue_ptr) }.get(&self.stack);
    self.stack.push(value);
}

OpCode::STORE_UPVALUE => {
    let idx = self.read_byte() as usize;
    let value = self.stack.last().cloned().unwrap_or(Object::Nil);  // peek 栈顶（不弹）
    let closure_ptr = self.current_frame().closure;
    let closure = unsafe { read_closure(closure_ptr) };   // 不可变 &MsClosure
    let upvalue_ptr = closure.upvalues[idx];
    // SAFETY: upvalue_ptr 指向由 alloc_upvalue 分配的有效 MsUpvalue。
    unsafe { read_upvalue(upvalue_ptr) }.set(&mut self.stack, value);
}
```

> **GC 写屏障占位（task 52/62）**：`STORE_UPVALUE` 修改堆对象（上值的 closed/栈槽）属堆引用写入。`14-gc.md:529` 明列 `STORE_UPVALUE` 在并发标记期需触发写屏障。MVP（task 52 STW GC）无并发标记，本任务不插入屏障；task 62 上线并发 GC 时须在 `set` 写入路径补 `write_barrier(...)`（编译器统一注入，`14-gc.md:766-771`）。

### 8. CLOSE_UPVALUE + close_upvalues_from

```rust
OpCode::CLOSE_UPVALUE => {
    let stack_top = self.stack.len() - 1;
    self.close_upvalues_from(stack_top);
    self.stack.pop();
}
```

在作用域结束（block 退出）时，编译器对每个 `is_captured` 局部变量发射 `CLOSE_UPVALUE`（见 §4 `end_scope`）。

```rust
/// 关闭所有 location >= last 的开放上值。
/// 依赖 open_upvalues 按 location 升序（§6 维护）：从末尾（最大 location）向前扫，
/// 遇 location < last 即停止（升序保证其前所有 location 更小，确属作用域外）。
fn close_upvalues_from(&mut self, last: usize) {
    let mut i = self.open_upvalues.len();
    while i > 0 {
        i -= 1;
        let ptr = self.open_upvalues[i];
        // SAFETY: ptr 指向由 alloc_upvalue 分配的有效 MsUpvalue。
        let upvalue = unsafe { read_upvalue(ptr) };
        if upvalue.location < last {
            break;
        }
        upvalue.close(&self.stack);   // 须在栈截断前调用（见 RETURN 改造）
        self.open_upvalues.remove(i);
    }
}
```

#### RETURN 改造（修改 task 27 的 RETURN — V4 修复）

task 27 的 RETURN（`src/vm/mod.rs` OpCode::Return 分支）先 `stack.truncate(old_base)` 再压返回值。**必须在 truncate 之前关闭本帧所有开放上值**，否则 `close()` 读取的 `stack[location]` 可能已位于截断区外（越界 panic）：

```rust
OpCode::RETURN => {
    let return_value = self.stack.pop().unwrap_or(Object::Nil);

    // 先关闭当前帧的所有开放上值（栈尚未截断，location 仍有效）。
    let old_base = self.call_stack.last().unwrap().stack_base;
    self.close_upvalues_from(old_base);   // ← task 28 新增：truncate 前关闭

    self.stack.truncate(old_base);        // 移除 callee(slot0)+args+locals
    self.call_stack.pop();
    self.stack.push(return_value);        // 返回值压入调用者栈顶

    // task 36 补：EXEC_DEFER（LIFO）。Phase 3.2 defer_stack 恒空，暂不做。
}
```

### 9. CALL 指令（task 27 已正确实现，本任务无需修改）

task 27 的 CALL 分支（`src/vm/mod.rs:597-622`）已按嵌套布局正确实现：

```rust
Object::Ref(ptr)
    if unsafe { (**ptr).type_tag } == TypeTag::CLOSURE as u8 =>
{
    let arity = {
        let closure = unsafe { read_closure(*ptr) };   // 不可变 &MsClosure
        let func = unsafe { read_function(closure.function) };
        func.function.arity                            // ← 嵌套访问（task 27 已对）
    };
    if argc != arity {
        return Err(format!("TypeError: expected {} arguments, got {}", arity, argc));
    }
    if self.call_stack.len() >= MAX_CALL_DEPTH {
        return Err("RecursionError: stack overflow".to_string());
    }
    self.call_stack.push(CallFrame::new(*ptr, callee_idx));
}
```

本任务**不改写 CALL**：task 28 的 `CLOSURE` 指令（§5）在运行期创建带 upvalues 的闭包并压栈，CALL 消费该闭包时经 `closure.function` 读取字节码与常量池的路径（task 27 已接线于 `current_frame_*` 辅助函数）对有/无 upvalues 的闭包一致。

> **CallFrame.closure**：字段类型 `*mut MsObjHeader`（task 27 已固定，`src/vm/frame.rs:7`）。顶层脚本也包装为无上值的 Closure（task 27 `VM::new` 已处理；本任务 §2 的 `alloc_closure` 扩展签名后，该处改为 `alloc_closure(alloc_function(...), Vec::new())`）。

> **callee_idx 边界**：执行前须 `argc + 1 <= self.stack.len()` 校验（task 25 FUNCTION 分支已做此前置保护，CLOSURE 分支共用）。

## 验证标准

1. 内层函数能正确捕获外层局部变量（引用捕获，非值捕获）
2. 闭包能修改外层变量（经 `nonlocal` 声明），修改对其他共享同一变量的闭包可见
3. 外层函数返回后，被捕获变量仍然存活（上值已 close 到堆）
4. 多个闭包共享同一变量时，修改互相可见
5. 嵌套多层闭包时上值链正确解析（`is_local=false` 递归复用父闭包上值）
6. 开放上值在变量离开作用域时正确关闭（`CLOSE_UPVALUE` + `end_scope`）
7. 所有函数调用统一使用 Closure 对象（`CLOSURE` 指令运行期包装，包括无上值函数）
8. `nonlocal` 声明的变量在赋值时走 `STORE_UPVALUE`；未声明 nonlocal 且非当前局部时赋值创建新局部（不穿透）
9. `nonlocal X` 但 X 不存在于外层作用域 → 编译错误 `"no binding for nonlocal 'X'"`
10. `RETURN` 在栈截断前关闭本帧所有开放上值（验证：被捕获变量在函数返回后仍可正确读取）
11. `TypeTag::UPVALUE = 17` 已加入全局 TypeTag 枚举（`src/vm/object.rs`）
12. 深递归场景下 `open_upvalues` 升序不变量成立（`close_upvalues_from` 不提前中断或遗漏）

## 测试用例

```ms
fn make_counter() {
    count = 0
    return fn() {
        nonlocal count        # 写外层变量须声明 nonlocal（04-functions.md:175）
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
    getter = fn() { return x }              # 只读捕获，无需 nonlocal
    setter = fn(v) {
        nonlocal x                          # 写外层变量须声明 nonlocal
        x = v
    }
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
