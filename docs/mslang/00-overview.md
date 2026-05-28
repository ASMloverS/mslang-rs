# mslang 语言概览

## 基本信息

| 属性 | 值 |
|---|---|
| 语言名称 | mslang |
| 文件后缀 | `.ms` |
| 实现语言 | Rust |
| 运行模型 | 编译到字节码 + 栈式虚拟机 |
| 类型系统 | 纯动态类型 |
| 内存管理 | 分代 GC（MVP: 标记-清除 → 目标: 并发三色标记清扫） |
| 代码块风格 | 花括号 `{}` |
| 分号 | 不需要，换行即语句结束 |
| 入口点 | 脚本模式，从文件顶部顺序执行 |
| 注释 | `#` 单行注释 |

## 设计哲学

mslang 是一门融合 Go 语法风格与 Python 动态特性的脚本语言：

- **简洁** — 类 Go 的花括号语法，无分号，无冗余声明
- **动态** — 纯动态类型，变量无需声明类型
- **表达力** — 支持 Python 风格的列表推导式、切片、生成器、装饰器
- **实用** — 内置 async/await 并发模型，支持 channel 通信
- **安全** — defer 语句确保资源清理，try/except 异常处理

## 快速示例

```ms
# hello.ms
const GREETING = "Hello, mslang!"

fn main_thinking(names) {
    result = []
    for name in names {
        msg = GREETING + " " + name
        result.push(msg)
    }
    return result
}

names = ["Alice", "Bob", "Charlie"]
for msg in main_thinking(names) {
    print(msg)
}
```

## 特性清单

### 核心特性

- [x] 动态类型系统
- [x] 变量声明：`x = val` / `var x = val` / `x := val`
- [x] 常量声明：`const NAME = val`
- [x] 花括号代码块，无分号
- [x] `#` 行注释

### 数据类型

- [x] 基本类型：int, float, bool, string, nil
- [x] 集合类型：list, dict, tuple, set

### 函数

- [x] 函数定义：`fn name(params) { body }`
- [x] First-class 函数
- [x] 闭包与上值捕获
- [x] 匿名函数：`fn(params) { body }`
- [x] 多返回值（元组解包）

### 控制流

- [x] if / elif / else
- [x] while
- [x] for..in
- [x] break / continue / return

### 高级特性

- [x] 列表推导式：`[expr for x in iter if cond]`
- [x] 切片操作：`seq[start:stop:step]`
- [x] 生成器 / yield
- [x] 装饰器 `@decorator`
- [x] with 语句（上下文管理器）
- [x] defer 语句

### 面向对象

- [x] Python 风格 class
- [x] 单继承：`class Child < Parent`
- [x] self 绑定
- [x] 魔术方法：`__init__`, `__repr__`, `__add__` 等

### 错误处理

- [x] try / except / finally
- [x] defer 语句（类似 Go）

### 并发

- [x] async / await
- [x] channel（有缓冲/无缓冲）
- [x] go 关键字启动协程

### 模块

- [x] `import mod`
- [x] `from mod import name`
- [x] `import mod as alias`

### 工具链

- [x] REPL 交互式命令行
- [x] 友好错误信息与堆栈跟踪
- [x] CLI 工具

## 文档索引

| 文档 | 内容 |
|---|---|
| [01-lexical](01-lexical.md) | 词法规范 — Token、关键字、字面量 |
| [02-types](02-types.md) | 类型系统 — 所有内置类型定义 |
| [03-syntax](03-syntax.md) | 语法规范 — 表达式与语句 |
| [04-functions](04-functions.md) | 函数系统 — 定义、闭包、多返回值 |
| [05-control-flow](05-control-flow.md) | 控制流 — 条件、循环、错误处理 |
| [06-oop](06-oop.md) | 面向对象 — class、继承、魔术方法 |
| [07-advanced](07-advanced.md) | 高级特性 — 装饰器、生成器、推导式、切片、with、defer |
| [08-concurrency](08-concurrency.md) | 并发模型 — async/await、channel |
| [09-modules](09-modules.md) | 模块系统 — import、包管理 |
| [10-builtins](10-builtins.md) | 内置函数与标准库 |
| [11-bytecode-vm](11-bytecode-vm.md) | 字节码与虚拟机设计 |
| [12-implementation-plan](12-implementation-plan.md) | 分阶段实现计划 |
| [13-capi](13-capi.md) | C API 设计 — 嵌入与扩展 |
| [14-gc](14-gc.md) | 垃圾回收系统 — MVP 标记-清除 + 目标并发三色标记清扫分代回收 |
