# defer 语句

## 所属阶段
Phase 4.4 - 控制流 + 高级语法

## 前置任务
27-call-frame

> defer 核心逻辑（LIFO 顺序、参数声明时求值）不依赖 try/except。
> defer 与 try/except 的交互语义（异常时 defer 也执行）在 Task 37 完成后补充集成测试验证。
> Task 37 的前置任务包含本任务，形成单向依赖。

## 目标
实现 defer 语句，支持在函数返回前按 LIFO 顺序执行延迟调用，参数在声明时求值，异常时也执行。

## 设计规格

参照 [05-control-flow](../05-control-flow.md) § defer：

### 语法

```
defer_stmt = "defer" expression
```

### 字节码指令

参照 [11-bytecode-vm](../11-bytecode-vm.md) § defer：

| OpCode | 操作数 | 说明 |
|---|---|---|
| `DEFER` | — | 注册 defer 调用 |
| `EXEC_DEFER` | — | 执行所有 defer（函数返回前） |

### defer 执行规则

1. defer 在函数**返回前**执行（包括正常返回和异常返回）
2. 多个 defer 按 **LIFO**（后进先出）顺序执行
3. defer 的参数在 **defer 声明时**求值，不是在执行时

### 与 RETURN 的关系

参照 [11-bytecode-vm](../11-bytecode-vm.md) § CallFrame，每个 CallFrame 维护 `defer_stack_base`，用于追踪当前帧的 defer 栈范围。RETURN 指令在返回前必须执行 EXEC_DEFER。

## 实现细节

### 1. 解析 defer 语句

`src/parser/statement.rs`：

```rust
fn parse_defer(&mut self) -> Result<Stmt> {
    self.consume(TokenKind::Defer)?;
    let expr = self.parse_expression()?;
    // defer 后面应该是一个函数调用表达式
    Ok(Stmt::Defer { expr })
}
```

defer 后的表达式应为函数调用形式（如 `print("msg")`、`fin.close()`）。

### 2. 编译 defer

`src/compiler/statement.rs`：

```
编译 defer expr:

1. 编译 expr 表达式（包括参数求值）
   → 此时参数值已在栈上，被包装为一个无参闭包
2. emit DEFER
   → VM 从栈上取出闭包，推入当前帧的 defer 栈
```

实际上，更简单的实现方式：

```
编译 defer print(x):

1. 将 print 和 x 的值捕获为一个闭包
2. emit DEFER（闭包作为操作数或在栈顶）
```

推荐实现：将 defer 后的表达式编译为一个无参闭包，存储在 defer 栈中。

```
编译 defer expr:

1. 创建一个新的编译单元（匿名函数，无参数）
2. 在匿名函数内编译 expr（此时捕获外部变量作为 upvalue）
3. emit CLOSURE anonymous_func
4. emit DEFER
```

### 3. DEFER 指令实现

`src/vm/mod.rs`：

```rust
OpCode::DEFER => {
    let closure = self.stack_pop();
    self.defer_stack.push(DeferEntry {
        closure,
        frame_base: self.current_frame().stack_base,
    });
}
```

### 4. EXEC_DEFER 指令实现

```rust
OpCode::EXEC_DEFER => {
    let base = self.current_frame().defer_stack_base;
    while self.defer_stack.len() > base {
        let entry = self.defer_stack.pop().unwrap();
        // 调用延迟闭包
        self.call_closure(entry.closure, 0)?;
        // 立即执行（因为是无参闭包，应立即返回）
    }
}
```

### 5. RETURN 指令修改

在 RETURN 指令中，返回值之前先执行所有 defer。参照 05-control-flow.md § defer 异常交互规则 2/5：
- 规则 2：函数正常返回 + defer 抛异常 → defer 异常向外传播，**返回值被丢弃**
- 规则 5：defer 在 return 求值之后执行，返回值已被保存（defer 不能修改返回值）

```rust
OpCode::RETURN => {
    let return_value = self.stack_pop();
    
    // 执行当前帧的所有 defer（LIFO）
    let base = self.current_frame().defer_stack_base;
    while self.defer_stack.len() > base {
        let entry = self.defer_stack.pop().unwrap();
        // defer 正常执行：返回值已保存，不受影响（规则 5）
        // defer 抛异常：返回值被丢弃，异常向外传播（规则 2）
        match self.call_closure(entry.closure, 0) {
            Ok(_) => continue,
            Err(defer_err) => {
                // defer 抛异常 → 丢弃返回值，传播 defer 异常
                drop(return_value);
                return Err(defer_err);
            }
        }
    }
    
    // 所有 defer 正常完成 → 恢复调用帧，压入返回值
    self.pop_frame();
    self.stack_push(return_value);
}
```

### 6. 异常时的 defer

当异常发生时，在沿调用栈传播异常之前，需要执行当前帧的 defer。
参照 05-control-flow.md § defer 异常交互规则 1/3/4：
- 规则 1：defer 抛异常时，LIFO 栈中后续 defer 仍继续执行（不被跳过）。最后的异常向外传播，之前的异常附加为 `__cause__`
- 规则 3：函数因异常返回 + defer 正常 → defer 执行后，原异常继续传播
- 规则 4：函数因异常返回 + defer 也抛异常 → 原异常附加到 defer 异常的 `__cause__`

```rust
fn throw_exception(&mut self, mut err: Object) -> Result<()> {
    loop {
        // 执行当前帧的 defer（LIFO），构建 __cause__ 异常链
        let base = self.current_frame().defer_stack_base;
        while self.defer_stack.len() > base {
            let entry = self.defer_stack.pop().unwrap();
            match self.call_closure(entry.closure, 0) {
                Ok(_) => continue,
                // defer 抛新异常 → 原异常设为新异常的 __cause__（规则 1/4）
                Err(defer_err) => {
                    self.set_cause(&defer_err, err);
                    err = defer_err;
                    // 继续执行后续 defer（规则 1：不被跳过）
                    continue;
                }
            }
        }
        
        // 查找异常处理器
        if let Some(handler) = self.find_exception_handler() {
            self.handle_exception(handler, err);
            return Ok(());
        }
        
        // 无处理器，弹出帧
        if self.call_stack.len() <= 1 {
            // 到达顶层，终止程序
            return Err(err.into());
        }
        self.pop_frame();
    }
}

/// 设置异常的 __cause__ 属性（参照 05-control-flow.md § 异常对象）
fn set_cause(&mut self, exc: &Object, cause: Object) {
    if let Object::Ref(ptr) = exc {
        if unsafe { (**ptr).type_tag } == TypeTag::INSTANCE as u8 {
            let inst = unsafe { read_instance_mut(*ptr) };
            inst.fields.insert("__cause__".to_string(), cause);
        }
    }
}
```

### 7. DeferEntry 结构

```rust
struct DeferEntry {
    closure: Object,      // 无参闭包
    frame_base: usize,    // 所属帧的栈基址
}
```

## 验证标准

1. defer 在函数返回前执行
2. 多个 defer 按 LIFO 顺序执行
3. defer 参数在声明时求值（非执行时）
4. 异常发生时 defer 也执行
5. defer 栈与函数帧正确关联，嵌套调用互不干扰
6. 函数正常返回 + defer 抛异常 → 返回值被丢弃，defer 异常传播（规则 2）
7. defer 抛异常时后续 defer 仍执行（规则 1）
8. 多个 defer 抛异常时构建 `__cause__` 异常链（规则 1/4）

## 测试用例

```ms
// test_defer.ms — defer 语句

// 基本 LIFO 顺序
fn example() {
    defer print("first")
    defer print("second")
    defer print("third")
    print("body")
}
example()

// 参数在声明时求值
fn with_params() {
    for i in range(3) {
        defer print(i)
    }
}
with_params()

// defer 与 return
fn with_return() {
    defer print("deferred")
    return 42
}
result = with_return()
print(result)

// defer 在异常时也执行
fn with_error() {
    defer print("cleanup")
    throw ValueError("oops")
}

try {
    with_error()
} except ValueError as e {
    print("caught: " + e.message)
}

// 嵌套函数的 defer 互不干扰
fn outer() {
    defer print("outer defer")
    fn inner() {
        defer print("inner defer")
    }
    inner()
    print("after inner")
}
outer()
```

预期输出：

```
body
third
second
first
0
1
2
deferred
42
cleanup
caught: oops
outer defer
inner defer
after inner
```
