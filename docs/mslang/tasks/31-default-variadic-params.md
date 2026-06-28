# 默认参数与可变参数

## 所属阶段
Phase 3.5 - 函数 + 闭包

## 前置任务
- 14-parser-collection-literals（Param 结构体 + `parse_param_list` 已实现）
- 27-call-frame（调用帧与函数调用）
- 28-closures（compile_fn_decl + CLOSURE 机制）
- 29-anonymous-functions（compile_fn_literal 镜像 compile_fn_decl）

## 目标
实现函数默认参数值和可变参数（`*rest`），完善函数参数系统，使参数组合（普通 → 默认 → 可变）正确工作。

## 设计规格

参照 [04-functions](../04-functions.md) § 默认参数 / 可变参数 / 参数组合 / 实参数量校验。

## 已完成（勿重复实现）

| 能力 | 实现位置 | 实现 task |
|---|---|---|
| `Param { name, default: Option<Expr>, is_variadic: bool }` | `src/ast/node.rs:65-68` | 09/14 |
| 解析器 `parse_param_list`（`*name` / `name = expr` / 普通） | `src/parser/statement.rs:247-281` | 14 |
| `compile_fn_decl`（CompilationUnit struct literal, slot-0, CLOSURE 发射） | `src/compiler/statement.rs:196-280` | 27/28 |
| `compile_fn_literal`（匿名函数，镜像 compile_fn_decl） | `src/compiler/expression.rs:509-600` | 29 |
| VM `OpCode::Call` handler（native + closure 分支, arity 校验） | `src/vm/mod.rs:720-790` | 25/27 |

## 实现细节

### 1. Function 结构体扩展

`src/vm/object.rs:490-496`，新增三个字段：

```rust
pub struct Function {
    pub name: String,
    pub arity: usize,               // 固定参数总数（普通 + 默认）
    pub code: Vec<u8>,
    pub constants: Vec<Object>,
    pub upvalue_count: usize,
    pub source_file: Option<String>,
    // --- 新增 ---
    pub default_values: Vec<Object>, // 编译期求值的默认值（每个默认参数一个，按序）
    pub has_variadic: bool,          // 是否有 *rest 参数
    pub required_arity: usize,       // 必需参数数量（普通参数，不含默认和可变）
}
```

- `default_values`：长度 = 默认参数数量。每个元素为编译期求值的常量值。
- `required_arity`：普通参数数量（无默认值、非可变）。
- `arity`：固定参数总数 = 普通参数 + 默认参数（不含可变）。**不含 slot-0 `<self>`**。
- 所有现有 Function 构造点（`compile_fn_decl` statement.rs:250、`compile_fn_literal` expression.rs 对应处、`Function::new` object.rs:500）需补齐新字段。

### 2. 编译期默认值求值

`04-functions.md:44`：默认值在定义时求值一次。MVP 策略：**仅允许常量字面量**（Int/Float/String/Bool/Nil）。

```rust
/// 编译期求值默认参数表达式。仅支持常量字面量。
fn eval_default(expr: &Expr) -> Result<Object, String> {
    match expr {
        Expr::Literal(Literal::Int(n)) => Ok(Object::Int(*n)),
        Expr::Literal(Literal::Float(n)) => Ok(Object::Float(*n)),
        Expr::Literal(Literal::String(s)) => Ok(alloc_string(s)),
        Expr::Literal(Literal::Bool(b)) => Ok(Object::Bool(*b)),
        Expr::Literal(Literal::Nil) => Ok(Object::Nil),
        _ => Err("default parameter value must be a constant literal \
                  (non-literal defaults not yet supported)".to_string()),
    }
}
```

> 非常量默认值（如 `items = []`）暂不支持——编译期报错。后续可编译为函数定义时执行的字节码。

### 3. compile_fn_decl / compile_fn_literal 参数处理

两个函数（statement.rs:196-280 + expression.rs:509-600）需同步修改。在 params 循环中（当前 statement.rs:217-223 仅 push Local），按 `param.is_variadic` 和 `param.default` 分类处理：

```rust
let mut required_arity = 0usize;
let mut default_values = Vec::new();
let mut has_variadic = false;

for param in params {
    if param.is_variadic {
        has_variadic = true;
    } else if param.default.is_some() {
        // 默认参数：编译期求值
        let val = Self::eval_default(param.default.as_ref().unwrap())?;
        default_values.push(val);
    } else {
        // 普通参数
        required_arity += 1;
    }
    // 所有参数都注册为 local（含 variadic 和 default）
    func_unit.locals.push(Local {
        name: param.name.clone(),
        depth: 0,
        is_captured: false,
    });
}
```

参数顺序校验（编译期）——在 params 循环前或后：

```rust
fn validate_param_order(params: &[Param]) -> Result<(), String> {
    let mut state = 0u8; // 0=normal, 1=default, 2=variadic
    for p in params {
        match (p.is_variadic, &p.default, state) {
            (false, None, _) => { if state > 0 { return Err("positional parameter after default/variadic".into()); } }
            (false, Some(_), _) => { if state > 1 { return Err("default parameter after variadic".into()); } state = 1; }
            (true, _, _) => { state = 2; }
            // *rest 不应有 default（解析器不会产出 is_variadic=true && default=Some）
        }
    }
    Ok(())
}
```

Function 构造时补齐新字段：

```rust
let function = Function {
    name: name.to_string(),
    arity: params.iter().filter(|p| !p.is_variadic).count(), // 固定参数（不含 variadic）
    code: func_unit.chunk.code,
    constants: func_unit.chunk.constants,
    upvalue_count: func_unit.upvalues.len(),
    source_file: self.source_file.clone(),
    default_values,     // ← 新增
    has_variadic,       // ← 新增
    required_arity,     // ← 新增
};
```

### 4. VM CALL handler 扩展

修改 `src/vm/mod.rs:758-781` 的 CLOSURE 分支。当前（行 770）为严格 `argc != arity`，改为：

```rust
Object::Ref(ptr)
    if unsafe { (**ptr).type_tag } == TypeTag::CLOSURE as u8 =>
{
    // 读出 arity / required_arity / has_variadic / default_values（不借用 self）
    let (arity, required_arity, has_variadic, func_ptr) = {
        debug_assert!(!ptr.is_null());
        let closure = unsafe { read_closure(*ptr) };
        let func = unsafe { read_function(closure.function) };
        let f = &func.function;
        (f.arity, f.required_arity, f.has_variadic, closure.function)
    };

    // 实参数量校验
    if has_variadic {
        if argc < required_arity {
            return Err(format!(
                "TypeError: expected at least {} arguments, got {}", required_arity, argc));
        }
    } else {
        if argc < required_arity || argc > arity {
            return Err(format!(
                "TypeError: expected {}-{} arguments, got {}", required_arity, arity, argc));
        }
    }

    if self.call_stack.len() >= MAX_CALL_DEPTH {
        return Err("RecursionError: stack overflow".to_string());
    }

    // 步骤 1：填充默认值（argc < arity 时）
    if argc < arity {
        let func = unsafe { &(*read_function(func_ptr)).function };
        let defaults_to_fill = arity - argc;
        let offset = argc - required_arity;
        for i in 0..defaults_to_fill {
            self.stack.push(func.default_values[offset + i].clone());
        }
    }

    // 步骤 2：处理可变参数
    if has_variadic {
        let fixed_end = callee_idx + 1 + arity;
        if self.stack.len() > fixed_end {
            // 多余实参收集为 list
            let varargs: Vec<Object> = self.stack.drain(fixed_end..).collect();
            self.push(alloc_list(varargs));
        } else {
            // 无多余实参 → 空 list
            self.push(alloc_list(Vec::new()));
        }
    }

    self.call_stack.push(CallFrame::new(*ptr, callee_idx));
}
```

关键顺序：**先填默认值，再收集可变参数**。因为默认值追加在固定参数之后（位置 argc..arity），而可变参数 drain 从 `fixed_end = callee_idx + 1 + arity` 开始。填完默认值后栈长恰好等于 `callee_idx + 1 + arity`，若无多余实参则 push 空 list。

### 5. 调用帧栈布局

对于 `fn example(a, b, c = 10, *rest)`：

```
调用 example(1, 2)：          调用 example(1, 2, 3, 4, 5)：
[closure] [1] [2] [10] [[]]   [closure] [1] [2] [3] [list(4,5)]
 ^         a   b   c   rest    ^         a   b   c   rest
 callee_idx                     callee_idx
```

### 6. GC 注意

`Function.default_values: Vec<Object>` 包含 GC-managed 引用（如 `alloc_string` 产生的 `Object::Ref`）。task 52 的 GC trace（`src/vm/gc.rs`）已遍历 `Function.constants`，需**同步扩展**以遍历 `Function.default_values`。否则默认值为堆对象时可能被误回收。

## 验证标准

1. 默认参数值在定义时求值，后续调用共享同一默认值
2. 省略默认参数时使用默认值，提供实参时覆盖
3. `*rest` 将多余实参收集为 list
4. 无多余实参时 `*rest` 为空 list `[]`
5. 参数顺序强制：普通 → 默认 → 可变（编译期报错）
6. 实参数量不足必需参数时报 `TypeError`
7. 实参数量超过固定参数且有可变参数时正确收集
8. 实参数量超过固定参数且无 variadic 时报 `TypeError`
9. 非常量默认值（如 `[]`）编译期报错

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
