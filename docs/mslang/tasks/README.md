# mslang 实现任务索引

按实现顺序排列。MVP = Phase 1 + 2 + 2.5 + 3。

## Phase 1 — 基础设施（Lexer + Parser）

| # | 任务 | 状态 |
|---|---|---|
| 01 | [项目骨架搭建](01-project-skeleton.md) | ✅ |
| 02 | [Token 类型定义](02-token-definition.md) | ✅ |
| 03 | [词法分析器核心框架](03-lexer-core.md) | ✅ |
| 04 | [数值字面量解析](04-lexer-number-literals.md) | ✅ |
| 05 | [字符串字面量解析](05-lexer-string-literals.md) | ✅ |
| 06 | [标识符与关键字解析](06-lexer-identifiers-keywords.md) | ✅ |
| 07 | [运算符与分隔符解析](07-lexer-operators-delimiters.md) | ✅ |
| 08 | [换行与语句终止规则](08-lexer-statement-termination.md) | ✅ |
| 09 | [AST 表达式节点定义](09-ast-expression-nodes.md) | ✅ |
| 10 | [AST 语句节点定义](10-ast-statement-nodes.md) | ✅ |
| 11 | [语法分析器核心框架](11-parser-core.md) | ✅ |
| 12 | [表达式解析（优先级爬升）](12-parser-expressions.md) | ✅ |
| 13 | [语句解析](13-parser-statements.md) | ✅ |
| 14 | [集合字面量与匿名函数解析](14-parser-collection-literals.md) | ✅ |
| 15 | [高级语句解析（defer/try/with/class/import）](15-parser-advanced-statements.md) | ✅ |

## Phase 2 — 字节码编译 + VM 核心

| # | 任务 | 状态 |
|---|---|---|
| 16 | [字节码指令集定义](16-opcode-definition.md) | ✅ |
| 17 | [编译器核心框架](17-compiler-core.md) | ✅ |
| 18 | [表达式编译](18-compile-expressions.md) | ✅ |
| 19 | [语句编译](19-compile-statements.md) | ✅ |
| 20 | [Object 系统基础类型](20-object-system-basic.md) | ✅ |
| 21 | [Object 运算符实现](21-object-system-operations.md) | ✅ |
| 22 | [Object 集合类型](22-object-system-collections.md) | ✅ |
| 23 | [虚拟机核心执行循环](23-vm-core.md) | ✅ |
| 24 | [VM 算术运算与控制流](24-vm-arithmetic-control.md) | ✅ |
| 25 | [基础内置函数](25-builtins-basic.md) | ✅ |
| 26 | [内置迭代器与容器函数](26-builtins-iterators.md) | ✅ |

## Phase 2.5 — GC 基础（Young 代半空间复制 + Old 代 STW 标记-清除）

| # | 任务 | 状态 |
|---|---|---|
| 52 | [垃圾回收（MVP：Young 代半空间复制 + Old 代 STW 标记-清除）](52-gc.md) | ✅ |

## Phase 3 — 函数 + 闭包（MVP 完成）

| # | 任务 | 状态 |
|---|---|---|
| 27 | [调用帧与函数调用](27-call-frame.md) | ✅ |
| 28 | [闭包与上值机制](28-closures.md) | ✅ |
| 29 | [匿名函数](29-anonymous-functions.md) | ✅ |
| 30 | [多返回值与元组解包](30-multi-return-tuple-unpack.md) | ✅ |
| 31 | [默认参数与可变参数](31-default-variadic-params.md) | ✅ |

## Phase 4 — 控制流 + 高级语法

| # | 任务 | 状态 |
|---|---|---|
| 32 | [for..in 循环与迭代器协议](32-for-in-iterator.md) | ✅ |
| 33 | [列表推导式](33-list-comprehension.md) | ✅ |
| 34 | [Dict/Set 推导式](34-dict-set-comprehension.md) | ✅ |
| 35 | [切片操作](35-slicing.md) | ✅ |
| 36 | [defer 语句](36-defer.md) | ✅ |
| 37 | [try/except/finally 异常处理](37-try-except-finally.md) | ✅ |
| 38 | [with 语句（上下文管理器）](38-with-statement.md) | ✅ |
| 39 | [生成器与 yield](39-generator-yield.md) | ✅ |

## Phase 5 — Class + 面向对象

| # | 任务 | 状态 |
|---|---|---|
| 40 | [Class 定义与实例化](40-class-definition.md) | ✅ |
| 41 | [self 绑定与实例属性](41-self-instance-attributes.md) | ✅ |
| 42 | [继承与 super](42-inheritance-super.md) | ✅ |
| 43 | [魔术方法](43-magic-methods.md) | ✅ |
| 44 | [装饰器](44-decorators.md) | ✅ |

## Phase 6 — 模块系统 + 标准库

| # | 任务 | 状态 |
|---|---|---|
| 45 | [模块系统（import）](45-module-system.md) | ✅ |
| 46 | [标准库 - io 模块](46-stdlib-io.md) | ✅ |
| 47 | [标准库 - math 模块](47-stdlib-math.md) | ✅ |
| 48 | [标准库 - os/string/time/path](48-stdlib-os-string-time.md) | ✅ |
| 49 | [标准库 - json 模块](49-stdlib-json.md) | ✅ |
| 50 | [内置类型方法 - String](50-builtin-methods-string.md) | ✅ |
| 51 | [内置类型方法 - List/Dict/Set](51-builtin-methods-list-dict-set.md) | ✅ |
| 60 | [标准库 - gc 模块](60-stdlib-gc.md) | ✅ |

### C API（Phase 6 末尾 — MVP 同步特性）

| # | 任务 | 状态 |
|---|---|---|
| 65 | [C API 基础设施（cbindgen + 手写类型头文件 + 构建集成）](65-capi-infrastructure.md) | ✅ |
| 66 | [C API — VM 生命周期与配置](66-capi-vm.md) | ✅ |
| 67 | [C API — 值创建与类型判断](67-capi-value-creation.md) | ✅ |
| 68 | [C API — 值转换、比较与通用操作](68-capi-value-convert.md) | ✅ |
| 69 | [C API — 集合操作（List/Dict/Tuple/Set + 迭代器）](69-capi-collections.md) | ✅ |
| 70 | [C API — 函数调用](70-capi-call.md) | ✅ |
| 71 | [C API — 异常处理](71-capi-error.md) | ✅ |
| 72 | [C API — C 扩展模块注册与动态加载](72-capi-module.md) | ✅ |
| 73 | [C API — Class 操作](73-capi-class.md) | ✅ |
| 74 | [C API — GC 交互（Root/写屏障/Finalizer/控制/统计）](74-capi-gc.md) | ✅ |
| 75 | [C API — 集成测试（嵌入 + 扩展端到端）](75-capi-integration-test.md) | ✅ |

> 设计规格见 [13-capi](../13-capi.md)。

## Phase 7 — 并发

| # | 任务 | 状态 |
|---|---|---|
| 53 | [async/await 协程](53-async-await.md) | ✅ |
| 54 | [Channel 通信](54-channel.md) | ✅ |
| 55 | [go 关键字与并发执行](55-go-concurrency.md) | ✅ |
| 59 | [select 语句（多 channel 复用）](59-select.md) | ✅ |
| 61 | [标准库 - async 模块](61-stdlib-async.md) | ✅ |

## Phase 7 后 — Async/Channel/Generator C API

| # | 任务 | 状态 |
|---|---|---|
| 76 | [C API — Async/Channel/Generator](76-capi-async-channel.md) | ✅ |

> 设计规格见 [13-capi](../13-capi.md) § call.h（异步部分）。依赖 Phase 7 并发特性（task 53-55）。

## Phase 7.5 — 并发 GC 优化（待创建）

| # | 任务 | 状态 |
|---|---|---|
| 62 | [并发标记（tri-color + 写屏障）](62-concurrent-mark.md) | ✅ |
| 63 | [并发清扫与 Compaction](63-concurrent-sweep-compaction.md) | ✅ |
| 64 | [GC 调优接口与自适应阈值](64-gc-tuning.md) | ✅ |
| 77 | [C API — 并发 GC 交互（并发写屏障/调优）](77-capi-concurrent-gc.md) | ✅ |
| — | Old 代 arena 迁移 + Compaction 实装（task 63 §8 延后，待创建） | 待创建 |

> 设计规格见 [14-gc](../14-gc.md) Phase 7.5。Task 77 与 62-64 协调。
> task 63 §8：Compaction 度量已落地（`fragmentation_ratio`/`should_compact`），但实装需先将
> Old 代迁移为连续 arena + free-list（当前为散布 Box，无碎片）。占位行如上，防 `compact_old` 死代码。

## Phase 8 — REPL + 工具链

| # | 任务 | 状态 |
|---|---|---|
| 56 | [REPL 交互式命令行](56-repl.md) | ✅ |
| 57 | [友好错误信息与堆栈跟踪](57-error-messages.md) | ✅ |
| 58 | [CLI 工具链](58-cli.md) | ✅ |

## Phase 9 — 标准库扩展

> 设计总纲见 [16-stdlib-expansion](../16-stdlib-expansion.md)。
> 顺序依赖：78 → 79 → 80 → 84（.ms 机制与 sorted_by 为 84 前置）；
> 81/82/83/85 相对独立，86 依赖 78（并复用 61 的 Future/EventLoop）。

| # | 任务 | 状态 |
|---|---|---|
| 78 | [stdlib.rs 目录拆分（纯移动重构）](78-stdlib-split.md) | ✅ |
| 79 | [嵌入式 .ms 标准库机制](79-embedded-ms.md) | ✅ |
| 80 | [标准库 - math/string 扩充与排序增强](80-stdlib-math-string-sort.md) | ✅ |
| 81 | [标准库 - random/encoding/uuid 模块](81-stdlib-random-encoding-uuid.md) | ✅ |
| 82 | [标准库 - fs 模块 / os 扩充 / sys 模块](82-stdlib-fs-os-sys.md) | ✅ |
| 83 | [标准库 - time 模块扩充](83-stdlib-time.md) | ✅ |
| 84 | [标准库 - heapq/collections/itertools/functools/test](84-stdlib-collections-itertools-functools-test.md) | ✅ |
| 85 | [标准库 - regex/hash 模块](85-stdlib-regex-hash.md) | ⬜ |
| 86 | [标准库 - http 模块与 external completion 架构](86-stdlib-http.md) | ⬜ |

