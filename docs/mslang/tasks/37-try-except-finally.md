# try/except/finally 异常处理

## 所属阶段
Phase 4.5 - 控制流 + 高级语法

## 前置任务
36-defer

## 目标
实现 try/except/finally 异常处理机制，包括异常对象、内置异常类型层级、throw 语句、异常传播、异常类型匹配。

## 设计规格

参照 [05-control-flow](../05-control-flow.md) § 错误处理：

### 语法

```
try_stmt    = "try" block except_clause* finally_clause?
except_clause = "except" type_spec? ("as" IDENTIFIER)? block
type_spec     = IDENTIFIER ("." IDENTIFIER)*
finally_clause = "finally" block
```

### 字节码指令

参照 [11-bytecode-vm](../11-bytecode-vm.md) § 异常：

| OpCode | 操作数 | 说明 |
|---|---|---|
| `TRY_ENTER` | `handler_offset(2)` | 进入 try 块，注册异常处理器 |
| `TRY_EXIT` | — | 离开 try 块，注销异常处理器 |
| `CATCH` | `type_idx(2)` | 检查异常是否匹配类型 |
| `THROW` | — | 抛出异常 |

### 异常对象属性

```
Error
├── message      # 错误消息（string）
├── type         # 错误类型名（string）
├── traceback    # 堆栈跟踪（string）
```

### 内置异常类型层级

```
Error
├── ValueError
├── TypeError
├── IndexError
├── KeyError
├── AttributeError
├── NameError
├── RuntimeError
├── IOError
├── ZeroDivisionError
├── OverflowError
└── StopIteration
```

### 语义

1. 执行 `try` 块
2. 若发生异常，按顺序检查 `except` 子句：
   - 不带类型：匹配所有异常
   - 带类型：匹配该类型及其子类型
   - `as name`：将异常对象绑定到 `name`
3. 无论是否发生异常，`finally` 块总是执行
4. 异常沿调用栈向上传播直到被捕获

## 实现细节

### 1. 异常对象实现

`src/vm/object.rs`：

内置异常类型在 VM 初始化时作为特殊的 Class 对象注册到全局变量中。

```rust
// 异常层级在 VM::new() 中初始化
fn init_exception_classes(&mut self) {
    let error_class = self.create_class("Error");
    error_class.set_attr("message", Object::Nil);
    error_class.set_attr("type", Object::Nil);
    error_class.set_attr("traceback", Object::Nil);
    
    let subclasses = [
        "ValueError", "TypeError", "IndexError", "KeyError",
        "AttributeError", "NameError", "RuntimeError", "IOError",
        "ZeroDivisionError", "OverflowError", "StopIteration",
    ];
    
    for name in &subclasses {
        let cls = self.create_class(name);
        cls.set_parent(error_class.clone());
        self.globals.insert(name.to_string(), cls);
    }
    
    self.globals.insert("Error".to_string(), error_class);
}
```

`throw` 语句创建异常实例：

```
throw ValueError("test")
→ 创建 ValueError 的实例（Instance）
→ 调用 __init__(self, "test")（或直接设置 message 字段）
→ 设置 type = "ValueError"
→ 设置 traceback = 当前调用栈信息
```

### 2. 编译 try/except/finally

`src/compiler/statement.rs`：

```
编译 try { body } except T1 as e { h1 } except { h2 } finally { fin }:

1. emit TRY_ENTER handler_start
2. 编译 try body
3. emit TRY_EXIT                  → 正常完成，注销处理器
4. emit JUMP finally_start        → 跳到 finally

5. handler_start:                  → 异常入口
   // 栈顶为异常对象
6. emit DUP
7. 加载 T1 类型
8. emit CATCH T1_idx
9. JUMP_IF_FALSE next_except
10. POP（弹出 bool）
11. STORE_LOCAL e                 → 绑定异常变量
12. POP（弹出异常对象）
13. 编译 h1
14. JUMP finally_start

15. next_except:
    // 裸 except
16. STORE_LOCAL _                  → 或直接 POP
17. 编译 h2
18. JUMP finally_start

19. finally_start:
20. 编译 fin
21. 结束
```

### 3. TRY_ENTER 指令

```rust
OpCode::TRY_ENTER => {
    let handler_offset = self.read_u16();
    self.exception_handlers.push(ExceptionHandler {
        catch_address: self.ip + handler_offset as usize,
        finally_address: None, // 后续设置
        frame_stack_base: self.current_frame().stack_base,
    });
}
```

### 4. TRY_EXIT 指令

```rust
OpCode::TRY_EXIT => {
    self.exception_handlers.pop();
}
```

### 5. CATCH 指令

```rust
OpCode::CATCH => {
    let type_idx = self.read_u16();
    let exc_type = self.constants[type_idx as usize].clone();
    
    let exception = self.stack_peek(0);
    let matches = self.exception_matches(exception, &exc_type);
    self.stack_push(Object::Bool(matches));
}
```

异常类型匹配（含子类）：

```rust
fn exception_matches(&self, exception: &Object, target_type: &str) -> bool {
    if let Object::Instance(inst) = exception {
        let mut class = inst.class.clone();
        loop {
            if class.name == target_type {
                return true;
            }
            match &class.parent {
                Some(parent) => class = parent.clone(),
                None => return false,
            }
        }
    }
    false
}
```

### 6. THROW 指令

```rust
OpCode::THROW => {
    let err = self.stack_pop();
    self.throw(err)?;
}
```

### 7. 异常传播

```rust
fn throw(&mut self, err: Object) -> Result<()> {
    // 执行当前帧的 defer
    self.exec_defers_for_current_frame();
    
    // 查找异常处理器
    while !self.exception_handlers.is_empty() {
        let handler = self.exception_handlers.last().unwrap();
        if handler.frame_stack_base >= self.current_frame().stack_base {
            self.ip = handler.catch_address;
            self.stack_push(err);
            return Ok(());
        }
        self.exception_handlers.pop();
    }
    
    // 无处理器，弹出帧
    if self.call_stack.len() > 1 {
        self.pop_frame();
        return self.throw(err);
    }
    
    // 顶层未捕获
    Err(MspError::RuntimeError {
        message: format_uncaught_error(&err),
    })
}
```

## 验证标准

1. try/except 正确捕获指定类型异常
2. 无类型 except 捕获所有异常
3. as 绑定正确将异常对象赋给变量
4. finally 块总是执行（正常和异常路径）
5. 异常沿调用栈传播
6. 子类异常匹配父类型 except
7. throw 正确创建和抛出异常
8. 未捕获异常终止程序并打印堆栈

## 测试用例

```ms
// test_try_except.ms — try/except/finally 异常处理

// 基本捕获
try {
    throw ValueError("test error")
} except ValueError as e {
    print("caught: " + e.message)
}

// finally 执行
try {
    x = 10 / 0
} except ZeroDivisionError as e {
    print("division error")
} finally {
    print("cleanup")
}

// 捕获所有
try {
    throw TypeError("type!")
} except {
    print("caught all")
}

// 多 except 子句
try {
    throw KeyError("missing")
} except ValueError as e {
    print("value error")
} except KeyError as e {
    print("key error: " + e.message)
}

// finally 在正常路径也执行
try {
    x = 42
} finally {
    print("always runs")
}

// 异常传播
fn inner() {
    throw RuntimeError("from inner")
}

fn outer() {
    inner()
}

try {
    outer()
} except RuntimeError as e {
    print("propagated: " + e.message)
}

// try/except/finally 组合
try {
    throw ValueError("combo")
} except ValueError as e {
    print("handled: " + e.message)
} finally {
    print("final cleanup")
}
```

预期输出：

```
caught: test error
division error
cleanup
caught all
key error: missing
always runs
propagated: from inner
handled: combo
final cleanup
```
