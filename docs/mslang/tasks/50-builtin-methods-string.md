# 内置类型方法 - String

## 所属阶段
Phase 6.2e - 标准库

## 前置任务
26-builtins-iterators
41-self-instance-attributes    # BoundMethod 基础设施（alloc_bound_method）
46-stdlib-io                   # lookup_file_method 分派模式（GET_ATTR → BoundMethod）

## 目标
为 mslang 的 String 类型实现所有内置方法，包括大小写转换、分割连接、查找替换、切片等操作。

## 设计规格

参照 [10-builtins](../10-builtins.md) § string 方法：

| 方法 | 签名 | 说明 |
|---|---|---|
| `s.length()` | `() -> int` | 字符串长度 |
| `s.upper()` | `() -> string` | 转大写 |
| `s.lower()` | `() -> string` | 转小写 |
| `s.strip()` | `() -> string` | 去除两端空白 |
| `s.split(sep?)` | `(string?) -> list` | 分割为列表 |
| `s.join(list)` | `(list) -> string` | 用字符串连接列表 |
| `s.replace(old, new)` | `(string, string) -> string` | 替换子串 |
| `s.contains(sub)` | `(string) -> bool` | 是否包含子串 |
| `s.startswith(prefix)` | `(string) -> bool` | 是否以指定前缀开头 |
| `s.endswith(suffix)` | `(string) -> bool` | 是否以指定后缀结尾 |
| `s.index(sub)` | `(string) -> int` | 查找子串位置 |
| `s.slice(start, end?)` | `(int, int?) -> string` | 切片 |

## 实现细节

> **对象模型约束**（task 20/25/46/47/48/49）：Object 枚举严格为 `{Nil, Bool, Int, Float, Ref}`，**无 `NativeFn` 变体**。原生函数经 `alloc_native_function(NativeFunction{name, func})` 包装为 `Object::Ref` + `TypeTag::FUNCTION`。`NativeFn` 签名为 `fn(&mut VM, &[Object]) -> Result<Object, String>`（切片，非 Vec）。字符串参数校验复用 task 46 的 `expect_string(arg: Option<&Object>, who: &str) -> Result<String, String>`（`src/vm/stdlib.rs:1330-1342`），**必须用 `args.get(N)`** 而非 `args[N]`。所有错误统一 `Result<_, String>`，错误消息前缀遵循 `"<ErrorType>: ..."`（`TypeError`/`ValueError`/`AttributeError`），与 task 46-49 一致。
>
> **Receiver 注入约定**（task 46 模式）：String 方法经 `GET_ATTR` 返回 `BoundMethod{receiver, method}`，后续 `CALL` 自动把 receiver 注入为 `args[0]`（见 `src/vm/mod.rs:2644-2651`）。因此 **native 函数内 `args[0]` 是 String receiver，用户参数从 `args[1]` 起**。所有方法实现需遵守此偏移规则。
>
> **位置语义统一**（关键约束）：`length()`、`index()`、`slice()` 三者均按**字符位置**（Unicode scalar，`chars().count()`），**非字节位置**。三者结果可互相对应（`s.slice(s.index(sub), ...)` 必正确）。与 `02-types.md:107-126` UTF-8 字符串语义一致。

### 1. 方法分发表（FileHandle 模式）

参照 task 46 的 `lookup_file_method`（`src/vm/stdlib.rs:121-139`），在 `src/vm/stdlib.rs` 实现：

```rust
/// String 方法名 → 原生函数（供 GET_ATTR 包装为 BoundMethod）。
/// 每次 GET_ATTR 由调用方 alloc_native_function 分配新对象（与 task 46
/// lookup_file_method 模式一致；性能优化留待 task 52+ 后 intern 表方案）。
pub fn lookup_string_method(name: &str) -> Option<NativeFn> {
    let func: NativeFn = match name {
        "length" => native_str_length,
        "upper" => native_str_upper,
        "lower" => native_str_lower,
        "strip" => native_str_strip,
        "split" => native_str_split,
        "join" => native_str_join,
        "replace" => native_str_replace,
        "contains" => native_str_contains,
        "startswith" => native_str_startswith,
        "endswith" => native_str_endswith,
        "index" => native_str_index,
        "slice" => native_str_slice,
        _ => return None,
    };
    Some(func)
}
```

GET_ATTR 侧（`src/vm/mod.rs`）负责 `alloc_native_function(NativeFunction{name, func})` 包装并 `alloc_bound_method` 绑定 receiver（见 §2b）。

### 1b. 新增辅助函数（stdlib.rs §辅助函数 段，紧邻 `expect_string:1330`）

```rust
/// 从预期为 Int 的参数提取 i64。Bool 不自动转 Int（与 task 47 expect_number 不同；
/// String.slice 等仅接受 int）。
fn expect_int(arg: Option<&Object>, who: &str) -> Result<i64, String> {
    match arg {
        Some(Object::Int(n)) => Ok(*n),
        other => Err(format!("TypeError: {} expects int, got {}",
            who, other.map(|o| o.type_name()).unwrap_or("missing"))),
    }
}

/// 校验首参数为 List Ref，返回裸指针。调用方 unsafe read_list 取内容
/// （借用约束：递归前必须释放 &mut Vec<Object>，参见 task 49 §3）。
fn expect_list_ref(arg: Option<&Object>, who: &str) -> Result<*mut MsObjHeader, String> {
    match arg {
        Some(Object::Ref(ptr)) if unsafe { (**ptr).type_tag } == TypeTag::LIST as u8 => Ok(*ptr),
        other => Err(format!("TypeError: {} expects list, got {}",
            who, other.map(|o| o.type_name()).unwrap_or("missing"))),
    }
}
```

### 2. GET_ATTR 集成（src/vm/mod.rs::OpCode::GetAttr 分支）

在 `OpCode::GetAttr` 处理逻辑中，紧随 `FILE_HANDLE` 分支（`mod.rs:2646-2660`）之后、catch-all `_`（`mod.rs:2661-2667`）之前插入 STRING 分支。**必须**在 catch-all 之前匹配；DICT 临时分支（`mod.rs:2512`）在更早位置，不影响。

```rust
Object::Ref(ptr)
    if unsafe { (**ptr).type_tag } == TypeTag::STRING as u8 =>
{
    match stdlib::lookup_string_method(&attr) {
        Some(func) => {
            // 与 task 46 FileHandle 一致：每次 GET_ATTR 分配新 NativeFunction
            // + BoundMethod。性能优化（intern 表 / INVOKE 直分派）留待 task 52+。
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
                "AttributeError: 'string' has no attribute '{}'", attr
            ));
        }
    }
}
```

后续 `CALL` 经 `BOUND_METHOD → FUNCTION` 路径自动把 receiver（String Ref）注入为 `args[0]`（见 `mod.rs:2644-2651` 注释），native 函数据此取 receiver 与用户参数。

### 3. 各方法实现细节

每个方法是独立的 `native_str_xxx` 函数，签名 `fn(&mut VM, &[Object]) -> Result<Object, String>`。`args[0]` 是 receiver（String Ref，BoundMethod 注入），用户参数从 `args.get(1)` 起。

#### 3.0 标量方法（length / upper / lower / strip）

```rust
fn native_str_length(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let s = expect_string(args.get(0), "length()")?;
    // 字符位置（Unicode scalar），非字节数。与 index/slice 一致。
    Ok(Object::Int(s.chars().count() as i64))
}
fn native_str_upper(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let s = expect_string(args.get(0), "upper()")?;
    Ok(alloc_string(&s.to_uppercase()))
}
fn native_str_lower(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let s = expect_string(args.get(0), "lower()")?;
    Ok(alloc_string(&s.to_lowercase()))
}
fn native_str_strip(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let s = expect_string(args.get(0), "strip()")?;
    // Rust trim() 按 Unicode White_Space（与 Python str.strip() 一致）。
    Ok(alloc_string(s.trim()))
}
```

#### 3.1 split（无参按 Unicode 空白；空分隔符报错）

```rust
fn native_str_split(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let recv = expect_string(args.get(0), "split(sep?)")?;
    let parts: Vec<Object> = if args.len() <= 1 {
        // 无参：按 Unicode White_Space 分割（Python str.split() 语义；
        // Rust split_whitespace 自动忽略前后与连续空白）。
        recv.split_whitespace().map(alloc_string).collect()
    } else {
        let sep = expect_string(args.get(1), "split(sep?)")?;
        if sep.is_empty() {
            // Python: ValueError: empty separator。Rust str::split("") 会返回
            // 含边界空串的怪异结果（["", "a", "b", ""]），故显式拒绝。
            return Err("ValueError: empty separator".to_string());
        }
        recv.split(&sep).map(alloc_string).collect()
    };
    Ok(alloc_list(parts))  // 注意：传 Vec 所有权，非 &parts
}
```

#### 3.2 join（强制 list 元素为 string）

```rust
fn native_str_join(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let sep = expect_string(args.get(0), "join(list)")?;
    let list_ptr = expect_list_ref(args.get(1), "join(list)")?;
    // SAFETY: expect_list_ref 校验 type_tag 为 LIST。借用约束：递归/分配前
    // 释放 &mut Vec<Object>，故先 clone 出元素。
    let items: Vec<Object> = unsafe { read_list(list_ptr) }.clone();
    // 强制 list 元素均为 string（Python 语义）；静默 Display 转换会引入
    // 风格化输出（如 3.0 而非 3）与类型混淆漏洞。
    let strs: Vec<String> = items.iter().map(|o| {
        match o {
            Object::Ref(p) if unsafe { (**p).type_tag } == TypeTag::STRING as u8 => {
                // SAFETY: type_tag 为 STRING。
                Ok(unsafe { read_str(*p) }.to_owned())
            }
            other => Err(format!(
                "TypeError: join() expects list of strings, got {}",
                other.type_name()
            )),
        }
    }).collect::<Result<_, _>>()?;
    Ok(alloc_string(&strs.join(&sep)))
}
```

#### 3.3 replace / contains / startswith / endswith

```rust
fn native_str_replace(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let recv = expect_string(args.get(0), "replace(old, new)")?;
    let old = expect_string(args.get(1), "replace(old, new)")?;
    let new = expect_string(args.get(2), "replace(old, new)")?;
    // 全部替换（spec 未要求 count 参数；与 Python str.replace(old, new, -1) 一致）。
    Ok(alloc_string(recv.replace(&old, &new)))
}
fn native_str_contains(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let recv = expect_string(args.get(0), "contains(sub)")?;
    let sub = expect_string(args.get(1), "contains(sub)")?;
    Ok(Object::Bool(recv.contains(&sub)))
}
fn native_str_startswith(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let recv = expect_string(args.get(0), "startswith(prefix)")?;
    let pfx = expect_string(args.get(1), "startswith(prefix)")?;
    Ok(Object::Bool(recv.starts_with(&pfx)))
}
fn native_str_endswith(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let recv = expect_string(args.get(0), "endswith(suffix)")?;
    let sfx = expect_string(args.get(1), "endswith(suffix)")?;
    Ok(Object::Bool(recv.ends_with(&sfx)))
}
```

#### 3.4 index（字符位置，与 slice 一致）

```rust
fn native_str_index(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let recv = expect_string(args.get(0), "index(sub)")?;
    let sub = expect_string(args.get(1), "index(sub)")?;
    match recv.find(&sub) {
        Some(byte_pos) => {
            // find 返回字节位置，转字符位置（与 length/slice 一致）。
            // 例："日本語".find("本") 字节位置 3 → 字符位置 1。
            let char_pos = recv[..byte_pos].chars().count() as i64;
            Ok(Object::Int(char_pos))
        }
        None => Err(format!("ValueError: substring '{}' not found", sub)),
    }
}
```

> **不匹配子串返回 ValueError**（与 Python str.index 一致；spec §验证标准 #4 要求"index 找不到子串时抛出错误"，错误类型选 ValueError 符合 mslang 既有惯例）。

#### 3.5 slice（字符位置 + 负索引 + 越界饱和）

```rust
fn native_str_slice(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let recv = expect_string(args.get(0), "slice(start, end?)")?;
    let start_i = expect_int(args.get(1), "slice(start, end?)")?;
    let end_opt = if args.len() > 2 {
        Some(expect_int(args.get(2), "slice(start, end?)")?)
    } else { None };
    let chars: Vec<char> = recv.chars().collect();
    let len = chars.len() as i64;
    // Python 语义：负索引相对末尾；越界饱和到 [0, len]（不报错，与 Python 一致）。
    let norm = |i: i64| -> i64 {
        if i < 0 { (len + i).max(0) } else { i.min(len) }
    };
    let s = norm(start_i);
    let e = match end_opt { Some(i) => norm(i), None => len };
    if s > e {
        // start > end（归一化后）：Python 返回空串；mslang 选择显式报错以暴露逻辑错误。
        return Err(format!(
            "ValueError: slice start {} > end {}", s, e
        ));
    }
    let result: String = chars[s as usize..e as usize].iter().collect();
    Ok(alloc_string(&result))
}
```

> **位置语义统一**：`length()` / `index()` / `slice()` 全部按字符位置（Unicode scalar）。`s.slice(s.index(sub), s.index(sub) + n)` 等组合用法保证正确。
>
> **越界饱和策略**：`slice(100, 200)` 在长度 5 的字符串上返回空串（start/end 均饱和到 len=5），不 panic。`slice(-1)` 返回最后字符。与 Python 一致。

## 验证标准

1. 所有 12 个字符串方法正确工作
2. Unicode 字符串正确处理（length / index / slice 使用字符位置，非字节）
3. `split` 无参数时按 Unicode 空白分割；`split("")` 抛 `ValueError: empty separator`
4. `index` 找不到子串时抛 `ValueError: substring '...' not found`
5. `slice` 参数越界时**饱和到 [0, len]**（返回空串，不 panic）；负索引相对末尾；`start > end`（归一化后）抛 `ValueError`
6. 对非 String 类型调用字符串方法返回 `AttributeError`
7. `join` 强制 list 元素为 string，否则 `TypeError`
8. `length`/`index`/`slice` 位置语义**互相对应**：`s.slice(s.index(sub), ...)` 必正确（Unicode 一致）
9. 错误消息前缀统一：`TypeError`（类型不匹配）/ `ValueError`（空分隔符、子串未找到、slice 反向）/ `AttributeError`（未知方法名）

## 测试用例

### test_string_methods.ms

```ms
s = "Hello World"
print(s.lower())
print(s.upper())
print("  trim  ".strip())
print("a,b,c".split(","))
print("-".join(["a", "b", "c"]))
print("hello".replace("l", "r"))
print("hello".contains("ell"))
print("hello".startswith("hel"))
print("hello".endswith("llo"))
```

预期输出：
```
hello world
HELLO WORLD
trim
[a, b, c]
a-b-c
herro
true
true
true
```

### test_string_advanced.ms

```ms
print("hello".length())
print("hello".index("ll"))
print("hello".slice(1, 3))
print("a  b   c".split())
print("日本語".length())
print("日本語".slice(0, 2))
```

预期输出：
```
5
2
el
[a, b, c]
3
日本
```

### test_string_unicode.ms（字符位置一致性）

```ms
# index 返回字符位置（非字节位置），与 slice/length 一致
print("日本語".index("本"))      # 字符位置 1（非字节位置 3）
# length/index/slice 组合使用：截取从"本"开始的 1 个字符
i = "日本語".index("本")
print("日本語".slice(i, i + 1))  # 本
# 负索引：相对末尾
print("hello".slice(-1))         # o
print("hello".slice(1, -1))      # ell（去首尾）
# 越界饱和：返回空串，不报错
print("hello".slice(100, 200))   # （空行）
```

预期输出：
```
1
本
o
ell

```

### test_string_error.ms（错误路径）

```ms
# split 空分隔符
try {
    "abc".split("")
} except e {
    print("split: " + str(e))
}
# index 未找到
try {
    "hello".index("xx")
} except e {
    print("index: " + str(e))
}
# join 非 string 元素
try {
    "-".join([1, 2, 3])
} except e {
    print("join: " + str(e))
}
# slice 反向（归一化后 start > end）
try {
    "hello".slice(3, 1)
} except e {
    print("slice: " + str(e))
}
# 未知方法
try {
    "hello".nosuch()
} except e {
    print("attr: " + str(e))
}
```

预期输出：
```
split: ValueError: empty separator
index: ValueError: substring 'xx' not found
join: TypeError: join() expects list of strings, got int
slice: ValueError: slice start 3 > end 1
attr: AttributeError: 'string' has no attribute 'nosuch'
```

> **错误路径备注**：当前 VM 中原生函数 `Err(String)` 不可被 try/except 捕获（仅显式 `throw` 可捕获；这是影响全部 stdlib 模块的既有 VM 限制，非本任务引入）。上述 `.ms` 测试记录的是错误契约；实际错误验证由 Rust 单元测试直接调用 native 完成（参照 task 49 §测试用例模式）。
