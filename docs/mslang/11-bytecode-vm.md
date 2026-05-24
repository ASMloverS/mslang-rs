# 字节码与虚拟机设计

## 概述

mslang 采用**编译到字节码 + 栈式虚拟机**的执行模型：

```
源码 (.ms) → Lexer → Token 流 → Parser → AST → Compiler → 字节码 → VM 执行
```

## 字节码指令集 (OpCode)

### 设计原则

- 栈式虚拟机：操作数从栈顶弹出，结果压入栈顶
- 指令长度：1 字节操作码 + 可变长度操作数
- 常量池：字符串、数字等常量存储在独立的常量池中

### 常量加载

| OpCode | 操作数 | 说明 |
|---|---|---|
| `CONSTANT` | `idx(2)` | 将常量池[idx]压栈 |
| `NIL` | — | 压入 nil |
| `TRUE` | — | 压入 true |
| `FALSE` | — | 压入 false |

### 局部变量

| OpCode | 操作数 | 说明 |
|---|---|---|
| `LOAD_LOCAL` | `slot(1)` | 将局部变量[slot]压栈 |
| `STORE_LOCAL` | `slot(1)` | 将栈顶存入局部变量[slot] |
| `LOAD_UPVALUE` | `idx(1)` | 将上值[idx]压栈 |
| `STORE_UPVALUE` | `idx(1)` | 将栈顶存入上值[idx] |
| `LOAD_GLOBAL` | `name_idx(2)` | 将全局变量压栈 |
| `STORE_GLOBAL` | `name_idx(2)` | 将栈顶存入全局变量 |

### 属性与下标

| OpCode | 操作数 | 说明 |
|---|---|---|
| `GET_ATTR` | `name_idx(2)` | obj.attr |
| `SET_ATTR` | `name_idx(2)` | obj.attr = val |
| `GET_INDEX` | — | obj[key] |
| `SET_INDEX` | — | obj[key] = val |
| `GET_SLICE` | `flags(1)` | obj[start:stop:step] |

### 算术运算

| OpCode | 说明 |
|---|---|
| `ADD` | a + b |
| `SUBTRACT` | a - b |
| `MULTIPLY` | a * b |
| `DIVIDE` | a / b |
| `FLOOR_DIV` | a // b |
| `MODULO` | a % b |
| `POWER` | a ** b |
| `NEGATE` | -a |

### 位运算

| OpCode | 说明 |
|---|---|
| `BIT_AND` | a & b |
| `BIT_OR` | a \| b |
| `BIT_XOR` | a ^ b |
| `BIT_NOT` | ~a |
| `LEFT_SHIFT` | a << b |
| `RIGHT_SHIFT` | a >> b |

### 比较运算

| OpCode | 操作数 | 说明 |
|---|---|---|
| `EQUAL` | — | a == b |
| `NOT_EQUAL` | — | a != b |
| `LESS` | — | a < b |
| `GREATER` | — | a > b |
| `LESS_EQUAL` | — | a <= b |
| `GREATER_EQUAL` | — | a >= b |
| `IS` | — | a is b |
| `IN` | — | a in b |

### 逻辑运算

| OpCode | 操作数 | 说明 |
|---|---|---|
| `NOT` | — | not a（逻辑取反） |
| `JUMP_IF_FALSE` | `offset(2)` | 为 falsy 则跳转 |
| `JUMP_IF_TRUE` | `offset(2)` | 为 truthy 则跳转 |
| `JUMP` | `offset(2)` | 无条件跳转 |
| `POP` | — | 弹出栈顶 |
| `DUP` | — | 复制栈顶 |

### 控制流

| OpCode | 操作数 | 说明 |
|---|---|---|
| `JUMP` | `offset(2)` | 无条件跳转 |
| `JUMP_BACK` | `offset(2)` | 向后跳转（循环用） |
| `LOOP` | `offset(2)` | 循环跳转 |
| `BREAK` | `offset(2)` | 跳出循环 |
| `CONTINUE` | `offset(2)` | 跳到循环开头 |

跳转偏移量为有符号 16 位整数，相对于当前指令位置。

### 函数调用

| OpCode | 操作数 | 说明 |
|---|---|---|
| `CALL` | `argc(1)` | 调用函数（argc 个参数） |
| `RETURN` | — | 从函数返回 |
| `TAIL_CALL` | `argc(1)` | 尾调用（优化） |

### 闭包

| OpCode | 操作数 | 说明 |
|---|---|---|
| `CLOSURE` | `func_idx(2)` | 创建闭包 |
| `CLOSE_UPVALUE` | — | 关闭上值 |

### 迭代

| OpCode | 操作数 | 说明 |
|---|---|---|
| `ITERATOR` | — | 创建迭代器 |
| `FOR_ITER` | `offset(2)` | 迭代下一步，结束则跳转 |
| `YIELD` | — | yield 暂停 |
| `YIELD_FROM` | — | yield from 委托 |

### 构造器

| OpCode | 操作数 | 说明 |
|---|---|---|
| `BUILD_LIST` | `count(1)` | 从栈顶 count 个元素构建 list |
| `BUILD_DICT` | `count(1)` | 从栈顶 count 对元素构建 dict |
| `BUILD_TUPLE` | `count(1)` | 从栈顶 count 个元素构建 tuple |
| `BUILD_SET` | `count(1)` | 从栈顶 count 个元素构建 set |
| `UNPACK` | `count(1)` | 解包序列到栈 |

### 类与实例

| OpCode | 操作数 | 说明 |
|---|---|---|
| `CLASS` | `name_idx(2)` | 创建类 |
| `METHOD` | `name_idx(2)` | 定义方法 |
| `INHERIT` | — | 继承父类 |
| `GET_SUPER` | `name_idx(2)` | 获取父类方法 |
| `INVOKE` | `name_idx(2), argc(1)` | 直接调用方法（优化） |

### defer

| OpCode | 操作数 | 说明 |
|---|---|---|
| `DEFER` | — | 注册 defer 调用 |
| `EXEC_DEFER` | — | 执行所有 defer（函数返回前） |

### 异常

| OpCode | 操作数 | 说明 |
|---|---|---|
| `THROW` | — | 抛出异常 |
| `TRY_ENTER` | `handler_offset(2)` | 进入 try 块 |
| `TRY_EXIT` | — | 离开 try 块 |
| `CATCH` | `type_idx(2)` | 捕获异常 |

### 其他

| OpCode | 操作数 | 说明 |
|---|---|---|
| `ASSERT` | — | 断言 |
| `IMPORT` | `module_idx(2)` | 导入模块 |
| `CHANNEL` | `buffer_size(1)` | 创建 channel |
| `SEND` | — | channel 发送 |
| `RECEIVE` | — | channel 接收 |
| `GO` | — | 启动协程 |
| `AWAIT` | — | await Future |
| `HALT` | — | 程序结束 |

## 编译单元 (CompilationUnit)

每个编译单元对应一个函数或脚本顶层：

```
CompilationUnit {
    constants: Vec<Value>          // 常量池
    code: Vec<u8>                  // 字节码
    lines: Vec<(usize, usize)>     // 行号信息（用于调试）
    locals: Vec<Local>             // 局部变量表
    upvalues: Vec<Upvalue>         // 上值表
    parent: Option<&CompilationUnit>
}
```

### Local

```
Local {
    name: String
    depth: usize          // 作用域深度
    is_captured: bool     // 是否被闭包捕获
}
```

### Upvalue

```
Upvalue {
    index: usize          // 外层局部变量索引
    is_local: bool        // 是直接的外层局部变量，还是外层的上值
}
```

## 对象系统

所有运行时值统一表示为 `Object`：

```rust
enum Object {
    Nil,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(Gc<String>),
    List(Gc<Vec<Object>>),
    Dict(Gc<DictMap>),
    Tuple(Gc<Vec<Object>>),
    Set(Gc<HashSet<Object>>),
    Function(Gc<Function>),
    Closure(Gc<Closure>),
    Class(Gc<Class>),
    Instance(Gc<Instance>),
    Module(Gc<Module>),
    Iterator(Gc<Iterator>),
    Generator(Gc<Generator>),
    Future(Gc<Future>),
    Channel(Gc<Channel>),
    BoundMethod(Gc<BoundMethod>),
}
```

### Function

```
Function {
    name: String
    arity: usize              // 必需参数数量
    code: Vec<u8>             // 字节码
    constants: Vec<Value>     // 常量池
    upvalue_count: usize
}
```

### Closure

```
Closure {
    function: Gc<Function>
    upvalues: Vec<Gc<Upvalue>>
}
```

### Class

```
Class {
    name: String
    methods: HashMap<String, Gc<Closure>>
    parent: Option<Gc<Class>>
}
```

### Instance

```
Instance {
    class: Gc<Class>
    fields: HashMap<String, Object>
}
```

## 虚拟机 (VM)

### 核心结构

```rust
struct VM {
    stack: Vec<Object>,                 // 值栈
    stack_base: usize,                  // 当前帧的栈基址
    call_stack: Vec<CallFrame>,         // 调用栈
    globals: HashMap<String, Object>,   // 全局变量
    defer_stack: Vec<DeferEntry>,       // defer 栈
    open_upvalues: Vec<Gc<Upvalue>>,    // 开放的上值
    event_loop: EventLoop,              // 事件循环（并发用）
    gc: GarbageCollector,               // GC
}
```

### CallFrame

```rust
struct CallFrame {
    closure: Gc<Closure>,    // 被调用的闭包
    ip: usize,               // 程序计数器
    stack_base: usize,       // 栈基址
    defer_stack_base: usize, // defer 栈基址
}
```

### 执行循环

```rust
fn run(&mut self) {
    loop {
        let opcode = self.read_byte();
        match opcode {
            OpCode::CONSTANT => { ... }
            OpCode::ADD => { ... }
            OpCode::CALL => { ... }
            // ...
            OpCode::HALT => return,
        }
    }
}
```

## 垃圾回收

### 策略：引用计数 + 标记-清除

#### 引用计数

- 每个 `Gc<T>` 包含引用计数
- `Gc::clone()` 增加计数
- `Gc::drop()` 减少计数
- 计数归零时立即释放

#### 标记-清除 GC

- 引用计数无法处理循环引用
- 定期运行标记-清除 GC 清理循环引用
- 触发条件：分配次数超过阈值

#### 标记-清除流程

1. **标记阶段**：从根集（栈、全局变量、调用栈、开放上值）出发，递归标记所有可达对象
2. **清除阶段**：遍历所有已分配对象，释放未标记的对象
3. **重置**：清除所有对象的标记

```rust
struct GarbageCollector {
    objects: Vec<GcBox>,           // 所有分配的对象
    bytes_allocated: usize,
    next_gc: usize,                // 下次 GC 触发阈值
    gray_stack: Vec<GcBox>,       // 灰色对象栈（用于标记）
}
```

### 写屏障

当将一个对象引用写入另一个对象时（如 `list[i] = obj`），可能需要写屏障以确保 GC 正确性。

MVP 阶段使用简单的 stop-the-world GC，写屏障暂不需要。

## 调试信息

### 行号表

编译单元中维护行号映射：

```
lines: Vec<(instruction_offset, source_line)>
```

用于运行时错误时输出堆栈跟踪。

### 堆栈跟踪格式

```
Error: division by zero
    at divmod (math.ms:5)
    at calculate (main.ms:12)
    at <main> (main.ms:20)
```

## 生成器执行模型

### Generator 帧

生成器需要保存完整的执行状态以便暂停和恢复。Generator 对象持有独立的栈帧副本：

```
Generator {
    frame: CallFrame         # 独立的调用帧（含 IP、栈基址）
    stack: Vec<Object>       # 独立的值栈副本
    locals: Vec<Object>      # 局部变量快照
    state: GeneratorState    # 状态
}

enum GeneratorState {
    Suspended,
    Running,
    Exhausted,
}
```

### yield 执行流程

1. `YIELD` 指令执行时：
   - 将当前栈顶值作为产出值保存
   - 快照当前 CallFrame（IP、栈、局部变量）到 Generator 对象
   - 将 Generator 状态设为 `Suspended`
   - 将产出值压入调用者的栈中
   - VM 从调用者的 `FOR_ITER` 继续执行

2. `FOR_ITER` / `__next__()` 恢复时：
   - 从 Generator 对象恢复 CallFrame（IP、栈、局部变量）
   - 将 Generator 状态设为 `Running`
   - VM 跳转到 Generator 的恢复点继续执行

3. 生成器函数执行完毕（return 或函数结束）：
   - 将 Generator 状态设为 `Exhausted`
   - `FOR_ITER` 检测到 `Exhausted` 后跳出循环

### yield from

`YIELD_FROM` 将当前 Generator 的执行委托给另一个可迭代对象：
- 内部创建子迭代器
- 每次产出时直接传递子迭代器的值，不经过中间层
- 子迭代器耗尽后，当前 Generator 继续

## 异步执行模型

### 协程与事件循环集成

async/await 与 VM 的核心执行循环集成方式：

```
EventLoop {
    ready_queue: Vec<Coroutine>      # 就绪协程队列
    paused: Vec<PausedCoroutine>     # 等待 Future 的暂停协程
}

PausedCoroutine {
    coroutine: Coroutine
    waiting_on: Gc<Future>           # 等待的 Future
    frame: CallFrame                 # 暂停时的执行帧快照
}

Coroutine {
    frame: CallFrame                 # 当前执行帧
    defer_stack: Vec<DeferEntry>     # 协程自己的 defer 栈
}
```

### AWAIT 指令流程

1. 求值 await 后的表达式，得到 Future 对象
2. 检查 Future 状态：
   - **Resolved**：直接将结果压栈，继续执行（不暂停）
   - **Rejected**：抛出异常
   - **Pending**：
     a. 快照当前 CallFrame 到 `PausedCoroutine`
     b. 将当前协程加入 `EventLoop.paused`
     c. VM 从 `ready_queue` 取下一个协程继续执行
3. 当 Future 完成（由 IO 回调或其他协程触发）：
   - 将暂停的协程从 `paused` 移到 `ready_queue`
   - 恢复时将 Future 结果压栈，继续执行

### GO 指令流程

1. 将表达式（通常为函数调用或闭包）包装为 `Coroutine`
2. 加入 `EventLoop.ready_queue`
3. 当前协程继续执行（不等待）

### 顶层 await

主脚本作为主协程在事件循环中执行。当遇到 `await` 时，主协程暂停，事件循环调度其他协程。主协程完成后程序退出。

### disassemble

调试模式下可以反汇编字节码：

```
== main.ms ==
0000 CONSTANT     0   "hello"
0002 CONSTANT     1   "world"
0004 ADD
0005 HALT
```
