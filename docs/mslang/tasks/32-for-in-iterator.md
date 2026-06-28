# for..in 循环与迭代器协议

## 所属阶段
Phase 4.1 - 控制流 + 高级语法

## 前置任务
- 13-parser-statements（for..in 语句解析已实现）
- 19-compile-statements（compile_for_in / compile_break / compile_continue 已实现）
- 25-builtins-basic（range() 内置函数）
- 26-builtins-iterators（ITERATOR / FOR_ITER VM handler + IteratorState + to_iterator）

## 目标
修复 for..in 循环的编译期 slot 冲突 bug，使其端到端工作；回填 VM 集合构造操作码（BuildList/BuildDict/BuildSet），使集合字面量可作为 for..in 迭代对象。

## 设计规格

参照 [05-control-flow](../05-control-flow.md) § for..in / break / continue、[11-bytecode-vm](../11-bytecode-vm.md) § 迭代 / 构造器。

## 已完成（勿重复实现）

| 能力 | 实现位置 | 实现 task |
|---|---|---|
| for..in 解析（单变量 + 双变量） | `src/parser/statement.rs` | 13 |
| AST `Stmt::ForIn { variable, second_variable, iterable, body }` | `src/ast/node.rs:112-117` | 10 |
| `compile_for_in`（ITERATOR → FOR_ITER → StoreLocal/UNPACK → body → JumpBack → Pop） | `src/compiler/statement.rs:443-509` | 19 |
| `compile_break` / `compile_continue`（LoopContext 栈 + Break/Continue/JumpBack opcode） | `src/compiler/statement.rs:511-531` | 19 |
| `OpCode::Iterator` / `ForIter` / `Break` / `Continue` / `JumpBack` | `src/compiler/opcode.rs:69-80` | 16 |
| VM `OpCode::Iterator` handler（`to_iterator` → `alloc_iterator`） | `src/vm/mod.rs:555-559` | 26 |
| VM `OpCode::ForIter` handler（迭代器常驻栈顶，next 值压入栈顶之上） | `src/vm/mod.rs:565-589` | 26 |
| VM `OpCode::Break` / `Continue` / `JumpBack` handler | `src/vm/mod.rs:487-546` | 24 |
| `IteratorState` 枚举（Range/ListIter/StringIter/DictKeys/Enumerate/Zip/Reversed） | `src/vm/object.rs:331-360` | 26 |
| `MsIterator` / `alloc_iterator` / `read_iterator` | `src/vm/object.rs:447-476` | 26 |
| `to_iterator()`（LIST/TUPLE/STRING/DICT/SET/ITERATOR → IteratorState） | `src/vm/builtins.rs:239-284` | 26 |
| `range()` 内置函数 → `alloc_iterator(IteratorState::Range)` | `src/vm/builtins.rs:640` | 25/26 |

## 实现细节

本 task 的实际工作仅三项：**修复 compile_for_in slot 冲突** + **修复 interpret 预分配 slot 0** + **回填 BuildList/BuildDict/BuildSet VM handler**。

### 1. 修复 compile_for_in slot 冲突（核心 bug）

**问题**：`compile_for_in`（statement.rs:443-509）先编译 iterable 表达式（压入栈），再声明循环变量（分配 slot）。StoreLocal 写入的 slot 与迭代器在栈上的位置重叠——StoreLocal 覆盖迭代器，导致 FOR_ITER 下次迭代读取到非迭代器值而崩溃。

**根因**：循环变量的栈 slot 在 iterable 之后分配，但 iterable 被压入与 slot 相同的栈位置。迭代器在 `stack[stack_base + slot]`，StoreLocal 覆盖之。

**修复**：在编译 iterable **之前**声明循环变量并发射 `Nil` 占位，预留栈 slot。这样 iterable/迭代器压入 slot 之上，StoreLocal 写入 slot 不影响迭代器。

修改 `src/compiler/statement.rs:443-509` `compile_for_in`：

```rust
fn compile_for_in(
    &mut self,
    variable: &str,
    second_variable: Option<&str>,
    iterable: &Expr,
    body: &[Stmt],
    line: usize,
) -> Result<(), String> {
    // ★ 先声明循环变量并预留栈 slot（Nil 占位），再编译 iterable
    //    确保迭代器压入 slot 之上，StoreLocal 不覆盖迭代器。
    if let Some(var2) = second_variable {
        self.declare_local(variable, line)?;
        let slot1 = self.resolve_local(variable)
            .ok_or("internal: loop var not found after declare")?;
        self.declare_local(var2, line)?;
        let slot2 = self.resolve_local(var2)
            .ok_or("internal: loop var not found after declare")?;
        self.emit_byte(OpCode::Nil as u8, line);  // reserve slot1
        self.emit_byte(OpCode::Nil as u8, line);  // reserve slot2

        self.compile_expression(iterable, line)?;
        self.emit_byte(OpCode::Iterator as u8, line);
        let loop_start = self.current_offset();
        let for_iter_exit = self.emit_jump(OpCode::ForIter, line);
        self.emit_byte(OpCode::Unpack as u8, line);
        self.emit_byte(2, line);
        self.emit_byte(OpCode::StoreLocal as u8, line);
        self.emit_byte(slot1 as u8, line);
        self.emit_byte(OpCode::StoreLocal as u8, line);
        self.emit_byte(slot2 as u8, line);
    } else {
        self.declare_local(variable, line)?;
        let slot = self.resolve_local(variable)
            .ok_or("internal: loop var not found after declare")?;
        self.emit_byte(OpCode::Nil as u8, line);  // reserve slot

        self.compile_expression(iterable, line)?;
        self.emit_byte(OpCode::Iterator as u8, line);
        let loop_start = self.current_offset();
        let for_iter_exit = self.emit_jump(OpCode::ForIter, line);
        self.emit_byte(OpCode::StoreLocal as u8, line);
        self.emit_byte(slot as u8, line);
    }

    self.current_loop.push(super::LoopContext {
        loop_start,
        break_jumps: Vec::new(),
    });
    for stmt in body {
        self.compile_statement(stmt, line)?;
    }
    let loop_ctx = self.current_loop.pop()
        .ok_or("internal: loop context stack underflow")?;
    let back_edge = self.emit_jump(OpCode::JumpBack, line);
    self.patch_jump_back(back_edge, loop_start)?;
    self.patch_jump(for_iter_exit)?;
    for jump in &loop_ctx.break_jumps {
        self.patch_jump(*jump)?;
    }
    self.emit_byte(OpCode::Pop as u8, line);
    Ok(())
}
```

修复后栈布局（单变量 `for item in range(3)`，顶层）：
```
[Nil]      ← slot 0 (<self>，interpret 预分配 — 见 §2）
[Nil]      ← slot 1 (item，预预留)     ← NEW
[iterator] ← slot 2+（iterable 编译后 ITERATOR 压入）
[element]  ← FOR_ITER 压入栈顶
```
StoreLocal 1 弹出 element → 写入 stack[1]，迭代器 stack[2] 不受影响。FOR_ITER 下次读取 stack[2] = 迭代器 ✓

### 2. 修复 interpret 预分配 slot 0

`src/vm/mod.rs:52-69`：push CallFrame 前未预压 slot 0（`<self>` 占位）。导致顶层表达式的栈位偏移。

```rust
pub fn interpret(&mut self, chunk: Chunk) -> Result<Object, String> {
    // ... 创建 Function + closure ...
    self.stack.push(Object::Nil);  // ★ 预分配 slot 0（<self> 占位）
    self.call_stack.push(CallFrame::new(closure_ptr, 0));
    self.run()
}
```

### 3. 回填 BuildList / BuildDict / BuildSet VM handler

`OpCode::BuildList`/`BuildDict`/`BuildSet` 已定义（opcode.rs:86-88）且编译器已发射（`compile_list_literal`/`compile_dict_literal`/`compile_set_literal`），但 VM 无 handler（走 `_ => "unimplemented opcode"`）。for..in 测试用例 `for item in [1,2,3]` 依赖 `BuildList`。

在 VM opcode match 中新增（参照 `BuildTuple` handler 模式，mod.rs 已有）：

```rust
OpCode::BuildList => {
    let count = self.read_byte()? as usize;
    let start = self.stack.len()
        .checked_sub(count)
        .ok_or("stack underflow in BUILD_LIST")?;
    let items: Vec<Object> = self.stack.drain(start..).collect();
    self.push(alloc_list(items))?;
}
OpCode::BuildDict => {
    let pairs = self.read_byte()? as usize;  // 键值对数量
    let needed = pairs.checked_mul(2)
        .ok_or("BUILD_DICT count overflow")?;
    let start = self.stack.len()
        .checked_sub(needed)
        .ok_or("stack underflow in BUILD_DICT")?;
    let mut map = DictMap::new();
    let mut i = start;
    for _ in 0..pairs {
        let key = self.stack[i].clone();
        let value = self.stack[i + 1].clone();
        map.insert(key, value);
        i += 2;
    }
    self.stack.truncate(start);
    self.push(alloc_dict(map))?;
}
OpCode::BuildSet => {
    let count = self.read_byte()? as usize;
    let start = self.stack.len()
        .checked_sub(count)
        .ok_or("stack underflow in BUILD_SET")?;
    let items: Vec<Object> = self.stack.drain(start..).collect();
    self.push(alloc_set(items))?;
}
```

需确认 `alloc_list`/`alloc_dict`/`alloc_set` 已在 VM mod.rs 导入（`alloc_list`/`alloc_dict`/`alloc_set` 均在 object.rs 定义，mod.rs 已有部分导入）。

## 验证标准

1. `for item in range(3)` 端到端正确遍历（输出 0, 1, 2）—— **核心回归**
2. `for item in [1, 2, 3]` 端到端正确遍历（依赖 BuildList handler）
3. `for ch in "abc"` 字符串遍历
4. `for key in d` 字典键遍历（依赖 BuildDict handler）
5. `for key, value in d.items()` 双变量解包
6. break 正确跳出循环
7. continue 正确跳到下一次迭代
8. 循环变量在循环结束后保持最后值
9. 嵌套循环 break/continue 只影响最内层
10. 对非可迭代类型使用 for..in 抛出 TypeError
11. 回归：现有 449 项测试零回归

## 测试用例

```ms
// range 遍历
for i in range(3) {
    print(i)
}

// 列表遍历
for item in [1, 2, 3] {
    print(item)
}

// 字符串遍历
for ch in "abc" {
    print(ch)
}

// 循环变量保持最后值
for x in range(5) {}
print(x)

// break
for i in range(100) {
    if i == 3 {
        break
    }
}
print(i)

// continue
result = []
for i in range(5) {
    if i % 2 == 0 {
        continue
    }
    result.push(i)
}
print(result)

// 嵌套循环
count = 0
for i in range(3) {
    for j in range(3) {
        count += 1
    }
}
print(count)
```

预期输出：

```
0
1
2
1
2
3
a
b
c
4
3
[1, 3]
9
```
