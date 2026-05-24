# 分阶段实现计划

## 项目结构

```
mslang-rs/
├── Cargo.toml
├── src/
│   ├── main.rs                 # CLI 入口
│   ├── lib.rs                  # 库入口
│   ├── lexer/
│   │   ├── mod.rs
│   │   └── token.rs            # Token 定义
│   ├── ast/
│   │   ├── mod.rs
│   │   └── node.rs             # AST 节点
│   ├── parser/
│   │   ├── mod.rs
│   │   ├── expression.rs       # 表达式解析
│   │   └── statement.rs        # 语句解析
│   ├── compiler/
│   │   ├── mod.rs
│   │   └── opcode.rs           # 字节码定义
│   ├── vm/
│   │   ├── mod.rs              # VM 主循环
│   │   ├── object.rs           # Object 系统
│   │   ├── frame.rs            # 调用帧
│   │   ├── gc.rs               # GC
│   │   ├── builtins.rs         # 内置函数
│   │   └── stdlib.rs           # 内置类型方法
│   ├── module/
│   │   ├── mod.rs
│   │   └── resolver.rs         # 模块解析
│   ├── async_runtime/
│   │   ├── mod.rs              # 事件循环
│   │   └── channel.rs          # Channel 实现
│   └── repl/
│       └── mod.rs              # REPL
├── stdlib/                     # 标准库（.ms 文件）
│   ├── io.ms
│   ├── math.ms
│   ├── os.ms
│   ├── string.ms
│   ├── time.ms
│   ├── json.ms
│   ├── path.ms
│   └── async.ms
└── tests/
    ├── lexer_tests.rs
    ├── parser_tests.rs
    ├── compiler_tests.rs
    ├── vm_tests.rs
    └── integration/
        ├── basic.ms
        ├── functions.ms
        ├── classes.ms
        └── ...
```

## Phase 1: 基础设施（Lexer + Parser）

**目标**：能将源码正确解析为 AST

### 1.1 项目骨架

- [ ] 初始化 Cargo 项目
- [ ] 定义模块结构
- [ ] CLI 入口（`main.rs`）
- [ ] 错误类型定义（`Error` enum）

**验证**：`cargo build` 编译通过

### 1.2 Token 定义

- [ ] 定义 `TokenKind` 枚举（所有 token 类型）
- [ ] 定义 `Token` 结构体（kind, lexeme, line, column）
- [ ] 定义 `Span` 结构体（位置信息）

**验证**：Token 定义完整，覆盖所有词法元素

### 1.3 词法分析器

- [ ] 基础框架：`Lexer::new(source) -> Lexer`
- [ ] 跳过空白符和注释
- [ ] 数值字面量（整数、浮点、十六进制、二进制、八进制）
- [ ] 字符串字面量（双引号 + 转义序列）
- [ ] 标识符和关键字
- [ ] 运算符（所有单字符和多字符运算符）
- [ ] 分隔符
- [ ] 错误恢复和错误报告

**验证**：所有 token 类型有对应的测试用例

### 1.4 AST 定义

- [ ] 表达式节点（字面量、二元、一元、调用、下标、属性等）
- [ ] 语句节点（变量、赋值、if、while、for、fn、class、import 等）
- [ ] 程序节点（顶层语句列表）
- [ ] 使用 `Box<T>` 和 `Vec<T>` 构建树结构

**验证**：AST 节点能表示所有语法结构

### 1.5 语法分析器

- [ ] Parser 框架（递归下降）
- [ ] 表达式解析（优先级爬升法）
- [ ] 语句解析
- [ ] 块解析（花括号）
- [ ] 函数定义解析
- [ ] 错误恢复（panic mode）和错误报告

**验证**：能正确解析以下结构：
```
- 算术表达式和运算符优先级
- 变量声明（var, :=, =）
- if/elif/else
- while / for..in
- fn 定义和调用
- 列表/dict/tuple/set 字面量
- 属性访问和方法调用
- 匿名函数
```

### 1.6 测试

- [ ] Lexer 单元测试
- [ ] Parser 单元测试
- [ ] 快照测试（AST 输出对比）

---

## Phase 2: 字节码编译 + VM 核心

**目标**：能执行基本脚本

### 2.1 OpCode 定义

- [ ] 定义所有字节码指令
- [ ] 反汇编器（调试用）

**验证**：所有 OpCode 有对应的字符串表示

### 2.2 编译器

- [ ] Compiler 框架（CompilationUnit）
- [ ] 常量池管理
- [ ] 局部变量表管理
- [ ] 表达式编译（栈式）
- [ ] 语句编译
- [ ] 控制流（if/while/for）的跳转指令
- [ ] 行号表生成

**验证**：基本表达式能正确编译为字节码

### 2.3 Object 系统

- [ ] `Object` 枚举定义
- [ ] 基本类型：Nil, Bool, Int, Float, String
- [ ] 集合类型：List, Dict, Tuple, Set
- [ ] 类型转换方法
- [ ] 运算符实现（Add, Sub, Eq 等）

**验证**：所有内置类型的运算正确

### 2.4 虚拟机核心

- [ ] VM 框架（栈、全局变量表）
- [ ] 指令执行循环
- [ ] 常量加载指令
- [ ] 算术运算指令
- [ ] 比较运算指令
- [ ] 变量存取指令
- [ ] 控制流指令（JUMP, JUMP_IF_FALSE）
- [ ] PRINT 和 HALT

**验证**：以下脚本可以执行：
```ms
x = 10
y = 20
print(x + y)        # 30

if x > 5 {
    print("big")    # big
}

i = 0
while i < 5 {
    print(i)
    i += 1
}
```

> **注意**：由于 mslang 支持顶层 await（见 [08-concurrency](08-concurrency.md)），VM 核心需要在初始设计中就支持可暂停的执行帧。建议在 CallFrame 设计时预留帧快照/恢复能力，以便 Phase 7 无缝集成 async/await。

### 2.5 内置函数

- [ ] print / println
- [ ] type / len / range
- [ ] int / float / str / bool / list / tuple / set / dict
- [ ] abs / max / min / sum

**验证**：内置函数正确调用

---

## Phase 3: 函数 + 闭包

**目标**：完整的函数系统

### 3.1 调用帧

- [ ] CallFrame 定义
- [ ] CALL 指令
- [ ] RETURN 指令
- [ ] 调用栈管理

**验证**：函数调用和返回正确

### 3.2 闭包

- [ ] Upvalue 机制
- [ ] CLOSURE 指令
- [ ] LOAD_UPVALUE / STORE_UPVALUE
- [ ] 开放上值和关闭上值
- [ ] 闭包捕获语义

**验证**：闭包正确捕获和修改外层变量

### 3.3 匿名函数

- [ ] 解析匿名函数
- [ ] 编译匿名函数
- [ ] 匿名函数作为表达式

**验证**：`fn(x) { return x * 2 }` 正确工作

### 3.4 多返回值

- [ ] 元组构造
- [ ] 元组解包赋值
- [ ] 多变量赋值

**验证**：`a, b = fn()` 正确解包

### 3.5 默认参数与可变参数

- [ ] 默认参数值编译
- [ ] `*args` 可变参数编译

**验证**：默认参数和可变参数正确工作

---

## Phase 4: 控制流 + 高级语法

**目标**：完整的控制流和语法糖

### 4.1 for..in 循环

- [ ] ITERATOR 指令
- [ ] FOR_ITER 指令
- [ ] 可迭代协议（__iter__, __next__）

**验证**：for..in 遍历各种可迭代对象

### 4.2 列表推导式

- [ ] 解析推导式语法
- [ ] 编译推导式（编译为循环 + 构建列表）
- [ ] 带过滤条件的推导式
- [ ] 嵌套推导式

**验证**：`[x*x for x in range(10)]` 正确求值

### 4.3 切片操作

- [ ] 解析切片语法
- [ ] GET_SLICE 指令
- [ ] 切片语义实现（start:stop:step, 负索引, 越界处理）

**验证**：切片操作返回正确结果

### 4.4 defer

- [ ] DEFER 指令
- [ ] defer 栈管理
- [ ] LIFO 执行语义
- [ ] defer 与异常的交互

**验证**：defer 在函数返回前按 LIFO 执行

### 4.5 try/except/finally

- [ ] TRY_ENTER / TRY_EXIT 指令
- [ ] CATCH 指令
- [ ] 异常对象创建
- [ ] 异常类型匹配
- [ ] finally 语义
- [ ] 异常传播

**验证**：异常被正确捕获和处理

### 4.6 with 语句

- [ ] 解析 with 语句
- [ ] __enter__ / __exit__ 调用
- [ ] 异常传递给 __exit__

**验证**：with 语句正确管理资源

### 4.7 生成器 / yield

- [ ] 解析 yield 表达式
- [ ] Generator 对象
- [ ] YIELD / YIELD_FROM 指令
- [ ] 生成器执行上下文保存/恢复

**验证**：生成器正确产生值

---

## Phase 5: Class + 面向对象

**目标**：Python 风格 class 系统

### 5.1 Class 对象

- [ ] Class 定义
- [ ] CLASS / METHOD 指令
- [ ] 实例化（__init__）
- [ ] 属性访问（GET_ATTR / SET_ATTR）

**验证**：能定义类和创建实例

### 5.2 self 和实例属性

- [ ] self 绑定
- [ ] 实例属性存储
- [ ] 动态属性添加

**验证**：self 正确引用实例

### 5.3 继承

- [ ] INHERIT 指令
- [ ] 方法解析顺序
- [ ] super 关键字
- [ ] 方法覆盖

**验证**：继承和方法覆盖正确工作

### 5.4 魔术方法

- [ ] __init__, __repr__, __str__
- [ ] __eq__, __lt__ 等比较方法
- [ ] __add__, __sub__ 等算术方法
- [ ] __len__, __getitem__, __setitem__
- [ ] __contains__, __iter__, __next__
- [ ] __enter__, __exit__
- [ ] __call__

**验证**：魔术方法在对应场景自动调用

### 5.5 装饰器

- [ ] 解析装饰器语法
- [ ] 编译装饰器（函数变换）
- [ ] 多重装饰器
- [ ] 带参数的装饰器

**验证**：装饰器正确包装函数

---

## Phase 6: 模块系统 + 标准库

**目标**：import 和基础标准库

### 6.1 模块系统

- [ ] IMPORT 指令
- [ ] 模块搜索路径
- [ ] 文件模块加载
- [ ] 包模块（目录 + index.ms）
- [ ] 模块缓存
- [ ] from...import 实现
- [ ] import as 实现

**验证**：跨文件模块导入正确工作

### 6.2 标准库（Rust 原生实现）

- [ ] `io` — 文件读写
- [ ] `math` — 数学函数
- [ ] `os` — 环境变量、工作目录
- [ ] `string` — 字符串工具
- [ ] `time` — 时间函数

**验证**：标准库函数可调用

---

## Phase 7: 并发

**目标**：async/await + channel

### 7.1 async/await

- [ ] async fn 解析和编译
- [ ] Future 对象
- [ ] AWAIT 指令
- [ ] 事件循环（EventLoop）
- [ ] 协程调度

**验证**：async/await 正确暂停和恢复

### 7.2 channel

- [ ] Channel 对象
- [ ] 有缓冲 channel
- [ ] 无缓冲 channel
- [ ] SEND / RECEIVE 指令
- [ ] channel 关闭和遍历

**验证**：channel 正确传递数据

### 7.3 go 关键字

- [ ] GO 指令
- [ ] 协程启动
- [ ] 并发执行

**验证**：go 启动的协程并发执行

> **注意**：`select`（多 channel 复用）已保留语法但未列入任何 Phase，计划在并发模型稳定后作为 Phase 7 增量实现。

---

## Phase 8: REPL + 工具链

**目标**：完善开发体验

### 8.1 REPL

- [ ] 交互式命令行
- [ ] 多行输入支持
- [ ] 表达式求值
- [ ] 上下文保持（变量、函数持久化）

**验证**：REPL 可以交互执行代码

### 8.2 错误信息

- [ ] 行号标注
- [ ] 错误高亮
- [ ] 堆栈跟踪格式化
- [ ] 友好的错误提示

**验证**：错误信息清晰有用

### 8.3 CLI

- [ ] `ms run script.ms`
- [ ] `ms eval "expression"`
- [ ] `ms repl`
- [ ] `ms check script.ms`
- [ ] `ms version`

**验证**：CLI 命令正确工作

---

## 里程碑时间线（建议）

| Phase | 内容 | 预估工作量 |
|---|---|---|
| Phase 1 | Lexer + Parser | 基础 |
| Phase 2 | Compiler + VM 核心 | 核心 |
| Phase 3 | 函数 + 闭包 | 核心 |
| Phase 4 | 控制流 + 高级语法 | 扩展 |
| Phase 5 | Class + OOP | 扩展 |
| Phase 6 | 模块 + 标准库 | 扩展 |
| Phase 7 | 并发 | 高级 |
| Phase 8 | REPL + 工具链 | 完善 |

**MVP = Phase 1 + 2 + 3**：能执行包含函数和闭包的脚本。
