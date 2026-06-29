# Dict/Set 推导式

## 所属阶段
Phase 4.2b - 控制流 + 高级语法

## 前置任务
33-list-comprehension, 22-object-system-collections

## 目标
实现字典推导式 `{key: val for x in iter}` 和集合推导式 `{expr for x in iter}`，包括可选过滤条件、多变量解构与嵌套 for 子句（语法见 `03-syntax.md` § 推导式）。

## 设计规格

参照 [07-advanced](../07-advanced.md) § dict 推导式 / set 推导式：

### Dict 推导式

```ms
squares_dict = {x: x*x for x in range(5)}
# {0: 0, 1: 1, 2: 4, 3: 9, 4: 16}
```

### Set 推导式

```ms
unique_lengths = {len(w) for w in ["a", "bb", "ccc", "bb"]}
# {1, 2, 3}
```

> **注**：字符串长度取内置 `len(w)`（task #25/#26 已支持字符串）；`.length()` 方法属 task #50（Phase 6），尚未实现，本任务不用（与 task #33 review-fix 同一约定）。

### 语法消歧

- `{key: val for ...}` → 字典推导式（冒号分隔键值）
- `{expr for ...}` → 集合推导式（无冒号）
- `{a, b, c}` → 集合字面量（无 `for` 关键字）
- `{k: v, ...}` → 字典字面量（无 `for` 关键字）

关键区分点：`{` 后第一个表达式之后是否跟随 `:` 和 `for`（dict 推导式），还是直接 `for`（set 推导式），还是 `,`/`}`（字面量）。

## 实现细节

### 1. 解析与 AST（已在 task #14/#09 实现，本任务不改动）

解析器与 AST 节点在 task #14「集合字面量与匿名函数解析」与 task #09「AST 表达式节点定义」中已完成，本任务**无需改动** `src/parser/` 与 `src/ast/`：

- `src/parser/expression.rs:727` `parse_dict_comprehension`、`:753` `parse_set_comprehension`：花括号消歧（`{k: v for ...}` / `{expr for ...}` / 字面量）、循环 `for` 子句（`while match For` 支持嵌套 `for_clause+`）、可选 `if` 过滤、多变量目标 `parse_for_targets`（`for k, v in ...`）。iterable 与 `if` 条件均用 `parse_or()`（排除三元 `if...else`，与 task #14 注一致）。
- `src/ast/node.rs:243-253`（**字段名以此为准**）：

```rust
Expr::DictComprehension {
    key_expr: Box<Expr>,
    value_expr: Box<Expr>,            // 注意：value_expr，非 val_expr
    for_clauses: Vec<ForClause>,      // 注意：for_clauses，非 clauses
    condition: Option<Box<Expr>>,     // 注意：condition，非 filter
}

Expr::SetComprehension {
    expr: Box<Expr>,
    for_clauses: Vec<ForClause>,
    condition: Option<Box<Expr>>,
}

// ForClause { targets: Vec<String>, iterable: Box<Expr> }（node.rs:72）
```

本任务仅在编译器侧为 `Expr::DictComprehension` / `Expr::SetComprehension` 实现 codegen（当前 `src/compiler/expression.rs:72-73` 返回 not-yet-implemented），并新增两条辅助 opcode。

### 2. 编译 Dict / Set 推导式

`src/compiler/expression.rs`：为 `compile_expression` 的 `Expr::DictComprehension { key_expr, value_expr, for_clauses, condition }` 与 `Expr::SetComprehension { expr, for_clauses, condition }` 分支新增 `compile_dict_comprehension` / `compile_set_comprehension`。

**三条必须遵守的不变量**（与 task #33 §2 完全一致）：

1. **slot 方案**（task #32）：迭代器与结果容器（dict/set）均存局部 slot，**不**压栈顶；`FOR_ITER` 操作数为 `iter_slot(1) + exit_offset(2)` 共 3 字节（`emit_for_iter`，见 `src/compiler/mod.rs:198`、`compile_for_in` `statement.rs:443-560`）。**不要**使用「迭代器压栈 + `FOR_ITER offset` + 结尾 `POP` 弹迭代器」旧方案。
2. **隐式作用域**（`03-syntax.md:528`）：推导式必须创建隐式作用域，循环变量不泄漏。整个 codegen 包裹 `begin_scope()` … `end_scope()`（`src/compiler/mod.rs:243-268`）。`end_scope` 按 `is_captured` 自动发 `CLOSE_UPVALUE` 或 `POP` 清理本作用域所有 local（iter slot / 循环变量 slot / 容器 slot）。
3. **结果留栈顶**：推导式是表达式，`end_scope` 清理前需 `LOAD_LOCAL container_slot` 把结果容器复制到栈顶作为返回值。

**容器初始化差异**：dict 推导式 `emit BUILD_DICT 0`；set 推导式 `emit BUILD_SET 0`；随后 `declare_local("__comp_container")`，`container_slot = resolve_local(...)`。

伪代码（dict 与 set 共用同一循环骨架，仅「最内层插入」与 opcode 不同；含多变量与嵌套）：

```
compile_dict_or_set_comprehension(container_kind, for_clauses, condition, payload):
    // container_kind: Dict → BUILD_DICT；Set → BUILD_SET
    // payload: dict → (key_expr, value_expr)；set → (expr,)
    begin_scope()

    emit BUILD_CONTAINER 0          // BUILD_DICT 0（dict）或 BUILD_SET 0（set）
    declare_local("__comp_container"); container_slot = resolve_local("__comp_container")

    // ★ 关键：所有子句的 iter/target slot 在进入任何循环之前一次性预留（同 task #33）。
    // 若延迟到各 compile_clause 内声明，嵌套子句的 Nil 占位会随外层每次迭代重入而
    // 反复 push，导致栈泄漏并破坏 end_scope 的 POP 清理；多变量 UNPACK 展开亦要求
    // 目标 slot 已位于栈低部，否则展开元素与目标位置重叠互相覆盖。
    clauses = []
    for (i, clause) in for_clauses.enumerate():
        emit Nil; declare_local("__comp_iter_<i>"); iter = resolve_local(...)
        targets = []
        for t in clause.targets:
            emit Nil; declare_local(t); targets.push(resolve_local(t))
        clauses.push({ iterable: clause.iterable, iter, targets })

    compile_clause(0, container_slot, clauses, payload)   // 递归各 for 子句，最内层插入

    emit LOAD_LOCAL container_slot               // 结果副本留栈顶
    end_scope()
```

每层 for 子句（结构同 task #33 的 `compile_comp_clause`，所有 slot 已由调用方预留）：

```
compile_clause(i, container_slot, clauses, payload):
    clause = clauses[i]
    compile_expression(clause.iterable); emit ITERATOR; emit STORE_LOCAL clause.iter

    loop_start = current_offset()
    exit = emit_for_iter(clause.iter)            // FOR_ITER iter_slot exit_offset(2)

    // 循环目标（task #30 定约：UNPACK 在 VM 内逆序压栈，编译器正序 STORE）
    if clause.targets.len() == 1:
        emit STORE_LOCAL clause.targets[0]
    else:
        emit UNPACK clause.targets.len()
        for slot in clause.targets:              // 正序
            emit STORE_LOCAL slot

    if i < clauses.len() - 1:
        compile_clause(i+1, container_slot, clauses, payload)   // 嵌套下一层
    else:
        // 最内层：可选过滤 + 插入
        if let Some(cond) = condition:
            compile_expression(cond)
            skip = emit_jump(JUMP_IF_FALSE)      // JumpIfFalse 仅 peek 不弹 cond
            emit POP                              // 真支：弹出 cond
            do_insert_or_add(container_slot, payload)
            end_jump = emit_jump(JUMP)            // ★ 越过假支清理 POP（不可省略：
                                                  //   否则真支跌入假支 POP 弹错值）
            patch_jump(skip)                      // 假支落地
            emit POP                              // 假支：弹出 cond
            patch_jump(end_jump)                  // 真假两支汇合
        else:
            do_insert_or_add(container_slot, payload)

    back = emit_jump(JUMP_BACK)
    patch_jump_back(back, loop_start)
    patch_jump(exit)                              // FOR_ITER 耗尽落地处
```

`do_insert_or_add(container_slot, payload)` —— 最内层插入（opcode 定义见 §3）：

```
// dict 推导式：
compile_expression(key_expr)          // 压 key
compile_expression(value_expr)        // 压 value（在 key 之上）
emit DICT_INSERT container_slot       // 弹 val 再弹 key，插入 container_slot 处的 dict

// set 推导式：
compile_expression(expr)              // 压 element
emit SET_ADD container_slot           // 弹 element，加入 container_slot 处的 set
```

> **去重 / 覆盖语义**（`02-types.md:191`、`src/vm/object.rs:98`）：dict 重复键 last-wins（`MsDict::insert` 经 HashMap 自动覆盖，保持插入顺序）；set 去重由 `HashSet` 自动。`DICT_INSERT`/`SET_ADD` 无需特殊处理即可满足验证 #2「集合正确去重」。

### 3. SET_ADD / DICT_INSERT 辅助指令

为避免依赖未实现的 OOP 基础设施（属性访问 task #41/#43、容器方法 task #50/#51，均属 Phase 5/6），并与 task #33 的 `LIST_APPEND slot` 对称，本任务采用专用 `SET_ADD slot` / `DICT_INSERT slot` 指令作为插入机制。

**指令定义**（须同步修订两处：① `src/compiler/opcode.rs` 构造器组新增 `SetAdd` / `DictInsert` 变体，并加入 `operand_size()`「1 字节操作数」分组（与 `ListAppend` 并列，见 `opcode.rs:156-161`）；② `11-bytecode-vm.md` § 构造器表追加两行）：

| OpCode | 操作数 | 说明 |
|---|---|---|
| `SET_ADD` | `slot(1)` | 弹出栈顶值，加入 `slot` 处的 set 局部变量 |
| `DICT_INSERT` | `slot(1)` | 先弹 val、再弹 key（共两个值），插入 `slot` 处的 dict 局部变量 |

> **弹出顺序**（`DICT_INSERT`）：因 `key_expr` 先编译压栈、`value_expr` 后编译压栈，栈顶为 val，故 `DICT_INSERT` 先弹 val 再弹 key。

**VM 执行契约**（`src/vm/mod.rs` 的 `OpCode::SetAdd` / `OpCode::DictInsert` 分支，镜像 `LIST_APPEND` handler `vm/mod.rs:672-699`）：

1. 读取 1 字节 `slot`。
2. 定位 `location = stack_base + slot`（`stack_base` 取当前调用帧）；越界 → `RuntimeError`。
3. 从 `stack[location]` 读取容器；类型守卫：`SET_ADD` 须为 `TypeTag::SET`、`DICT_INSERT` 须为 `TypeTag::DICT`；不符 → `TypeError`。
4. 弹出操作数（`SET_ADD` 弹 1 个；`DICT_INSERT` 先弹 val 再弹 key），原地 `insert`（set: `read_set(ptr).insert(v)`；dict: `read_dict(ptr).insert(k, v)`）。
5. **不**向栈顶 push 任何值（结果容器由 §2 末尾 `LOAD_LOCAL container_slot` 显式取出）。

> **不可哈希值**（`02-types.md:341-352`）：若 `key` / `element` 为 list/dict/set/NaN，须抛可被 `try/except` 捕获的 `TypeError`。`Object::hash`（`object.rs:1830`）在不可哈希类型上 `panic`，故 handler 须用 `std::panic::catch_unwind` 包裹 `insert` 并转为 `Err`，**不可**让 panic 终止 VM 进程。（注：现有 `BUILD_DICT`/`BUILD_SET` handler 亦有此 panic 基线问题；本任务新 handler 取更安全的 catch_unwind 路径。）

**操作数大小**：`operand_size() = 1`（归入 `opcode.rs` 的「1 字节操作数」分组，与 `BUILD_LIST` 等 count 类指令一致）。

> **GC 前瞻（task #52）**：原地修改 dict/set 内部结构；插入的值若为 `Object::Ref`，task #52 已为 `TypeTag::DICT`/`SET` 注册 trace，无需额外改动。

## 验证标准

1. 字典推导式正确生成键值对
2. 集合推导式正确去重
3. 带过滤条件的推导式正确过滤
4. 推导式与字面量语法正确消歧
5. 推导式结果类型正确（dict/set）
6. 推导式内循环变量不泄漏到外部作用域（隐式作用域，`03-syntax.md:528`）
7. 多变量 `for k, v in ...` 与嵌套 `for ... for ...` 正确编译

## 测试用例

```ms
// test_dict_set_comprehension.ms — Dict/Set 推导式

// Dict 推导式
squares_dict = {x: x*x for x in range(5)}
print(squares_dict)

// Dict 推导式带过滤
even_squares = {x: x*x for x in range(10) if x % 2 == 0}
print(even_squares)

// Set 推导式
unique_lengths = {len(w) for w in ["a", "bb", "ccc", "bb"]}
print(unique_lengths)

// Set 推导式带过滤
big_numbers = {x for x in [1, 5, 3, 8, 2, 9, 4] if x > 3}
print(big_numbers)

// 消歧测试：普通字典字面量
d = {"a": 1, "b": 2}
print(d)

// 消歧测试：普通集合字面量
s = {1, 2, 3}
print(s)
```

预期输出：

```
{0: 0, 1: 1, 2: 4, 3: 9, 4: 16}
{0: 0, 2: 4, 4: 16, 6: 36, 8: 64}
{1, 2, 3}
{4, 5, 8, 9}
{a: 1, b: 2}
{1, 2, 3}
```

> **Display 注**：第 5 行 `{a: 1, b: 2}` 字符串键不带引号——mslang 既有容器 `Display`（`object.rs:1695-1702`）打印容器内字符串时不加引号（task #33 已确认同一行为）。若后续实现 `repr` 式带引号输出，需与此处及 task #33 同步更新。
