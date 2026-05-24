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

```
list_comp = "[" expression "for" IDENTIFIER "in" expression ("if" expression)? "]"
```

嵌套形式：

```
nested_comp = "[" expression for_clause+ ("if" expression)? "]"
for_clause  = "for" IDENTIFIER "in" expression
```

### 语义

- 列表推导式是语法糖，编译为等价的 BUILD_LIST + 循环
- `[expr for x in iter]` 等价于创建空列表，遍历 iter，每次将 expr 结果追加
- `[expr for x in iter if cond]` 中，仅当 cond 为 truthy 时才追加
- 嵌套推导式 `[expr for row in matrix for x in row]` 按从左到右顺序展开嵌套循环

## 实现细节

### 1. 解析列表推导式

`src/parser/expression.rs`：

当解析到 `[` 后，检查后续 token 序列中是否存在 `for` 关键字（在相同括号层级内）来区分普通列表字面量和列表推导式。

```
parse_list():
    consume '['
    if peek == ']': return empty ListLiteral
    
    first_expr = parse_expression()
    
    if peek == 'for':
        → parse_list_comprehension(first_expr)
    else:
        → parse_list_literal(first_expr)
```

推导式解析：

```
parse_list_comprehension(first_expr):
    expr = first_expr
    
    clauses = []
    while peek == 'for':
        consume 'for'
        var_name = consume_identifier()
        consume 'in'
        iterable = parse_expression()
        clauses.append(ForClause(var_name, iterable))
    
    filter = None
    if peek == 'if':
        consume 'if'
        filter = parse_expression()
    
    consume ']'
    return ListComprehension { expr, clauses, filter }
```

### 2. AST 节点

`src/ast/node.rs`：

```rust
struct ListComprehension {
    expr: Box<Expr>,
    clauses: Vec<ForClause>,
    filter: Option<Box<Expr>>,
}

struct ForClause {
    var_name: String,
    iterable: Box<Expr>,
}
```

### 3. 编译列表推导式

`src/compiler/expression.rs`：

编译为等价循环：

```
编译 [expr for x in iter if cond]：

1. emit BUILD_LIST 0          → 创建空列表
2. 编译 iter 表达式
3. emit ITERATOR               → 创建迭代器
4. loop_start:
5. emit FOR_ITER end_offset
6. STORE_LOCAL x              → 循环变量
7. [可选] 编译 cond
8. [可选] emit JUMP_IF_FALSE skip_append
9. [可选] emit POP             → 弹出 cond
10. 编译 expr
11. emit CALL 1 (list.append) 或使用专用指令
12. skip_append:
13. emit JUMP_BACK loop_start
14. end:
15. emit POP                   → 弹出迭代器
```

嵌套推导式编译为嵌套循环，每个 `for` 子句对应一层循环，最内层执行追加操作。

### 4. 专用追加指令（可选优化）

可引入 `LIST_APPEND slot` 指令，直接向 `slot` 处的列表追加栈顶值，避免方法查找开销。此优化可延后实施。

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

// 过滤字符串
names = ["Alice", "Bob", "Charlie", "David"]
long_names = [n for n in names if n.length() > 3]
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
