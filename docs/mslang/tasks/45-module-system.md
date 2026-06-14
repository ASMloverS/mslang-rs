# 模块系统（import）

## 所属阶段
Phase 6.1 - 模块系统 + 标准库

## 前置任务
44-decorators

## 目标
实现完整的模块系统，支持 `import`、`from...import`、`import as`，包括模块搜索、加载、缓存、作用域隔离和导出规则。

## 设计规格

参照 [09-modules](../09-modules.md) § 模块解析、[11-bytecode-vm](../11-bytecode-vm.md) § IMPORT 指令：

### 语法规则

```
import_stmt = "import" module_path ("as" IDENTIFIER)?
            | "from" module_path "import" import_targets

module_path = IDENTIFIER ("." IDENTIFIER)*

import_targets = import_target ("," import_target)*
import_target = IDENTIFIER ("as" IDENTIFIER)?
```

### IMPORT 指令

| OpCode | 操作数 | 说明 |
|---|---|---|
| `IMPORT` | `module_idx(2)` | 导入模块，将 Module 对象压栈 |

### 搜索路径

按以下顺序搜索模块：
1. **当前目录** — 脚本所在目录
2. **标准库目录** — mslang 安装目录下的 `stdlib/`
3. **MS_PATH** — 环境变量指定的路径（用 `;` 分隔）

### 安全模块加载（`import @std`）

参照 [09-modules](../09-modules.md) § 安全模块加载：

```
import @std math         # 强制从标准库目录加载，跳过当前目录搜索
from @std io import open
```

`@std` 前缀确保只从标准库目录搜索模块，避免当前目录下的同名文件被恶意替换。在安全模式（`MS_SAFE=1`）下，所有非 `@std` 的 import 被禁止。

搜索规则更新：

```
import foo -> foo.ms, foo/index.ms, stdlib/foo.ms
import @std foo -> stdlib/foo.ms（仅标准库目录）
```

### 安全模式

参照 [09-modules](../09-modules.md)：

- 环境变量 `MS_SAFE=1` 启用安全模式
- CLI 参数 `ms run --safe` 启用安全模式
- 安全模式下：
  - 禁止 `os.exec()`
  - 禁止 `import` 非标准库模块（只允许 `import @std xxx`）
  - 禁止文件写入操作
  - 限制网络访问

### 搜索规则

```
import foo -> foo.ms, foo/index.ms, stdlib/foo.ms
import os.path -> os/path.ms, os/path/index.ms
import @std math -> stdlib/math.ms（仅标准库）
```

### 导出规则

- `fn`、`class`、`const` 顶层定义默认可被外部访问
- `var`、`:=`、`=` 顶层变量为模块私有

### 模块缓存

- 模块在**首次被导入**时执行其顶层代码，然后缓存
- 后续再次 import 同一模块不重新执行，直接返回缓存

### 模块作用域

- 每个模块有独立的全局作用域
- 模块内顶层变量相互隔离

## 实现细节

### 1. Module 对象

`src/vm/object.rs` 中新增：

```rust
struct Module {
    name: String,
    exports: HashMap<String, Object>,
    globals: HashMap<String, Object>,
}
```

- `name`：模块名（如 `"math_utils"`）
- `exports`：可被外部访问的名称（fn, class, const）
- `globals`：模块自己的全局作用域（包含私有变量）

### 2. ModuleResolver

`src/module/resolver.rs`：

```rust
struct ModuleResolver {
    search_paths: Vec<PathBuf>,
    cache: HashMap<String, *mut MsObjHeader>,  // 指向 MsModule（TypeTag::MODULE）
}
```

- `search_paths`：初始化时按优先级填入当前目录、stdlib/、MS_PATH
- `cache`：已加载模块缓存，键为模块路径字符串
- `resolve(name: &str) -> Result<PathBuf>`：按搜索规则查找模块文件
- `load(name: &str, vm: &mut VM) -> Result<*mut MsObjHeader>`：编译并执行 .ms 文件，构建 Module 对象（TypeTag::MODULE）

### 3. 编译器改动

`src/compiler/mod.rs`：

- `import foo`：编译为 `IMPORT module_idx`，模块名存入常量池
- `import foo as bar`：编译为 `IMPORT module_idx` + `STORE_GLOBAL "bar"`
- `from foo import a, b as c`：编译为 `IMPORT module_idx` + `GET_ATTR "a"` + `STORE_GLOBAL "a"` + `GET_ATTR "b"` + `STORE_GLOBAL "c"`

### 4. VM IMPORT 指令处理

`src/vm/mod.rs`：

```rust
OpCode::IMPORT => {
    let module_name = self.read_constant(idx);
    let module_ptr = self.module_resolver.load(&module_name, self)?;
    // Module 通过 Ref + TypeTag::MODULE 表示（参照 Task 20 对象模型）
    self.stack.push(Object::Ref(module_ptr));
}
```

### 5. 模块加载流程

1. 检查缓存：如果已加载，直接返回缓存的 Module
2. **循环导入检测**：检查 `loading_stack`，若模块已在加载中（部分初始化），参照 [09-modules](../09-modules.md) § 循环导入：
   - 返回已部分初始化的 Module（允许访问已定义的名称）
   - 访问尚未初始化的名称时抛出 `NameError`
3. 解析路径：调用 `ModuleResolver::resolve` 查找 .ms 文件
4. 读取源码：读取文件内容
5. 编译：Lexer → Parser → Compiler，生成字节码
6. **加入 loading_stack**：标记为「加载中」（防止循环导入时重复执行）
7. 执行：在新的全局作用域中执行字节码
8. 构建导出表：扫描顶层定义，fn/class/const 加入 exports
9. **移出 loading_stack**：标记为「加载完成」
10. 缓存：将 Module 存入缓存

```rust
struct ModuleResolver {
    search_paths: Vec<PathBuf>,
    cache: HashMap<String, *mut MsObjHeader>,
    loading_stack: HashSet<String>,  // 正在加载中的模块（循环导入检测）
}

fn load(&mut self, name: &str, vm: &mut VM) -> Result<*mut MsObjHeader, String> {
    // 1. 已缓存 → 直接返回
    if let Some(ptr) = self.cache.get(name) {
        return Ok(*ptr);
    }

    // 2. 循环导入检测：模块正在加载中 → 返回部分初始化的 Module
    if self.loading_stack.contains(name) {
        // 返回部分初始化的 Module（已在 cache 中预留空壳）
        // 访问未初始化名称时由 GET_ATTR 触发 NameError
        let partial = self.cache.get(name).copied()
            .ok_or_else(|| format!("ImportError: circular import detected for '{}'", name))?;
        return Ok(partial);
    }

    // 3. 解析路径
    let path = self.resolve(name)?;

    // 4. 预分配空 Module 并加入 loading_stack + cache（支持循环导入部分访问）
    let partial_module = alloc_module(name, HashMap::new(), HashMap::new());
    let partial_ptr = if let Object::Ref(p) = partial_module { p } else { unreachable!() };
    self.loading_stack.insert(name.to_string());
    self.cache.insert(name.to_string(), partial_ptr);

    // 5-7. 编译并执行
    let source = std::fs::read_to_string(&path)
        .map_err(|e| format!("ImportError: cannot load '{}': {}", name, e))?;
    let unit = compile(&source, name)?;
    let (exports, globals) = vm.execute_module(unit)?;

    // 8. 填充 Module
    unsafe {
        let module = read_module_mut(partial_ptr);
        module.exports = exports;
        module.globals = globals;
    }

    // 9. 标记为加载完成
    self.loading_stack.remove(name);

    Ok(partial_ptr)
}
```

## 验证标准

1. `import module_name` 正确加载并执行模块文件
2. `module.name` 可访问模块导出的 fn/class/const
3. `from module import name` 正确提取指定名称
4. `import module as alias` 别名正常工作
5. 模块只执行一次（缓存生效）
6. 模块私有变量（var/:=/=）不可从外部访问
7. 包模块（目录 + index.ms）正确加载
8. 模块作用域相互隔离
9. `import @std math` 正确从标准库目录加载（跳过当前目录）
10. `from @std io import open` 正确加载标准库模块的指定名称
11. 安全模式（`MS_SAFE=1`）下，非 `@std` import 被拒绝
12. 循环导入（A imports B, B imports A）不导致死循环：部分初始化的模块可访问已定义名称，未定义名称抛出 NameError

## 测试用例

### math_utils.ms

```ms
const VERSION = "1.0"

fn add(a, b) {
    return a + b
}

fn multiply(a, b) {
    return a * b
}
```

### main.ms

```ms
import math_utils

print(math_utils.VERSION)
print(math_utils.add(3, 4))
print(math_utils.multiply(3, 4))
```

预期输出：
```
1.0
7
12
```

### from_import.ms

```ms
from math_utils import add, multiply as mul

print(add(1, 2))
print(mul(3, 4))
```

预期输出：
```
3
12
```

### 缓存测试 cache_test.ms

```ms
import math_utils
import math_utils

print("cache ok")
```

预期输出（math_utils 顶层代码只执行一次）：
```
cache ok
```

### 私有变量测试 private_test.ms

```ms
import math_utils

# VERSION 是 const，可访问
print(math_utils.VERSION)

# 以下应报错或返回 nil（私有变量不可访问）
# math_utils 内部如用 var 声明的变量，外部不应看到
```
