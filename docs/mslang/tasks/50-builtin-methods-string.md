# 内置类型方法 - String

## 所属阶段
Phase 6.2e - 标准库

## 前置任务
26-builtins-iterators

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

### 1. 方法分发表

`src/vm/stdlib.rs` 或 `src/vm/builtins.rs` 中实现字符串方法分派：

```rust
fn call_string_method(
    method: &str,
    receiver: &str,
    args: Vec<Object>,
) -> Result<Object> {
    match method {
        "length" => Ok(Object::Int(receiver.chars().count() as i64)),
        "upper" => Ok(Object::String(Gc::new(receiver.to_uppercase()))),
        "lower" => Ok(Object::String(Gc::new(receiver.to_lowercase()))),
        "strip" => Ok(Object::String(Gc::new(receiver.trim().to_string()))),
        "split" => { ... }
        "join" => { ... }
        "replace" => { ... }
        "contains" => { ... }
        "startswith" => { ... }
        "endswith" => { ... }
        "index" => { ... }
        "slice" => { ... }
        _ => Err(MspError::RuntimeError(
            format!("string has no method '{}'", method)
        )),
    }
}
```

### 2. VM 方法调用集成

当 VM 执行 `GET_ATTR` 发现目标是 String 且属性名对应已知方法时，返回 `BoundMethod` 对象。调用时走 `call_string_method` 分派。

或者，在 `INVOKE` 指令中直接检查 String 类型并分派，避免创建中间对象（性能优化）。

### 3. 各方法实现细节

**split**：
```rust
"split" => {
    if args.is_empty() {
        let parts: Vec<Object> = receiver.split_whitespace()
            .map(|s| Object::String(Gc::new(s.to_string())))
            .collect();
        Ok(Object::List(Gc::new(parts)))
    } else {
        let sep = expect_string(&args[0])?;
        let parts: Vec<Object> = receiver.split(&sep)
            .map(|s| Object::String(Gc::new(s.to_string())))
            .collect();
        Ok(Object::List(Gc::new(parts)))
    }
}
```

**join**：
```rust
"join" => {
    let list = expect_list(&args[0])?;
    let parts: Vec<String> = list.iter()
        .map(|o| o.to_string())
        .collect();
    Ok(Object::String(Gc::new(parts.join(receiver))))
}
```

**replace**：
```rust
"replace" => {
    let old = expect_string(&args[0])?;
    let new = expect_string(&args[1])?;
    Ok(Object::String(Gc::new(receiver.replace(&old, &new))))
}
```

**contains**：
```rust
"contains" => {
    let sub = expect_string(&args[0])?;
    Ok(Object::Bool(receiver.contains(&sub)))
}
```

**index**：
```rust
"index" => {
    let sub = expect_string(&args[0])?;
    match receiver.find(&sub) {
        Some(pos) => Ok(Object::Int(pos as i64)),
        None => Err(MspError::RuntimeError(
            format!("substring '{}' not found", sub)
        )),
    }
}
```

**slice**：
```rust
"slice" => {
    let start = expect_int(&args[0])? as usize;
    let chars: Vec<char> = receiver.chars().collect();
    let end = if args.len() > 1 {
        expect_int(&args[1])? as usize
    } else {
        chars.len()
    };
    let sliced: String = chars[start..end].iter().collect();
    Ok(Object::String(Gc::new(sliced)))
}
```

- 使用 `chars()` 而非字节索引，确保 Unicode 正确

## 验证标准

1. 所有 12 个字符串方法正确工作
2. Unicode 字符串正确处理（length、slice 使用 chars 而非字节）
3. `split` 无参数时按空白分割
4. `index` 找不到子串时抛出错误
5. `slice` 参数越界时优雅处理
6. 对非 String 类型调用字符串方法返回错误

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
