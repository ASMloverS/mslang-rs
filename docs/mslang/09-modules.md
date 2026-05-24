# 模块系统

## 概述

mslang 使用基于文件的模块系统，每个 `.ms` 文件是一个模块。

## import 语法

### import module

```ms
import math
```

导入 `math` 模块，通过 `math.name` 访问其导出内容。

### from...import

```ms
from os import path
from io import open, print
from math import sqrt as root
```

从模块中导入特定的名称，可以直接使用。

### import as

```ms
import math as m
import os.path as pathutil
```

为导入的模块指定别名。

### 语法规则

```
import_stmt = "import" module_path ("as" IDENTIFIER)?
            | "from" module_path "import" import_targets

module_path = IDENTIFIER ("." IDENTIFIER)*

import_targets = import_target ("," import_target)*
import_target = IDENTIFIER ("as" IDENTIFIER)?
```

## 模块解析

### 搜索路径

import 时按以下顺序搜索模块：

1. **当前目录** — 脚本所在目录
2. **标准库目录** — mslang 安装目录下的 `stdlib/`
3. **MSLANG_PATH** — 环境变量指定的路径（用 `;` 分隔）

### 搜索规则

```
import foo
```

搜索以下文件（按顺序）：

1. `foo.ms` — 文件模块
2. `foo/index.ms` — 包模块（目录）
3. 标准库中的 `foo.ms`

```
import os.path
```

搜索：

1. `os/path.ms`
2. `os/path/index.ms`

### 文件模块

单个 `.ms` 文件即为一个模块：

```
# math_utils.ms
fn add(a, b) {
    return a + b
}

fn multiply(a, b) {
    return a * b
}
```

所有顶层声明都是模块的导出内容。

### 包模块

目录中包含 `index.ms` 的文件夹可作为包：

```
mylib/
├── index.ms          # 包入口
├── utils.ms          # 子模块
└── helpers.ms        # 子模块
```

```ms
import mylib                  # 加载 mylib/index.ms
import mylib.utils            # 加载 mylib/utils.ms
from mylib.utils import tool  # 从子模块导入
```

## 模块执行

### 首次导入

模块在**首次被导入**时执行其顶层代码，然后将导出的名称缓存。

后续再次 import 同一模块不会重新执行，直接使用缓存。

```ms
# counter.ms
print("counter module loaded")
count = 0

fn increment() {
    count += 1
    return count
}
```

```ms
import counter     # 打印 "counter module loaded"
counter.increment()  # 1

import counter     # 不会再次打印（使用缓存）
counter.increment()  # 2
```

### 模块作用域

每个模块有独立的全局作用域。模块内的顶层变量是模块私有的，只有函数和 class 可以被外部访问。

```ms
# config.ms
internal_state = "private"    # 模块私有

fn get_state() {              # 可被外部访问
    return internal_state
}
```

### 循环导入

循环导入是允许的，但需注意：

- 如果模块 A 导入模块 B，模块 B 又导入模块 A
- 先被导入的模块可能只部分初始化
- 建议避免循环导入，或使用延迟导入（在函数内 import）
- 访问未初始化的名称将抛出 `NameError`（而非静默返回 `nil`）
- 使用 `ms check` 可在开发时检测潜在的循环导入问题

```ms
# a.ms
import b

fn hello() {
    return "from a"
}

# b.ms
import a

fn world() {
    return "from b"
}
```

## 导出规则

### 默认导出

模块中所有顶层 `fn`、`class` 和 `const` 定义默认可被外部访问。

顶层变量（`var`, `:=`, `=`）默认为模块私有。

```ms
# utils.ms
const VERSION = "1.0"      # 可导出

fn helper() {              # 可导出
    return "help"
}

class Config {             # 可导出
    fn __init__(self) {
        self.data = {}
    }
}
```

### 显式导出（可选）

如果需要更精细的控制，可以使用导出声明（后续版本考虑）：

```ms
# 保留语法，暂不实现
export fn public_fn() { ... }
export class PublicClass { ... }
```

`export` 为保留关键字（见 [01-lexical](01-lexical.md)），不可用作变量名。

MVP 阶段采用简单的规则：函数、class、const 可访问，普通变量私有。

## 标准库结构

```
stdlib/
├── io.ms            # I/O 操作
├── math.ms          # 数学函数
├── os.ms            # 操作系统接口
├── string.ms        # 字符串工具
├── time.ms          # 时间相关
├── json.ms          # JSON 编解码
├── regex.ms         # 正则表达式
├── collections.ms   # 高级数据结构
├── http.ms          # HTTP 客户端
├── net.ms           # 网络操作
├── fs.ms            # 文件系统操作
├── async.ms         # 异步工具
├── path.ms          # 路径操作
└── test.ms          # 测试框架
```

详见 [10-builtins](10-builtins.md)。

## CLI 与模块

### 运行脚本

```
ms run script.ms
```

脚本所在目录自动加入模块搜索路径。

### 运行模块

```
ms run mylib.utils
```

等价于运行 `mylib/utils.ms`。

### REPL

```
ms repl
```

REPL 中可以使用 import 导入模块。

### 检查

```
ms check script.ms
```

只做语法检查，不执行。

### 格式化

```
ms fmt script.ms
```

格式化源码（后续版本）。
