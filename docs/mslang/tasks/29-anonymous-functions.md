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
nums.map(fn(x) { return x * x })
```

## 实现细节

### 1. AST 节点扩展

在 AST 中区分具名函数和匿名函数。可以复用 `FnDecl` 节点，将 `name` 设为 `Option<String>`；或新建 `FnLiteral` 表达式节点：

```rust
pub enum Expr {
    // ...
    FnLiteral {
        params: Vec<String>,
        body: Box<Block>,
    },
}
```

选择方案：新增 `FnLiteral` 表达式节点，与 `FnDecl` 语句节点分离，语义更清晰。

### 2. 解析器扩展

参照 [04-functions](../04-functions.md) 的语法定义，在表达式解析中增加匿名函数分支：

```rust
fn parse_primary(&mut self) -> Expr {
    if self.match_token(TokenKind::Fn) {
        // 检查下一个 token 是否为 '('
        // 如果是 '(' 则为匿名函数
        // 如果是 IDENTIFIER 则为具名函数声明（不在此处理）
        self.expect(TokenKind::LeftParen)?;
        let params = self.parse_param_list()?;
        self.expect(TokenKind::RightParen)?;
        let body = self.parse_block()?;
        return Expr::FnLiteral { params, body: Box::new(body) };
    }
    // ... 其他 primary 表达式
}
```

注意：具名函数声明 `fn name(...) {}` 是语句，在 `parse_statement` 中处理；匿名函数 `fn(...) {}` 是表达式，在 `parse_primary` 中处理。区分点为 `fn` 后是否跟随 `IDENTIFIER`。

### 3. 编译器扩展

匿名函数编译与具名函数类似，区别在于：

```rust
fn compile_fn_literal(&mut self, params: &[String], body: &Block) {
    let mut func_unit = CompilationUnit::new();
    func_unit.name = "<anonymous>".to_string();
    func_unit.arity = params.len();

    for param in params {
        func_unit.locals.push(Local {
            name: param.clone(),
            depth: 0,
            is_captured: false,
        });
    }

    let saved_unit = std::mem::replace(&mut self.unit, func_unit);
    self.compile_block(body);
    self.emit(OpCode::NIL);
    self.emit(OpCode::RETURN);

    let func_unit = std::mem::replace(&mut self.unit, saved_unit);

    let function = Function {
        name: func_unit.name,
        arity: func_unit.arity,
        code: func_unit.code,
        constants: func_unit.constants,
        upvalue_count: func_unit.upvalues.len(),
    };
    let idx = self.add_constant(Object::Function(Gc::new(function)));

    self.emit_with_operand(OpCode::CLOSURE, idx as u16);

    for upvalue in &func_unit.upvalues {
        self.emit_byte(if upvalue.is_local { 1 } else { 0 });
        self.emit_byte(upvalue.index as u8);
    }
}
```

### 4. 运行时

运行时无需新增逻辑。匿名函数经过 `CLOSURE` 指令包装后就是普通的 Closure 对象，与具名函数的调用方式完全一致。

### 5. 一等公民验证

匿名函数作为表达式，天然支持：
- **赋值**：`f = fn(x) { return x }` — 编译为 `CLOSURE + STORE_GLOBAL/LOCAL`
- **传参**：`apply(fn(x) { ... }, 1)` — 编译为 `CLOSURE(匿名) + CONSTANT(1) + CALL(2)`
- **返回**：`return fn() { ... }` — 编译为 `CLOSURE + RETURN`
- **集合存储**：`{"key": fn() { ... }}` — 编译为 `CLOSURE + BUILD_DICT`

## 验证标准

1. 匿名函数能正确解析为 `FnLiteral` AST 节点
2. 匿名函数编译为 `name = "<anonymous>"` 的 Function 对象
3. 匿名函数作为闭包能正确捕获外层变量
4. 匿名函数可赋值给变量并通过变量名调用
5. 匿名函数可作为参数传递给其他函数
6. 匿名函数可存储在 dict/list 等集合中并通过下标访问后调用
7. 匿名函数无显式 return 时返回 nil

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
```

预期输出：

```
10
16
7
21
```
