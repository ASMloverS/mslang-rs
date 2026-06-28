# 匿名函数

## 所属阶段
Phase 3.3 - 函数 + 闭包

## 前置任务
- 28-closures（闭包与上值机制）

## 目标
实现匿名函数（函数字面量）的解析、编译与运行，使匿名函数作为一等公民可赋值、传参、存储于集合。

## 设计规格

### 语法

参照 [04-functions](../04-functions.md) § 匿名函数：

```
fn_literal = "fn" "(" param_list? ")" block
```

匿名函数与具名函数的区别仅在于缺少函数名标识符。

### 语义

参照 [04-functions](../04-functions.md) § First-class 函数：

- 匿名函数是完全功能的闭包，可捕获上值
- 可以有任意复杂的函数体
- 可以赋值给变量、作为参数传递、作为返回值、存储在数据结构中

### 编译

- 匿名函数编译为 `Function` 对象，`name = "<anonymous>"`
- 上值捕获与具名函数完全一致（复用 Task 28 的上值机制）
- 运行时通过 `CLOSURE` 指令包装为 Closure

### 示例

参照 [04-functions](../04-functions.md)：

```ms
double = fn(x) { return x * 2 }

# 作为参数传递（经用户定义的高阶函数，非内置 list.map —— 后者属 task 51）
fn apply(f, x) { return f(x) }
print(apply(fn(x) { return x * x }, 4))
```

> **注**：`04-functions.md:140` 示例 `nums.map(fn(x){...})` 依赖 List 的 `.map()` 方法（[51-builtin-methods-list-dict-set](./51-builtin-methods-list-dict-set.md)，未实装）。本任务验证改用用户定义的高阶函数（`apply`）。

## 实现细节

### 1. AST 节点（task 14 已实现，本任务不修改）

`Expr::FnLiteral` 已由 [14-parser-collection-literals](./14-parser-collection-literals.md) 落地（`src/ast/node.rs:234-237`）：

```rust
pub enum Expr {
    // ...
    FnLiteral {
        params: Vec<Param>,   // Param = { name, default, is_variadic }（ast/node.rs:65-69）
        body: Vec<Stmt>,      // block 即语句向量（语言中无独立 Block 类型）
    },
    // ...
}
```

> **Param 结构**：`{ name: String, default: Option<Expr>, is_variadic: bool }`。task 14 的解析器已接受默认参数（`fn(x, y=10)`）与可变参数（`fn(*rest)`）语法。其语义实装（arity 按必需参数计算、调用期默认值填充）由 [31-default-variadic-params](./31-default-variadic-params.md) 负责；本任务 `arity = params.len()`（全部参数计为必需），含默认/可变参数的匿名函数在 task 31 前须全实参传递。

### 2. 解析器（task 14 已实现，本任务不修改）

匿名函数解析已由 task 14 完成：

- `parse_fn_literal`（`src/parser/expression.rs:803-811`）：消费 `fn` → `(` param_list `)` block，构造 `Expr::FnLiteral`。
- `is_fn_literal`（`src/parser/expression.rs:558`）：判定 `fn` 后是否为 `(`（匿名函数）vs `IDENTIFIER`（具名声明）。
- 分派点（`src/parser/expression.rs:466`）：`TokenKind::Fn if self.is_fn_literal() => self.parse_fn_literal()`。
- 测试 `test_fn_literal`（`src/parser/expression.rs:1046`）已覆盖。

具名函数声明 `fn name(...){}` 是语句（`parse_statement`）；匿名函数 `fn(...){}` 是表达式（`parse_primary`）。区分点 `fn` 后是否跟随 `IDENTIFIER`——已在 task 12/14 正确处理，本任务无需改动。

### 3. 编译器扩展（src/compiler/expression.rs）

> **替换 stub**：当前 `compile_expression` 对 `Expr::FnLiteral` 返回错误（`src/compiler/expression.rs:66-68`，"fn literal compilation not yet implemented (task 29)"）。本任务将其替换为真实编译。

`compile_fn_literal` 是 [28-closures](./28-closures.md) `compile_fn_decl`（`src/compiler/statement.rs:193-277`）的**精简镜像**——除两点外完全一致：(1) `name = "<anonymous>"`；(2) **不发 `STORE_GLOBAL`**（匿名函数是表达式，闭包值留栈作为表达式结果，由外层赋值/传参/集合构造消费）。上值机制（parent 链接、is_captured 回填、CLOSURE 发射）与具名函数完全相同，确保匿名闭包能正确捕获外层变量。

```rust
/// 编译匿名函数字面量（task 29）。
/// 镜像 compile_fn_decl（statement.rs:193），差异：name=<anonymous>、不发 STORE_GLOBAL。
fn compile_fn_literal(
    &mut self,
    params: &[crate::ast::node::Param],
    body: &[Stmt],
    line: usize,
) -> Result<(), String> {
    let mut func_unit = CompilationUnit {
        chunk: super::Chunk::new(),
        // slot 0 预留给被调用者（closure 自身），与 CALL 的 stack_base=callee_idx 自洽。
        // 参数从 slot 1 起（与 compile_fn_decl 一致 — task 27 订正 A3/V1）。
        locals: vec![Local {
            name: "<self>".to_string(),
            depth: 0,
            is_captured: false,
        }],
        upvalues: Vec::new(),
        scope_depth: 0,
        parent: std::ptr::null(),
    };
    for param in params {
        func_unit.locals.push(Local {
            name: param.name.clone(),
            depth: 0,
            is_captured: false,
        });
    }

    // 换出父单元，编译函数体。parent 指向 saved_unit（裸指针，规避 self-referential
    // 借用冲突 — task 28 方案），使 resolve_upvalue_recursive 可攀爬外层。
    let saved_unit = std::mem::replace(&mut self.unit, func_unit);
    self.unit.parent = std::ptr::addr_of!(saved_unit);
    self.compile_block(body, line)?;
    self.emit_byte(OpCode::Nil as u8, line);    // 隐式 return nil
    self.emit_byte(OpCode::Return as u8, line);
    let func_unit = std::mem::replace(&mut self.unit, saved_unit);

    // 上值捕获回填（task 28）：is_local=true 的上值对应父单元局部变量，
    // 标记 is_captured 驱动 end_scope 发射 CLOSE_UPVALUE。
    let captured_locals: Vec<usize> = func_unit
        .upvalues
        .iter()
        .filter(|uv| uv.is_local)
        .map(|uv| uv.index)
        .collect();
    for idx in captured_locals {
        if idx < self.unit.locals.len() {
            self.unit.locals[idx].is_captured = true;
        }
    }

    // 存 Function 入常量池，发 CLOSURE(func_idx) + 逐上值操作数。
    let function = Function {
        name: "<anonymous>".to_string(),
        arity: params.len(),   // task 31 前全部计为必需（见 §1 Param 说明）
        code: func_unit.chunk.code,
        constants: func_unit.chunk.constants,
        upvalue_count: func_unit.upvalues.len(),
        source_file: self.source_file.clone(),
    };
    let func_idx = self.add_constant(alloc_function(function));
    let func_idx = u16::try_from(func_idx)
        .map_err(|_| "constant pool overflow: more than 65535 constants".to_string())?;

    self.emit_byte(OpCode::Closure as u8, line);
    self.emit_bytes(&func_idx.to_be_bytes(), line);
    for uv in &func_unit.upvalues {
        self.emit_byte(if uv.is_local { 1 } else { 0 }, line);
        let idx = u8::try_from(uv.index).map_err(|_| {
            format!("upvalue index {} exceeds 255 (function too large)", uv.index)
        })?;
        self.emit_byte(idx, line);
    }
    // 不发 STORE_GLOBAL —— 闭包值留栈，作为表达式结果供外层消费。
    Ok(())
}
```

> **DRY 提示**：`compile_fn_literal` 与 `compile_fn_decl` 共享 ~90% 逻辑（单元构造、parent 链接、体编译、is_captured 回填、CLOSURE 发射）。实现时可提取共享辅助 `compile_function_body(params, body, line) -> (Function, Vec<Upvalue>)`，两者仅在外层包装（具名 → STORE_GLOBAL；匿名 → 留栈）上分叉。是否提取由实现者权衡可读性。

### 4. 运行时

运行时无需新增逻辑。匿名函数经过 `CLOSURE` 指令包装后就是普通的 Closure 对象，与具名函数的调用方式完全一致。

### 5. 一等公民验证

匿名函数作为表达式，天然支持：
- **赋值**：`f = fn(x) { return x }` — 编译为 `CLOSURE + STORE_GLOBAL/LOCAL`
- **传参**：`apply(fn(x) { ... }, 1)` — 编译为 `CLOSURE(匿名) + CONSTANT(1) + CALL(2)`
- **返回**：`return fn() { ... }` — 编译为 `CLOSURE + RETURN`
- **集合存储**：`{"key": fn() { ... }}` — 编译为 `CLOSURE + BUILD_DICT`

## 验证标准

1. `Expr::FnLiteral` 经编译器生成可调用的 Closure（task 14 已验证解析；本任务验证编译+运行端到端）
2. 匿名函数编译为 `name = "<anonymous>"` 的 Function 对象
3. 匿名函数作为闭包能正确捕获外层变量（引用捕获，经 task 28 的 parent 链接 + CLOSURE 上值操作数）
4. 匿名函数可赋值给变量并通过变量名调用
5. 匿名函数可作为参数传递给其他函数（高阶函数）
6. 匿名函数可存储在 dict/list 等集合中并通过下标访问后调用
7. 匿名函数无显式 return 时返回 nil
8. `nonlocal` 声明在匿名函数内生效（经 task 28 赋值编译统一处理，对具名/匿名函数均生效——本任务自动继承，无需额外工作）
9. **分阶段限制**：含默认参数（`fn(x, y=10)`）或可变参数（`fn(*rest)`）的匿名函数，task 31 前须全实参传递（`arity = params.len()`）；task 31 实装默认值填充与 arity 按必需参数计算

## 测试用例

```ms
double = fn(x) { return x * 2 }
print(double(5))

apply = fn(f, x) {
    return f(x)
}
print(apply(fn(x) { return x * x }, 4))

ops = {"add": fn(a, b) { return a + b }, "mul": fn(a, b) { return a * b }}
print(ops["add"](3, 4))
print(ops["mul"](3, 4))

# 匿名闭包捕获外层变量（验证标准 3）
fn make_counter() {
    count = 0
    return fn() {
        nonlocal count
        count += 1
        return count
    }
}
counter = make_counter()
print(counter())
print(counter())
```

预期输出：

```
10
16
7
12
1
2
```
