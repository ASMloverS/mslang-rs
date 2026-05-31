# 多返回值与元组解包

## 所属阶段
Phase 3.4 - 函数 + 闭包

## 前置任务
- 27-call-frame（调用帧与函数调用）

## 目标
实现多返回值（元组构造）、元组解包赋值、多变量赋值，使函数能返回多个值并通过解包语法接收。

## 设计规格

### 多返回值

参照 [04-functions](../04-functions.md) § 多返回值（元组）：

```ms
fn divmod(a, b) {
    return a // b, a % b
}
```

多返回值本质是返回元组：`return expr1, expr2, expr3` 等价于 `return (expr1, expr2, expr3)`。

### 元组构造

参照 [11-bytecode-vm](../11-bytecode-vm.md) § 构造器：

| OpCode | 操作数 | 说明 |
|---|---|---|
| `BUILD_TUPLE` | `count(1)` | 从栈顶 count 个元素构建 tuple |

### 元组解包

参照 [11-bytecode-vm](../11-bytecode-vm.md) § 构造器：

| OpCode | 操作数 | 说明 |
|---|---|---|
| `UNPACK` | `count(1)` | 解包序列到栈，弹出序列对象，压入 count 个元素 |

### 元组解包赋值

```ms
a, b, c = expr
```

语义：求值 `expr` 得到可迭代对象，解包后分别赋值给 `a`、`b`、`c`。

### 交换

```ms
a, b = b, a
```

语义：先构建右侧元组 `(b, a)`，再解包赋值给左侧。

### 元组字面量

```ms
t = (1, 2, 3)
```

括号包围的逗号分隔表达式为元组字面量。

### 元组打印格式

元组的 `print` / `to_string` 表示为 `(elem1, elem2, ...)`。单元素元组为 `(elem,)`。

## 实现细节

### 1. Tuple 对象

Tuple 通过 `Object::Ref(*mut MsObjHeader)` + `TypeTag::TUPLE` 存储（参照 [20-object-system-basic](./20-object-system-basic.md) 及 [22-object-system-collections](./22-object-system-collections.md) 的 `alloc_tuple`/`read_tuple`）。需实现：

```rust
impl Object {
    pub fn format_tuple(elements: &[Object]) -> String {
        if elements.len() == 1 {
            return format!("({},)", elements[0]);
        }
        let items: Vec<String> = elements.iter().map(|e| e.to_string()).collect();
        format!("({})", items.join(", "))
    }
}
```

### 2. BUILD_TUPLE 指令

```rust
OpCode::BUILD_TUPLE => {
    let count = self.read_byte() as usize;
    let start = self.stack.len() - count;
    let elements: Vec<Object> = self.stack.drain(start..).collect();
    self.stack.push(alloc_tuple(elements));
}
```

### 3. UNPACK 指令

```rust
OpCode::UNPACK => {
    let count = self.read_byte() as usize;
    let iterable = self.stack.pop().unwrap();

    let elements: Vec<Object> = match &iterable {
        Object::Ref(ptr) => {
            let tag = unsafe { (*(*ptr)).type_tag };
            if tag == TypeTag::TUPLE as u8 {
                unsafe { read_tuple(*ptr) }.clone()
            } else if tag == TypeTag::LIST as u8 {
                unsafe { read_list(*ptr) }.clone()
            } else {
                return self.runtime_error("cannot unpack non-iterable")
            }
        }
        _ => return self.runtime_error("cannot unpack non-iterable"),
    };

    if elements.len() != count {
        return self.runtime_error(&format!(
            "wrong number of values to unpack: expected {}, got {}",
            count,
            elements.len()
        ));
    }

    for elem in elements {
        self.stack.push(elem);
    }
}
```

### 4. 编译 return 语句 — 多返回值

```rust
fn compile_return(&mut self, values: &[Expr]) {
    if values.is_empty() {
        self.emit(OpCode::NIL);
    } else if values.len() == 1 {
        self.compile_expr(&values[0]);
    } else {
        for value in values {
            self.compile_expr(value);
        }
        self.emit_with_operand(OpCode::BUILD_TUPLE, values.len() as u8);
    }
    self.emit(OpCode::RETURN);
}
```

解析器中需要识别 `return expr1, expr2, ...` 语法：`return` 后的逗号分隔表达式列表。

### 5. 编译解包赋值

```ms
a, b, c = expr
```

```rust
fn compile_unpack_assign(&mut self, targets: &[String], value: &Expr) {
    self.compile_expr(value);
    self.emit_with_operand(OpCode::UNPACK, targets.len() as u8);

    for target in targets.iter().rev() {
        match self.resolve_local(target) {
            Some(slot) => self.emit_with_operand(OpCode::STORE_LOCAL, slot as u8),
            None => {
                let idx = self.add_constant(Object::String(target.clone().into()));
                self.emit_with_operand(OpCode::STORE_GLOBAL, idx as u16);
            }
        }
    }
}
```

注意：`UNPACK` 将元素按原序压栈，因此赋值需要从栈顶开始，即按逆序（`rev()`）生成 `STORE` 指令。

### 6. 编译交换赋值

```ms
a, b = b, a
```

右侧 `b, a` 编译为：
1. `LOAD_LOCAL b` → `LOAD_LOCAL a` → `BUILD_TUPLE 2`

然后左侧 `a, b` 编译为：
1. `UNPACK 2` → `STORE_LOCAL a`（栈顶）→ `STORE_LOCAL b`

整体字节码：
```
LOAD_LOCAL b
LOAD_LOCAL a
BUILD_TUPLE 2
UNPACK 2
STORE_LOCAL a
STORE_LOCAL b
```

关键：右侧先求值为元组，再解包，保证交换语义正确。

### 7. 解析器扩展

赋值语句需要识别多目标模式：

```rust
fn parse_assignment(&mut self) -> Stmt {
    let expr = self.parse_expr()?;

    if self.match_token(TokenKind::Equal) {
        if let Expr::Var(name) = &expr {
            let value = self.parse_expr()?;
            return Stmt::Assign { target: name.clone(), value: Box::new(value) };
        }

        if let Expr::Tuple(targets) | Expr::List(targets) = &expr {
            let value = self.parse_expr()?;
            let names: Vec<String> = targets.iter().map(|t| {
                match t {
                    Expr::Var(name) => name.clone(),
                    _ => panic!("invalid assignment target"),
                }
            }).collect();
            return Stmt::UnpackAssign { targets: names, value: Box::new(value) };
        }
    }

    Stmt::Expr(expr)
}
```

右侧表达式中的逗号需要特殊处理：在赋值语句右侧，`a, b` 应解析为元组构造而非序列表达式。

### 8. 元组字面量解析

括号表达式 `(...)` 需要区分：
- 单表达式 `(expr)` — 分组，不是元组
- 逗号分隔 `(expr, expr, ...)` — 元组字面量
- 单元素后跟逗号 `(expr,)` — 单元素元组

```rust
fn parse_paren_expr(&mut self) -> Expr {
    self.expect(TokenKind::LeftParen)?;
    if self.check(TokenKind::RightParen) {
        self.expect(TokenKind::RightParen)?;
        return Expr::Tuple(vec![]);
    }

    let first = self.parse_expr()?;
    if !self.match_token(TokenKind::Comma) {
        self.expect(TokenKind::RightParen)?;
        return first;
    }

    let mut elements = vec![first];
    while !self.check(TokenKind::RightParen) {
        elements.push(self.parse_expr()?);
        if !self.match_token(TokenKind::Comma) {
            break;
        }
    }
    self.expect(TokenKind::RightParen)?;
    Expr::Tuple(elements)
}
```

## 验证标准

1. `return a, b, c` 正确返回元组
2. 元组可通过变量接收：`result = divmod(10, 3)` 得到 tuple
3. 元组可解包赋值：`q, r = divmod(10, 3)` 正确解包
4. 解包数量不匹配时抛出运行时错误
5. 交换 `a, b = b, a` 正确工作
6. 元组打印格式正确：`(1, 2, 3)`
7. 单元素元组打印为 `(1,)`
8. 可对 list 和 tuple 进行解包

## 测试用例

```ms
fn divmod(a, b) {
    return a // b, a % b
}

q, r = divmod(10, 3)
print(q)
print(r)

result = divmod(10, 3)
print(result)

a = 1
b = 2
a, b = b, a
print(a)
print(b)
```

预期输出：

```
3
1
(3, 1)
2
1
```
