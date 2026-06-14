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
    receiver_ptr: *mut MsObjHeader,  // 指向 MsList
    args: Vec<Object>,
    vm: &mut VM,
) -> Result<Object> {
    let list = unsafe { read_list(receiver_ptr) };
    match method {
        "length" => Ok(Object::Int(list.len() as i64)),
        "push" => {
            let val = args.into_iter().next().ok_or(...)?;
            list.push(val);
            Ok(Object::Nil)
        }
        "pop" => {
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
        "sort" => { list.sort_by(|a, b| a.cmp(b)); Ok(Object::Nil) }
        "reverse" => { list.reverse(); Ok(Object::Nil) }
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
    Ok(alloc_list(&result))
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
    Ok(alloc_list(&result))
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
    receiver_ptr: *mut MsObjHeader,  // 指向 MsDict
    args: Vec<Object>,
) -> Result<Object> {
    let dict = unsafe { read_dict(receiver_ptr) };
    match method {
        "length" => Ok(Object::Int(dict.len() as i64)),
        "keys" => {
            let keys: Vec<Object> = dict.keys().cloned().collect();
            Ok(alloc_list(keys))
        }
        "values" => {
            let vals: Vec<Object> = dict.items().iter().map(|(_, v)| (*v).clone()).collect();
            Ok(alloc_list(vals))
        }
        "items" => {
            let items: Vec<Object> = dict.items().iter()
                .map(|(k, v)| alloc_tuple(vec![(*k).clone(), (*v).clone()]))
                .collect();
            Ok(alloc_list(items))
        }
        "get" => {
            let key = dict_key_from(&args[0])?;
            let default = if args.len() > 1 { args[1].clone() } else { Object::Nil };
            Ok(dict.get(&key).cloned().unwrap_or(default))
        }
        "merge" => {
            let other_ptr = expect_dict_ptr(&args[0])?;
            let other = unsafe { read_dict(other_ptr) };
            for (k, v) in other.items() {
                dict.insert((*k).clone(), (*v).clone());
            }
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
    receiver_ptr: *mut MsObjHeader,  // 指向 MsSet
    args: Vec<Object>,
) -> Result<Object> {
    let set = unsafe { read_set(receiver_ptr) };
    match method {
        "length" => Ok(Object::Int(set.len() as i64)),
        "add" => { set.insert(args[0].clone()); Ok(Object::Nil) }
        "remove" => { set.remove(&args[0]); Ok(Object::Nil) }
        "contains" => Ok(Object::Bool(set.contains(&args[0]))),
        "union" => {
            let other_ptr = expect_set_ptr(&args[0])?;
            let other = unsafe { read_set(other_ptr) };
            let result: HashSet<Object> = set.union(other).cloned().collect();
            Ok(alloc_set(result))
        }
        "intersection" => {
            let other_ptr = expect_set_ptr(&args[0])?;
            let other = unsafe { read_set(other_ptr) };
            let result: HashSet<Object> = set.intersection(other).cloned().collect();
            Ok(alloc_set(result))
        }
        "difference" => {
            let other_ptr = expect_set_ptr(&args[0])?;
            let other = unsafe { read_set(other_ptr) };
            let result: HashSet<Object> = set.difference(other).cloned().collect();
            Ok(alloc_set(result))
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
        Object::Ref(ptr) => {
            let tag = unsafe { (*(*ptr)).type_tag };
            if tag == TypeTag::STRING as u8 {
                call_string_method(name, *ptr, args, vm)
            } else if tag == TypeTag::LIST as u8 {
                call_list_method(name, *ptr, args, vm)
            } else if tag == TypeTag::DICT as u8 {
                call_dict_method(name, *ptr, args)
            } else if tag == TypeTag::SET as u8 {
                call_set_method(name, *ptr, args)
            } else {
                Err(MspError::RuntimeError(format!("type has no method '{}'", name)))
            }
        }
        // ... 其他类型
        _ => Err(MspError::RuntimeError(format!("type has no method '{}'", name)))
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
