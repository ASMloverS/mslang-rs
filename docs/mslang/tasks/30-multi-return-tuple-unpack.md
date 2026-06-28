# 多返回值与元组解包

## 所属阶段
Phase 3.4 - 函数 + 闭包

## 前置任务
- 18-compile-expressions（`compile_tuple_literal` 已发射 `BuildTuple`）
- 22-object-system-collections（`alloc_tuple`/`read_tuple`/Display 已实现）
- 26-builtins-iterators（UNPACK VM handler 已实现）
- 27-call-frame（调用帧与函数调用）
- 28-closures（`compile_return` 多返回值已实现）

## 目标
实现多返回值（元组构造）、元组解包赋值、多变量赋值，使函数能返回多个值并通过解包语法接收。

## 设计规格

参照 [04-functions](../04-functions.md) § 多返回值（元组）、[11-bytecode-vm](../11-bytecode-vm.md) § 构造器（`BUILD_TUPLE`/`UNPACK`）、[02-types](../02-types.md) § Tuple、[03-syntax](../03-syntax.md) § 多目标赋值语义。

## 已完成（勿重复实现）

以下均在先行 task 中完成，本 task **不涉及 parser、不涉及 `compile_return`、不涉及 UNPACK VM handler、不涉及 tuple Display**：

| 能力 | 实现位置 | 实现 task |
|---|---|---|
| 元组字面量 AST `Expr::TupleLiteral { elements }` | `src/ast/node.rs` | 09 |
| 多目标赋值 AST `Stmt::Assign { target: TupleLiteral, op, value: TupleLiteral }` | `src/ast/node.rs:91` | 09/10/13 |
| 解析器 `parse_grouping_or_tuple`（含 `()` / `(x,)` / `(x, y)` / 分组） | `src/parser/expression.rs` | 14 |
| 解析器 `return a, b, c` → `Stmt::Return { values: Vec<Expr> }` | `src/parser/statement.rs:215` | 13 |
| 解析器多目标赋值 `a, b = 1, 2` | `src/parser/statement.rs` | 13 |
| `compile_tuple_literal` 发射 `BuildTuple` + count | `src/compiler/expression.rs:480-487` | 18 |
| `compile_return` 多返回值（0→Nil / 1→expr / >1→BuildTuple+count） | `src/compiler/statement.rs:283-300` | 28 |
| VM `OpCode::Unpack` handler（tuple/list 解包 + count 校验 + ValueError + 逆序压栈） | `src/vm/mod.rs:591-627` | 26 |
| `alloc_tuple` / `read_tuple` / `MsTuple` 结构 | `src/vm/object.rs:159,267-289` | 22 |
| Tuple `Display`（含 `(1,)` 单元素格式） | `src/vm/object.rs:1702-1710` | 22 |

## 实现细节

本 task 的实际工作仅两项：**VM 新增 `BuildTuple` 处理分支** + **编译器 `compile_store_target` 新增 `TupleLiteral` 分支**。

### 1. VM 新增 `OpCode::BuildTuple` 处理分支

`OpCode::BuildTuple` 已在 `src/compiler/opcode.rs:87` 定义、编译端已发射（`compile_tuple_literal` expression.rs:483、`compile_return` statement.rs:294），但 VM opcode 分派中**无对应 `=>` 分支**（mod.rs:801-802 走 `_ => "unimplemented opcode"` 默认分支）。

在 VM 执行循环（`src/vm/mod.rs`）的 opcode `match` 中新增：

```rust
// BUILD_TUPLE count：从栈顶弹出 count 个元素，构建 tuple 对象并压栈。
OpCode::BuildTuple => {
    let count = self.read_byte()? as usize;
    let start = self.stack.len()
        .checked_sub(count)
        .ok_or("RuntimeError: stack underflow in BUILD_TUPLE")?;
    let elements: Vec<Object> = self.stack.drain(start..).collect();
    self.push(alloc_tuple(elements))?;
}
```

需导入 `alloc_tuple`（VM mod.rs 已有 `use ... alloc_tuple`，确认可用）。

### 2. `compile_store_target` 新增 `Expr::TupleLiteral` 分支

`src/compiler/expression.rs:285-326` 的 `compile_store_target` 仅处理 `Identifier`/`Index`/`Dot`。多目标赋值 `a, b = expr` 的 target 为 `Expr::TupleLiteral`，当前走 `_ => Err("Invalid assignment target")`。

新增分支（放在 `_ =>` 前）：

```rust
Expr::TupleLiteral { elements: targets } => {
    let count = u8::try_from(targets.len())
        .map_err(|_| format!(
            "too many unpack targets (max 255, got {})", targets.len()
        ))?;
    self.emit_byte(OpCode::Unpack as u8, line);
    self.emit_byte(count, line);
    // UNPACK（mod.rs:624）逆序压入 elements，使 elements[0] 位于栈顶。
    // 因此按正序迭代 targets：targets[0] 在栈顶，先 store。
    for target in targets {
        self.compile_store_target(target, line)?;
    }
}
```

关键：**正序迭代**（非 `rev()`）。UNPACK 在 VM 中已逆序压栈（`elements.into_iter().rev()`），使 `elements[0]` 位于栈顶。编译器按 `targets[0], targets[1], ...` 正序生成 STORE 指令，每次从栈顶弹出，恰好匹配。

赋值语句 `a, b = expr` 的完整字节码流（经 `compile_assign_stmt` → `compile_assignment`）：

```
<compile value>          # 求值右值 → 栈顶: tuple
DUP                      # 栈顶: tuple, tuple
UNPACK n                 # 弹出顶部 tuple → 逆序压入 n 个元素
STORE targets[0]         # 弹出 elements[0]（栈顶）→ targets[0]
STORE targets[1]         # 弹出 elements[1] → targets[1]
...
POP                      # compile_assign_stmt 丢弃 DUP 保留的表达式值
```

### 3. `compile_load_target` 拒绝 TupleLiteral

`src/compiler/expression.rs:275-282` 的 `compile_load_target`（复合赋值读取目标当前值）新增 `TupleLiteral` 分支，返回编译错误：

```rust
Expr::TupleLiteral { .. } => {
    Err("compound assignment cannot target a tuple".to_string())
}
```

多目标复合赋值（`a, b += expr`）在 `03-syntax.md` 语法中不合法（`target_list` 仅支持 `=`），解析器不会产出此形式，此分支为防御性校验。

## 验证标准

1. `return a, b, c` 正确返回元组（`compile_return` 已实现，需 VM `BuildTuple` handler 才能 runtime 通过）
2. 元组字面量 `(1, 2, 3)` 正确构造（`compile_tuple_literal` 已实现，需 VM `BuildTuple` handler）
3. 元组可通过变量接收：`result = divmod(10, 3)` 得到 tuple
4. 元组可解包赋值：`q, r = divmod(10, 3)` 正确解包（需 `compile_store_target` TupleLiteral 分支）
5. 解包数量不匹配时抛出运行时 `ValueError`（UNPACK handler 已实现）
6. 交换 `a, b = b, a` 正确工作
7. 元组打印格式正确：`(1, 2, 3)`（Display 已实现）
8. 单元素元组打印为 `(1,)`（Display 已实现）
9. 可对 list 和 tuple进行解包（UNPACK handler 已支持）

## 测试用例

```ms
fn divmod(a, b) {
    return a // b, a % b
}

q, r = divmod(10, 3)
print(q)

result = divmod(10, 3)
print(result)

a = 1
b = 2
a, b = b, a
print(a)
print(b)

# 元组字面量
t = (1, 2, 3)
print(t)
print(len(t))

# 单元素元组
single = (42,)
print(single)
```

预期输出：

```
3
(3, 1)
2
1
(1, 2, 3)
3
(42,)
```
