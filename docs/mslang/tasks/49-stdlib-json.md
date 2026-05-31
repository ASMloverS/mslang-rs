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

### 1. 模块注册

`src/vm/stdlib.rs`：

```rust
fn register_json_module(vm: &mut VM) -> *mut MsObjHeader {  // 返回指向 MsModule 的指针
    let mut exports = HashMap::new();
    exports.insert("parse".into(), Object::NativeFn(native_json_parse));
    exports.insert("stringify".into(), Object::NativeFn(native_json_stringify));
    Module::new("json", exports)
}
```

### 2. json.parse 实现

两种方案：

**方案 A：使用 serde_json（推荐）**

```toml
# Cargo.toml
[dependencies]
serde_json = "1"
```

```rust
fn native_json_parse(vm: &mut VM, args: Vec<Object>) -> Result<Object> {
    let json_str = expect_string(&args[0])?;
    let value: serde_json::Value = serde_json::from_str(&json_str)
        .map_err(|e| MspError::RuntimeError(format!("json parse error: {}", e)))?;
    json_value_to_object(value)
}
```

转换函数：

```rust
fn json_value_to_object(v: serde_json::Value) -> Result<Object> {
    match v {
        serde_json::Value::Null => Ok(Object::Nil),
        serde_json::Value::Bool(b) => Ok(Object::Bool(b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(Object::Int(i))
            } else {
                Ok(Object::Float(n.as_f64().unwrap()))
            }
        }
        serde_json::Value::String(s) => Ok(alloc_string(&s)),
        serde_json::Value::Array(arr) => {
            let list: Result<Vec<Object>> = arr.into_iter()
                .map(json_value_to_object).collect();
            Ok(alloc_list(list?))
        }
        serde_json::Value::Object(map) => {
            let mut dict = DictMap::new();
            for (k, v) in map {
                dict.insert(alloc_string(&k), json_value_to_object(v)?);
            }
            Ok(alloc_dict(dict))
        }
    }
}
```

**方案 B：手动解析**

不引入依赖，自行实现 JSON 解析器。MVP 阶段不推荐，除非有减少依赖的明确需求。

### 3. json.stringify 实现

```rust
fn native_json_stringify(vm: &mut VM, args: Vec<Object>) -> Result<Object> {
    let json_val = object_to_json_value(&args[0])?;
    let s = serde_json::to_string(&json_val)
        .map_err(|e| MspError::RuntimeError(format!("json stringify error: {}", e)))?;
    Ok(Object::String(Gc::new(s)))
}

fn object_to_json_value(obj: &Object) -> Result<serde_json::Value> {
    match obj {
        Object::Nil => Ok(serde_json::Value::Null),
        Object::Bool(b) => Ok(serde_json::Value::Bool(*b)),
        Object::Int(i) => Ok(serde_json::json!(*i)),
        Object::Float(f) => Ok(serde_json::json!(*f)),
        Object::Ref(ptr) => {
            let tag = unsafe { (*(*ptr)).type_tag };
            if tag == TypeTag::STRING as u8 {
                Ok(serde_json::Value::String(unsafe { read_str(*ptr) }.to_owned()))
            } else if tag == TypeTag::LIST as u8 {
                let arr: Result<Vec<_>> = unsafe { read_list(*ptr) }.iter()
                    .map(object_to_json_value).collect();
                Ok(serde_json::Value::Array(arr?))
            } else if tag == TypeTag::DICT as u8 {
                let mut map = serde_json::Map::new();
                for (k, v) in unsafe { read_dict(*ptr) }.items() {
                    let key_str = match k {
                        Object::Ref(kptr) if unsafe { (*(*kptr)).type_tag } == TypeTag::STRING as u8 => {
                            unsafe { read_str(*kptr) }.to_owned()
                        }
                        _ => return Err(MspError::RuntimeError(
                            "JSON dict key must be string".to_string()
                        )),
                    };
                    map.insert(key_str, object_to_json_value(v)?);
                }
                Ok(serde_json::Value::Object(map))
            } else {
                Err(MspError::RuntimeError(
                    format!("cannot serialize type to JSON: {:?}", obj.type_name())
                ))
            }
        }
        _ => Err(MspError::RuntimeError(
            format!("cannot serialize type to JSON: {:?}", obj.type_name())
        )),
    }
}
```

### 4. 错误处理

- `json.parse` 对无效 JSON 字符串抛出 RuntimeError
- `json.stringify` 对不可序列化的类型（如 Function）抛出 RuntimeError

## 验证标准

1. JSON 对象正确解析为 dict
2. JSON 数组正确解析为 list
3. 字符串、数字、布尔、null 正确映射
4. 嵌套结构正确处理
5. `json.stringify` 输出合法 JSON
6. round-trip：`json.stringify(json.parse(s))` 语义等价
7. 非法 JSON 输入返回清晰错误信息

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

### test_json_error.ms

```ms
import json

# 以下应抛出错误
try {
    json.parse("invalid json")
} except e {
    print("caught: " + str(e))
}
```

预期输出：
```
caught: json parse error: ...
```
