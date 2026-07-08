# 内置类型方法 - List/Dict/Set

## 所属阶段
Phase 6.2f - 标准库

## 前置任务
26-builtins-iterators
41-self-instance-attributes    # BoundMethod 基础设施（alloc_bound_method）
46-stdlib-io                   # GET_ATTR → BoundMethod 分派模式
50-builtin-methods-string      # expect_int/expect_list_ref 范式、GET_ATTR STRING 分支位置

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

> **对象模型约束**（task 20/25/46/47/48/49/50）：Object 枚举严格为 `{Nil, Bool, Int, Float, Ref}`，**无 `NativeFn` 变体**。原生函数经 `alloc_native_function(NativeFunction{name, func})` 包装为 `Object::Ref` + `TypeTag::FUNCTION`。`NativeFn` 签名为 `fn(&mut VM, &[Object]) -> Result<Object, String>`（切片，非 Vec）。所有错误统一 `Result<_, String>`，错误消息前缀遵循 `"<ErrorType>: ..."`，与 task 46-50 一致：
> - `TypeError` — 类型不匹配（含"不可哈希类型作为 set 元素/dict 键"、"非 callable 参数"、"非 list/dict/set receiver"）
> - `ValueError` — 值非法（空 list reduce、slice 反向）
> - `IndexError` — 索引越界（list.pop(i)/list.insert(i,...) 归一化后越界）
> - `KeyError` — 键/元素不存在（dict.remove(key)、set.remove(val)，见 `02-types.md:187` 与 `10-builtins.md:246`）
> - `AttributeError` — 未知方法名（由 GET_ATTR catch-all 统一处理，方法实现内不重复）
>
> **参数访问约定**：**必须用 `args.get(N)`** 而非 `args[N]`（task 50 §0 强制）。参数不足时返回 `Err("TypeError: <method> requires N arguments".to_string())`，不得 panic。
>
> **Receiver 注入约定**（task 46/50 模式）：方法经 `GET_ATTR` 返回 `BoundMethod{receiver, method}`，后续 `CALL` 自动把 receiver 注入为 `args[0]`（见 `src/vm/mod.rs:2644-2651`）。因此 **native 函数内 `args[0]` 是 receiver（List/Dict/Set Ref），用户参数从 `args.get(1)` 起**。
>
> **辅助函数来源**：
> - `expect_int(arg, who)`、`expect_list_ref(arg, who)`、`expect_string(arg, who)` — 复用 task 50 §1b（`stdlib.rs:1330-1342` 及紧邻段）
> - `expect_callable(arg, who)`、`expect_dict_ref(arg, who)`、`expect_set_ref(arg, who)` — 本任务 §1b 新增，范式同 `expect_list_ref`
>
> **借用约束**（task 49 §3 + task 50 §3.2）：`read_list`/`read_dict`/`read_set` 返回 `&mut Vec`/`&mut DictMap`/`&mut HashSet`。**在调用 `vm.call_function`（用户回调）、`alloc_list/dict/set`（分配，可能触发 GC）、或递归进入 VM 前必须 clone 出所需数据并释放借用**。否则：① GC 半空间复制改写 Ref 指针后旧 `&mut` 悬垂；② 回调内再次访问同一 receiver 形成 `&mut` 别名（panic on RefCell 或 UB on raw）。
>
> **自引用别名 UB**（`lst.push(lst)`、`d.merge(d)`、`s.union(s)`）：修改 receiver 前**必须先 clone 出 other/args 的全部内容**，迭代 clone 而非 receiver 本身（见 §3 merge、§4 union）。
>
> **GC 写屏障前瞻**（task 52 依赖，`14-gc.md:541-543`）：本任务 MVP 期 GC 为 STW（Phase 2.5），`alloc_list`/`alloc_dict`/`alloc_set` 内 Ref 元素无需写屏障。task 52 并发 GC 上线时，**必须**为这三个 alloc 函数与 `LIST_PUSH`/`DICT_SET`/`SET_ADD` 注入混合写屏障（Old→Young 跨代引用）。同时须确认 LIST/DICT/SET 的 TypeDescriptor.trace（task 20/22 已注册）遍历 Ref 元素。
>
> **可哈希性约束**（`02-types.md:339-350`）：Set 元素与 Dict 键必须为 int/float/bool/string/nil/tuple；list/dict/set **不可哈希**。`{-0.0: 1, 0.0: 2}` 视为同一键；NaN 不可哈希（`hash(NaN)` 抛 TypeError）。Set/Dict 底层须用自定义 `hash_key(&Object) -> Result<u64, String>` + `key_eq(&Object, &Object) -> bool`，而非直接 `HashSet<Object>`（见 §1b）。
>
> **位置语义**（与 task 50 §3.4-3.5 一致）：`list.index(val)`、`list.slice(start, end?)`、`list.pop(index)`、`list.insert(index, val)` 均支持负索引（相对末尾），归一化函数见 §1b `normalize_index`。越界抛 `IndexError`。

### 1. 方法分发表（FileHandle / String 模式）

参照 task 46 的 `lookup_file_method`（`src/vm/stdlib.rs:121-139`）与 task 50 的 `lookup_string_method`（`50-builtin-methods-string.md:49-66`），在 `src/vm/stdlib.rs` 实现三个独立分发表。每次 `GET_ATTR` 由调用方 `alloc_native_function` 分配新对象（与 task 46/50 一致；性能优化留待 task 52+ 后 intern 表方案）。

```rust
/// List 方法名 → 原生函数（供 GET_ATTR 包装为 BoundMethod）。
pub fn lookup_list_method(name: &str) -> Option<NativeFn> {
    let func: NativeFn = match name {
        "length" => native_list_length,
        "push" => native_list_push,
        "pop" => native_list_pop,
        "insert" => native_list_insert,
        "remove" => native_list_remove,
        "index" => native_list_index,
        "contains" => native_list_contains,
        "sort" => native_list_sort,
        "reverse" => native_list_reverse,
        "slice" => native_list_slice,
        "map" => native_list_map,
        "filter" => native_list_filter,
        "reduce" => native_list_reduce,
        _ => return None,
    };
    Some(func)
}

/// Dict 方法名 → 原生函数。
pub fn lookup_dict_method(name: &str) -> Option<NativeFn> {
    let func: NativeFn = match name {
        "length" => native_dict_length,
        "keys" => native_dict_keys,
        "values" => native_dict_values,
        "items" => native_dict_items,
        "get" => native_dict_get,
        "set" => native_dict_set,
        "remove" => native_dict_remove,
        "contains" => native_dict_contains,
        "merge" => native_dict_merge,
        _ => return None,
    };
    Some(func)
}

/// Set 方法名 → 原生函数。
pub fn lookup_set_method(name: &str) -> Option<NativeFn> {
    let func: NativeFn = match name {
        "length" => native_set_length,
        "add" => native_set_add,
        "remove" => native_set_remove,
        "contains" => native_set_contains,
        "union" => native_set_union,
        "intersection" => native_set_intersection,
        "difference" => native_set_difference,
        _ => return None,
    };
    Some(func)
}
```

GET_ATTR 侧（`src/vm/mod.rs`）负责 `alloc_native_function` 包装并 `alloc_bound_method` 绑定 receiver（见 §5）。

### 1b. 新增辅助函数（stdlib.rs §辅助函数 段，紧邻 task 50 `expect_int:1330`）

```rust
/// 校验首参数为 Dict Ref，返回裸指针。调用方 unsafe read_dict 取内容
/// （借用约束：修改前必须释放 &mut DictMap，参见对象模型约束段）。
fn expect_dict_ref(arg: Option<&Object>, who: &str) -> Result<*mut MsObjHeader, String> {
    match arg {
        Some(Object::Ref(ptr)) if unsafe { (**ptr).type_tag } == TypeTag::DICT as u8 => Ok(*ptr),
        other => Err(format!("TypeError: {} expects dict, got {}",
            who, other.map(|o| o.type_name()).unwrap_or("missing"))),
    }
}

/// 校验首参数为 Set Ref，返回裸指针。
fn expect_set_ref(arg: Option<&Object>, who: &str) -> Result<*mut MsObjHeader, String> {
    match arg {
        Some(Object::Ref(ptr)) if unsafe { (**ptr).type_tag } == TypeTag::SET as u8 => Ok(*ptr),
        other => Err(format!("TypeError: {} expects set, got {}",
            who, other.map(|o| o.type_name()).unwrap_or("missing"))),
    }
}

/// 校验参数为 callable（Function/Closure/BoundMethod Ref）。
/// 本 MVP 检查 type_tag 为 FUNCTION 或 CLOSURE 或 BOUND_METHOD。
fn expect_callable(arg: Option<&Object>, who: &str) -> Result<Object, String> {
    match arg {
        Some(o @ Object::Ref(ptr)) => {
            let tag = unsafe { (**ptr).type_tag };
            if tag == TypeTag::FUNCTION as u8
                || tag == TypeTag::CLOSURE as u8
                || tag == TypeTag::BOUND_METHOD as u8
            { Ok(o.clone()) }
            else {
                Err(format!("TypeError: {} expects callable, got {}",
                    who, o.type_name()))
            }
        }
        other => Err(format!("TypeError: {} expects callable, got {}",
            who, other.map(|o| o.type_name()).unwrap_or("missing"))),
    }
}

/// 列表索引归一化（负索引相对末尾，越界返回 None）。
/// 与 task 50 §3.5 slice 的 norm 语义一致；list.pop/insert 共用。
fn normalize_index(i: i64, len: usize) -> Option<usize> {
    let len = len as i64;
    let n = if i < 0 { len + i } else { i };
    if n < 0 || n >= len {
        None
    } else {
        Some(n as usize)
    }
}

/// 可哈希键校验 + 哈希值计算（供 Set 元素 / Dict 键复用）。
/// 遵循 02-types.md:339-350：仅 int/float/bool/string/nil/tuple 可哈希；
/// NaN 抛 TypeError；-0.0 与 0.0 哈希值相同。
fn hash_key(obj: &Object) -> Result<u64, String> {
    match obj {
        Object::Nil => Ok(0),
        Object::Bool(b) => Ok(if *b { 1 } else { 0 }),
        Object::Int(n) => Ok(*n as u64),
        Object::Float(f) => {
            if f.is_nan() {
                Err("TypeError: unhashable type: NaN".to_string())
            } else {
                // -0.0 与 0.0 同哈希（02-types.md:352）
                Ok((*f).to_bits() as u64)
            }
        }
        Object::Ref(ptr) => {
            let tag = unsafe { (**ptr).type_tag };
            if tag == TypeTag::STRING as u8 || tag == TypeTag::TUPLE as u8 {
                // STRING/TUPLE 的哈希复用 task 20/22 已注册的 hash 实现
                Ok(unsafe { hash_heap_obj(*ptr) })
            } else {
                Err(format!("TypeError: unhashable type: '{}'", obj.type_name()))
            }
        }
    }
}

/// 两键相等判定（配合 hash_key，处理 -0.0 == 0.0 语义）。
fn key_eq(a: &Object, b: &Object) -> bool {
    a == b  // Object: PartialEq 已在 task 20/21 实现（含 -0.0 == 0.0）
}
```

> **Set/Dict 底层选型**：`02-types.md:339-350` 限定可哈希类型，但 Object 枚举不区分。**禁止直接 `HashSet<Object>`**（若 Object impl Hash 基于指针则两个相等 list 被视为不同元素，违反集合语义；若未 impl 则编译失败）。Set/Dict 内部须用自定义容器包装 `hash_key`/`key_eq`（如 `Map<Object, ()>` + 自定义 Hasher，或 `Vec<Object>` + 线性查找）。本任务假设 task 20/22 已提供满足此约束的 `SetMap`/`DictMap` 类型，`add`/`insert` 在底层调用 `hash_key` 校验。

### 2. List 标量方法实现

每个方法是独立的 `native_list_xxx` 函数，签名 `fn(&mut VM, &[Object]) -> Result<Object, String>`。`args[0]` 是 receiver（List Ref，BoundMethod 注入），用户参数从 `args.get(1)` 起。

```rust
fn native_list_length(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_list_ref(args.get(0), "length()")?;
    let len = unsafe { read_list(ptr) }.len();
    Ok(Object::Int(len as i64))
}

fn native_list_push(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_list_ref(args.get(0), "push(value)")?;
    // 先取出 val（clone）再 push，避免 args 与 receiver 别名（如 lst.push(lst)）
    // 时 Vec 扩容导致 args 内 Ref 悬垂。
    let val = args.get(1).cloned()
        .ok_or_else(|| "TypeError: push(value) requires 1 argument".to_string())?;
    // SAFETY: expect_list_ref 校验 type_tag 为 LIST。
    unsafe { read_list(ptr) }.push(val);
    Ok(Object::Nil)
}

fn native_list_pop(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_list_ref(args.get(0), "pop(index?)")?;
    // 先 clone 长度判断所需信息，避免 read_list 长期借用与可能的扩容/GC 冲突。
    let len = unsafe { read_list(ptr) }.len();
    let idx = if args.len() <= 1 {
        // 无参：弹出末尾元素
        if len == 0 {
            return Err("IndexError: pop from empty list".to_string());
        }
        len - 1
    } else {
        let i = expect_int(args.get(1), "pop(index?)")?;
        // 负索引支持（M6/V3）：normalize_index 已处理 i<0 与越界。
        normalize_index(i, len)
            .ok_or_else(|| format!("IndexError: pop index {} out of range for length {}",
                i, len))?
    };
    // SAFETY: idx 已校验 < len。read_list 取 &mut 后立即 remove 并返回所有权。
    let popped = unsafe { read_list(ptr) }.remove(idx);
    Ok(popped)
}

fn native_list_insert(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_list_ref(args.get(0), "insert(index, value)")?;
    let i = expect_int(args.get(1), "insert(index, value)")?;
    let val = args.get(2).cloned()
        .ok_or_else(|| "TypeError: insert(index, value) requires 2 arguments".to_string())?;
    let len = unsafe { read_list(ptr) }.len();
    // 负索引支持（M6/V3）；insert 允许 idx == len（等价于 push）。
    let n = if i < 0 { len as i64 + i } else { i };
    if n < 0 || n > len as i64 {
        return Err(format!(
            "IndexError: insert index {} out of range for length {}", i, len
        ));
    }
    // SAFETY: n 已校验 0..=len。
    unsafe { read_list(ptr) }.insert(n as usize, val);
    Ok(Object::Nil)
}

fn native_list_remove(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_list_ref(args.get(0), "remove(value)")?;
    let val = args.get(1).cloned()
        .ok_or_else(|| "TypeError: remove(value) requires 1 argument".to_string())?;
    // 在借用内查找位置（position 返回 Option<usize>），释放借用后再 remove。
    // Object: PartialEq 由 task 20/21 提供。
    let found_idx = {
        let list = unsafe { read_list(ptr) };
        list.iter().position(|x| x == &val)
    };
    match found_idx {
        Some(idx) => {
            // SAFETY: idx 由 position 保证 < list.len()。
            let _removed = unsafe { read_list(ptr) }.remove(idx);
            Ok(Object::Nil)
        }
        None => Err("ValueError: remove(): value not in list".to_string()),
    }
}

fn native_list_index(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_list_ref(args.get(0), "index(value)")?;
    let val = args.get(1).cloned()
        .ok_or_else(|| "TypeError: index(value) requires 1 argument".to_string())?;
    // SAFETY: expect_list_ref 校验 type_tag 为 LIST。
    let list = unsafe { read_list(ptr) };
    match list.iter().position(|x| x == &val) {
        Some(idx) => Ok(Object::Int(idx as i64)),
        None => Err("ValueError: index(): value not in list".to_string()),
    }
}

fn native_list_contains(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_list_ref(args.get(0), "contains(value)")?;
    let val = args.get(1).cloned()
        .ok_or_else(|| "TypeError: contains(value) requires 1 argument".to_string())?;
    // SAFETY: expect_list_ref 校验 type_tag 为 LIST。
    let found = unsafe { read_list(ptr) }.iter().any(|x| x == &val);
    Ok(Object::Bool(found))
}
```

> **`remove(value)` 未找到返回 `ValueError`**（与 `list.index` 一致；Python `list.remove` 同样抛 `ValueError`）。**注意 `remove` 按 `==` 比较删除首个匹配**，区别于 `pop(index)` 按位置删除。

### 2b. List 排序 / 反转 / 切片

```rust
fn native_list_sort(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_list_ref(args.get(0), "sort()")?;
    // 先 clone 出元素排序，再写回（避免 long borrow 与 GC/扩容冲突）。
    // sort 返回 nil（原地排序）；这里 clone + 排序 + swap 回去。
    let mut items = unsafe { read_list(ptr) }.clone();
    // CmpOp 与 OpCode 解耦（task 21，object.rs:392/612），
    // 与 task 26 §sorted 同模式：比较失败须上抛 TypeError，不可静默错序。
    let mut err: Option<String> = None;
    items.sort_by(|a, b| {
        if err.is_some() { return std::cmp::Ordering::Equal; }
        match a.compare(b, CmpOp::Less) {
            Ok(Object::Bool(true)) => std::cmp::Ordering::Less,
            Ok(_) => match a.compare(b, CmpOp::Greater) {
                Ok(Object::Bool(true)) => std::cmp::Ordering::Greater,
                Ok(_) => std::cmp::Ordering::Equal,
                Err(e) => { err = Some(e); std::cmp::Ordering::Equal }
            },
            Err(e) => { err = Some(e); std::cmp::Ordering::Equal }
        }
    });
    if let Some(e) = err { return Err(e); }
    // SAFETY: 排序后写回原 list（保持身份一致）。
    unsafe { read_list(ptr) }.clear();
    unsafe { read_list(ptr) }.extend(items);
    Ok(Object::Nil)
}

fn native_list_reverse(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_list_ref(args.get(0), "reverse()")?;
    // SAFETY: expect_list_ref 校验 type_tag 为 LIST。
    unsafe { read_list(ptr) }.reverse();
    Ok(Object::Nil)
}

fn native_list_slice(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_list_ref(args.get(0), "slice(start, end?)")?;
    let start_i = expect_int(args.get(1), "slice(start, end?)")?;
    let end_opt = if args.len() > 2 {
        Some(expect_int(args.get(2), "slice(start, end?)")?)
    } else { None };
    // 先 clone 释放借用（与 task 50 §3.5 slice 一致语义：字符位置 → 元素位置）。
    let items = unsafe { read_list(ptr) }.clone();
    let len = items.len() as i64;
    let norm = |i: i64| -> i64 {
        if i < 0 { (len + i).max(0) } else { i.min(len) }
    };
    let s = norm(start_i);
    let e = match end_opt { Some(i) => norm(i), None => len };
    if s > e {
        return Err(format!("ValueError: slice start {} > end {}", s, e));
    }
    let sliced: Vec<Object> = items[s as usize..e as usize].to_vec();
    Ok(alloc_list(sliced))
}
```

> **`sort()` 使用 CmpOp 解耦**（`26-builtins-iterators.md:318-343`）：禁止 `a.cmp(b)`（Object 含 Float NaN 与 Ref，不 impl Ord；混类型排序须抛 TypeError）。`sort()` 返回 nil（原地），与设计规格 `lst.sort() -> nil` 一致。
>
> **`slice` 语义与 task 50 §3.5 一致**：负索引相对末尾，越界饱和到 `[0, len]`，`start > end`（归一化后）抛 ValueError。

### 3. List 高阶方法（map / filter / reduce）

> **借用约束**（R1/R3）：回调 `vm.call_function` 可能触发分配（→ GC）或递归访问同一 list。必须先 `clone` 出元素，释放 `read_list` 的 `&mut` 借用，再迭代调用回调。

```rust
fn native_list_map(vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_list_ref(args.get(0), "map(fn)")?;
    let fn_obj = expect_callable(args.get(1), "map(fn)")?;
    // 先 clone 释放借用（R1）：回调内可能触发 GC 或再次访问此 list。
    let items = unsafe { read_list(ptr) }.clone();
    let mut result = Vec::with_capacity(items.len());
    for item in items.iter() {
        let mapped = vm.call_function(&fn_obj, vec![item.clone()])?;
        result.push(mapped);
    }
    Ok(alloc_list(result))  // 注意：传 Vec 所有权（V6：非 &result）
}

fn native_list_filter(vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_list_ref(args.get(0), "filter(fn)")?;
    let fn_obj = expect_callable(args.get(1), "filter(fn)")?;
    let items = unsafe { read_list(ptr) }.clone();
    let mut result = Vec::new();
    for item in items.iter() {
        let cond = vm.call_function(&fn_obj, vec![item.clone()])?;
        if cond.is_truthy() {
            result.push(item.clone());
        }
    }
    Ok(alloc_list(result))
}

fn native_list_reduce(vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_list_ref(args.get(0), "reduce(fn, init?)")?;
    let fn_obj = expect_callable(args.get(1), "reduce(fn, init?)")?;
    let items = unsafe { read_list(ptr) }.clone();
    let (mut acc, start) = if args.len() > 2 {
        // reduce(fn, init)：acc 从 init 起，迭代全部元素
        (args.get(2).cloned().unwrap(), 0)
    } else {
        // reduce(fn)：acc 从首元素起，跳过首元素
        if items.is_empty() {
            return Err(
                "ValueError: reduce() of empty list with no initial value".to_string()
            );
        }
        (items[0].clone(), 1)
    };
    for item in items.iter().skip(start) {
        acc = vm.call_function(&fn_obj, vec![acc, item.clone()])?;
    }
    Ok(acc)
}
```

> **`reduce` 空列表无 init 抛 ValueError**（与 Python `TypeError: reduce() of empty sequence` 一致；mslang 既有惯例用 ValueError 表值非法）。**单元素列表无 init** 返回首元素（正确）。

### 4. Dict 方法实现

```rust
fn native_dict_length(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_dict_ref(args.get(0), "length()")?;
    Ok(Object::Int(unsafe { read_dict(ptr) }.len() as i64))
}

fn native_dict_keys(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_dict_ref(args.get(0), "keys()")?;
    // 返回 list 快照（设计规格 keys() -> list）；大 dict 全量克隆，见 R4。
    let keys: Vec<Object> = unsafe { read_dict(ptr) }.keys().cloned().collect();
    Ok(alloc_list(keys))
}

fn native_dict_values(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_dict_ref(args.get(0), "values()")?;
    let vals: Vec<Object> = unsafe { read_dict(ptr) }.values().cloned().collect();
    Ok(alloc_list(vals))
}

fn native_dict_items(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_dict_ref(args.get(0), "items()")?;
    // items 返回 (key, value) tuple list；保持插入顺序（02-types.md:191）。
    let items: Vec<Object> = unsafe { read_dict(ptr) }.items().iter()
        .map(|(k, v)| alloc_tuple(vec![(*k).clone(), (*v).clone()]))
        .collect();
    Ok(alloc_list(items))
}

fn native_dict_get(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_dict_ref(args.get(0), "get(key, default?)")?;
    let key = args.get(1).cloned()
        .ok_or_else(|| "TypeError: get(key, default?) requires 1-2 arguments".to_string())?;
    // 可哈希校验（02-types.md:339-350）：list/dict/set 作键抛 TypeError。
    hash_key(&key)?;
    let default = if args.len() > 2 { args.get(2).cloned().unwrap() } else { Object::Nil };
    let dict = unsafe { read_dict(ptr) };
    // read-only 查找，借用安全。
    Ok(dict.get(&key).cloned().unwrap_or(default))
}

fn native_dict_set(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_dict_ref(args.get(0), "set(key, value)")?;
    let key = args.get(1).cloned()
        .ok_or_else(|| "TypeError: set(key, value) requires 2 arguments".to_string())?;
    let val = args.get(2).cloned()
        .ok_or_else(|| "TypeError: set(key, value) requires 2 arguments".to_string())?;
    // 可哈希校验（V4：拒绝 list/dict/set 作键）。
    hash_key(&key)?;
    // SAFETY: expect_dict_ref 校验 type_tag 为 DICT。
    unsafe { read_dict(ptr) }.insert(key, val);
    Ok(Object::Nil)
}

fn native_dict_remove(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_dict_ref(args.get(0), "remove(key)")?;
    let key = args.get(1).cloned()
        .ok_or_else(|| "TypeError: remove(key) requires 1 argument".to_string())?;
    hash_key(&key)?;
    // M4：键不存在抛 KeyError（02-types.md:187 明确要求）。
    if unsafe { read_dict(ptr) }.remove(&key).is_none() {
        return Err("KeyError: key not found".to_string());
    }
    Ok(Object::Nil)
}

fn native_dict_contains(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_dict_ref(args.get(0), "contains(key)")?;
    let key = args.get(1).cloned()
        .ok_or_else(|| "TypeError: contains(key) requires 1 argument".to_string())?;
    hash_key(&key)?;
    let found = unsafe { read_dict(ptr) }.contains_key(&key);
    Ok(Object::Bool(found))
}

fn native_dict_merge(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_dict_ref(args.get(0), "merge(other)")?;
    let other_ptr = expect_dict_ref(args.get(1), "merge(other)")?;
    // V2 自引用防护：先 clone other 全部 (k,v)，释放借用，再插入 receiver。
    // 否则 d.merge(d) 在迭代 dict 时修改 dict → HashMap 结构破坏 / 无限循环。
    let pairs: Vec<(Object, Object)> = unsafe { read_dict(other_ptr) }.items().iter()
        .map(|(k, v)| ((*k).clone(), (*v).clone()))
        .collect();
    for (k, v) in pairs {
        // SAFETY: other_ptr 借用已释放。
        unsafe { read_dict(ptr) }.insert(k, v);
    }
    Ok(Object::Nil)
}
```

> **`d.remove(key)` 键不存在抛 `KeyError`**（`02-types.md:187`），与 `d[key]` 返回 nil 的宽松语义区分。`d.contains(key)` 与 `d.get(key, default?)` 用于安全探测。
>
> **Dict 键可哈希校验**（V4）：所有写入路径（`set`/`remove`/`merge`/`get`/`contains`）均经 `hash_key` 校验，拒绝 list/dict/set 作键，拒绝 NaN（`02-types.md:352`）。

### 5. Set 方法实现

```rust
fn native_set_length(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_set_ref(args.get(0), "length()")?;
    Ok(Object::Int(unsafe { read_set(ptr) }.len() as i64))
}

fn native_set_add(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_set_ref(args.get(0), "add(value)")?;
    let val = args.get(1).cloned()
        .ok_or_else(|| "TypeError: add(value) requires 1 argument".to_string())?;
    // V4 可哈希校验：拒绝 list/dict/set/NaN 作元素（02-types.md:339-352）。
    hash_key(&val)?;
    // SAFETY: expect_set_ref 校验 type_tag 为 SET。
    unsafe { read_set(ptr) }.insert(val);
    Ok(Object::Nil)
}

fn native_set_remove(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_set_ref(args.get(0), "remove(value)")?;
    let val = args.get(1).cloned()
        .ok_or_else(|| "TypeError: remove(value) requires 1 argument".to_string())?;
    hash_key(&val)?;
    // M4：元素不存在抛 KeyError（10-builtins.md:246 明确要求）。
    if unsafe { read_set(ptr) }.remove(&val).is_none() {
        return Err("KeyError: element not found".to_string());
    }
    Ok(Object::Nil)
}

fn native_set_contains(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_set_ref(args.get(0), "contains(value)")?;
    let val = args.get(1).cloned()
        .ok_or_else(|| "TypeError: contains(value) requires 1 argument".to_string())?;
    // contains 对不可哈希类型返回 false（与 dict.contains 一致）；
    // 不抛错以便 in 运算符等场景使用。
    let found = if hash_key(&val).is_ok() {
        unsafe { read_set(ptr) }.contains(&val)
    } else {
        false
    };
    Ok(Object::Bool(found))
}

fn native_set_union(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_set_ref(args.get(0), "union(other)")?;
    let other_ptr = expect_set_ref(args.get(1), "union(other)")?;
    // V2 自引用防护：先 clone 双方，避免 s.union(s) 在迭代时修改。
    let a = unsafe { read_set(ptr) }.clone();
    let b = unsafe { read_set(other_ptr) }.clone();
    // 集合运算；底层 SetMap 已用 hash_key/key_eq（§1b）。
    let result: SetMap = a.union(&b).cloned().collect();
    Ok(alloc_set(result))
}

fn native_set_intersection(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_set_ref(args.get(0), "intersection(other)")?;
    let other_ptr = expect_set_ref(args.get(1), "intersection(other)")?;
    let a = unsafe { read_set(ptr) }.clone();
    let b = unsafe { read_set(other_ptr) }.clone();
    let result: SetMap = a.intersection(&b).cloned().collect();
    Ok(alloc_set(result))
}

fn native_set_difference(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_set_ref(args.get(0), "difference(other)")?;
    let other_ptr = expect_set_ref(args.get(1), "difference(other)")?;
    let a = unsafe { read_set(ptr) }.clone();
    let b = unsafe { read_set(other_ptr) }.clone();
    let result: SetMap = a.difference(&b).cloned().collect();
    Ok(alloc_set(result))
}
```

> **`s.remove(val)` 元素不存在抛 `KeyError`**（`10-builtins.md:246`）。`s.contains(val)` 对不可哈希类型返回 `false`（不抛错），以支持 `val in s` 运算符的宽松语义。
>
> **Set 运算符关系**（L6，`02-types.md:239-242`）：`s1 | s2`、`s1 & s2`、`s1 - s2`、`s1 ^ s2` 运算符由 OpCode `BIT_OR`/`BIT_AND`/`BIT_XOR`/`SUBTRACT` 在 Set receiver 上分派，**应复用本节 `union/intersection/difference` 实现**（或在 OpCode 侧直接调用 `native_set_union` 等）。**对称差 `^` 无对应方法**（设计规格未定义 `s.symmetric_difference(other)`），归属本任务的运算符实现需补齐；若仅做方法则 `^` 运算符归后续任务（建议在本任务验证标准中标注）。

### 6. GET_ATTR 集成（src/vm/mod.rs::OpCode::GetAttr 分支）

在 `OpCode::GetAttr` 处理逻辑中，紧随 task 50 的 `STRING` 分支（`mod.rs` 中 `TypeTag::STRING` 分支之后）插入 `LIST`/`DICT`/`SET` 三个 `type_tag` 分支。**必须**在 catch-all `_` 之前匹配（与 task 50 §2 同位置约定）。

```rust
Object::Ref(ptr)
    if unsafe { (**ptr).type_tag } == TypeTag::LIST as u8 =>
{
    match stdlib::lookup_list_method(&attr) {
        Some(func) => {
            // 与 task 46/50 一致：每次 GET_ATTR 分配新 NativeFunction + BoundMethod。
            let method_obj = alloc_native_function(
                NativeFunction { name: attr.clone(), func }
            );
            // SAFETY: alloc_native_function 恒返回 Ref。
            let method_ptr = match method_obj {
                Object::Ref(p) => p,
                _ => unreachable!("alloc_native_function must return Ref"),
            };
            self.push(alloc_bound_method(obj.clone(), method_ptr))?;
        }
        None => {
            return Err(format!(
                "AttributeError: 'list' has no attribute '{}'", attr
            ));
        }
    }
}

Object::Ref(ptr)
    if unsafe { (**ptr).type_tag } == TypeTag::DICT as u8 =>
{
    match stdlib::lookup_dict_method(&attr) {
        Some(func) => {
            let method_obj = alloc_native_function(
                NativeFunction { name: attr.clone(), func }
            );
            let method_ptr = match method_obj {
                Object::Ref(p) => p,
                _ => unreachable!(),
            };
            self.push(alloc_bound_method(obj.clone(), method_ptr))?;
        }
        None => {
            return Err(format!(
                "AttributeError: 'dict' has no attribute '{}'", attr
            ));
        }
    }
}

Object::Ref(ptr)
    if unsafe { (**ptr).type_tag } == TypeTag::SET as u8 =>
{
    match stdlib::lookup_set_method(&attr) {
        Some(func) => {
            let method_obj = alloc_native_function(
                NativeFunction { name: attr.clone(), func }
            );
            let method_ptr = match method_obj {
                Object::Ref(p) => p,
                _ => unreachable!(),
            };
            self.push(alloc_bound_method(obj.clone(), method_ptr))?;
        }
        None => {
            return Err(format!(
                "AttributeError: 'set' has no attribute '{}'", attr
            ));
        }
    }
}
```

后续 `CALL` 经 `BOUND_METHOD → FUNCTION` 路径自动把 receiver（List/Dict/Set Ref）注入为 `args[0]`（见 `mod.rs:2644-2651`，task 50 §2 注释），native 函数据此取 receiver 与用户参数。

> **不使用 `INVOKE` 指令**（M1）：`INVOKE`（`11-bytecode-vm.md:155`）是 class 实例方法调用的优化指令，内置类型方法经 `GET_ATTR` → `BoundMethod` → `CALL` 主路径分派，与 task 46（FileHandle）/ task 50（String）保持一致。



1. List 的 14 个方法全部正确工作（length/push/pop/insert/remove/index/contains/sort/reverse/slice/map/filter/reduce）
2. Dict 的 9 个方法全部正确工作（length/keys/values/items/get/set/remove/contains/merge）
3. Set 的 7 个方法全部正确工作（length/add/remove/contains/union/intersection/difference）
4. `map/filter/reduce` 正确调用回调函数；回调内触发 GC 不导致悬垂引用（R1 借用约束）
5. 空集合的边界情况正确处理（`[].pop()` → IndexError、`[].reduce(fn)` → ValueError、`{}.remove("x")` → KeyError）
6. **负索引支持**（M6）：`lst.pop(-1)` 弹出末尾；`lst.insert(-1, x)` 插入末尾前；`lst.slice(-2)` 取末两元素
7. **错误类型细分**（M5）：TypeError（类型不匹配/不可哈希/非 callable）、ValueError（空 list reduce/slice 反向/value 未找到）、IndexError（索引越界）、KeyError（dict.remove/set.remove 键缺失）、AttributeError（未知方法名）
8. **可哈希性校验**（V4）：`s.add([1,2])` → TypeError；`{[1,2]: 1}` 字面量构造（task 22）与本任务 `d.set([1,2], v)` 均拒绝；`hash(NaN)` → TypeError
9. **自引用安全**（V2）：`d.merge(d)` 不死循环；`s.union(s)` 返回自身拷贝
10. **GET_ATTR 分派**（M1）：`lst.push(1)` 经 GET_ATTR → BoundMethod → CALL 路径，receiver 自动注入为 args[0]
11. `d[key]` 返回 nil（键不存在）vs `d.remove(key)` 抛 KeyError（语义区分，`02-types.md:187`）
12. `.length()` 与全局 `len()` 语义一致（L5，`10-builtins.md:95`：推荐 len()，length() 遗留兼容）

## 关于 Set 运算符（L6 说明）

`02-types.md:239-242` 定义了 Set 运算符 `|`、`&`、`-`、`^`。这些运算符由 OpCode `BIT_OR`/`BIT_AND`/`BIT_XOR`/`SUBTRACT` 在 Set receiver 上分派，**应复用本任务的 `native_set_union/intersection/difference` 实现**。若本任务仅实现方法而运算符归后续任务，须在验证标准中标注"运算符实现 pending"；建议本任务一并实现运算符分派（复用已有方法逻辑）。

对称差 `s ^ other`（`02-types.md:242`）无对应方法（设计规格未定义 `s.symmetric_difference(other)`），运算符实现时须补齐。

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

### test_list_negative_index.ms（M6 负索引）

```ms
lst = [10, 20, 30, 40, 50]
print(lst.pop(-1))      # 50（弹出末尾）
print(lst)              # [10, 20, 30, 40]
lst.insert(-1, 99)      # 在末尾前插入
print(lst)              # [10, 20, 30, 99, 40]
print(lst.slice(-2))    # [99, 40]（末两元素）
print(lst.slice(1, -1)) # [20, 30, 99]（去首尾）
```

预期输出：
```
50
[10, 20, 30, 40]
[10, 20, 30, 99, 40]
[99, 40]
[20, 30, 99]
```

### test_error.ms（错误契约，参照 task 50 §test_string_error.ms）

```ms
# pop 空列表
try { [].pop() } except e { print("pop_empty: " + str(e)) }
# pop 索引越界
try { [1,2].pop(10) } except e { print("pop_oob: " + str(e)) }
# reduce 空列表无 init
try { [].reduce(fn(a,b) { return a+b }) } except e { print("reduce_empty: " + str(e)) }
# remove 未找到
try { [1,2].remove(99) } except e { print("remove_nf: " + str(e)) }
# index 未找到
try { [1,2].index(99) } except e { print("index_nf: " + str(e)) }
# slice 反向
try { [1,2].slice(3, 1) } except e { print("slice_rev: " + str(e)) }
# dict.remove 键不存在 → KeyError
try { {"a": 1}.remove("z") } except e { print("dict_rem: " + str(e)) }
# set.remove 元素不存在 → KeyError
try { {1,2}.remove(99) } except e { print("set_rem: " + str(e)) }
# 不可哈希作 set 元素 → TypeError
try { {1,2}.add([1,2]) } except e { print("set_add: " + str(e)) }
# 不可哈希作 dict 键 → TypeError
try { {"a": 1}.set([1,2], 3) } except e { print("dict_set: " + str(e)) }
# map 非 callable → TypeError
try { [1,2].map(42) } except e { print("map_nc: " + str(e)) }
# 未知方法 → AttributeError
try { [1,2].nosuch() } except e { print("attr: " + str(e)) }
```

预期输出：
```
pop_empty: IndexError: pop from empty list
pop_oob: IndexError: pop index 10 out of range for length 2
reduce_empty: ValueError: reduce() of empty list with no initial value
remove_nf: ValueError: remove(): value not in list
index_nf: ValueError: index(): value not in list
slice_rev: ValueError: slice start 3 > end 1
dict_rem: KeyError: key not found
set_rem: KeyError: element not found
set_add: TypeError: unhashable type: 'list'
dict_set: TypeError: unhashable type: 'list'
map_nc: TypeError: map(fn) expects callable, got int
attr: AttributeError: 'list' has no attribute 'nosuch'
```

> **错误路径备注**（同 task 50 §test_string_error.ms）：当前 VM 中原生函数 `Err(String)` 不可被 try/except 捕获（仅显式 `throw` 可捕获；影响全部 stdlib 模块的既有 VM 限制，非本任务引入）。上述 `.ms` 测试记录错误契约；实际错误验证由 Rust 单元测试直接调用 native 函数完成（参照 task 49/50 §测试用例模式）。

### test_self_reference.ms（V2 自引用安全）

```ms
# d.merge(d) 不死循环
d = {"a": 1}
d.merge(d)
print(d.length())    # 1（自合并无新键）

# s.union(s) 返回自身拷贝
s = {1, 2, 3}
u = s.union(s)
print(u.length())    # 3

# lst.push(lst_x) 其中 lst_x 是独立 list（非自引用，安全）
a = [1, 2]
b = [3, 4]
a.push(b)
print(a.length())    # 3
```

预期输出：
```
1
3
3
```
