# Dict/Set 推导式

## 所属阶段
Phase 4.2b - 控制流 + 高级语法

## 前置任务
33-list-comprehension

## 目标
实现字典推导式 `{key: val for x in iter}` 和集合推导式 `{expr for x in iter}`，包括可选过滤条件。

## 设计规格

参照 [07-advanced](../07-advanced.md) § dict 推导式 / set 推导式：

### Dict 推导式

```ms
squares_dict = {x: x*x for x in range(5)}
# {0: 0, 1: 1, 2: 4, 3: 9, 4: 16}
```

### Set 推导式

```ms
unique_lengths = {w.length() for w in ["a", "bb", "ccc", "bb"]}
# {1, 2, 3}
```

### 语法消歧

- `{key: val for ...}` → 字典推导式（冒号分隔键值）
- `{expr for ...}` → 集合推导式（无冒号）
- `{a, b, c}` → 集合字面量（无 `for` 关键字）
- `{k: v, ...}` → 字典字面量（无 `for` 关键字）

关键区分点：`{` 后第一个表达式之后是否跟随 `:` 和 `for`（dict 推导式），还是直接 `for`（set 推导式），还是 `,`/`}`（字面量）。

## 实现细节

### 1. 解析

`src/parser/expression.rs`：

在解析 `{` 后的内容时，按以下逻辑区分：

```
parse_brace_expr():
    consume '{'
    if peek == '}': return empty DictLiteral
    
    first = parse_expression()
    
    if peek == ':' && peek_ahead(1) != 'for'（第二个表达式后是 for）:
        → 可能为 dict 字面量或 dict 推导式
        consume ':'
        val_expr = parse_expression()
        if peek == 'for':
            → parse_dict_comprehension(first, val_expr)
        else:
            → parse_dict_literal(first, val_expr) 继续解析键值对
    
    elif peek == 'for':
        → parse_set_comprehension(first)
    
    elif peek == ',' || peek == '}':
        → parse_set_literal(first)
    
    elif peek == ':':
        → parse_dict_literal(first)
```

### 2. AST 节点

```rust
struct DictComprehension {
    key_expr: Box<Expr>,
    val_expr: Box<Expr>,
    clauses: Vec<ForClause>,
    filter: Option<Box<Expr>>,
}

struct SetComprehension {
    expr: Box<Expr>,
    clauses: Vec<ForClause>,
    filter: Option<Box<Expr>>,
}
```

### 3. 编译

#### Dict 推导式

```
编译 {key_expr: val_expr for x in iter if cond}：

1. emit BUILD_DICT 0          → 创建空字典
2. 编译 iter → ITERATOR
3. loop_start:
4. emit FOR_ITER end
5. STORE_LOCAL x
6. [可选] cond → JUMP_IF_FALSE skip
7. 编译 key_expr
8. 编译 val_expr
9. emit SET_INDEX (或 DICT_INSERT)
   栈: dict, key, val → dict[key] = val
10. skip:
11. emit JUMP_BACK loop_start
12. end: POP 迭代器
```

#### Set 推导式

```
编译 {expr for x in iter if cond}：

1. emit BUILD_SET 0           → 创建空集合
2. 编译 iter → ITERATOR
3. loop_start:
4. emit FOR_ITER end
5. STORE_LOCAL x
6. [可选] cond → JUMP_IF_FALSE skip
7. 编译 expr
8. emit SET_ADD               → 向集合添加元素
9. skip:
10. emit JUMP_BACK loop_start
11. end: POP 迭代器
```

### 4. SET_ADD / DICT_INSERT 辅助指令

`src/compiler/opcode.rs`：

| OpCode | 操作数 | 说明 |
|---|---|---|
| `SET_ADD` | `slot(1)` | 将栈顶元素添加到 slot 处的 set |
| `DICT_INSERT` | `slot(1)` | 将栈顶两个元素（key、val）插入 slot 处的 dict |

`slot` 指向构建中的 dict/set 在局部变量表中的位置。

## 验证标准

1. 字典推导式正确生成键值对
2. 集合推导式正确去重
3. 带过滤条件的推导式正确过滤
4. 推导式与字面量语法正确消歧
5. 推导式结果类型正确（dict/set）

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
unique_lengths = {w.length() for w in ["a", "bb", "ccc", "bb"]}
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
{5, 8, 9, 4}
{"a": 1, "b": 2}
{1, 2, 3}
```
