# 列表推导式

## 所属阶段
Phase 4.2a - 控制流 + 高级语法

## 前置任务
32-for-in-iterator

## 目标
实现列表推导式语法，支持基本形式 `[expr for x in iter]`、带过滤条件 `[expr for x in iter if cond]` 以及嵌套推导式 `[expr for x in iter1 for y in iter2]`。

## 设计规格

参照 [07-advanced](../07-advanced.md) § 列表推导式：

### 语法

> 与 [03-syntax](../03-syntax.md) § 推导式一致，循环目标支持多变量解构。

```
list_comp = "[" expression "for" IDENTIFIER ("," IDENTIFIER)? "in" expression ("if" expression)? "]"
```

嵌套形式：

```
nested_comp = "[" expression for_clause+ ("if" expression)? "]"
for_clause  = "for" IDENTIFIER ("," IDENTIFIER)? "in" expression
```

### 语义

- 列表推导式是语法糖，编译为等价的 BUILD_LIST + 循环
- `[expr for x in iter]` 等价于创建空列表，遍历 iter，每次将 expr 结果追加
- `[expr for x in iter if cond]` 中，仅当 cond 为 truthy 时才追加
- 嵌套推导式 `[expr for row in matrix for x in row]` 按从左到右顺序展开嵌套循环

## 实现细节

### 1. 解析与 AST（已在 task #14 实现，本任务不改动）

解析器与 AST 节点在 task #14「集合字面量与匿名函数解析」中已完成，本任务**无需改动** `src/parser/` 与 `src/ast/`：

- `src/parser/expression.rs:590-655`：`parse_list_literal`（`[` 后 peek `for` 即转入推导式）、`parse_list_comprehension`（循环 `for` 子句 + 可选 `if`）、`parse_for_targets`（支持 `for x, y in ...` 多变量）。
- iterable 与 `if` 条件均用 `parse_or()`（**非** `parse_expression()`），以排除三元 `expr if c else b`，避免 `[x for x in y if z]` 的 `if` 被误当三元消费。
- `src/ast/node.rs:72-75, 238-242`：既有节点形状如下（**字段名以此为准**）：

```rust
pub struct ForClause {
    pub targets: Vec<String>,        // 多变量：for x, y in ... → ["x","y"]
    pub iterable: Box<Expr>,
}

Expr::ListComprehension {
    expr: Box<Expr>,
    for_clauses: Vec<ForClause>,     // 注意：是 for_clauses，非 clauses
    condition: Option<Box<Expr>>,    // 注意：是 condition，非 filter
}
```

本任务仅在编译器侧为 `Expr::ListComprehension` 实现 codegen（当前 `src/compiler/expression.rs:67-72` 返回 `not yet implemented`）。

### 2. 编译列表推导式

`src/compiler/expression.rs`：为 `compile_expression` 的 `Expr::ListComprehension { expr, for_clauses, condition }` 分支（当前 `src/compiler/expression.rs:67-72` 返回 not-yet-implemented）新增 `compile_list_comprehension`。

**三条必须遵守的不变量**（本任务文档此前版本违反，现修正）：

1. **slot 方案**（task #32）：迭代器与结果 list 均存局部 slot，**不**压栈顶；`FOR_ITER` 操作数为 `iter_slot(1) + exit_offset(2)` 共 3 字节（见 `src/compiler/mod.rs:198` `emit_for_iter`、`src/compiler/statement.rs:443-560` `compile_for_in`）。**不要**使用旧的「迭代器压栈 + `FOR_ITER offset` + 结尾 `POP` 弹迭代器」方案。
2. **隐式作用域**（`03-syntax.md:528`）：推导式必须创建隐式作用域，循环变量不泄漏。整个 codegen 包裹 `begin_scope()` … `end_scope()`（`src/compiler/mod.rs:243-268`，注释已明确为「函数边界和推导式隐式作用域」预留）。`end_scope` 会按 `is_captured` 自动发 `CLOSE_UPVALUE` 或 `POP` 清理本作用域所有 local（iter slot / 循环变量 slot / list slot），满足验证 #4。
3. **结果留栈顶**：推导式是表达式，`end_scope` 清理前需 `LOAD_LOCAL list_slot` 把结果 list 复制到栈顶作为表达式返回值。

伪代码（单变量单层；多变量/多层见后续注释）：

```
compile_list_comprehension(expr, for_clauses, condition):
    begin_scope()

    emit BUILD_LIST 0                      // 栈顶即空 list
    declare_local("__comp_list")           // list_slot = 当前 local 数
    list_slot = resolve_local("__comp_list")

    compile_clause(0, list_slot)           // 递归各 for 子句，最内层追加

    emit LOAD_LOCAL list_slot              // 结果副本留栈顶
    end_scope()                            // 清理本作用域 local
```

每层 for 子句（结构同 `compile_for_in`，省略错误处理）：

```
compile_clause(i, list_slot):
    emit Nil; declare_local("__comp_iter_<i>"); iter_slot = resolve_local(...)
    compile_expression(for_clauses[i].iterable)
    emit ITERATOR
    emit STORE_LOCAL iter_slot

    loop_start = current_offset()
    exit = emit_for_iter(iter_slot)        // FOR_ITER iter_slot exit_offset(2)

    // 循环目标：参照 compile_for_in，每个 target 先 Nil 占位 + declare_local
    if targets.len() == 1:
        emit Nil; declare_local(targets[0]); slot = resolve_local(...)
        emit STORE_LOCAL slot
    else:
        各 target 预留 Nil + declare_local → target_slots
        emit UNPACK targets.len()
        逆序 emit STORE_LOCAL target_slots[k]

    if i < for_clauses.len() - 1:
        compile_clause(i+1, list_slot)     // 嵌套下一层
    else:
        // 最内层：可选过滤 + 追加
        if let Some(cond) = condition:
            compile_expression(cond)
            skip = emit_jump(JUMP_IF_FALSE)
            emit POP                       // 弹出 cond（真值分支）
            do_append(list_slot, expr)
            patch_jump(skip)
            emit POP                       // 跳过分支落地：弹出残留 cond
        else:
            do_append(list_slot, expr)

    back = emit_jump(JUMP_BACK)
    patch_jump_back(back, loop_start)
    patch_jump(exit)                       // FOR_ITER 耗尽落地处
```

`do_append(list_slot, expr)`——将 `expr` 结果追加到 list。**本任务实现方案 A**：

```
do_append(list_slot, expr):
    emit LOAD_LOCAL list_slot              // list（作为接收者）
    emit GET_ATTR "append"                 // 绑定方法
    compile_expression(expr)               // 参数 value
    emit CALL 1
    emit POP                               // append 返回 nil/旧长度，弹出
```

**多变量**：`for_clauses[i].targets.len() > 1` 时，迭代器每次须产出恰好 `targets.len()` 个元素，否则运行时抛 `ValueError`（与 `03-syntax.md:228` for..in 双变量语义一致）。

**多层嵌套**：`for_clauses` 按顺序对应嵌套循环——最外层先进入、最内层执行过滤与追加；每个子句的 `iter_slot`/`target_slot` 都在同一个 `begin_scope` 内声明，由 `end_scope` 统一清理。

### 3. 专用追加指令（可选优化，暂不实施）

可引入 `LIST_APPEND slot` 指令直接向 `slot` 处的列表追加栈顶值，避免每轮 `GET_ATTR "append"` + `CALL 1` 的方法查找开销（`11-bytecode-vm.md` 当前**未定义**此 opcode；若引入需同步修订 `11-bytecode-vm.md` 构造器表）。

**本任务采用 §2 方案 A（GET_ATTR + CALL）**，`LIST_APPEND` 延后至性能优化阶段，届时一并更新规格文档。

## 验证标准

1. 基本推导式产生正确列表
2. 带过滤条件的推导式正确过滤元素
3. 嵌套推导式正确展开
4. 推导式内循环变量不泄漏到外部作用域
5. 推导式内可引用外部变量

## 测试用例

```ms
// test_list_comprehension.ms — 列表推导式

// 基本推导式
squares = [x * x for x in range(10)]
print(squares)

// 带过滤条件
evens = [x for x in range(20) if x % 2 == 0]
print(evens)

// 过滤字符串（len() 内置函数已支持字符串，见 src/vm/builtins.rs:181；
//        字符串的 .length() 方法属 task #50，尚未实现，故此处不用）
names = ["Alice", "Bob", "Charlie", "David"]
long_names = [n for n in names if len(n) > 3]
print(long_names)

// 嵌套推导式（展平矩阵）
matrix = [[1, 2, 3], [4, 5, 6], [7, 8, 9]]
flat = [x for row in matrix for x in row]
print(flat)

// 引用外部变量
factor = 10
scaled = [x * factor for x in range(5)]
print(scaled)
```

预期输出：

```
[0, 1, 4, 9, 16, 25, 36, 49, 64, 81]
[0, 2, 4, 6, 8, 10, 12, 14, 16, 18]
["Alice", "Charlie", "David"]
[1, 2, 3, 4, 5, 6, 7, 8, 9]
[0, 10, 20, 30, 40]
```
