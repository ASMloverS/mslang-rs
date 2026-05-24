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
3. **MSLANG_PATH** — 环境变量指定的路径（用 `;` 分隔）

### 搜索规则

```
import foo -> foo.ms, foo/index.ms, stdlib/foo.ms
import os.path -> os/path.ms, os/path/index.ms
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
    cache: HashMap<String, Gc<Module>>,
}
```

- `search_paths`：初始化时按优先级填入当前目录、stdlib/、MSLANG_PATH
- `cache`：已加载模块缓存，键为模块路径字符串
- `resolve(name: &str) -> Result<PathBuf>`：按搜索规则查找模块文件
- `load(name: &str, vm: &mut VM) -> Result<Gc<Module>>`：编译并执行 .ms 文件，构建 Module 对象

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
    let module = self.module_resolver.load(&module_name, self)?;
    self.stack.push(Object::Module(module));
}
```

### 5. 模块加载流程

1. 检查缓存：如果已加载，直接返回缓存的 Module
2. 解析路径：调用 `ModuleResolver::resolve` 查找 .ms 文件
3. 读取源码：读取文件内容
4. 编译：Lexer → Parser → Compiler，生成字节码
5. 执行：在新的全局作用域中执行字节码
6. 构建导出表：扫描顶层定义，fn/class/const 加入 exports
7. 缓存：将 Module 存入缓存

## 验证标准

1. `import module_name` 正确加载并执行模块文件
2. `module.name` 可访问模块导出的 fn/class/const
3. `from module import name` 正确提取指定名称
4. `import module as alias` 别名正常工作
5. 模块只执行一次（缓存生效）
6. 模块私有变量（var/:=/=）不可从外部访问
7. 包模块（目录 + index.ms）正确加载
8. 模块作用域相互隔离

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
