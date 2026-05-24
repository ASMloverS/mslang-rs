# with 语句（上下文管理器）

## 所属阶段
Phase 4.6 - 控制流 + 高级语法

## 前置任务
37-try-except-finally

## 目标
实现 with 语句，支持上下文管理器协议（`__enter__` / `__exit__`），包括异常传递和异常抑制。

## 设计规格

参照 [05-control-flow](../05-control-flow.md) § with 语句：

### 语法

```
with_stmt = "with" expression ("as" IDENTIFIER)? block
```

### 上下文管理器协议

```ms
fn __enter__(self) -> value
fn __exit__(self, err_type, err_msg, traceback) -> bool
```

### with 执行流程

1. 求值 `with` 后的表达式，得到上下文管理器对象
2. 调用 `__enter__()`，返回值绑定到 `as` 变量（如有）
3. 执行块体
4. 离开块时（正常或异常），调用 `__exit__(err_type, err_msg, traceback)`
5. 若块内有异常，异常信息传递给 `__exit__`
6. 若 `__exit__` 返回 `true`，异常被抑制；返回 `false` 或 `nil` 则异常继续传播

### 嵌套 with

```ms
with ctx1 as a {
    with ctx2 as b {
        // ...
    }
}
```

## 实现细节

### 1. 编译 with 语句

`src/compiler/statement.rs`：

```
编译 with expr as name { body }:

1. 编译 expr                    → 压栈上下文管理器
2. emit DUP                     → 复制一份用于 __exit__
3. emit GET_ATTR "__enter__"
4. emit CALL 0                  → 调用 __enter__()
5. if name: emit STORE_LOCAL name
   else: emit POP
6. emit TRY_ENTER handler       → 注册异常处理器
7. 编译 body
8. emit TRY_EXIT                → 正常完成
9. emit JUMP cleanup

10. handler:                    → 异常路径
    // 栈顶为异常对象
    保存异常到临时局部变量
    emit EXEC_DEFER（如果帧内有 defer）

11. cleanup:
    // 正常和异常路径汇合
    加载步骤 2 复制的上下文管理器
    emit GET_ATTR "__exit__"
    
    if 正常路径:
        push nil, nil, nil      → 无异常
    if 异常路径:
        push err_type, err_msg, traceback
    
    emit CALL 3                 → 调用 __exit__(err, msg, tb)
    
    if 异常路径:
        检查 __exit__ 返回值
        如果为 true → POP 异常（抑制）
        如果为 false → THROW 异常（继续传播）
```

### 2. 简化编译方案

由于 with 的编译较为复杂，可先使用等价的 try/finally 方式：

```
编译 with expr as name { body } 等价为:

_tmp = expr
_value = _tmp.__enter__()
try {
    name = _value (或忽略)
    body
} finally {
    _tmp.__exit__(err_type, err_msg, traceback)
}
```

实际字节码序列：

```
1. 编译 expr
2. DUP → 保存管理器引用
3. GET_ATTR "__enter__" + CALL 0
4. STORE_LOCAL name (或 POP)
5. // 保存管理器到临时变量
   STORE_LOCAL _tmp_ctx
6. TRY_ENTER handler
7. 编译 body
8. TRY_EXIT
9. JUMP cleanup

handler:
10. STORE_LOCAL _exc      → 保存异常
11. TRY_EXIT              → 注销处理器

cleanup:
12. LOAD_LOCAL _tmp_ctx
13. GET_ATTR "__exit__"
14. LOAD_LOCAL _exc       → 异常对象或 nil
15. GET_ATTR "type" (或 nil)
16. GET_ATTR "message" (或 nil)
17. GET_ATTR "traceback" (或 nil)
18. CALL 3
19. // 检查返回值决定是否抑制
    如果有异常且 __exit__ 返回 false/nil → 重新抛出
```

### 3. 异常信息传递

当块内发生异常时，需要构造三个参数传递给 `__exit__`：

| 参数 | 正常时 | 异常时 |
|---|---|---|
| `err_type` | `nil` | 异常类名（如 `"ValueError"`） |
| `err_msg` | `nil` | 异常的 message 属性 |
| `traceback` | `nil` | 堆栈跟踪字符串 |

### 4. 异常抑制逻辑

```rust
// __exit__ 返回后检查
let suppress = self.stack_pop();
if has_exception && !is_truthy(&suppress) {
    // 重新抛出异常
    let exc = self.stack_pop();
    self.throw(exc)?;
}
```

## 验证标准

1. with 正确调用 `__enter__` 和 `__exit__`
2. `as` 变量绑定 `__enter__` 返回值
3. 正常退出时 `__exit__` 参数全为 nil
4. 异常退出时 `__exit__` 接收异常信息
5. `__exit__` 返回 true 时异常被抑制
6. `__exit__` 返回 false 时异常继续传播
7. 嵌套 with 正确工作

## 测试用例

```ms
// test_with.ms — with 语句

// 基本上下文管理器（使用字典模拟，待 class 实现后替换）
fn test_basic() {
    ctx = {
        "__enter__": fn(self) { print("enter"); return self },
        "__exit__": fn(self, err, msg, tb) { print("exit"); return false }
    }
    with ctx as c {
        print("body")
    }
}
test_basic()

// with 中发生异常
fn test_exception() {
    ctx = {
        "__enter__": fn(self) { print("enter"); return self },
        "__exit__": fn(self, err, msg, tb) {
            print("exit with: " + str(err))
            return false
        }
    }
    try {
        with ctx as c {
            print("before error")
            throw ValueError("oops")
            print("unreachable")
        }
    } except ValueError as e {
        print("caught: " + e.message)
    }
}
test_exception()

// __exit__ 抑制异常
fn test_suppress() {
    ctx = {
        "__enter__": fn(self) { return self },
        "__exit__": fn(self, err, msg, tb) {
            print("suppressing: " + str(err))
            return true
        }
    }
    with ctx as c {
        throw ValueError("suppressed")
    }
    print("after with")
}
test_suppress()

// 嵌套 with
fn test_nested() {
    ctx1 = {
        "__enter__": fn(self) { print("enter1"); return self },
        "__exit__": fn(self, err, msg, tb) { print("exit1"); return false }
    }
    ctx2 = {
        "__enter__": fn(self) { print("enter2"); return self },
        "__exit__": fn(self, err, msg, tb) { print("exit2"); return false }
    }
    with ctx1 as a {
        with ctx2 as b {
            print("nested body")
        }
    }
}
test_nested()
```

预期输出：

```
enter
body
exit
enter
before error
exit with: ValueError
caught: oops
suppressing: ValueError
after with
enter1
enter2
nested body
exit2
exit1
```
