# mslang

一门融合 Go 语法风格与 Python 动态特性的脚本语言，使用 Rust 实现。

## 示例

```ms
const GREETING = "Hello, mslang!"

fn greet(names) {
    result = []
    for name in names {
        result.push(GREETING + " " + name)
    }
    return result
}

for msg in greet(["Alice", "Bob", "Charlie"]) {
    print(msg)
}
```

## 特性

| 分类 | 特性 |
|---|---|
| 类型 | 动态类型 — int, float, bool, string, nil, list, dict, tuple, set |
| 函数 | first-class 函数、闭包、匿名函数、默认/可变参数、多返回值 |
| 控制流 | if/elif/else, while, for..in, break/continue, 三元表达式 |
| 高级 | 列表/dict/set 推导式、切片、生成器/yield、装饰器、with、defer |
| OOP | Python 风格 class、单继承、魔术方法、运算符重载 |
| 错误处理 | try/except/finally, throw, defer |
| 并发 | async/await, go 协程, channel |
| 模块 | import, from...import, import as |

## 运行模型

```
源码 (.ms) → Lexer → Token → Parser → AST → Compiler → 字节码 → VM 执行
```

- 编译到字节码 + 栈式虚拟机
- 分代 GC（MVP: 标记-清除 → 目标: 并发三色标记清扫）
- 花括号代码块，无分号，`#` 行注释
- 脚本模式，从文件顶部顺序执行

## CLI

```
ms run script.ms      运行脚本
ms eval "1 + 2"       求值表达式
ms repl               交互式 REPL
ms check script.ms    语法检查
ms version            版本信息
```

## 构建

```
cargo build
cargo test
```

## 文档

| 文档 | 内容 |
|---|---|
| [00-overview](docs/mslang/00-overview.md) | 语言概览 |
| [01-lexical](docs/mslang/01-lexical.md) | 词法规范 |
| [02-types](docs/mslang/02-types.md) | 类型系统 |
| [03-syntax](docs/mslang/03-syntax.md) | 语法规范 |
| [04-functions](docs/mslang/04-functions.md) | 函数系统 |
| [05-control-flow](docs/mslang/05-control-flow.md) | 控制流 |
| [06-oop](docs/mslang/06-oop.md) | 面向对象 |
| [07-advanced](docs/mslang/07-advanced.md) | 高级特性 |
| [08-concurrency](docs/mslang/08-concurrency.md) | 并发模型 |
| [09-modules](docs/mslang/09-modules.md) | 模块系统 |
| [10-builtins](docs/mslang/10-builtins.md) | 内置函数与标准库 |
| [11-bytecode-vm](docs/mslang/11-bytecode-vm.md) | 字节码与虚拟机设计 |
| [12-implementation-plan](docs/mslang/12-implementation-plan.md) | 分阶段实现计划 |
| [13-capi](docs/mslang/13-capi.md) | C API 设计 |
| [14-gc](docs/mslang/14-gc.md) | 垃圾回收系统 |
| [任务索引](docs/mslang/tasks/README.md) | 66 项实现任务 |

## License

MIT
