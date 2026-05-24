# 内置迭代器与容器函数

## 所属阶段
Phase 2.5b - 字节码编译 + VM 核心

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

`src/vm/builtins.rs`（扩展任务 25）
`src/vm/object.rs`（添加 Iterator 变体）

### Object 枚举扩展

```rust
#[derive(Clone, Debug)]
pub enum Object {
    // ... 已有变体 ...
    Iterator(Gc<IteratorState>),
}

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
```

### IteratorState next() 协议

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
                    Some(Object::String(Gc::new(ch.to_string())))
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
                        let tuple = Object::Tuple(Gc::new(vec![
                            Object::Int(*index as i64),
                            val,
                        ]));
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
                Some(Object::Tuple(Gc::new(values)))
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

```rust
fn to_iterator(obj: &Object) -> Result<IteratorState, String> {
    match obj {
        Object::List(items) => Ok(IteratorState::ListIter {
            items: items.borrow().data.clone(),
            index: 0,
        }),
        Object::Tuple(items) => Ok(IteratorState::ListIter {
            items: items.borrow().data.clone(),
            index: 0,
        }),
        Object::String(s) => Ok(IteratorState::StringIter {
            chars: s.borrow().data.chars().collect(),
            index: 0,
        }),
        Object::Dict(map) => Ok(IteratorState::DictKeys {
            keys: map.borrow().data.keys().cloned().collect(),
            index: 0,
        }),
        Object::Iterator(state) => Ok(state.borrow().data.clone()),
        _ => Err(format!(
            "TypeError: '{}' object is not iterable",
            obj.type_name()
        )),
    }
}
```

### range 函数

```rust
fn builtin_range(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let (start, end, step) = match args.len() {
        1 => {
            let end = int_arg(&args[0], "range()")?;
            (0, end, 1)
        }
        2 => {
            let start = int_arg(&args[0], "range()")?;
            let end = int_arg(&args[1], "range()")?;
            (start, end, 1)
        }
        3 => {
            let start = int_arg(&args[0], "range()")?;
            let end = int_arg(&args[1], "range()")?;
            let step = int_arg(&args[2], "range()")?;
            if step == 0 {
                return Err("ValueError: range() step must not be zero".to_string());
            }
            (start, end, step)
        }
        _ => return Err("range() requires 1-3 arguments".to_string()),
    };
    Ok(Object::Iterator(Gc::new(IteratorState::Range {
        current: start,
        end,
        step,
    })))
}

fn int_arg(obj: &Object, ctx: &str) -> Result<i64, String> {
    match obj {
        Object::Int(n) => Ok(*n),
        _ => Err(format!("TypeError: {} argument must be int", ctx)),
    }
}
```

### sorted 函数

```rust
fn builtin_sorted(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let arg = args.get(0).ok_or("sorted() requires 1 argument")?;
    let mut items = match arg {
        Object::List(items) => items.borrow().data.clone(),
        Object::Tuple(items) => items.borrow().data.clone(),
        _ => {
            return Err(format!(
                "TypeError: '{}' object is not iterable",
                arg.type_name()
            ))
        }
    };
    items.sort_by(|a, b| {
        match a.compare(b, &OpCode::Less) {
            Ok(Object::Bool(true)) => std::cmp::Ordering::Less,
            Ok(Object::Bool(false)) => match a.compare(b, &OpCode::Greater) {
                Ok(Object::Bool(true)) => std::cmp::Ordering::Greater,
                _ => std::cmp::Ordering::Equal,
            },
            _ => std::cmp::Ordering::Equal,
        }
    });
    Ok(Object::List(Gc::new(items)))
}
```

### enumerate / zip 函数

```rust
fn builtin_enumerate(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let arg = args.get(0).ok_or("enumerate() requires 1 argument")?;
    let inner = to_iterator(arg)?;
    Ok(Object::Iterator(Gc::new(IteratorState::Enumerate {
        inner: Box::new(inner),
        index: 0,
    })))
}

fn builtin_zip(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    if args.is_empty() {
        return Err("zip() requires at least 1 argument".to_string());
    }
    let iterators: Result<Vec<IteratorState>, String> =
        args.iter().map(|a| to_iterator(a)).collect();
    Ok(Object::Iterator(Gc::new(IteratorState::Zip {
        iterators: iterators?,
    })))
}
```

### reversed 函数

```rust
fn builtin_reversed(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let arg = args.get(0).ok_or("reversed() requires 1 argument")?;
    let items = match arg {
        Object::List(items) => items.borrow().data.clone(),
        Object::Tuple(items) => items.borrow().data.clone(),
        _ => {
            return Err(format!(
                "TypeError: '{}' object is not reversible",
                arg.type_name()
            ))
        }
    };
    Ok(Object::Iterator(Gc::new(IteratorState::Reversed {
        items,
        index: items.len(),
    })))
}
```

### map / filter 函数（Phase 3 存根）

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

```rust
fn builtin_list(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    if args.is_empty() {
        return Ok(Object::List(Gc::new(Vec::new())));
    }
    let arg = &args[0];
    match arg {
        Object::String(s) => {
            let items: Vec<Object> = s
                .borrow()
                .data
                .chars()
                .map(|c| Object::String(Gc::new(c.to_string())))
                .collect();
            Ok(Object::List(Gc::new(items)))
        }
        Object::List(items) => {
            Ok(Object::List(Gc::new(items.borrow().data.clone())))
        }
        Object::Tuple(items) => {
            Ok(Object::List(Gc::new(items.borrow().data.clone())))
        }
        _ => Err(format!(
            "TypeError: cannot convert {} to list",
            arg.type_name()
        )),
    }
}

fn builtin_tuple(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    if args.is_empty() {
        return Ok(Object::Tuple(Gc::new(Vec::new())));
    }
    let items = match &args[0] {
        Object::List(items) => items.borrow().data.clone(),
        Object::Tuple(items) => items.borrow().data.clone(),
        _ => {
            return Err(format!(
                "TypeError: cannot convert {} to tuple",
                args[0].type_name()
            ))
        }
    };
    Ok(Object::Tuple(Gc::new(items)))
}

fn builtin_set_fn(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    if args.is_empty() {
        return Ok(Object::Set(Gc::new(HashSetWrapper {
            inner: HashSet::new(),
        })));
    }
    let mut inner = HashSet::new();
    match &args[0] {
        Object::List(items) => {
            for item in &items.borrow().data {
                inner.insert(item.clone());
            }
        }
        Object::Tuple(items) => {
            for item in &items.borrow().data {
                inner.insert(item.clone());
            }
        }
        _ => {
            return Err(format!(
                "TypeError: cannot convert {} to set",
                args[0].type_name()
            ))
        }
    }
    Ok(Object::Set(Gc::new(HashSetWrapper { inner })))
}
```

### VM 中 ITERATOR / FOR_ITER 指令

```rust
OpCode::Iterator => {
    let iterable = self.pop();
    let iter_state =
        to_iterator(&iterable).map_err(|e| format!("RuntimeError: {}", e))?;
    self.push(Object::Iterator(Gc::new(iter_state)));
}

OpCode::ForIter => {
    let offset = self.read_u16() as usize;
    let top = self.stack.len() - 1;
    let done = match &mut self.stack[top] {
        Object::Iterator(state) => state.borrow_mut().data.next().is_none(),
        _ => return Err("RuntimeError: not an iterator".to_string()),
    };
    if done {
        let frame = self.frames.last_mut().unwrap();
        frame.ip += offset;
    }
}
```

> **注意**：`FOR_ITER` 在迭代未结束时，将迭代产生的值留在迭代器栈顶之上（即 push next value）。迭代器对象本身保持在栈上供下一次 `FOR_ITER` 使用。

### 内置函数注册扩展

在 `register_builtins` 中追加：

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
("set", usize::MAX, builtin_set_fn),
("dict", 0, builtin_dict_empty),
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
                Object::String(Gc::new("a".to_string())),
                Object::String(Gc::new("b".to_string())),
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
            Object::Tuple(Gc::new(vec![
                Object::Int(0),
                Object::String(Gc::new("a".to_string())),
            ]))
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
                        Object::String(Gc::new("x".to_string())),
                        Object::String(Gc::new("y".to_string())),
                    ],
                    index: 0,
                },
            ],
        };
        let mut iter = iter;
        let first = iter.next().unwrap();
        assert_eq!(
            first,
            Object::Tuple(Gc::new(vec![
                Object::Int(1),
                Object::String(Gc::new("x".to_string())),
            ]))
        );
    }

    #[test]
    fn test_any_all() {
        let mut vm = VM::new();
        vm.register_builtins();

        let result = builtin_any(&mut vm, &[Object::List(Gc::new(vec![
            Object::Bool(false), Object::Bool(true),
        ]))]).unwrap();
        assert_eq!(result, Object::Bool(true));

        let result = builtin_all(&mut vm, &[Object::List(Gc::new(vec![
            Object::Bool(true), Object::Bool(false),
        ]))]).unwrap();
        assert_eq!(result, Object::Bool(false));
    }
}
```
