# 切片操作

## 所属阶段
Phase 4.3 - 控制流 + 高级语法

## 前置任务
18-compile-expressions, 22-object-system-collections

## 目标
实现下标与切片的**运行时**：索引访问 `seq[i]`（含负索引）、索引赋值 `seq[i] = v`、切片 `seq[start:stop:step]`（含负索引、默认值、越界裁剪、步长）。完成 `parse_slice` 解析桩与三个 VM handler（`GET_INDEX` / `SET_INDEX` / `GET_SLICE`），支持 list、string、tuple、dict（dict 仅索引，切片不支持）。

> **范围说明**：解析器下标分发与 `is_slice`（task #12）、`Expr::Slice` AST（task #09）、`compile_slice` / `compile_index` 编译（task #18）、三条 opcode 定义（task #16）**均已完成**。本 task 仅补齐：(1) `parse_slice` 解析桩体；(2) 三个 VM handler。当前三者皆落 `unimplemented opcode`（`src/vm/mod.rs` 分发末尾），故 `lst[i]` / `seq[1:3]` 现阶段运行期失败——本 task 修复之。

## 设计规格

参照 [07-advanced](../07-advanced.md) § 切片、[02-types](../02-types.md) § List/String/Tuple/Dict（下标与切片）、[03-syntax](../03-syntax.md) § 后缀表达式（index/slice，:462-477）、[11-bytecode-vm](../11-bytecode-vm.md) § 属性与下标：

### 语法

```
index = expression "[" expression "]"
slice = expression "[" slice_part? ":" slice_part? (":" slice_part)? "]"
slice_part = expression?
```

### 索引语义（GET_INDEX / SET_INDEX）

| 容器 | `seq[i]` 读 | `seq[i] = v` 写 |
|---|---|---|
| list | 负索引支持；越界抛 `IndexError` | 负索引支持；越界抛 `IndexError` |
| tuple | 负索引支持；越界抛 `IndexError` | `TypeError`（不可变） |
| string | `s[i]` 返回单字符 string；负索引；越界抛 `IndexError` | `TypeError`（不可变） |
| dict | `d[k]` 存在返回值，**不存在返回 `nil`**（不抛异常）；不可哈希键抛 `TypeError` | `d[k] = v` 设置/覆盖；不可哈希键抛 `TypeError` |

> 索引 `i` 必须为整数（list/tuple/string）或任意可哈希值（dict key），否则 `TypeError: indices must be integers` / `TypeError: unhashable type`。负索引 `-n` 等价于 `length - n`（list/tuple/string）。

### 切片参数规则

| 参数 | 默认值 | 含义 |
|---|---|---|
| `start` | `0`（step > 0 时）/ `length-1`（step < 0 时） | 起始索引（含） |
| `stop` | `length`（step > 0 时）/ `-1`（step < 0 时） | 结束索引（不含） |
| `step` | `1` | 步长（不可为 0） |

### 字节码指令

参照 [11-bytecode-vm](../11-bytecode-vm.md) § 属性与下标：

| OpCode | 操作数 | 栈布局（底→顶） | 说明 |
|---|---|---|---|
| `GET_INDEX` | — | `[obj, key]` → `[result]` | `obj[key]` |
| `SET_INDEX` | — | `[obj, key, val]` → `[]` | `obj[key] = val` |
| `GET_SLICE` | `flags(1)` | `[obj, start?, stop?, step?]` → `[result]` | `obj[start:stop:step]` |

`GET_SLICE` 的 `flags` 位掩码：bit 0 = start 是否存在、bit 1 = stop 是否存在、bit 2 = step 是否存在。`obj` 固定在栈底，按 flags 依次压入存在的 start/stop/step；VM 按缺失参数填默认值。

### 切片语义

- 负索引 `-n` 等价于 `length - n`
- 越界索引自动裁剪到有效范围，不报错
- 切片总是返回**新对象**（不修改原对象）
- step 为负数时反向切片；step 为 0 抛 `ValueError`
- 切片索引须为整数，否则 `TypeError`

### 适用类型（切片）

| 类型 | 返回类型 |
|---|---|
| list | list |
| string | string |
| tuple | tuple |

dict / set 不可切片（`TypeError: '...' object is not sliceable`）。

## 实现细节

### 1. 解析切片（仅 `parse_slice` 桩体，其余已完成）

`is_slice`（`src/parser/expression.rs:568`，已完成）在 `[` 已消费后扫描至匹配 `]`，嵌套深度 0 处遇 `:` 即判定切片，否则为索引（由 task #12 的下标分发处理，构造 `Expr::Index`）。本 task 仅需实现 `parse_slice` 桩（`src/parser/expression.rs:839`，当前返回 `unimplemented_expr`）：

```
parse_slice(object):  // 调用前 '[' 已消费；is_slice() 已确认含顶层 ':'
    // start（可空）
    start = if peek == Colon { None } else { Some(parse_expression()) }
    expect(Colon)
    // stop（可空）
    stop = if peek == RightBracket || peek == Colon { None } else { Some(parse_expression()) }
    // step（可空）
    step = if peek == Colon {
        consume(Colon)
        if peek == RightBracket { None } else { Some(parse_expression()) }
    } else { None }
    expect(RightBracket)
    return Expr::Slice { object, start, stop, step }
```

> iterable 部分用 `parse_expression()`（切片分量是完整表达式，非推导式 iterable，无需 `parse_or` 限制）。多变量/嵌套不适用于切片。

### 2. AST 节点（已在 task #09 实现，本任务不改动）

`Expr::Slice { object: Box<Expr>, start: Option<Box<Expr>>, stop: Option<Box<Expr>>, step: Option<Box<Expr>> }`（`src/ast/node.rs:326`，含 Display）。索引访问节点 `Expr::Index` 同样已存在（task #09/#12）。本任务不改 `src/ast/`。

### 3. 编译（已在 task #18 实现，本任务不改动）

`compile_slice`（`src/compiler/expression.rs:442-467`）发射 `GET_SLICE flags`，flags 位域 bit0=start/bit1=stop/bit2=step，与下文 §4 VM 弹出顺序一致；索引读写编译为 `GET_INDEX`/`SET_INDEX`（`expression.rs:394`/`:331`）。本任务不改 `src/compiler/`。

### 4. VM handler：GET_INDEX / SET_INDEX / GET_SLICE

`src/vm/mod.rs`：新增三个分发分支。错误类型为 `Result<_, String>`（与现有 handler 一致），用 `self.pop()?` / `self.push(...)` / `self.read_byte()?`，索引经 `require_int` 校验。

#### GET_INDEX（obj[key] → result）

```
OpCode::GetIndex => {
    let key = self.pop()?;
    let obj = self.pop()?;
    self.push(get_item(obj, key)?)?;
}

fn get_item(obj: Object, key: Object) -> Result<Object, String> {
    match obj {
        Object::Ref(ptr) => match unsafe { (**ptr).type_tag } {
            LIST => {
                let i = normalize_index(require_int(&key)?, read_list(*ptr).len())?;  // 越界→IndexError
                Ok(read_list(*ptr)[i].clone())
            }
            TUPLE => {
                let i = normalize_index(require_int(&key)?, read_tuple(*ptr).len())?;
                Ok(read_tuple(*ptr)[i].clone())
            }
            STRING => {
                let chars: Vec<char> = read_str(*ptr).chars().collect();  // Unicode 按字符
                let i = normalize_index(require_int(&key)?, chars.len())?;
                Ok(alloc_string(&chars[i].to_string()))
            }
            DICT => {
                // d[k] 不存在返回 nil（02-types:181）；不可哈希 key 在 HashMap 哈希阶段 panic
                let got = std::panic::catch_unwind(AssertUnwindSafe(|| read_dict(*ptr).get(&key).cloned()));
                match got { Ok(v) => Ok(v.unwrap_or(Object::Nil)), Err(p) => Err(unhashable_message(p)) }
            }
            _ => Err(format!("TypeError: '{}' object is not subscriptable", obj.type_name())),
        },
        _ => Err(format!("TypeError: '{}' object is not subscriptable", obj.type_name())),
    }
}
```

#### SET_INDEX（obj[key] = val）

```
OpCode::SetIndex => {  // 栈：[obj, key, val]
    let val = self.pop()?;
    let key = self.pop()?;
    let obj = self.pop()?;
    set_item(obj, key, val)?;   // 不压栈
}

fn set_item(obj: Object, key: Object, val: Object) -> Result<(), String> {
    match obj {
        Object::Ref(ptr) => match unsafe { (**ptr).type_tag } {
            LIST => {
                let i = normalize_index(require_int(&key)?, read_list(*ptr).len())?;
                read_list(*ptr)[i] = val; Ok(())
            }
            DICT => {
                // 不可哈希 key 在哈希阶段 panic；catch_unwind 转 TypeError
                let r = std::panic::catch_unwind(AssertUnwindSafe(|| { read_dict(*ptr).insert(key, val) }));
                match r { Ok(_) => Ok(()), Err(p) => Err(unhashable_message(p)) }
            }
            STRING | TUPLE => Err(format!("TypeError: '{}' object does not support item assignment", obj.type_name())),
            _ => Err(format!("TypeError: '{}' object does not support item assignment", obj.type_name())),
        },
        _ => Err(format!("TypeError: '{}' object does not support item assignment", obj.type_name())),
    }
}
```

#### GET_SLICE（obj[start:stop:step] → result）

```
OpCode::GetSlice => {
    let flags = self.read_byte()?;
    let step = if flags & 0b100 != 0 { Some(require_int(&self.pop()?)?) } else { None };
    let stop = if flags & 0b010 != 0 { Some(require_int(&self.pop()?)?) } else { None };
    let start = if flags & 0b001 != 0 { Some(require_int(&self.pop()?)?) } else { None };
    let obj = self.pop()?;
    self.push(slice_object(obj, start, stop, step.unwrap_or(1))?)?;
}
```

> 弹出顺序（LIFO）：编译端压栈顺序为 obj → start → stop → step（见 §3），故 VM 先弹 step、再 stop、再 start、最后 obj。flags 位域与编译端一致（bit0=start/bit1=stop/bit2=step）。

### 5. slice_object 实现

按容器类型分别用 `read_list` / `read_str` / `read_tuple` 取视图，统一调 `slice_bounds`（§6）算 `(start, stop, step)`，再按步长迭代收集下标，构造**同类型新对象**：

```
fn slice_object(obj: Object, start: Option<i64>, stop: Option<i64>, step: i64) -> Result<Object, String> {
    match obj {
        Object::Ref(ptr) => match unsafe { (**ptr).type_tag } {
            LIST => {
                let items = unsafe { read_list(*ptr) };
                let (s, e, st) = slice_bounds(items.len(), start, stop, step)?;
                let mut out = Vec::new();
                let mut i = s;
                while (st > 0 && i < e) || (st < 0 && i > e) { out.push(items[i as usize].clone()); i += st; }
                Ok(alloc_list(out))
            }
            STRING => {
                let chars: Vec<char> = unsafe { read_str(*ptr) }.chars().collect();   // Unicode 按字符
                let (s, e, st) = slice_bounds(chars.len(), start, stop, step)?;
                let mut out = String::new();
                let mut i = s;
                while (st > 0 && i < e) || (st < 0 && i > e) { out.push(chars[i as usize]); i += st; }
                Ok(alloc_string(&out))
            }
            TUPLE => {
                let items = unsafe { read_tuple(*ptr) };
                let (s, e, st) = slice_bounds(items.len(), start, stop, step)?;
                let mut out = Vec::new();
                let mut i = s;
                while (st > 0 && i < e) || (st < 0 && i > e) { out.push(items[i as usize].clone()); i += st; }
                Ok(alloc_tuple(out))
            }
            _ => Err(format!("TypeError: '{}' object is not sliceable", obj.type_name())),
        },
        _ => Err(format!("TypeError: '{}' object is not sliceable", obj.type_name())),
    }
}
```

> **安全下标**：`slice_bounds` 保证迭代变量 `i` 落在 `[0, len)`（越界已裁剪，stop 为排他边界不被访问），故 `i as usize` 安全。dict/set 落入 `_` 分支抛 `TypeError`。

### 6. 边界调整 slice_bounds（i64 全程，正确 Python 语义）

等价 CPython `PySlice_UnpackIndices` + `PySlice_AdjustIndices`。**全程用 `i64` 计算**，仅在 §5 取元素时 `as usize`（已确保非负且 `< len`），杜绝负数 `as usize` 回绕：

```
fn slice_bounds(len: usize, start: Option<i64>, stop: Option<i64>, step: i64) -> Result<(i64, i64, i64), String> {
    if step == 0 { return Err("ValueError: slice step cannot be zero".to_string()); }  // 非 panic
    let len = len as i64;
    let mut start = start.unwrap_or(if step > 0 { 0 } else { len - 1 });
    let mut stop  = stop.unwrap_or(if step > 0 { len } else { -1 });
    // 归一化 + 裁剪：负索引加 len；再裁到 [lo, hi]
    let adj = |idx: i64, lo: i64, hi: i64| -> i64 {
        let mut i = if idx < 0 { idx + len } else { idx };
        if i < lo { lo } else if i > hi { hi } else { i }
    };
    if step > 0 { start = adj(start, 0, len); stop = adj(stop, 0, len); }
    else        { start = adj(start, -1, len - 1); stop = adj(stop, -1, len - 1); }
    Ok((start, stop, step))
}

/// list/tuple/string 整数索引归一化：负索引加 len；越界抛 IndexError。
fn normalize_index(idx: i64, len: usize) -> Result<usize, String> {
    let len = len as i64;
    let i = if idx < 0 { idx + len } else { idx };
    if i < 0 || i >= len {
        return Err(format!("IndexError: index {} out of range for length {}", idx, len));
    }
    Ok(i as usize)
}

/// 取 Object 的整数值；非 int 抛 TypeError。
fn require_int(obj: &Object) -> Result<i64, String> {
    match obj { Object::Int(n) => Ok(*n), other => Err(format!("TypeError: indices must be integers, got '{}'", other.type_name())) }
}
```

> **`unhashable_message` 复用**：dict 下标读写对不可哈希 key 的处理复用 task #34 已有的 `unhashable_message(payload)` 辅助（`src/vm/mod.rs`），将 `Object::hash` 的 panic payload 转为 `TypeError: unhashable type: '...'` 字符串。

## 验证标准

1. 基本切片 `[a:b]` 正确
2. 省略参数 `[:b]`、`[a:]`、`[::]` 正确
3. 步长 `[::step]` 正确
4. 反向切片 `[::-1]`、`[8:2:-1]` 正确
5. 负索引正确转换为正索引
6. 越界索引被裁剪而不报错
7. list → list、string → string、tuple → tuple 类型正确
8. 切片不修改原对象
9. 索引访问 `seq[i]`（含负索引）正确
10. 索引访问越界抛 `IndexError`；不可变类型（string/tuple）索引赋值抛 `TypeError`
11. dict `d[k]` 读（不存在返回 nil）/ `d[k]=v` 写正确；不可哈希键抛 `TypeError`

## 测试用例

```ms
// test_slicing.ms — 下标与切片

lst = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9]
s = "hello world"
t = (0, 1, 2, 3, 4)
d = {"a": 1, "b": 2}

// 基本切片
print(lst[2:5])
print(lst[:3])
print(lst[7:])
print(lst[::2])
print(lst[::-1])
print(lst[-3:])

// 字符串切片
print(s[0:5])
print(s[-5:])

// tuple 切片
print(t[1:3])

// 越界裁剪
print(lst[0:100])
print(lst[100:200])

// 负索引组合与步长
print(lst[-5:-2])
print(lst[1::2])
print(lst[8:2:-1])

// 下标访问（GET_INDEX）
print(lst[0])
print(lst[-1])
print(s[0])
print(t[-1])
print(d["a"])
print(d["missing"])

// 下标赋值（SET_INDEX）
original = [1, 2, 3]
sliced = original[0:2]
original[0] = 99
print(sliced)
print(original)
```

预期输出：

```
[2, 3, 4]
[0, 1, 2]
[7, 8, 9]
[0, 2, 4, 6, 8]
[9, 8, 7, 6, 5, 4, 3, 2, 1, 0]
[7, 8, 9]
hello
world
(1, 2)
[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]
[]
[5, 6, 7]
[1, 3, 5, 7, 9]
[8, 7, 6, 5, 4, 3]
0
9
h
4
1
nil
[1, 2]
[99, 2, 3]
```

> **Display 注**：字符串容器内/顶层均按既有 Display（`object.rs`）无引号输出（`h`、`hello`、`world`），与 task #33/#34 同一行为。`d["missing"]` 返回 `nil` 并打印 `nil`（`02-types:181`：dict 访问不存在键返回 nil）。
