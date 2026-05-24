# 切片操作

## 所属阶段
Phase 4.3 - 控制流 + 高级语法

## 前置任务
32-for-in-iterator

## 目标
实现切片操作 `seq[start:stop:step]`，支持负索引、默认值、越界裁剪，适用于 list、string、tuple。

## 设计规格

参照 [07-advanced](../07-advanced.md) § 切片：

### 语法

```
slice = expression "[" slice_part? ":" slice_part? (":" slice_part)? "]"
slice_part = expression | (空)
```

### 参数规则

| 参数 | 默认值 | 含义 |
|---|---|---|
| `start` | `0`（step > 0 时）/ `length-1`（step < 0 时） | 起始索引（含） |
| `stop` | `length`（step > 0 时）/ `-1`（step < 0 时） | 结束索引（不含） |
| `step` | `1` | 步长 |

### 字节码指令

参照 [11-bytecode-vm](../11-bytecode-vm.md) § 属性与下标：

| OpCode | 操作数 | 说明 |
|---|---|---|
| `GET_SLICE` | `flags(1)` | obj[start:stop:step] |

`flags` 位掩码：

| 位 | 含义 |
|---|---|
| bit 0 | start 是否存在 |
| bit 1 | stop 是否存在 |
| bit 2 | step 是否存在 |

栈布局：`obj` 固定在栈底，根据 flags 依次压入 start、stop、step（如存在）。VM 根据缺失参数填入默认值。

### 语义

- 负索引 `-n` 等价于 `length - n`
- 越界索引自动裁剪到有效范围，不报错
- 切片总是返回**新对象**
- step 为负数时反向切片

### 适用类型

| 类型 | 返回类型 |
|---|---|
| list | list |
| string | string |
| tuple | tuple |

## 实现细节

### 1. 解析切片

`src/parser/expression.rs`：

在解析 subscript（`expr[...]`）时区分索引访问和切片：

```
parse_subscript(obj):
    consume '['
    
    if peek == ':':
        → start 缺失
        start = None
    else:
        start = Some(parse_expression())
    
    if peek == ':':
        consume ':'
        → 这是切片
        
        if peek == ']' || peek == ':':
            stop = None
        else:
            stop = Some(parse_expression())
        
        if peek == ':':
            consume ':'
            if peek == ']':
                step = None
            else:
                step = Some(parse_expression())
        else:
            step = None
        
        consume ']'
        return Slice { obj, start, stop, step }
    else:
        → 这是索引访问
        consume ']'
        return Index { obj, index: start.unwrap() }
```

### 2. AST 节点

```rust
struct Slice {
    object: Box<Expr>,
    start: Option<Box<Expr>>,
    stop: Option<Box<Expr>>,
    step: Option<Box<Expr>>,
}
```

### 3. 编译切片

`src/compiler/expression.rs`：

```
编译 Slice:

1. 编译 object → 压栈
2. flags = 0
3. if start: 编译 start → 压栈, flags |= 0b001
4. if stop: 编译 stop → 压栈, flags |= 0b010
5. if step: 编译 step → 压栈, flags |= 0b100
6. emit GET_SLICE flags
```

### 4. GET_SLICE 指令实现

`src/vm/mod.rs`：

```rust
OpCode::GET_SLICE => {
    let flags = self.read_byte() as u8;
    
    let step = if flags & 0b100 != 0 {
        self.stack_pop().as_int()
    } else {
        1
    };
    
    let stop = if flags & 0b010 != 0 {
        Some(self.stack_pop().as_int())
    } else {
        None
    };
    
    let start = if flags & 0b001 != 0 {
        Some(self.stack_pop().as_int())
    } else {
        None
    };
    
    let obj = self.stack_pop();
    let result = slice_object(obj, start, stop, step)?;
    self.stack_push(result);
}
```

### 5. slice_object 实现

```rust
fn slice_object(obj: Object, start: Option<i64>, stop: Option<i64>, step: i64) -> Result<Object> {
    let len = obj.len();
    
    let (adjusted_start, adjusted_stop) = adjust_slice_bounds(len, start, stop, step);
    
    match &obj {
        Object::List(items) => {
            let sliced: Vec<Object> = items.iter()
                .enumerate()
                .filter(|(i, _)| *i >= adjusted_start && *i < adjusted_stop)
                .skip_while(|(i, _)| (*i - adjusted_start) % step.abs() as usize != 0)
                .map(|(_, v)| v.clone())
                .collect();
            Ok(Object::List(Gc::new(sliced)))
        }
        // String, Tuple 类似
        _ => Err(...)
    }
}
```

### 6. 边界调整

```rust
fn adjust_slice_bounds(len: usize, start: Option<i64>, stop: Option<i64>, step: i64) -> (usize, usize) {
    let len = len as i64;
    let (start, stop) = match step {
        s if s > 0 => {
            let s = start.unwrap_or(0).normalize(len);
            let e = stop.unwrap_or(len).normalize(len);
            (s.clamp(0, len), e.clamp(0, len))
        }
        s if s < 0 => {
            let s = start.unwrap_or(len - 1).normalize(len);
            let e = stop.unwrap_or(-1).normalize(len);
            (e.clamp(-1, len - 1), s.clamp(-1, len - 1))
        }
        _ => panic!("slice step cannot be zero"),
    };
    (start as usize, stop as usize)
}

fn normalize_index(idx: i64, len: i64) -> i64 {
    if idx < 0 { len + idx } else { idx }
}
```

## 验证标准

1. 基本切片 `[a:b]` 正确
2. 省略参数 `[:b]`、`[a:]`、`[::]` 正确
3. 步长 `[::step]` 正确
4. 反向切片 `[::-1]` 正确
5. 负索引正确转换为正索引
6. 越界索引被裁剪而不报错
7. list → list、string → string、tuple → tuple 类型正确
8. 原对象不被修改

## 测试用例

```ms
// test_slicing.ms — 切片操作

// 基本切片
lst = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9]
print(lst[2:5])
print(lst[:3])
print(lst[7:])
print(lst[::2])
print(lst[::-1])
print(lst[-3:])

// 字符串切片
s = "hello world"
print(s[0:5])
print(s[-5:])

// tuple 切片
t = (0, 1, 2, 3, 4)
print(t[1:3])

// 越界裁剪
print(lst[0:100])
print(lst[100:200])

// 负索引组合
print(lst[-5:-2])

// 带步长
print(lst[1::2])

// 切片不修改原对象
original = [1, 2, 3]
sliced = original[0:2]
original[0] = 99
print(sliced)
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
[1, 2]
```
