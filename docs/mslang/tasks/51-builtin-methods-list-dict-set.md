# 内置类型方法 - List/Dict/Set

## 所属阶段
Phase 6.2f - 标准库

## 前置任务
26-builtins-iterators

## 目标
为 mslang 的 List、Dict、Set 类型实现所有内置方法。

## 设计规格

参照 [10-builtins](../10-builtins.md) § list/dict/set 方法：

### List 方法

| 方法 | 签名 | 说明 |
|---|---|---|
| `lst.length()` | `() -> int` | 列表长度 |
| `lst.push(val)` | `(value) -> nil` | 尾部追加 |
| `lst.pop()` | `() -> value` | 弹出末尾元素 |
| `lst.pop(index)` | `(int) -> value` | 弹出指定位置元素 |
| `lst.insert(index, val)` | `(int, value) -> nil` | 插入元素 |
| `lst.remove(val)` | `(value) -> nil` | 删除第一个匹配 |
| `lst.index(val)` | `(value) -> int` | 查找元素位置 |
| `lst.contains(val)` | `(value) -> bool` | 是否包含 |
| `lst.sort()` | `() -> nil` | 原地排序 |
| `lst.reverse()` | `() -> nil` | 原地反转 |
| `lst.slice(start, end?)` | `(int, int?) -> list` | 切片 |
| `lst.map(fn)` | `(function) -> list` | 映射 |
| `lst.filter(fn)` | `(function) -> list` | 过滤 |
| `lst.reduce(fn, init?)` | `(function, value?) -> value` | 归约 |

### Dict 方法

| 方法 | 签名 | 说明 |
|---|---|---|
| `d.length()` | `() -> int` | 键值对数量 |
| `d.keys()` | `() -> list` | 返回键列表 |
| `d.values()` | `() -> list` | 返回值列表 |
| `d.items()` | `() -> list` | 返回 (key, value) 对列表 |
| `d.get(key, default?)` | `(value, value?) -> value` | 获取值（不存在返回默认值） |
| `d.set(key, val)` | `(value, value) -> nil` | 设置键值 |
| `d.remove(key)` | `(value) -> nil` | 删除键 |
| `d.contains(key)` | `(value) -> bool` | 是否包含键 |
| `d.merge(other)` | `(dict) -> nil` | 合并另一个 dict |

### Set 方法

| 方法 | 签名 | 说明 |
|---|---|---|
| `s.length()` | `() -> int` | 元素数量 |
| `s.add(val)` | `(value) -> nil` | 添加元素 |
| `s.remove(val)` | `(value) -> nil` | 删除元素 |
| `s.contains(val)` | `(value) -> bool` | 是否包含 |
| `s.union(other)` | `(set) -> set` | 并集 |
| `s.intersection(other)` | `(set) -> set` | 交集 |
| `s.difference(other)` | `(set) -> set` | 差集 |

## 实现细节

### 1. List 方法分派

`src/vm/stdlib.rs` 或 `src/vm/builtins.rs`：

```rust
fn call_list_method(
    method: &str,
    receiver: &Gc<Vec<Object>>,
    args: Vec<Object>,
    vm: &mut VM,
) -> Result<Object> {
    match method {
        "length" => Ok(Object::Int(receiver.borrow().len() as i64)),
        "push" => {
            let val = args.into_iter().next().ok_or(...)?;
            receiver.borrow_mut().push(val);
            Ok(Object::Nil)
        }
        "pop" => {
            let mut list = receiver.borrow_mut();
            if args.is_empty() {
                list.pop().ok_or_else(|| MspError::RuntimeError("pop from empty list".into()))
            } else {
                let idx = expect_int(&args[0])? as usize;
                if idx < list.len() {
                    Ok(list.remove(idx))
                } else {
                    Err(MspError::RuntimeError("index out of bounds".into()))
                }
            }
        }
        "sort" => { receiver.borrow_mut().sort_by(|a, b| a.cmp(b)); Ok(Object::Nil) }
        "reverse" => { receiver.borrow_mut().reverse(); Ok(Object::Nil) }
        "map" => { ... }
        "filter" => { ... }
        "reduce" => { ... }
        // ...
    }
}
```

### 2. map/filter/reduce 实现

**map**：
```rust
"map" => {
    let fn_obj = expect_callable(&args[0])?;
    let list = receiver.borrow();
    let mut result = Vec::with_capacity(list.len());
    for item in list.iter() {
        let mapped = vm.call_function(&fn_obj, vec![item.clone()])?;
        result.push(mapped);
    }
    Ok(Object::List(Gc::new(result)))
}
```

**filter**：
```rust
"filter" => {
    let fn_obj = expect_callable(&args[0])?;
    let list = receiver.borrow();
    let mut result = Vec::new();
    for item in list.iter() {
        let cond = vm.call_function(&fn_obj, vec![item.clone()])?;
        if cond.is_truthy() {
            result.push(item.clone());
        }
    }
    Ok(Object::List(Gc::new(result)))
}
```

**reduce**：
```rust
"reduce" => {
    let fn_obj = expect_callable(&args[0])?;
    let list = receiver.borrow();
    let mut acc = if args.len() > 1 {
        args[1].clone()
    } else {
        list.first().ok_or_else(|| MspError::RuntimeError("reduce on empty list".into()))?.clone()
    };
    let start = if args.len() > 1 { 0 } else { 1 };
    for item in list.iter().skip(start) {
        acc = vm.call_function(&fn_obj, vec![acc, item.clone()])?;
    }
    Ok(acc)
}
```

### 3. Dict 方法分派

```rust
fn call_dict_method(
    method: &str,
    receiver: &Gc<DictMap>,
    args: Vec<Object>,
) -> Result<Object> {
    match method {
        "length" => Ok(Object::Int(receiver.borrow().len() as i64)),
        "keys" => {
            let keys: Vec<Object> = receiver.borrow().keys()
                .map(|k| Object::String(Gc::new(k.clone())))
                .collect();
            Ok(Object::List(Gc::new(keys)))
        }
        "values" => {
            let vals: Vec<Object> = receiver.borrow().values().cloned().collect();
            Ok(Object::List(Gc::new(vals)))
        }
        "items" => {
            let items: Vec<Object> = receiver.borrow().iter()
                .map(|(k, v)| Object::Tuple(Gc::new(vec![
                    Object::String(Gc::new(k.clone())),
                    v.clone(),
                ])))
                .collect();
            Ok(Object::List(Gc::new(items)))
        }
        "get" => {
            let key = dict_key_from(&args[0])?;
            let default = if args.len() > 1 { args[1].clone() } else { Object::Nil };
            Ok(receiver.borrow().get(&key).cloned().unwrap_or(default))
        }
        "merge" => {
            let other = expect_dict(&args[0])?;
            receiver.borrow_mut().extend(other.iter().map(|(k, v)| (k.clone(), v.clone())));
            Ok(Object::Nil)
        }
        // ...
    }
}
```

### 4. Set 方法分派

```rust
fn call_set_method(
    method: &str,
    receiver: &Gc<HashSet<Object>>,
    args: Vec<Object>,
) -> Result<Object> {
    match method {
        "length" => Ok(Object::Int(receiver.borrow().len() as i64)),
        "add" => { receiver.borrow_mut().insert(args[0].clone()); Ok(Object::Nil) }
        "remove" => { receiver.borrow_mut().remove(&args[0]); Ok(Object::Nil) }
        "contains" => Ok(Object::Bool(receiver.borrow().contains(&args[0]))),
        "union" => {
            let other = expect_set(&args[0])?;
            let result: HashSet<Object> = receiver.borrow().union(&other).cloned().collect();
            Ok(Object::Set(Gc::new(result)))
        }
        "intersection" => {
            let other = expect_set(&args[0])?;
            let result: HashSet<Object> = receiver.borrow().intersection(&other).cloned().collect();
            Ok(Object::Set(Gc::new(result)))
        }
        "difference" => {
            let other = expect_set(&args[0])?;
            let result: HashSet<Object> = receiver.borrow().difference(&other).cloned().collect();
            Ok(Object::Set(Gc::new(result)))
        }
        _ => Err(MspError::RuntimeError(format!("set has no method '{}'", method))),
    }
}
```

### 5. VM 集成

在 `INVOKE` 指令处理中，根据接收者类型分派到对应方法实现：

```rust
OpCode::INVOKE => {
    let name = self.read_constant(name_idx);
    let argc = self.read_byte();
    let receiver = self.stack[self.stack.len() - argc - 1];
    match &receiver {
        Object::String(_) => call_string_method(...)
        Object::List(_) => call_list_method(...)
        Object::Dict(_) => call_dict_method(...)
        Object::Set(_) => call_set_method(...)
        // ... 其他类型
    }
}
```

## 验证标准

1. List 的 14 个方法全部正确工作
2. Dict 的 9 个方法全部正确工作
3. Set 的 7 个方法全部正确工作
4. `map/filter/reduce` 正确调用回调函数
5. 空集合的边界情况正确处理
6. 类型错误给出清晰提示

## 测试用例

### test_list_methods.ms

```ms
lst = [3, 1, 4, 1, 5]
lst.sort()
print(lst)
lst.push(9)
print(lst)
lst.pop()
print(lst)
lst.insert(0, 0)
print(lst)
lst.remove(1)
print(lst)
print(lst.contains(4))
print(lst.index(3))
print(lst.length())
```

预期输出：
```
[1, 1, 3, 4, 5]
[1, 1, 3, 4, 5, 9]
[1, 1, 3, 4, 5]
[0, 1, 1, 3, 4, 5]
[0, 1, 3, 4, 5]
true
2
5
```

### test_dict_methods.ms

```ms
d = {"a": 1, "b": 2}
print(d.keys())
print(d.values())
print(d.items())
print(d.get("c", 0))
d.merge({"c": 3})
print(d)
print(d.contains("a"))
print(d.length())
```

预期输出：
```
[a, b]
[1, 2]
[(a, 1), (b, 2)]
0
{a: 1, b: 2, c: 3}
true
3
```

### test_set_methods.ms

```ms
s = {1, 2, 3}
s.add(4)
print(s.contains(4))
print(s.union({5, 6}))
print(s.intersection({2, 3, 7}))
print(s.difference({1, 2}))
print(s.length())
```

预期输出：
```
true
{1, 2, 3, 4, 5, 6}
{2, 3}
{3, 4}
4
```

### test_higher_order.ms

```ms
lst = [1, 2, 3, 4, 5]
doubled = lst.map(fn(x) { return x * 2 })
print(doubled)

evens = lst.filter(fn(x) { return x % 2 == 0 })
print(evens)

total = lst.reduce(fn(a, b) { return a + b }, 0)
print(total)
```

预期输出：
```
[2, 4, 6, 8, 10]
[2, 4]
15
```
