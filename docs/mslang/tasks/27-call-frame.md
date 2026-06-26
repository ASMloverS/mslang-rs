# 调用帧与函数调用

## 所属阶段
Phase 3.1 - 函数 + 闭包

## 前置任务
- 19-compile-statements（FnDecl 当前为 stub，本 task 实装编译）
- 23-vm-core（CallFrame/frames 现所在，本 task 重构为 closure 指针 + call_stack）
- 25-builtins-basic（native CALL 的 `TypeTag::FUNCTION` 分支已落地，本 task 不改写、仅新增 CLOSURE 分支）
- 26-builtins-iterators

## 目标
实现 `CallFrame` 结构、`CALL` / `RETURN` 指令、函数声明的编译与调用编译，使 VM 能正确执行用户定义的函数并返回结果。

## 设计规格

### CallFrame

参照 [11-bytecode-vm](../11-bytecode-vm.md) § CallFrame：

```
CallFrame {
    closure: *mut MsObjHeader,  // 指向 MsClosure；Phase 3.1 先用 Function 包装为单闭包
    ip: usize,                  // 程序计数器
    stack_base: usize,          // 当前帧的栈基址
    defer_stack_base: usize,    // defer 栈基址
}
```

### Function 对象

参照 [11-bytecode-vm](../11-bytecode-vm.md) § Function：

```
Function {
    name: String
    arity: usize              // 必需参数数量
    code: Vec<u8>             // 函数体字节码
    constants: Vec<Value>     // 函数体常量池（独立）
    upvalue_count: usize      // 上值数量（Phase 3.1 中为 0）
}
```

### CALL 指令

参照 [11-bytecode-vm](../11-bytecode-vm.md) § 函数调用：

| OpCode | 操作数 | 说明 |
|---|---|---|
| `CALL` | `argc(1)` | 调用函数，argc 为参数数量 |

执行逻辑：
1. 从栈顶弹出 `argc` 个参数
2. 弹出被调用者（callee）
3. 校验 callee 是否为 Callable（Function / Closure / Builtin）
4. 校验参数数量与 `arity` 匹配
5. 保存当前 CallFrame 的 `ip` 到调用栈
6. 创建新 CallFrame：`closure = callee`，`ip = 0`，`stack_base = 当前栈顶 - argc`
7. 将参数复制到新帧的局部变量槽位（slot 0..argc）
8. VM 切换到新 CallFrame 执行

### RETURN 指令

| OpCode | 操作数 | 说明 |
|---|---|---|
| `RETURN` | — | 从函数返回 |

执行逻辑：
1. 弹出栈顶作为返回值
2. 恢复上一个 CallFrame（从调用栈弹出）
3. 恢复 `stack_base`
4. 将返回值压入恢复后的栈顶
5. VM 切换到恢复的 CallFrame 继续执行

### 函数声明编译

参照 [04-functions](../04-functions.md) § 函数定义：

```
fn_def = "fn" IDENTIFIER "(" param_list? ")" block
```

编译步骤：
1. 解析函数名和参数列表，确定 `arity`
2. 创建新的 `CompilationUnit`（独立 code + constants + locals）
3. 将参数注册为局部变量（slot 0, 1, ..., arity-1）
4. 编译函数体（block）中的语句
5. 若函数体最后没有 `RETURN`，自动追加 `NIL + RETURN`（隐式返回 nil）
6. 构建 `Function` 对象，存入父编译单元的常量池
7. 在父编译单元中生成 `CONSTANT(func_idx)` 将 Function 压栈
8. 在父编译单元中生成 `STORE_GLOBAL(name_idx)` 将函数名绑定到全局

### 函数调用编译

参照 [04-functions](../04-functions.md) § First-class 函数：

调用表达式 `callee(arg1, arg2, ...)` 编译步骤：
1. 编译 callee 表达式，结果压栈
2. 依次编译每个参数表达式，结果压栈
3. 生成 `CALL(argc)`

### 调用栈管理

参照 [11-bytecode-vm](../11-bytecode-vm.md) § 虚拟机核心：

- `VM.call_stack: Vec<CallFrame>` 管理嵌套调用
- 初始帧（顶层脚本）也作为一个 CallFrame 存在于调用栈底部
- 调用栈最大深度限制（`MAX_CALL_DEPTH`，建议 **1000**，对齐 Python 默认；256 过低会使 factorial(1000) 等深递归误触栈溢出），防止栈溢出。该常量被 task 28/31/36/37/70 共用
- `defer_stack_base` 在 Phase 3.1 中暂不使用，预留即可

## 实现细节

### 1. src/vm/frame.rs — CallFrame 定义

```rust
use crate::vm::object::{Object, MsObjHeader};

#[derive(Clone)]
pub struct CallFrame {
    pub closure: *mut MsObjHeader,  // 指向 MsClosure（由 task 28 定义）
    pub ip: usize,
    pub stack_base: usize,
    pub defer_stack_base: usize,
}

impl CallFrame {
    pub fn new(closure: *mut MsObjHeader, stack_base: usize) -> Self {
        Self {
            closure,
            ip: 0,
            stack_base,
            defer_stack_base: 0,
        }
    }

    pub fn snapshot(&self) -> CallFrame {
        self.clone()
    }
}
```

### 2. src/vm/object.rs — Function / Closure 堆对象

> **TypeTag 约定（订正 A2）**：`TypeTag::FUNCTION` 已被 task 25 的 `MsNativeFunction`（内置函数）占用。故用户可调用对象**必须**经 `TypeTag::CLOSURE` 表示。Phase 3.1 引入**最小 `MsClosure`**（包裹 `MsFunction`，`upvalues` 恒空）；完整 upvalue 机制由 task 28 实装。`MsFunction`（用户函数体）内部仍可用 `TypeTag::FUNCTION` 作存储 tag——但**绝不直接作为 CALL 的被调用者**（CALL 只认 CLOSURE；FUNCTION 分支属 task 25 native）。

```rust
/// 用户函数体（堆对象，TypeTag::FUNCTION）。仅由 MsClosure 内部持有，
/// CALL 不直接匹配此 tag（避免与 MsNativeFunction 混淆 — 订正 A2/V2）。
#[repr(C)]
pub struct MsFunction {
    pub header: MsObjHeader,
    pub function: Function,   // name/arity/code/constants/upvalue_count/source_file
}

pub struct Function {
    pub name: String,
    pub arity: usize,
    pub code: Vec<u8>,
    pub constants: Vec<Object>,
    pub upvalue_count: usize,
    pub source_file: Option<String>,
}

impl Function {
    pub fn new(name: String, arity: usize) -> Self {
        Self {
            name, arity,
            code: Vec::new(),
            constants: Vec::new(),
            upvalue_count: 0,
            source_file: None,
        }
    }
}

/// 分配 MsFunction 堆对象（TypeTag::FUNCTION），返回 Object::Ref。
/// MVP：Box 分配；task 52-gc 替换为 TLAB bump 分配。
pub fn alloc_function(function: Function) -> Object {
    let ms_fn = Box::new(MsFunction {
        header: MsObjHeader {
            gc_meta: 0,
            type_tag: TypeTag::FUNCTION as u8,
            size: std::mem::size_of::<MsFunction>() as u16,
            _padding: 0,
            class_ptr: 0,
        },
        function,
    });
    Object::Ref(Box::into_raw(ms_fn) as *mut MsObjHeader)
}

/// 读取 MsFunction（alloc_function 的对偶）。
///
/// # Safety
/// `ptr` 必须指向由 `alloc_function` 分配的、在 `'a` 期间有效的 `MsFunction`。
pub unsafe fn read_function<'a>(ptr: *mut MsObjHeader) -> &'a MsFunction {
    &*(ptr as *mut MsFunction)
}

/// 最小闭包（TypeTag::CLOSURE）。Phase 3.1：upvalues 恒空（task 28 实装真实上值）。
/// 这是用户代码唯一可调用的形式 — CALL 的被调用者必须是 CLOSURE（订正 A2）。
#[repr(C)]
pub struct MsClosure {
    pub header: MsObjHeader,
    pub function: *mut MsObjHeader,      // 指向 MsFunction
    pub upvalues: Vec<*mut MsObjHeader>, // Phase 3.1 为空
}

/// 分配 MsClosure（TypeTag::CLOSURE），包裹一个 MsFunction。
pub fn alloc_closure(function: Object) -> Object {
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
        upvalues: Vec::new(),
    });
    Object::Ref(Box::into_raw(cl) as *mut MsObjHeader)
}

/// 读取 MsClosure（alloc_closure 的对偶）。
///
/// # Safety
/// `ptr` 必须指向由 `alloc_closure` 分配的、在 `'a` 期间有效的 `MsClosure`。
pub unsafe fn read_closure<'a>(ptr: *mut MsObjHeader) -> &'a MsClosure {
    &*(ptr as *mut MsClosure)
}
```

> **GC 前瞻（task 52）**：新增 `MsFunction`（含 `Vec<u8>` code、`Vec<Object>` constants）与 `MsClosure`（含 `Vec` upvalues + function 指针）。task 52 的 TypeDescriptor 须为 FUNCTION/CLOSURE 注册真实 `trace`/`copy_for_gc`（当前为 noop 占位）：MsClosure.trace 须遍历 `function` 与各 upvalue；MsFunction.trace 须遍历 constants 中的 Ref。

### 3. src/compiler/mod.rs — 函数声明编译

```rust
fn compile_fn_decl(&mut self, node: &FnDecl) {
    let mut func_unit = CompilationUnit::new();
    func_unit.name = node.name.clone();
    func_unit.arity = node.params.len();

    // 订正 A3/V1：预留 slot 0 给被调用者（closure 自身），与 CALL 的
    // stack_base = callee_idx 自洽（slot 0 = stack[stack_base] = callee）。
    // 参数从 slot 1 起注册（slot 1..arity）。否则 param0 会读到 callee。
    func_unit.locals.push(Local {
        name: "<self>".into(), // 占位 slot 0（closure 自身）
        depth: 0,
        is_captured: false,
    });
    for param in node.params.iter() {
        func_unit.locals.push(Local {
            name: param.clone(),
            depth: 0,
            is_captured: false,
        });
    }

    let saved_unit = std::mem::replace(&mut self.unit, func_unit);
    self.compile_block(&node.body);
    self.emit(OpCode::NIL);
    self.emit(OpCode::RETURN);

    let func_unit = std::mem::replace(&mut self.unit, saved_unit);

    // MsFunction 存入常量池，再包装为 MsClosure（CLOSURE）— 用户可调用形式。
    let function = alloc_function(Function {
        name: func_unit.name,
        arity: func_unit.arity,
        code: func_unit.code,
        constants: func_unit.constants,
        upvalue_count: 0,
        source_file: self.source_file.clone(),
    });
    let closure = alloc_closure(function); // 订正 A2：发布的是 CLOSURE
    let idx = self.add_constant(closure);

    self.emit_constant(idx);
    let name_idx = self.add_constant(alloc_string(&node.name));
    self.emit_with_operand(OpCode::STORE_GLOBAL, name_idx as u16);
}
```

### 4. src/compiler/mod.rs — 函数调用编译

```rust
fn compile_call(&mut self, callee: &Expr, args: &[Expr]) -> Result<(), String> {
    // V3：argc 为单字节（CALL | argc(1)），>255 须编译期报错，避免 as u8 静默截断。
    if args.len() > u8::MAX as usize {
        return Err(format!(
            "too many arguments ({} > max 255) in call", args.len()
        ));
    }
    self.compile_expr(callee);
    for arg in args {
        self.compile_expr(arg);
    }
    self.emit_with_operand(OpCode::CALL, args.len() as u8);
    Ok(())
}
```

### 5. src/vm/mod.rs — CALL 指令执行

> **订正 A1/A2**：task 25 已实现 `TypeTag::FUNCTION`（native `MsNativeFunction`）分支（`mod.rs:519-544`），**本 task 不改写该分支**，仅**新增 `TypeTag::CLOSURE` 分支**处理用户函数。删除原先不存在的 `Object::BuiltinFunc` 分支（Object 无此变体）。下方仅展示新增的 CLOSURE 分支（与 task 25 的 FUNCTION 分支并列于同一 `OpCode::Call` match）。

```rust
// 新增分支（与 task 25 的 TypeTag::FUNCTION native 分支并列）：
Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::CLOSURE as u8 => {
    let (arity, func_ptr) = {
        // SAFETY：type_tag 为 CLOSURE，指针由 alloc_closure 分配。
        let closure = unsafe { read_closure(*ptr) };
        // 经 closure.function 取 MsFunction 读 arity（不借用 self）。
        let func = unsafe { read_function(closure.function) };
        (func.function.arity, closure.function)
    };
    if argc != arity {
        return self.runtime_error(
            &format!("expected {} arguments, got {}", arity, argc)
        );
    }
    if self.call_stack.len() >= MAX_CALL_DEPTH {
        return self.runtime_error("stack overflow");
    }

    // stack_base = callee_idx：slot 0 = callee（closure 自身），参数在 slot 1..argc
    // （与 compile_fn_decl 的 slot-0 预留约定自洽 — 订正 A3/V1）。
    let stack_base = callee_idx;
    self.call_stack.push(CallFrame::new(*ptr, stack_base));
    // CallFrame::new 已设 stack_base；ip 默认 0。不再二次赋值。
    let _ = func_ptr; // Phase 3.1 不使用 upvalues；task 28 在此初始化上值
}
_ => {} // 其余 callable（FUNCTION native 已由 task 25 分支处理；BOUND_METHOD/INSTANCE __call__ 由 task 41/43）
```

> **V3 修复（argc 溢出）**：编译侧 `compile_call` 须在 `args.len() as u8` 前断言 `args.len() <= u8::MAX as usize`，否则编译期报错（`"too many arguments (max 255)"`），避免静默截断致 arity 校验失真。CALL argc 为单字节（`11-bytecode-vm.md:47` `CALL | argc(1)`）。

> **callee_idx 边界**：执行前须 `argc + 1 <= self.stack.len()` 校验（task 25 的 FUNCTION 分支已在 `mod.rs:522` 做了同样保护），CLOSURE 分支共用此前置校验。

### 6. src/vm/mod.rs — RETURN 指令执行

```rust
OpCode::RETURN => {
    let return_value = self.stack.pop().unwrap_or(Object::Nil);

    // task 36 补：弹出本帧前须执行其 `[defer_stack_base..defer_stack.len())` 的
    // defer 条目（EXEC_DEFER，LIFO）。Phase 3.1 defer_stack 恒空，此处暂不做。
    let old_base = self.call_stack.last().unwrap().stack_base;
    self.stack.truncate(old_base);   // 移除 callee(slot0)+args+locals
    self.call_stack.pop();

    self.stack.push(return_value);   // 返回值落在调用者栈顶（替代原 callee+args 区段）
}
```

> **值栈按帧分段不变量（R4）**：CALL/RETURN 经 `stack_base` 维持每帧 `[stack_base..stack_top)` 区段独立。生成器/async 的完整栈段快照（含区间拷贝）推迟到 task 39/53（与 task 23 声明一致）；本 task 的 `CallFrame::snapshot()` 仅 clone 字段。

### 7. 顶层脚本 CallFrame

VM 初始化时创建顶层 CallFrame：

```rust
impl VM {
    pub fn new() -> Self {
        let main_function = Function::new("<main>".into(), 0);
        // main 也经 alloc_closure（CLOSURE）包装，与 CallFrame.closure 约定一致（订正 A2）。
        let Object::Ref(main_ptr) = alloc_closure(alloc_function(main_function))
            else { unreachable!() };
        let main_frame = CallFrame::new(main_ptr, 0);
        Self {
            // 预留 slot 0（callee 占位），修复 task 26 发现的「顶层预留 slot 0
            // 但 VM 栈未预分配 → StoreLocal 1 越界」bug（订正 A3）。
            stack: vec![Object::Nil],
            call_stack: vec![main_frame],
            globals: HashMap::new(),
            defer_stack: Vec::new(),
            open_upvalues: Vec::new(),
            // ...
        }
    }
}
```

## 验证标准

1. 函数声明正确编译为 Function 对象并存入常量池
2. 函数名绑定到全局变量表
3. 函数调用正确设置 CallFrame，参数正确传递
4. RETURN 正确恢复上一个 CallFrame，返回值正确压栈
5. 参数数量不匹配时抛出运行时错误
6. 嵌套调用深度超过限制时抛出栈溢出错误
7. 函数无显式 return 时返回 nil
8. 递归调用正确工作
9. `print` 等内置函数仍正常工作（经 task 25 的 `TypeTag::FUNCTION` native 分支，本 task 不改写）
10. CallFrame 可被 clone 用于帧快照（Phase 4 生成器和 Phase 7 async/await 依赖此能力）

## 测试用例

```ms
fn greet(name) {
    return "Hello, " + name
}

fn add(a, b) {
    return a + b
}

print(greet("World"))
print(add(3, 4))

fn factorial(n) {
    if n <= 1 {
        return 1
    }
    return n * factorial(n - 1)
}
print(factorial(10))
```

预期输出：

```
Hello, World
7
3628800
```
