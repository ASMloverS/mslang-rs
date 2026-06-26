# 内置迭代器与容器函数

## 所属阶段
Phase 2 - 字节码编译 + VM 核心

## 前置任务
- 25-builtins-basic
- 22-object-system-collections

## 目标

实现内置迭代器类型和容器操作函数，使 `for..in` 循环能够遍历各种可迭代对象。

## 设计规格

引用 [10-builtins.md](../10-builtins.md) 容器函数，[05-control-flow.md](../05-control-flow.md) for..in 循环语义。

### 迭代器函数

| 函数 | 说明 |
|---|---|
| `range(end)` | 0 到 end-1 |
| `range(start, end)` | start 到 end-1 |
| `range(start, end, step)` | 带步长 |
| `sorted(iterable)` | 排序返回新列表 |
| `reversed(iterable)` | 反转返回迭代器 |
| `enumerate(iterable)` | 返回 (index, value) 对 |
| `zip(*iterables)` | 并行迭代 |
| `map(fn, iterable)` | 映射 |
| `filter(fn, iterable)` | 过滤 |
| `any(iterable)` | 任一为 truthy |
| `all(iterable)` | 全部为 truthy |

### 容器构造函数

| 函数 | 说明 |
|---|---|
| `list()` | 空列表 |
| `list(iterable)` | 从可迭代对象构造 |
| `tuple()` | 空元组 |
| `tuple(iterable)` | 从可迭代对象构造 |
| `set()` | 空集合 |
| `set(iterable)` | 从可迭代对象构造 |
| `dict()` | 空字典 |

### for..in 可迭代对象

引用 [05-control-flow.md](../05-control-flow.md)：list, tuple, dict, set, string, range, 生成器。

## 实现细节

### 文件位置

`src/vm/builtins.rs`（扩展 task 25；其中 range/list/tuple/set/dict 为**覆盖替换** task 25 已注册的同名实现）
`src/vm/object.rs`（添加 MsIterator 堆对象）
`src/vm/mod.rs`（添加 `ITERATOR`/`FOR_ITER` 的 VM 执行分支）

### 对象模型说明

迭代器以 `Object::Ref(*mut MsObjHeader)` 存储，type_tag 为 `TypeTag::ITERATOR`（= 11，见 `20-object-system-basic.md` 权威 TypeTag 与 `14-gc.md:102`）。内部状态封装于 `MsIterator` 堆对象，引用 [20-object-system-basic](./20-object-system-basic.md) 的 `MsObjHeader`。

> **任务边界（订正 `12-implementation-plan.md` 与 task 25 注释的歧义）**：本任务拥有 **VM 侧迭代器基础设施 + 内置可迭代对象**（range→迭代器、`MsIterator`/`IteratorState`/`to_iterator`、`ITERATOR`/`FOR_ITER` 的 VM 执行）。编译器已在 task 19 发出 `ITERATOR`/`FOR_ITER`（`src/compiler/statement.rs:329-378`），故 VM 必须在本任务实现这两条 opcode，否则任何 `for..in` 运行期失败。用户层 `__iter__`/`__next__` 协议归 **task 32 / 43**；`12-implementation-plan.md:337-339` 的「ITERATOR/FOR_ITER/可迭代协议」Phase 4 条目应据此拆分（opcode 执行归本任务，协议归 task 32/43）。

> **GC 前瞻（task 52 依赖）**：`IteratorState` 的 `ListIter`/`DictKeys`/`Reversed`/`Enumerate`/`Zip` 持有 `Vec<Object>`，其中可能含 `Object::Ref` 堆指针。按 `14-gc.md:124`，每个类型须注册 `trace` 函数。task 52 GC 上线时**必须**为 `TypeTag::ITERATOR` 注册 trace，遍历 `IteratorState` 内全部 `Object::Ref`，否则被引用对象将被误回收导致悬垂指针。本任务采用 MVP 泄漏分配（`Box::into_raw`），task 52 前 GC 不运行，故当前安全。

### MsIterator 堆对象与分配

```rust
#[derive(Clone, Debug)]
pub enum IteratorState {
    Range {
        current: i64,
        end: i64,
        step: i64,
    },
    ListIter {
        items: Vec<Object>,
        index: usize,
    },
    StringIter {
        chars: Vec<char>,
        index: usize,
    },
    DictKeys {
        keys: Vec<Object>,
        index: usize,
    },
    Enumerate {
        inner: Box<IteratorState>,
        index: usize,
    },
    Zip {
        iterators: Vec<IteratorState>,
    },
    Reversed {
        items: Vec<Object>,
        index: usize,
    },
}

#[repr(C)]
pub struct MsIterator {
    pub header: MsObjHeader,
    pub state:  IteratorState,
}

/// 分配 Iterator 堆对象，返回 Object::Ref。
pub fn alloc_iterator(state: IteratorState) -> Object {
    let obj = Box::new(MsIterator {
        header: MsObjHeader {
            gc_meta:   0,
            type_tag:  TypeTag::ITERATOR as u8,
            size:      std::mem::size_of::<MsIterator>() as u16,
            _padding:  0,
            class_ptr: 0,
        },
        state,
    });
    Object::Ref(Box::into_raw(obj) as *mut MsObjHeader)
}

/// 读取 MsIterator 的可变状态引用。
///
/// # Safety
/// `ptr` 必须指向由 `alloc_iterator` 分配的有效 `MsIterator`。
/// 生命周期由调用方约束（`'a`），**不得**用 `'static`——遵循 task 20 read_* 约定。
pub unsafe fn read_iterator<'a>(ptr: *mut MsObjHeader) -> &'a mut MsIterator {
    &mut *(ptr as *mut MsIterator)
}
```

### IteratorState next() 协议

> **i64 溢出边界**：Range 的 `*current += *step`（下文）在极端区间溢出时 debug 构建 panic、release 回绕（Python range 为任意精度）。MVP 接受 i64 限制；如需严格语义，改 `checked_add` 并在溢出时提前返回 `None`（终止迭代）或经调用方上抛 `OverflowError`（须将 `next` 升级为 `Result`）。

```rust
impl IteratorState {
    pub fn next(&mut self) -> Option<Object> {
        match self {
            IteratorState::Range { current, end, step } => {
                if (*step > 0 && *current < *end)
                    || (*step < 0 && *current > *end)
                {
                    let val = Object::Int(*current);
                    *current += *step;
                    Some(val)
                } else {
                    None
                }
            }

            IteratorState::ListIter { items, index } => {
                if *index < items.len() {
                    let val = items[*index].clone();
                    *index += 1;
                    Some(val)
                } else {
                    None
                }
            }

            IteratorState::StringIter { chars, index } => {
                if *index < chars.len() {
                    let ch = chars[*index];
                    *index += 1;
                    Some(alloc_string(&ch.to_string()))
                } else {
                    None
                }
            }

            IteratorState::DictKeys { keys, index } => {
                if *index < keys.len() {
                    let val = keys[*index].clone();
                    *index += 1;
                    Some(val)
                } else {
                    None
                }
            }

            IteratorState::Enumerate { inner, index } => {
                match inner.next() {
                    Some(val) => {
                        let tuple = alloc_tuple(vec![
                            Object::Int(*index as i64),
                            val,
                        ]);
                        *index += 1;
                        Some(tuple)
                    }
                    None => None,
                }
            }

            IteratorState::Zip { iterators } => {
                let mut values = Vec::new();
                for it in iterators.iter_mut() {
                    match it.next() {
                        Some(val) => values.push(val),
                        None => return None,
                    }
                }
                Some(alloc_tuple(values))
            }

            IteratorState::Reversed { items, index } => {
                if *index > 0 {
                    *index -= 1;
                    Some(items[*index].clone())
                } else {
                    None
                }
            }
        }
    }
}
```

### to_iterator 工具函数

> **性能说明**：本函数对 list/tuple/dict/set 做**整表克隆**入 `IteratorState`（大集合下内存翻倍）。MVP 接受此开销以换取实现简洁与 `FOR_ITER` 无别名安全；后续可改为持有源对象引用 + 索引。

```rust
fn to_iterator(obj: &Object) -> Result<IteratorState, String> {
    match obj {
        Object::Ref(ptr) => {
            let tag = unsafe { (*(*ptr)).type_tag };
            if tag == TypeTag::LIST as u8 {
                Ok(IteratorState::ListIter {
                    items: unsafe { read_list(*ptr) }.clone(),
                    index: 0,
                })
            } else if tag == TypeTag::TUPLE as u8 {
                Ok(IteratorState::ListIter {
                    items: unsafe { read_tuple(*ptr) }.clone(),
                    index: 0,
                })
            } else if tag == TypeTag::STRING as u8 {
                Ok(IteratorState::StringIter {
                    chars: unsafe { read_str(*ptr) }.chars().collect(),
                    index: 0,
                })
            } else if tag == TypeTag::DICT as u8 {
                Ok(IteratorState::DictKeys {
                    keys: unsafe { read_dict(*ptr) }.keys().cloned().collect(),
                    index: 0,
                })
            } else if tag == TypeTag::SET as u8 {
                Ok(IteratorState::ListIter {
                    items: unsafe { read_set(*ptr) }.iter().cloned().collect(),
                    index: 0,
                })
            } else if tag == TypeTag::ITERATOR as u8 {
                Ok(unsafe { read_iterator(*ptr) }.state.clone())
            } else {
                Err(format!(
                    "TypeError: '{}' object is not iterable",
                    obj.type_name()
                ))
            }
        }
        _ => Err(format!(
            "TypeError: '{}' object is not iterable",
            obj.type_name()
        )),
    }
}
```

### range 函数

> **替换 task 25 的 range**：task 25 的 `builtin_range` 返回 **list**（`builtins.rs:586-613`），其注释自称「task 32 升级为惰性迭代器」。但设计规格 `10-builtins.md:97-99` 要求 `range(...) -> iterator`，且编译器已发 `ITERATOR`/`FOR_ITER`，故本任务**就地替换** task 25 的 range 为迭代器版本（符合规格），并须同步：①更新 `builtins.rs:585` 注释为「task 26 升级为迭代器」；②更新 task 25 的 range 测试（`builtins.rs:827`、`src/vm/mod.rs:1495`）以适配迭代器输出（如经 `for..in` 或 `list(range(...))` 消费）。`int_arg` 复用 task 25 已有的 `require_int`，不重复定义。

```rust
fn builtin_range(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    // require_int 复用 task 25（builtins.rs:617），不重复定义。
    let (start, end, step) = match args.len() {
        1 => {
            let end = require_int(&args[0])?;
            (0, end, 1)
        }
        2 => {
            let start = require_int(&args[0])?;
            let end = require_int(&args[1])?;
            (start, end, 1)
        }
        3 => {
            let start = require_int(&args[0])?;
            let end = require_int(&args[1])?;
            let step = require_int(&args[2])?;
            if step == 0 {
                return Err("ValueError: range() step must not be zero".to_string());
            }
            (start, end, step)
        }
        _ => return Err("range() requires 1-3 arguments".to_string()),
    };
    Ok(alloc_iterator(IteratorState::Range {
        current: start,
        end,
        step,
    }))
}
```

### sorted 函数

```rust
fn builtin_sorted(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let arg = args.get(0).ok_or("sorted() requires 1 argument")?;
    // 统一走 to_iterator，接受任意可迭代对象（list/tuple/string/set/dict/range/iterator）。
    let mut items: Vec<Object> = Vec::new();
    let mut iter = to_iterator(arg)?;
    while let Some(v) = iter.next() {
        items.push(v);
    }
    // CmpOp 与 OpCode 解耦（task 21，object.rs:392/612）。
    // 比较失败须上抛 TypeError（不可吞错致静默错序）。
    let mut err: Option<String> = None;
    items.sort_by(|a, b| {
        if err.is_some() {
            return std::cmp::Ordering::Equal;
        }
        match a.compare(b, CmpOp::Less) {
            Ok(Object::Bool(true)) => std::cmp::Ordering::Less,
            Ok(_) => match a.compare(b, CmpOp::Greater) {
                Ok(Object::Bool(true)) => std::cmp::Ordering::Greater,
                Ok(_) => std::cmp::Ordering::Equal,
                Err(e) => {
                    err = Some(e);
                    std::cmp::Ordering::Equal
                }
            },
            Err(e) => {
                err = Some(e);
                std::cmp::Ordering::Equal
            }
        }
    });
    if let Some(e) = err {
        return Err(e);
    }
    Ok(alloc_list(items))
}
```

### enumerate / zip 函数

```rust
fn builtin_enumerate(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let arg = args.get(0).ok_or("enumerate() requires 1 argument")?;
    let inner = to_iterator(arg)?;
    Ok(alloc_iterator(IteratorState::Enumerate {
        inner: Box::new(inner),
        index: 0,
    }))
}

fn builtin_zip(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    if args.is_empty() {
        return Err("zip() requires at least 1 argument".to_string());
    }
    let iterators: Result<Vec<IteratorState>, String> =
        args.iter().map(|a| to_iterator(a)).collect();
    Ok(alloc_iterator(IteratorState::Zip {
        iterators: iterators?,
    }))
}
```

### reversed 函数

```rust
fn builtin_reversed(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let arg = args.get(0).ok_or("reversed() requires 1 argument")?;
    let items = match arg {
        Object::Ref(ptr) => {
            let tag = unsafe { (*(*ptr)).type_tag };
            if tag == TypeTag::LIST as u8 {
                unsafe { read_list(*ptr) }.clone()
            } else if tag == TypeTag::TUPLE as u8 {
                unsafe { read_tuple(*ptr) }.clone()
            } else if tag == TypeTag::STRING as u8 {
                // reversed("abc") -> ["c","b","a"]（Python 对等）
                unsafe { read_str(*ptr) }
                    .chars()
                    .map(|c| alloc_string(&c.to_string()))
                    .collect()
            } else {
                return Err(format!(
                    "TypeError: '{}' object is not reversible",
                    arg.type_name()
                ))
            }
        }
        _ => {
            return Err(format!(
                "TypeError: '{}' object is not reversible",
                arg.type_name()
            ))
        }
    };
    let len = items.len();
    Ok(alloc_iterator(IteratorState::Reversed {
        items,
        index: len,
    }))
}
```

### map / filter 函数（Phase 3 存根）

> **最终形态**：设计规格 `10-builtins.md:104-105` 规定 `map(fn, iterable) -> list`、`filter(fn, iterable) -> list`，即**急切求值返回 list**（非惰性迭代器）。本任务因依赖用户函数调用（CALL 用户函数 / 闭包，task 27/28）暂以存根返回 Err；task 27/28 完成后须实现为：对 `to_iterator(iterable)` 逐项调用 `fn`，收集结果为 list 返回。

```rust
fn builtin_map(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let _fn_arg = args.get(0).ok_or("map() requires 2 arguments")?;
    let _iterable = args.get(1).ok_or("map() requires 2 arguments")?;
    Err("map() requires function call support (Phase 3)".to_string())
}

fn builtin_filter(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let _fn_arg = args.get(0).ok_or("filter() requires 2 arguments")?;
    let _iterable = args.get(1).ok_or("filter() requires 2 arguments")?;
    Err("filter() requires function call support (Phase 3)".to_string())
}
```

### any / all 函数

```rust
fn builtin_any(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let arg = args.get(0).ok_or("any() requires 1 argument")?;
    let mut iter = to_iterator(arg)?;
    while let Some(val) = iter.next() {
        if val.is_truthy() {
            return Ok(Object::Bool(true));
        }
    }
    Ok(Object::Bool(false))
}

fn builtin_all(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let arg = args.get(0).ok_or("all() requires 1 argument")?;
    let mut iter = to_iterator(arg)?;
    while let Some(val) = iter.next() {
        if !val.is_truthy() {
            return Ok(Object::Bool(false));
        }
    }
    Ok(Object::Bool(true))
}
```

### 容器构造函数

> **替换 task 25 的同名函数**：task 25 已注册 `list/tuple/set/dict`（`builtins.rs:98-101`，arity 均为 1，且 `builtin_list` 缺 SET 分支）。本任务以其**迭代器统一版本覆盖**之（`globals.insert` 覆盖旧值），改为 arity 可变（`usize::MAX`），支持 0 参空构造，并统一走 `to_iterator` 接受全部可迭代对象（含 set，修复 task 25 的 SET 回退）。`dict` 单函数同时处理空构造与 dict 拷贝。

```rust
/// 将任意可迭代对象消费为 Vec<Object>（DRY：供 list/tuple/set 构造复用）。
fn collect_iter(arg: &Object) -> Result<Vec<Object>, String> {
    let mut out = Vec::new();
    let mut it = to_iterator(arg)?;
    while let Some(v) = it.next() {
        out.push(v);
    }
    Ok(out)
}

fn builtin_list(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    if args.is_empty() {
        return Ok(alloc_list(Vec::new()));
    }
    if args.len() > 1 {
        return Err("list() takes 0 or 1 arguments".to_string());
    }
    Ok(alloc_list(collect_iter(&args[0])?))
}

fn builtin_tuple(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    if args.is_empty() {
        return Ok(alloc_tuple(Vec::new()));
    }
    if args.len() > 1 {
        return Err("tuple() takes 0 or 1 arguments".to_string());
    }
    Ok(alloc_tuple(collect_iter(&args[0])?))
}

fn builtin_set(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    if args.is_empty() {
        return Ok(alloc_set(HashSet::new()));
    }
    if args.len() > 1 {
        return Err("set() takes 0 or 1 arguments".to_string());
    }
    let items = collect_iter(&args[0])?;
    Ok(alloc_set(items.into_iter().collect()))
}

/// dict() 空字典；dict(d) dict 拷贝。
/// 从 (k, v) 对可迭代对象构造（dict([(k,v),...])）依赖元组解包迭代，
/// 随 task 30（多返回值与元组解包）完善，本 MVP 暂不支持并显式报错。
fn builtin_dict(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    if args.is_empty() {
        return Ok(alloc_dict(DictMap::new()));
    }
    if args.len() > 1 {
        return Err("dict() takes 0 or 1 arguments".to_string());
    }
    match &args[0] {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::DICT as u8 => {
            Ok(alloc_dict(unsafe { read_dict(*ptr) }.clone()))
        }
        _ => Err(format!(
            "TypeError: cannot convert '{}' to dict (MVP: only dict supported)",
            args[0].type_name()
        )),
    }
}
```

### VM 中 ITERATOR / FOR_ITER 指令

```rust
OpCode::Iterator => {
    let iterable = self.pop();
    let iter_state =
        to_iterator(&iterable).map_err(|e| format!("RuntimeError: {}", e))?;
    self.push(alloc_iterator(iter_state));
}

OpCode::ForIter => {
    // offset 为相对「操作数后一字节」的前向偏移，与编译器 patch_jump
    // （其他前向跳转同口径）一致；迭代结束时 ip += offset 跳到循环出口。
    let offset = self.read_u16() as usize;
    // 先取出 next 值并结束 &mut stack 借用，再 push，避免与 self.push 冲突。
    let next_val: Option<Object> = {
        let top = self.stack.len() - 1;
        match &mut self.stack[top] {
            Object::Ref(ptr) if unsafe { (*(*ptr)).type_tag } == TypeTag::ITERATOR as u8 => {
                unsafe { read_iterator(*ptr) }.state.next()
            }
            _ => return Err("RuntimeError: not an iterator".to_string()),
        }
    };
    match next_val {
        Some(v) => self.push(v),          // 迭代值留在迭代器之上，供 StoreLocal/Unpack 消费
        None => {
            let frame = self.frames.last_mut().unwrap();
            frame.ip += offset;           // 耗尽：跳到循环出口（编译器已发 Pop 弹出迭代器）
        }
    }
}
```

> **注意**：`FOR_ITER` 在迭代未结束时，将迭代产生的值留在迭代器栈顶之上（即 push next value）。迭代器对象本身保持在栈上供下一次 `FOR_ITER` 使用。

### 内置函数注册扩展

在 `register_builtins` 中追加。`globals.insert` 对同名键**覆盖**：`range/list/tuple/set/dict` 已在 task 25 注册，此处以其迭代器版本覆盖（range 改迭代器；构造函数改可变参数并走 `to_iterator`）；`sorted/reversed/enumerate/zip/map/filter/any/all` 为新增。`set`/`dict` 的函数名即 `builtin_set`/`builtin_dict`（**不是** `builtin_set_fn`/`builtin_dict_empty`——后者未定义）。

```rust
("range", usize::MAX, builtin_range),
("sorted", 1, builtin_sorted),
("reversed", 1, builtin_reversed),
("enumerate", 1, builtin_enumerate),
("zip", usize::MAX, builtin_zip),
("map", 2, builtin_map),
("filter", 2, builtin_filter),
("any", 1, builtin_any),
("all", 1, builtin_all),
("list", usize::MAX, builtin_list),
("tuple", usize::MAX, builtin_tuple),
("set", usize::MAX, builtin_set),
("dict", usize::MAX, builtin_dict),
```

## 验证标准

1. `range(5)` 产生 0, 1, 2, 3, 4
2. `range(2, 8)` 产生 2-7
3. `range(0, 10, 2)` 产生 0, 2, 4, 6, 8
4. `sorted([3, 1, 2])` 返回 `[1, 2, 3]`
5. `enumerate(["a", "b"])` 产生 `(0, "a"), (1, "b")`
6. `zip([1, 2], ["x", "y"])` 产生 `(1, "x"), (2, "y")`
7. `any([false, false, true])` 返回 `true`
8. `all([true, true, false])` 返回 `false`
9. `list("abc")` 返回 `["a", "b", "c"]`
10. `set([1, 2, 2])` 返回 `{1, 2}`
11. for..in 循环能正确遍历 range、list、string、dict

## 测试用例

```ms
# test_builtins_iterators.ms
for i in range(5) {
    print(i)
}

nums = [3, 1, 4, 1, 5]
print(sorted(nums))

for i, v in enumerate(["a", "b"]) {
    print(i, v)
}

for a, b in zip([1, 2], ["x", "y"]) {
    print(a, b)
}
```

预期输出：
```
0
1
2
3
4
[1, 1, 3, 4, 5]
0 a
1 b
1 x
2 y
```

### Rust 单元测试

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_range_basic() {
        let mut iter = IteratorState::Range {
            current: 0,
            end: 5,
            step: 1,
        };
        let values: Vec<Object> = std::iter::from_fn(|| iter.next()).collect();
        assert_eq!(values.len(), 5);
        assert_eq!(values[0], Object::Int(0));
        assert_eq!(values[4], Object::Int(4));
    }

    #[test]
    fn test_range_with_step() {
        let mut iter = IteratorState::Range {
            current: 0,
            end: 10,
            step: 2,
        };
        let values: Vec<Object> = std::iter::from_fn(|| iter.next()).collect();
        assert_eq!(values, vec![
            Object::Int(0), Object::Int(2),
            Object::Int(4), Object::Int(6), Object::Int(8),
        ]);
    }

    #[test]
    fn test_range_negative_step() {
        let mut iter = IteratorState::Range {
            current: 5,
            end: 0,
            step: -1,
        };
        let values: Vec<Object> = std::iter::from_fn(|| iter.next()).collect();
        assert_eq!(values.len(), 5);
        assert_eq!(values[0], Object::Int(5));
        assert_eq!(values[4], Object::Int(1));
    }

    #[test]
    fn test_enumerate() {
        let inner = IteratorState::ListIter {
            items: vec![
                alloc_string("a"),
                alloc_string("b"),
            ],
            index: 0,
        };
        let mut iter = IteratorState::Enumerate {
            inner: Box::new(inner),
            index: 0,
        };
        let first = iter.next().unwrap();
        assert_eq!(
            first,
            alloc_tuple(vec![
                Object::Int(0),
                alloc_string("a"),
            ])
        );
    }

    #[test]
    fn test_zip() {
        let iter = IteratorState::Zip {
            iterators: vec![
                IteratorState::ListIter {
                    items: vec![Object::Int(1), Object::Int(2)],
                    index: 0,
                },
                IteratorState::ListIter {
                    items: vec![
                        alloc_string("x"),
                        alloc_string("y"),
                    ],
                    index: 0,
                },
            ],
        };
        let mut iter = iter;
        let first = iter.next().unwrap();
        assert_eq!(
            first,
            alloc_tuple(vec![
                Object::Int(1),
                alloc_string("x"),
            ])
        );
    }

    #[test]
    fn test_any_all() {
        let mut vm = VM::new();
        vm.register_builtins();

        let result = builtin_any(&mut vm, &[alloc_list(vec![
            Object::Bool(false), Object::Bool(true),
        ])]).unwrap();
        assert_eq!(result, Object::Bool(true));

        let result = builtin_all(&mut vm, &[alloc_list(vec![
            Object::Bool(true), Object::Bool(false),
        ])]).unwrap();
        assert_eq!(result, Object::Bool(false));
    }
}
```
