# 标准库 - json 模块

## 所属阶段
Phase 6.2d - 标准库

## 前置任务
45-module-system

## 目标
实现 `json` 标准库模块，支持 JSON 字符串解析为 mslang 值，以及 mslang 值序列化为 JSON 字符串。

## 设计规格

参照 [10-builtins](../10-builtins.md) § json：

### json 模块 API

| 函数 | 签名 | 说明 |
|---|---|---|
| `json.parse(string)` | `(string) -> value` | 解析 JSON 字符串为 mslang 值 |
| `json.stringify(value)` | `(value) -> string` | 将 mslang 值序列化为 JSON 字符串 |

### 类型映射

| JSON 类型 | mslang 类型 |
|---|---|
| `null` | `nil` |
| `true` / `false` | `bool` |
| 整数 | `int` |
| 浮点数 | `float` |
| 字符串 | `string` |
| 数组 | `list` |
| 对象 | `dict` |

### 嵌套结构

`json.parse` 和 `json.stringify` 必须正确处理任意层级的嵌套结构。

## 实现细节

> **对象模型约束**（task 20/25/46/47/48）：Object 枚举严格为 `{Nil, Bool, Int, Float, Ref}`，**无 `NativeFn` 变体**。原生函数经 `alloc_native_function(NativeFunction{name, func})` 包装为 `Object::Ref` + `TypeTag::FUNCTION`。`NativeFn` 签名为 `fn(&mut VM, &[Object]) -> Result<Object, String>`（切片，非 Vec）。Module 经 `alloc_module(name)` + `read_module_mut` 构造（**无 `Module::new` API**）。字符串参数校验复用 task 46 的 `expect_string(arg: Option<&Object>, who: &str) -> Result<String, String>`（`src/vm/stdlib.rs:809-821`），**必须用 `args.get(N)`** 而非 `args[N]`。所有错误统一 `Result<_, String>`，错误消息前缀遵循 `"<ErrorType>: ..."`（如 `"ValueError: ..."`、`"TypeError: ..."`），与 task 46-48 一致。

### 1. 模块注册

`src/vm/stdlib.rs`，签名与 task 46-48 一致（无 vm 参数）：

```rust
pub fn register_json_module() -> *mut MsObjHeader {
    let mut exports = std::collections::HashMap::new();
    exports.insert("parse".to_string(),
        alloc_native_function(NativeFunction{ name: "parse".to_string(), func: native_json_parse }));
    exports.insert("stringify".to_string(),
        alloc_native_function(NativeFunction{ name: "stringify".to_string(), func: native_json_stringify }));
    let m = alloc_module("json");
    match m {
        Object::Ref(p) => {
            // SAFETY: alloc_module 返回有效 MsModule Ref。
            unsafe { read_module_mut(p).exports = exports; }
            p
        }
        _ => unreachable!("alloc_module must return Ref"),
    }
}
```

### 1b. ModuleResolver 集成

`src/vm/mod.rs::VM::new`，紧随 task 48 path 注册之后：

```rust
// task 49：注册原生 json 模块 + 模块函数 arity。
vm.module_resolver
    .native_modules
    .insert("json".to_string(), stdlib::register_json_module());
vm.native_arities.insert("parse".to_string(), 1);
vm.native_arities.insert("stringify".to_string(), 1);
```

### 1c. import 清单（stdlib.rs 顶部）

```rust
use super::object::{
    alloc_dict, alloc_list, alloc_module, alloc_string,
    read_dict, read_list, read_module_mut, read_str,
    DictMap, MsObjHeader, Object, TypeTag,
};
use std::collections::HashSet;
```

### 2. json.parse 实现

> **依赖选择**：task 46-48 全程零外部依赖（task 47 用纯整数算法实现 `unix_to_ymdhms` 替代 chrono）。本任务**默认方案 B（手动解析）**以保持一致；方案 A（serde_json）作为可选路径，需在 PR 中显式说明依赖审核理由（编译时间、二进制体积、unsafe 审计）。

**方案 A：使用 serde_json（可选，需依赖审核）**

```toml
# Cargo.toml
[dependencies]
serde_json = "1"
```

```rust
const MAX_NESTING: u32 = 1000;

fn native_json_parse(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let json_str = expect_string(args.get(0), "parse(string)")?;
    let value: serde_json::Value = serde_json::from_str(&json_str)
        .map_err(|e| format!("ValueError: json parse error at line {} column {}",
            e.line(), e.column()))?;
    json_value_to_object(value, 0)
}

fn json_value_to_object(v: serde_json::Value, depth: u32) -> Result<Object, String> {
    // 深度计数：防御恶意嵌套输入导致的栈溢出（serde_json 默认 128 层，但手动构造的
    // serde_json::Value 可能更深）。MAX_NESTING=1000 兼顾常规用例与栈安全。
    if depth > MAX_NESTING {
        return Err(format!("ValueError: json nesting exceeds {} levels", MAX_NESTING));
    }
    match v {
        serde_json::Value::Null => Ok(Object::Nil),
        serde_json::Value::Bool(b) => Ok(Object::Bool(b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(Object::Int(i))
            } else {
                // 超出 i64 范围的整数（含大数）退化为 f64，精度可能损失（与 Python JSON 一致）。
                // as_f64 仅在非有限值时返回 None（serde_json 内部保证 Number 不含 NaN/Inf），
                // 但显式 ok_or_else 防御依赖内部不变性的脆弱性。
                let f = n.as_f64()
                    .ok_or_else(|| format!("ValueError: json number out of f64 range: {}", n))?;
                Ok(Object::Float(f))
            }
        }
        serde_json::Value::String(s) => Ok(alloc_string(&s)),
        serde_json::Value::Array(arr) => {
            let list: Result<Vec<Object>, String> = arr.into_iter()
                .map(|v| json_value_to_object(v, depth + 1)).collect();
            Ok(alloc_list(list?))
        }
        serde_json::Value::Object(map) => {
            let mut dict = DictMap::new();
            for (k, v) in map {
                dict.insert(alloc_string(&k), json_value_to_object(v, depth + 1)?);
            }
            Ok(alloc_dict(dict))
        }
    }
}
```

> **错误消息策略**：仅引用 `e.line()`/`e.column()`（serde_json Error 自带），**不**包含 `e.to_string()`（可能含原文片段，泄露敏感字段）。

**方案 B：手动解析（默认推荐，零依赖）**

```rust
const MAX_NESTING: u32 = 1000;

/// 简易递归下降 JSON 解析器（约 300 行）。覆盖 task 49 §验证标准 #1-#7：
/// null/true/false/number/string/array/object，UTF-8 字符串字面量（\" \\ \/ \b \f \n \r \t \uXXXX），
/// 数字（i64 精度优先，超界退化 f64）。
fn native_json_parse(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let json_str = expect_string(args.get(0), "parse(string)")?;
    let bytes = json_str.as_bytes();
    let mut p = JsonParser { src: bytes, pos: 0 };
    p.skip_ws();
    let v = p.parse_value(0)?;
    p.skip_ws();
    if p.pos != bytes.len() {
        return Err(format!("ValueError: json trailing characters at byte {}", p.pos));
    }
    Ok(v)
}

struct JsonParser<'a> { src: &'a [u8], pos: usize }

impl<'a> JsonParser<'a> {
    fn parse_value(&mut self, depth: u32) -> Result<Object, String> {
        if depth > MAX_NESTING {
            return Err(format!("ValueError: json nesting exceeds {} levels", MAX_NESTING));
        }
        self.skip_ws();
        match self.src.get(self.pos) {
            Some(b'{') => self.parse_object(depth + 1),
            Some(b'[') => self.parse_array(depth + 1),
            Some(b'"') => Ok(alloc_string(&self.parse_string()?)),
            Some(b't') | Some(b'f') => self.parse_bool(),
            Some(b'n') => self.parse_null(),
            Some(c) if c.is_ascii_digit() || *c == b'-' => self.parse_number(),
            _ => Err(format!("ValueError: json unexpected byte at {}", self.pos)),
        }
    }
    // skip_ws / parse_string（处理转义与 \uXXXX UTF-8 重建）/ parse_number
    // （i64::from_str 优先，失败回退 f64::from_str）/ parse_array / parse_object
    // / parse_bool / parse_null：实现略，每段 < 50 行。
}
```

> 手动解析器各子方法实现略（遵循 §验证标准与 [RFC 8259](https://datatracker.ietf.org/doc/html/rfc8259)），实现者按上述签名补全。子方法不得使用 `unwrap`/`panic`，所有越界返回 `Err`。

### 3. json.stringify 实现

`stringify` 直接构建 `String`（方案 B 与 parse 一致，无 serde_json 依赖）。

```rust
const MAX_NESTING: u32 = 1000;

fn native_json_stringify(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let obj = args.get(0).ok_or("ValueError: stringify expects 1 argument")?;
    let mut out = String::new();
    let mut seen: HashSet<usize> = HashSet::new(); // 循环引用检测
    stringify_into(obj, &mut out, 0, &mut seen)?;
    Ok(alloc_string(&out))
}

fn stringify_into(
    obj: &Object, out: &mut String, depth: u32, seen: &mut HashSet<usize>,
) -> Result<(), String> {
    if depth > MAX_NESTING {
        return Err(format!("ValueError: nesting exceeds {} levels", MAX_NESTING));
    }
    match obj {
        Object::Nil => out.push_str("null"),
        Object::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Object::Int(i) => { use std::fmt::Write; let _ = write!(out, "{}", i); }
        Object::Float(f) => {
            // NaN/Infinity 不是合法 JSON 数字（RFC 8259）。serde_json.to_string 对此返回 Err；
            // 本实现显式报错，与 02-types.md:97-99 中 float NaN/Inf 的特殊性一致。
            if !f.is_finite() {
                return Err(format!("ValueError: cannot serialize non-finite float: {}", f));
            }
            // -0.0 保留字面量（与 0.0 在 mslang 中 == 相等，但 JSON 文本不同）。
            use std::fmt::Write;
            let _ = write!(out, "{}", f);
        }
        Object::Ref(ptr) => {
            // SAFETY: Ref 来自 alloc_* 系列，type_tag 可读。
            let tag = unsafe { (**ptr).type_tag };
            if tag == TypeTag::STRING as u8 {
                // SAFETY: type_tag 为 STRING，ptr 由 alloc_string 分配。
                let s = unsafe { read_str(*ptr) };
                push_json_string(s, out);
            } else if tag == TypeTag::LIST as u8 {
                // 循环引用检测：用指针地址判重，避免 list 自引用导致无限递归。
                if !seen.insert(*ptr as usize) {
                    return Err("ValueError: circular reference".to_string());
                }
                // SAFETY: type_tag 为 LIST。借用约束：递归前 drop &mut Vec<Object>。
                let items: Vec<Object> = {
                    let v = unsafe { read_list(*ptr) };
                    v.clone()
                };
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 { out.push(','); }
                    stringify_into(item, out, depth + 1, seen)?;
                }
                out.push(']');
                seen.remove(&(*ptr as usize));
            } else if tag == TypeTag::DICT as u8 {
                if !seen.insert(*ptr as usize) {
                    return Err("ValueError: circular reference".to_string());
                }
                // SAFETY: type_tag 为 DICT。
                let items: Vec<(Object, Object)> = {
                    let d = unsafe { read_dict(*ptr) };
                    d.items().into_iter().map(|(k, v)| (k.clone(), v.clone())).collect()
                };
                out.push('{');
                for (i, (k, v)) in items.iter().enumerate() {
                    if i > 0 { out.push(','); }
                    let key_str = match k {
                        Object::Ref(kptr) if unsafe { (**kptr).type_tag } == TypeTag::STRING as u8 => {
                            // SAFETY: type_tag 为 STRING。
                            unsafe { read_str(*kptr) }.to_owned()
                        }
                        _ => return Err(format!(
                            "TypeError: JSON dict key must be string, got {}", k.type_name()
                        )),
                    };
                    push_json_string(&key_str, out);
                    out.push(':');
                    stringify_into(v, out, depth + 1, seen)?;
                }
                out.push('}');
                seen.remove(&(*ptr as usize));
            } else {
                // tuple/set/function/class/instance/file_handle/...：Phase 6.2d 不支持
                // __to_json__ 魔术方法，统一拒绝（TypeError，与 task 46-48 类型错误风格一致）。
                return Err(format!(
                    "TypeError: cannot serialize {} to JSON", obj.type_name()
                ));
            }
        }
    }
    Ok(())
}

/// 转义 JSON 字符串字面量：`"`、`\`、控制字符（< 0x20）、非 ASCII（直接 UTF-8 输出）。
fn push_json_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\x08' => out.push_str("\\b"),
            '\x0c' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                use std::fmt::Write;
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}
```

> **借用约束**：`read_list` / `read_dict` 返回 `&mut Vec<Object>` / `&mut DictMap`，递归 `stringify_into` 前必须 clone 出元素，**不可**在持有 `&mut` 的同时再递归调用（会产生重叠 `&mut`，违反 Rust 借用检查 / 触发 UB）。

### 4. 错误处理

错误类型与消息前缀遵循 task 46-48 约定（`"<ErrorType>: ..."`）：

| 场景 | 错误前缀 | 消息示例 |
|---|---|---|
| `json.parse` 入参非 string | `TypeError` | `TypeError: parse(string) expects string, got int` |
| `json.parse` 输入非合法 JSON | `ValueError` | `ValueError: json parse error at line 1 column 5`（仅含位置，**不**含原文片段以防敏感数据泄露） |
| `json.parse` 嵌套深度 > 1000 | `ValueError` | `ValueError: json nesting exceeds 1000 levels` |
| `json.parse` 尾随字符 | `ValueError` | `ValueError: json trailing characters at byte 12` |
| `json.stringify` 缺参 | `ValueError` | `ValueError: stringify expects 1 argument` |
| `json.stringify` 不可序列化类型（tuple/set/function/class/instance/...） | `TypeError` | `TypeError: cannot serialize function to JSON` |
| `json.stringify` 字典键非 string | `TypeError` | `TypeError: JSON dict key must be string, got int` |
| `json.stringify` NaN/Infinity | `ValueError` | `ValueError: cannot serialize non-finite float: NaN` |
| `json.stringify` 循环引用 | `ValueError` | `ValueError: circular reference` |
| `json.stringify` 嵌套深度 > 1000 | `ValueError` | `ValueError: nesting exceeds 1000 levels` |

#### 类型映射补充（覆盖原表未列出的边界）

| mslang 值 | JSON 输出 | 说明 |
|---|---|---|
| `Int`（超出 i64 范围，仅 parse 时） | 退化为 `Float` | 与 Python JSON 一致，精度可能损失 |
| `Float::NaN` / `Float::Infinity` | **报错** | RFC 8259 不允许；与 `02-types.md:97-99` 特殊浮点值语义一致 |
| `Float::-0.0` | `-0.0`（保留字面量） | `-0.0 == 0.0` 为 true（02-types.md:99），但 JSON 文本保留区分 |
| `Tuple` | **报错 TypeError** | Phase 6.2d 不映射为数组（避免与"不可变"语义冲突；后续可单独决策） |
| `Set` | **报错 TypeError** | 无序集合映射到 JSON 数组需排序策略，超出本任务范围 |
| `Function` / `Class` / `Instance` / `Module` / `FileHandle` / `Closure` / `BoundMethod` | **报错 TypeError** | Phase 6.2d 不支持 `__to_json__` 魔术方法 |

#### 已知限制（不在本任务范围内修复）

- **错误路径内存泄漏**：parse 在递归中途失败（`?` 早返回）时，已分配的 Box<String>/Box<Vec>/Box<DictMap> 失去引用但不会被回收（MVP 阶段无 GC，14-gc.md:34）。task 52 GC 上线后自动消解。
- **字符串长度上限**：`alloc_string` 内部 `bytes.len() as u32`（object.rs:191）隐式截断，JSON 字符串 > 4 GiB 时长度被截断导致后续 read 越界。本任务在调用 `alloc_string` 前不显式校验（属 object.rs 的边界），但实现者应知晓此限制。

## 验证标准

1. JSON 对象正确解析为 dict
2. JSON 数组正确解析为 list
3. 字符串、数字、布尔、null 正确映射
4. 嵌套结构正确处理
5. `json.stringify` 输出合法 JSON
6. round-trip：`json.stringify(json.parse(s))` 语义等价（**含浮点数与超出 i64 的整数**）
7. 非法 JSON 输入返回清晰错误信息（仅含位置，不含原文片段）
8. NaN / Infinity 序列化抛 `ValueError`；`-0.0` 字面量保留
9. 循环引用序列化抛 `ValueError: circular reference`
10. 嵌套深度 > 1000 抛 `ValueError`（parse 与 stringify 双向）

## 测试用例

### test_json.ms

```ms
import json

data = json.parse('{"name": "Alice", "age": 30}')
print(data["name"])
print(data["age"])

text = json.stringify({"x": 1, "y": [2, 3]})
print(text)
```

预期输出：
```
Alice
30
{"x":1,"y":[2,3]}
```

### test_json_nested.ms

```ms
import json

nested = json.parse('{"a": {"b": [1, 2, {"c": true}]}}')
print(nested["a"]["b"][2]["c"])

arr = json.parse('[1, null, "hello", [4, 5]]')
print(arr[1])
print(arr[3])

output = json.stringify({"list": [1, 2, 3], "nested": {"key": "val"}})
print(output)
```

预期输出：
```
true
nil
[4, 5]
{"list":[1,2,3],"nested":{"key":"val"}}
```

### test_json_numbers.ms

```ms
import json

# 浮点数 round-trip
f = json.parse("3.14")
print(type(f) == "float")
print(json.stringify(f))

# 负零保留
neg_zero = json.parse("-0.0")
print(json.stringify(neg_zero))

# 超出 i64 的整数退化为 float
big = json.parse("99999999999999999999")
print(type(big) == "float")

# 负数与混合
mixed = json.parse('[-1, 2.5, 1000000]')
print(mixed[0])
print(mixed[1])
print(mixed[2])
```

预期输出：
```
true
3.14
-0.0
true
-1
2.5
1000000
```

### test_json_error.ms

```ms
import json

# parse 非法 JSON
try {
    json.parse("invalid json")
} except e {
    print("caught: " + str(e))
}

# stringify NaN
try {
    nan = 0.0 / 0.0
    json.stringify(nan)
} except e {
    print("caught: " + str(e))
}

# stringify 不可序列化类型（function）
try {
    json.stringify(print)
} except e {
    print("caught: " + str(e))
}

# 缺参
try {
    json.stringify()
} except e {
    print("caught: " + str(e))
}
```

预期输出：
```
caught: ValueError: json parse error at line 1 column 1
caught: ValueError: cannot serialize non-finite float: NaN
caught: TypeError: cannot serialize function to JSON
caught: ValueError: stringify expects 1 argument
```

### test_json_circular.ms

```ms
import json

# 循环引用：list 自引用
a = []
a.push(a)
try {
    json.stringify(a)
} except e {
    print("caught: " + str(e))
}

# 循环引用：dict 互引用
d1 = {}
d2 = {}
d1["link"] = d2
d2["back"] = d1
try {
    json.stringify(d1)
} except e {
    print("caught: " + str(e))
}
```

预期输出：
```
caught: ValueError: circular reference
caught: ValueError: circular reference
```

### test_json_depth.ms

```ms
import json

# 构造 1001 层嵌套（超出 MAX_NESTING=1000）
depth = 1001
s = "[" * depth + "]" * depth
try {
    json.parse(s)
} except e {
    print("parse caught: " + str(e))
}

# stringify 超深嵌套（递归构造深 list）
nested = 1
i = 0
while i < 1001 {
    nested = [nested]
    i = i + 1
}
try {
    json.stringify(nested)
} except e {
    print("stringify caught: " + str(e))
}
```

预期输出：
```
parse caught: ValueError: json nesting exceeds 1000 levels
stringify caught: ValueError: nesting exceeds 1000 levels
```

预期输出：
```
caught: json parse error: ...
```
