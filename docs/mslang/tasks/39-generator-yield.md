# 生成器与 yield

## 所属阶段
Phase 4.7 - 控制流 + 高级语法

## 前置任务
28-closures, 37-try-except-finally

## 目标
实现生成器函数与 yield/yield from，支持帧快照/恢复机制，生成器对象可作为迭代器在 for..in 中使用。同时实现生成器表达式（圆括号推导式）的编译与执行。

> **注**：生成器表达式的 AST 节点 `GeneratorExpression` 在 Task 09 中已定义（`expr` + `for_clauses: Vec<ForClause>` + `condition`），Task 14 中已解析为该节点。本任务负责其编译（编译为生成器函数 + yield）和 VM 执行。

## 设计规格

参照 [07-advanced](../07-advanced.md) § 生成器：

### yield 语法

包含 `yield` 的函数自动成为生成器函数。

```ms
fn countdown(n) {
    while n > 0 {
        yield n
        n = n - 1
    }
}
```

### 生成器语义

- 调用生成器函数**不执行函数体**，而是返回一个 Generator 对象
- Generator 实现 `__iter__`（返回 self）和 `__next__`（恢复执行）
- `yield expr` 暂停执行并返回 `expr` 的值
- 函数体执行完毕时抛出 `StopIteration`

### yield from

```ms
yield from flatten(item)  // 委托给另一个可迭代对象
```

### 字节码指令

参照 [11-bytecode-vm](../11-bytecode-vm.md) § 迭代：

| OpCode | 操作数 | 说明 |
|---|---|---|
| `YIELD` | — | yield 暂停 |
| `YIELD_FROM` | — | yield from 委托 |

### Generator 对象

参照 [11-bytecode-vm](../11-bytecode-vm.md) § 生成器执行模型：

```
Generator {
    frame: CallFrame         # 独立的调用帧（含 IP、栈基址）
    stack: Vec<Object>       # 独立的值栈副本
    locals: Vec<Object>      # 局部变量快照
    state: GeneratorState    # 状态
}

GeneratorState {
    Suspended,
    Running,
    Exhausted,
}
```

## 实现细节

### 1. 编译期检测生成器函数

`src/compiler/mod.rs`：

在编译函数体时，检测是否包含 `yield` 或 `yield from` 语句。如果包含，将 Function 标记为 `is_generator: true`。

```rust
struct Function {
    name: String,
    arity: usize,
    code: Vec<u8>,
    constants: Vec<Value>,
    upvalue_count: usize,
    is_generator: bool,    // 新增标记
}
```

编译器在遇到 `yield` 时设置当前编译单元的 `is_generator` 标志，函数编译完成后将该标志写入 Function。

### 2. 生成器函数调用

当 VM 执行 `CALL` 且目标是 `is_generator == true` 的函数时，不立即执行，而是创建 Generator 对象：

```rust
OpCode::CALL => {
    let argc = self.read_byte();
    let callee = self.stack_peek(argc);
    
    if let Object::Ref(ptr) = callee {
        if unsafe { (*ptr).type_tag } == TypeTag::CLOSURE as u8 {
            let func = unsafe { read_function(unsafe { read_closure(ptr) }.function) };
            if func.is_generator {
                return self.call_generator(ptr, argc);
            }
        }
    }
    // 普通函数调用
    self.call_function(argc);
}
```

```rust
fn call_generator(&mut self, closure_ptr: *mut MsObjHeader, argc: u8) -> Result<()> {
    // 创建独立的 CallFrame
    let frame = CallFrame {
        closure: closure_ptr,
        ip: 0,
        stack_base: self.stack.len(),
        defer_stack_base: self.defer_stack.len(),
    };
    
    // 创建独立栈，复制参数
    let mut gen_stack = Vec::new();
    for i in 0..=argc as usize {
        gen_stack.push(self.stack_peek(argc as usize - i));
    }
    
    let closure = unsafe { read_closure(closure_ptr) };
    let func = unsafe { read_function(closure.function) };
    let generator = MsGenerator {
        frame,
        stack: gen_stack,
        locals: vec![Object::Nil; func.locals_count],
        state: GeneratorState::Suspended,
        receiver: None,  // yield from 用的子迭代器
    };
    
    // 弹出 callee 和参数，压入 Generator
    for _ in 0..=argc as usize { self.stack_pop(); }
    self.stack_push(alloc_generator(generator));
    Ok(())
}
```

### 3. YIELD 指令

```rust
OpCode::YIELD => {
    let value = self.stack_pop();
    
    // 保存当前帧状态到 Generator（由调用者持有）
    // 注意：生成器运行在独立的帧中，需要特殊处理
    let gen = self.current_generator_mut();
    gen.frame.ip = self.current_frame().ip;
    gen.stack = self.current_frame_stack().to_vec();
    gen.locals = self.current_frame_locals().to_vec();
    gen.state = GeneratorState::Suspended;
    
    // 切换回调用者帧
    self.pop_generator_frame();
    
    // 将 yield 值压入调用者栈
    self.stack_push(value);
}
```

### 4. __next__ 调用（恢复生成器）

当调用 `gen.__next__()` 或 `FOR_ITER` 遇到 Generator 时：

```rust
fn resume_generator(&mut self, gen: &mut Generator) -> Result<Object> {
    match gen.state {
        GeneratorState::Exhausted => {
            return Err(StopIteration);
        }
        GeneratorState::Running => {
            return Err(RuntimeError("generator already running"));
        }
        GeneratorState::Suspended => {}
    }
    
    gen.state = GeneratorState::Running;
    
    // 保存调用者帧
    let caller_frame = self.current_frame().clone();
    
    // 将生成器帧推入调用栈
    self.push_generator_frame(gen);
    
    // 执行直到下一个 YIELD 或函数结束
    self.run_until_yield_or_return()
    
    // 结果在栈顶
}
```

### 5. YIELD_FROM 指令

```rust
OpCode::YIELD_FROM => {
    let iterable = self.stack_pop();
    
    // 创建子迭代器
    let sub_iter = self.create_iterator(iterable)?;
    
    // 存储到 Generator 的 receiver 字段
    let gen = self.current_generator_mut();
    gen.receiver = Some(sub_iter);
    
    // 产出子迭代器的下一个值
    self.yield_from_next();
}
```

```rust
fn yield_from_next(&mut self) -> Result<()> {
    let gen = self.current_generator_mut();
    let sub_iter = gen.receiver.as_mut().unwrap();
    
    match sub_iter.next() {
        Some(value) => {
            // 保存状态并 yield value
            gen.state = GeneratorState::Suspended;
            self.stack_push(value);
            // YIELD 逻辑...
        }
        None => {
            // 子迭代器耗尽，清除 receiver
            gen.receiver = None;
            // 继续当前生成器
        }
    }
}
```

### 6. FOR_ITER 与 Generator 集成

```rust
OpCode::FOR_ITER => {
    let offset = self.read_u16();
    let iter = self.stack_peek_mut(0);
    
    match iter {
        Object::Ref(ptr) if unsafe { (*(*ptr)).type_tag } == TypeTag::GENERATOR as u8 => {
            match self.resume_generator(*ptr) {
                Ok(value) => self.stack_push(value),
                Err(StopIteration) => {
                    unsafe { read_generator(*ptr) }.state = GeneratorState::Exhausted;
                    self.stack_pop(); // 弹出 generator
                    self.ip += offset as usize;
                }
                Err(e) => return Err(e),
            }
        }
        _ => {
            // 普通迭代器逻辑
            match iter.next() {
                Some(value) => self.stack_push(value),
                None => {
                    self.stack_pop();
                    self.ip += offset as usize;
                }
            }
        }
    }
}
```

### 7. 生成器函数结束时

当生成器函数执行到末尾或 `return` 时：

```rust
// 在生成器帧中遇到 RETURN 时
fn generator_return(&mut self, value: Object) {
    let gen = self.current_generator_mut();
    gen.state = GeneratorState::Exhausted;
    
    self.pop_generator_frame();
    // 不压入返回值（生成器的 return 值被忽略，抛出 StopIteration）
}
```

### 8. 生成器表达式编译

参照 [03-syntax](../03-syntax.md) § gen_expr、[07-advanced](../07-advanced.md) § 生成器表达式：

生成器表达式 `(expr for x in iter if cond)` 在编译时被变换为一个匿名生成器函数：

```rust
fn compile_generator_expression(&mut self, gen_expr: &GeneratorExpression) -> Result<()> {
    // 变换为:
    // fn __gen_expr_0(iter) {
    //     for x in iter {
    //         if cond { yield expr }
    //     }
    // }
    //
    // 编译为等价的嵌套循环 + yield
    self.begin_function("__gen_expr", 0);
    self.compile_gen_expr_body(gen_expr)?;
    self.end_function(true) // is_generator = true
}
```

- 生成的函数标记为 `is_generator = true`
- 多个 `for_clause` 编译为嵌套循环
- `condition` 编译为 `if` 包裹的 `yield`

### 9. 生成器关闭与 GC 清理（CLOSE_GENERATOR）

参照 [05-control-flow](../05-control-flow.md) § GeneratorExit、[07-advanced](../07-advanced.md) § 生成器关闭、[11-bytecode-vm](../11-bytecode-vm.md) § CLOSE_GENERATOR：

当生成器被 GC 回收但尚未耗尽（state != Exhausted）时，VM 必须自动注入 `GeneratorExit` 异常并恢复生成器帧执行，以触发 defer/finally 资源清理（如关闭文件句柄）。

#### CLOSE_GENERATOR 指令

```rust
OpCode::CloseGenerator => {
    let gen_obj = self.stack_pop();
    if let Object::Ref(ptr) = gen_obj {
        if unsafe { (**ptr).type_tag } == TypeTag::GENERATOR as u8 {
            self.close_generator(*ptr)?;
        }
    }
    Ok(())
}
```

#### close_generator 实现

```rust
fn close_generator(&mut self, gen_ptr: *mut MsObjHeader) -> Result<()> {
    let gen = unsafe { read_generator(gen_ptr) };
    match gen.state {
        GeneratorState::Exhausted | GeneratorState::Closed => return Ok(()),
        GeneratorState::Running => return Ok(()), // 正在运行，不重复关闭
        GeneratorState::Suspended => {}
    }

    // 注入 GeneratorExit 异常并恢复执行
    let gen_exit = self.create_exception("GeneratorExit", "generator closed");
    self.resume_generator_with_exception(gen_ptr, gen_exit)?;

    // 恢复后生成器应执行完 defer/finally 并到达 Exhausted 状态
    // 若生成器内部捕获了 GeneratorExit（不应发生），忽略
    Ok(())
}
```

#### GC finalizer 钩子

在 Task 52-gc 的 finalizer 阶段，对 state == Suspended 的 Generator 对象调用 `close_generator`：

```rust
// 参照 14-gc.md § finalizer 队列
fn finalize_generator(obj: *mut MsObjHeader) {
    let gen = unsafe { read_generator(obj) };
    if gen.state == GeneratorState::Suspended {
        // 通过 VM 引用注入 GeneratorExit 并恢复执行
        // 注意：finalizer 在 GC 安全点执行，此时 mutator 已暂停
        vm.close_generator(obj).ok();
    }
}
```

> **注意**：生成器的 GC finalizer 需要访问 VM 状态以恢复执行帧。Task 52-gc 的 finalizer 队列设计需支持携带 VM 引用或使用回调机制。

## 验证标准

1. 调用生成器函数返回 Generator 对象，不执行函数体
2. `__next__()` 恢复执行到下一个 yield
3. for..in 能正确遍历生成器
4. 生成器结束时抛出 StopIteration
5. yield from 正确委托给子可迭代对象
6. 生成器帧快照/恢复正确保留所有局部变量
7. 无限生成器（如 fibonacci）可惰性求值
8. 生成器表达式 `(x*x for x in range(10))` 返回惰性 Generator 对象
9. 带过滤的生成器表达式 `(x for x in nums if x > 0)` 正确过滤
10. 未耗尽的生成器被 GC 回收时自动注入 GeneratorExit，触发 defer/finally 清理

## 测试用例

```ms
// test_generator.ms — 生成器与 yield

// 基本生成器
fn countdown(n) {
    while n > 0 {
        yield n
        n = n - 1
    }
}

for i in countdown(5) {
    print(i)
}

// 手动迭代
fn gen3() {
    yield 10
    yield 20
    yield 30
}

g = gen3()
print(g.__next__())
print(g.__next__())
print(g.__next__())

// 无限生成器
fn fibonacci() {
    a, b = 0, 1
    while true {
        yield a
        a, b = b, a + b
    }
}

fib = fibonacci()
print(fib.__next__())
print(fib.__next__())
print(fib.__next__())
print(fib.__next__())

// yield from
fn flatten(nested) {
    for item in nested {
        if type(item) == "list" {
            yield from flatten(item)
        } else {
            yield item
        }
    }
}

for v in flatten([1, [2, 3], [4, [5, 6]]]) {
    print(v)
}

// 生成器中引用外部变量
fn make_counter(start) {
    n = start
    while true {
        yield n
        n += 1
    }
}

c = make_counter(100)
print(c.__next__())
print(c.__next__())
print(c.__next__())
```

预期输出：

```
5
4
3
2
1
10
20
30
0
1
1
2
1
2
3
4
5
6
100
101
102
```

### test_generator_expression.ms

```ms
# 生成器表达式 — 惰性求值
squares = (x * x for x in range(5))
for s in squares {
    print(s)
}

# 带过滤的生成器表达式
nums = [1, -2, 3, -4, 5]
positives = (x for x in nums if x > 0)
for p in positives {
    print(p)
}
```

预期输出：

```
0
1
4
9
16
1
3
5
```
