# 调用帧与函数调用

## 所属阶段
Phase 3.1 - 函数 + 闭包

## 前置任务
- 26-builtins-iterators（Phase 2.5 完成的内置函数与迭代器）

## 目标
实现 `CallFrame` 结构、`CALL` / `RETURN` 指令、函数声明的编译与调用编译，使 VM 能正确执行用户定义的函数并返回结果。

## 设计规格

### CallFrame

参照 [11-bytecode-vm](../11-bytecode-vm.md) § CallFrame：

```
CallFrame {
    closure: Gc<Closure>,    // 被调用的闭包（Phase 3.1 先用 Function 包装为单闭包）
    ip: usize,               // 程序计数器
    stack_base: usize,       // 当前帧的栈基址
    defer_stack_base: usize, // defer 栈基址
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
- 调用栈最大深度限制（建议 256），防止栈溢出
- `defer_stack_base` 在 Phase 3.1 中暂不使用，预留即可

## 实现细节

### 1. src/vm/frame.rs — CallFrame 定义

```rust
use crate::vm::object::{Object, Gc};

#[derive(Clone)]
pub struct CallFrame {
    pub function: Gc<Function>,
    pub ip: usize,
    pub stack_base: usize,
    pub defer_stack_base: usize,
}

impl CallFrame {
    pub fn new(function: Gc<Function>, stack_base: usize) -> Self {
        Self {
            function,
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

### 2. src/vm/object.rs — Function 对象

```rust
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
            name,
            arity,
            code: Vec::new(),
            constants: Vec::new(),
            upvalue_count: 0,
            source_file: None,
        }
    }
}
```

在 `Object` 枚举中已有 `Function(Gc<Function>)` 变体（参照 [11-bytecode-vm](../11-bytecode-vm.md) § 对象系统）。

### 3. src/compiler/mod.rs — 函数声明编译

```rust
fn compile_fn_decl(&mut self, node: &FnDecl) {
    let func_idx = self.constant_pool.len();

    let mut func_unit = CompilationUnit::new();
    func_unit.name = node.name.clone();
    func_unit.arity = node.params.len();

    for (i, param) in node.params.iter().enumerate() {
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

    let function = Object::Function(Gc::new(Function {
        name: func_unit.name,
        arity: func_unit.arity,
        code: func_unit.code,
        constants: func_unit.constants,
        upvalue_count: 0,
        source_file: self.source_file.clone(),
    }));
    let idx = self.add_constant(function);

    self.emit_constant(idx);
    let name_idx = self.add_constant(Object::String(node.name.clone().into()));
    self.emit_with_operand(OpCode::STORE_GLOBAL, name_idx as u16);
}
```

### 4. src/compiler/mod.rs — 函数调用编译

```rust
fn compile_call(&mut self, callee: &Expr, args: &[Expr]) {
    self.compile_expr(callee);
    for arg in args {
        self.compile_expr(arg);
    }
    self.emit_with_operand(OpCode::CALL, args.len() as u8);
}
```

### 5. src/vm/mod.rs — CALL 指令执行

```rust
OpCode::CALL => {
    let argc = self.read_byte() as usize;
    let stack_top = self.stack.len();

    let callee_idx = stack_top - argc - 1;
    let callee = self.stack[callee_idx].clone();

    match callee {
        Object::Function(func) => {
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
                func.clone(),
                self.current_frame().stack_base,
            ));

            let frame = self.call_stack.last_mut().unwrap();
            frame.stack_base = stack_base;
            frame.ip = 0;
        }
        Object::BuiltinFunc(builtin) => {
            let args: Vec<Object> = self.stack.drain(stack_top - argc..).collect();
            self.stack.pop();
            let result = (builtin.func)(&args)?;
            self.stack.push(result);
        }
        _ => return self.runtime_error("not a callable object"),
    }
}
```

### 6. src/vm/mod.rs — RETURN 指令执行

```rust
OpCode::RETURN => {
    let return_value = self.stack.pop().unwrap_or(Object::Nil);

    let old_base = self.call_stack.last().unwrap().stack_base;
    self.stack.truncate(old_base);
    self.call_stack.pop();

    self.stack.push(return_value);
}
```

### 7. 顶层脚本 CallFrame

VM 初始化时创建顶层 CallFrame：

```rust
impl VM {
    pub fn new() -> Self {
        let main_function = Function::new("<main>".into(), 0);
        let main_frame = CallFrame::new(Gc::new(main_function), 0);
        Self {
            stack: Vec::new(),
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
9. `print` 等内置函数仍正常工作（通过 BuiltinFunc 分支）
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
