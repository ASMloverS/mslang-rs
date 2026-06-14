# 默认参数与可变参数

## 所属阶段
Phase 3.5 - 函数 + 闭包

## 前置任务
- 27-call-frame（调用帧与函数调用）

## 目标
实现函数默认参数值和可变参数（`*rest`），完善函数参数系统，使参数组合（普通 → 默认 → 可变）正确工作。

## 设计规格

### 默认参数

参照 [04-functions](../04-functions.md) § 默认参数：

```ms
fn greet(name, prefix = "Hello") {
    return prefix + ", " + name
}
```

- 默认参数值在**函数定义时**求值一次（与 Python 一致）
- 调用时若省略带默认值的参数，使用定义时求得的默认值
- 调用时若提供实参，覆盖默认值

### 可变参数

参照 [04-functions](../04-functions.md) § 可变参数：

```ms
fn sum(*numbers) {
    total = 0
    for n in numbers {
        total += n
    }
    return total
}
```

- `*rest` 将多余的位置参数收集为一个 list
- `rest` 在函数体内作为普通局部变量使用，类型为 list

### 参数组合

参照 [04-functions](../04-functions.md) § 参数组合：

```ms
fn example(a, b, c = 10, *rest) {
    # a, b: 必需参数
    # c: 带默认值的参数
    # rest: 可变参数（list）
}
```

参数顺序规则：**普通参数 → 默认参数 → 可变参数**

### 实参数量校验

| 场景 | 有效实参数量 |
|---|---|
| `fn(a, b)` | 必须 == 2 |
| `fn(a, b, c = 10)` | 2 或 3 |
| `fn(*rest)` | >= 0 |
| `fn(a, *rest)` | >= 1 |
| `fn(a, b, c = 10, *rest)` | >= 2 |

## 实现细节

### 1. Function 对象扩展

```rust
pub struct Function {
    pub name: String,
    pub arity: usize,
    pub code: Vec<u8>,
    pub constants: Vec<Object>,
    pub upvalue_count: usize,

    pub default_params: Vec<usize>,       // 默认参数在常量池中的起始索引列表
    pub has_variadic: bool,               // 是否有可变参数
    pub required_arity: usize,            // 必需参数数量（不含默认和可变）
}
```

- `required_arity`：普通参数数量（必需）
- `arity`：全部固定参数数量（普通 + 默认）
- `default_params`：长度为默认参数数量，每个元素为对应默认值在函数自身常量池中的索引
- `has_variadic`：是否有 `*rest` 参数

### 2. 解析器扩展

参数列表解析需要支持三种参数类型：

```rust
struct Param {
    name: String,
    kind: ParamKind,
}

enum ParamKind {
    Normal,
    Default(Expr),
    Variadic,
}
```

解析逻辑：

```rust
fn parse_param_list(&mut self) -> Result<Vec<Param>> {
    let mut params = Vec::new();
    let mut seen_variadic = false;

    while !self.check(TokenKind::RightParen) {
        if seen_variadic {
            return self.error("variadic parameter must be last");
        }

        if self.match_token(TokenKind::Star) {
            let name = self.expect_ident()?;
            params.push(Param { name, kind: ParamKind::Variadic });
            seen_variadic = true;
        } else {
            let name = self.expect_ident()?;
            if self.match_token(TokenKind::Equal) {
                let default_expr = self.parse_expr()?;
                params.push(Param { name, kind: ParamKind::Default(default_expr) });
            } else {
                params.push(Param { name, kind: ParamKind::Normal });
            }
        }

        if !self.match_token(TokenKind::Comma) {
            break;
        }
    }

    self.validate_param_order(&params)?;
    Ok(params)
}

fn validate_param_order(&self, params: &[Param]) -> Result<()> {
    let mut state = 0; // 0=normal, 1=default, 2=variadic
    for p in params {
        match p.kind {
            ParamKind::Normal => {
                if state > 0 { return self.error("normal parameter after default/variadic"); }
            }
            ParamKind::Default(..) => {
                if state > 1 { return self.error("default parameter after variadic"); }
                state = 1;
            }
            ParamKind::Variadic => {
                state = 2;
            }
        }
    }
    Ok(())
}
```

### 3. 编译器 — 默认参数值

默认参数值在函数定义时求值，存入函数的常量池：

```rust
fn compile_fn_decl(&mut self, node: &FnDecl) {
    let mut func_unit = CompilationUnit::new();
    func_unit.name = node.name.clone();

    let mut required_arity = 0;
    let mut default_param_values = Vec::new();

    for param in &node.params {
        match &param.kind {
            ParamKind::Normal => {
                required_arity += 1;
                func_unit.locals.push(Local {
                    name: param.name.clone(),
                    depth: 0,
                    is_captured: false,
                });
            }
            ParamKind::Default(expr) => {
                let mut default_compiler = CompilationUnit::new();
                self.compile_expr_in_unit(&mut default_compiler, expr);
                let default_value = self.eval_constant(expr);
                let const_idx = func_unit.constants.len();
                func_unit.constants.push(default_value);
                default_param_values.push(const_idx);

                func_unit.locals.push(Local {
                    name: param.name.clone(),
                    depth: 0,
                    is_captured: false,
                });
            }
            ParamKind::Variadic => {
                func_unit.has_variadic = true;
                func_unit.locals.push(Local {
                    name: param.name.clone(),
                    depth: 0,
                    is_captured: false,
                });
            }
        }
    }

    func_unit.required_arity = required_arity;
    func_unit.arity = func_unit.locals.len();

    // ... 编译函数体 ...
}
```

### 4. 编译器 — 调用点处理

调用时不需要特殊处理。实参数量校验和默认值填充在运行时的 CALL 指令中完成。

### 5. VM — CALL 指令扩展

```rust
OpCode::CALL => {
    let argc = self.read_byte() as usize;
    let callee_idx = self.stack.len() - argc - 1;
    let callee = self.stack[callee_idx].clone();

    match callee {
        Object::Closure(closure) => {
            let func = &closure.function;
            let required = func.required_arity;
            let total_fixed = func.arity;
            let has_variadic = func.has_variadic;

            if has_variadic {
                if argc < required {
                    return self.runtime_error(&format!(
                        "{} expects at least {} arguments, got {}",
                        func.name, required, argc
                    ));
                }
            } else {
                if argc < required || argc > total_fixed {
                    return self.runtime_error(&format!(
                        "{} expects {}-{} arguments, got {}",
                        func.name, required, total_fixed, argc
                    ));
                }
            }

            if self.call_stack.len() >= MAX_CALL_DEPTH {
                return self.runtime_error("stack overflow");
            }

            // 填充默认参数值
            if argc < total_fixed {
                let defaults_to_fill = total_fixed - argc;
                for i in 0..defaults_to_fill {
                    let const_idx = func.default_params[argc - required + i];
                    self.stack.push(func.constants[const_idx].clone());
                }
            }

            // 处理可变参数：将多余实参收集为 list
            let varargs_start = if has_variadic { total_fixed } else { argc };
            if has_variadic && argc > total_fixed {
                let extra = argc - total_fixed;
                let varargs: Vec<Object> = self.stack.drain(
                    self.stack.len() - extra - (callee_idx + 1)..self.stack.len()
                ).collect();
                // 修正：从 callee 之后、固定参数之后取出多余参数
                let stack_len = self.stack.len();
                let drain_start = callee_idx + 1 + total_fixed;
                let varargs: Vec<Object> = self.stack.drain(drain_start..).collect();
                self.stack.push(alloc_list(&varargs));
            } else if has_variadic {
                self.stack.push(alloc_list(&[]));
            }

            let stack_base = callee_idx;
            self.call_stack.push(CallFrame::new_closure(
                closure,
                stack_base,
            ));
        }
        // ... BuiltinFunc 等 ...
    }
}
```

### 6. 调用帧参数布局

调用新帧时栈布局：

```
[callee] [arg0] [arg1] ... [argN-1] [defaults...] [varargs_list]
 ^                                                   ^
 stack_base                                      new stack_top
```

对于 `fn(a, b, c = 10, *rest)`，调用 `example(1, 2)` 时：

```
[closure] [1] [2] [10] []
 ^        a   b   c   rest
 stack_base
```

### 7. Function 的 Display / Debug

带默认参数的函数打印信息应包含参数签名：

```
<function greet(name, prefix = "Hello")>
```

## 验证标准

1. 默认参数值在定义时求值，后续调用共享同一默认值
2. 省略默认参数时使用默认值，提供实参时覆盖
3. `*rest` 将多余实参收集为 list
4. 无多余实参时 `*rest` 为空 list `[]`
5. 参数顺序强制：普通 → 默认 → 可变
6. 实参数量不足必需参数时报错
7. 实参数量超过固定参数且有可变参数时正确收集
8. 实参数量超过固定参数且无 variadic 时报错

## 测试用例

```ms
fn greet(name, prefix = "Hello") {
    return prefix + ", " + name
}

print(greet("Alice"))
print(greet("Alice", "Hi"))

fn sum(*numbers) {
    total = 0
    for n in numbers {
        total += n
    }
    return total
}

print(sum(1, 2, 3))
print(sum(1, 2, 3, 4, 5))

fn example(a, b, c = 10, *rest) {
    print(a)
    print(b)
    print(c)
    print(rest)
}

example(1, 2)
example(1, 2, 3)
example(1, 2, 3, 4, 5)
```

预期输出：

```
Hello, Alice
Hi, Alice
6
15
1
2
10
[]
1
2
3
[]
1
2
3
[4, 5]
```
