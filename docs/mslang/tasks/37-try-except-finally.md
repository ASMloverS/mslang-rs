# try/except/finally 异常处理

## 所属阶段
Phase 4.5 - 控制流 + 高级语法

## 前置任务
36-defer

## 目标
实现 try/except/finally 异常处理机制，包括异常对象、内置异常类型层级、throw 语句、异常传播、异常类型匹配。

## 设计规格

参照 [05-control-flow](../05-control-flow.md) § 错误处理：

### 语法

```
try_stmt    = "try" block except_clause* finally_clause?
except_clause = "except" type_spec? ("as" IDENTIFIER)? block
type_spec     = IDENTIFIER ("." IDENTIFIER)*
finally_clause = "finally" block
```

### 字节码指令

参照 [11-bytecode-vm](../11-bytecode-vm.md) § 异常。本 task 在该表基础上**新增 3 条 opcode**（spec 回写，见 §11）：

| OpCode | 操作数 | 说明 |
|---|---|---|
| `TRY_ENTER` | `handler_offset(2) finally_offset(2)` | 进入 try 块，注册异常处理器（finally_offset=0xFFFF 表示无 finally） |
| `TRY_EXIT` | — | 离开 try 块，注销异常处理器（正常完成 / early-exit 出口均须 emit） |
| `CATCH` | `name_idx(2)` | 栈顶异常的类名是否匹配常量池[name_idx]（字符串类名，含父类链）；压 bool |
| `THROW` | — | 弹出栈顶异常对象并抛出（`throw <expr>`；string 自动包装为 RuntimeError） |
| `RETHROW` | — | **新增**：重抛当前帧 `current_exc`（裸 `throw`）；若为空抛 `RuntimeError("nothing to rethrow")` |
| `FINALLY_END` | — | **新增**：finally 块末尾；若 `current_exc` 非空则重抛（finally-on-propagation），否则继续 |
| `CLEAR_CURRENT_EXC` | — | **新增**：except 命中分支清除 `current_exc`（标记异常已处理，FINALLY_END 不误重抛） |

> **CATCH 操作数语义**：`name_idx(2)` 指向**字符串常量**（异常类名，如 `"ValueError"`），不是 Class 对象。匹配含父类链（查静态 MRO 表，见 §5）。裸 `except`（无类型）不 emit CATCH，直接匹配。

### 异常对象的 TypeTag（本 task 新增，spec 回写见 §11）

Phase 4.5 阶段 Class/Instance 尚未实现（Phase 5），故异常对象采用**最小自包含表示**，不依赖 OOP：

| TypeTag | 值 | 用途 |
|---|---|---|
| `EXCEPTION` | 18 | 异常实例（`MsException`：class_name + message + traceback + cause） |
| `EXCEPTION_CLASS` | 19 | 内置异常类对象（`MsExceptionClass`：仅 name），注册为全局变量，CALL 时构造 EXCEPTION |

### 异常对象属性

```
Error
├── message      # 错误消息（string）
├── type         # 错误类型名（string）
├── traceback    # 堆栈跟踪（string）
├── __cause__    # 链式异常中的原始异常（Error 或 nil）
```

### 内置异常类型层级

```
Error
├── ValueError
├── TypeError
├── IndexError
├── KeyError
├── AttributeError
├── NameError
├── RuntimeError
├── IOError
├── ZeroDivisionError
├── OverflowError
├── StopIteration
└── GeneratorExit       # 生成器关闭（内部异常，不可被用户 except 捕获）
```

### 语义

1. 执行 `try` 块
2. 若发生异常，按顺序检查 `except` 子句：
   - 不带类型：匹配所有异常
   - 带类型：匹配该类型及其子类型
   - `as name`：将异常对象绑定到 `name`
3. 无论是否发生异常，`finally` 块总是执行
4. 异常沿调用栈向上传播直到被捕获

## 实现细节

### 1. 异常对象实现（最小表示，不依赖 Phase 5 OOP）

`src/vm/object.rs` 新增两个 TypeTag（=18/19，接 `UPVALUE=17` 之后）与两个结构体：

```rust
/// 内置异常类对象（TypeTag::EXCEPTION_CLASS）。仅承载类名，作为全局变量；
/// 被 CALL 时构造 MsException（见 CALL handler 新分支）。Phase 5 升级为正式 Class（§10）。
#[repr(C)]
pub struct MsExceptionClass {
    pub header: MsObjHeader,
    pub name: String,        // "ValueError" / "TypeError" / ... / "Error"
}

/// 异常实例（TypeTag::EXCEPTION）。自包含 4 字段，对应 05-control-flow.md:216-221 的属性。
#[repr(C)]
pub struct MsException {
    pub header: MsObjHeader,
    pub class_name: String,  // → e.type
    pub message: Object,     // → e.message（string）
    pub traceback: Object,   // → e.traceback（string，捕获点见 §9）
    pub cause: Object,       // → e.__cause__（Exception 或 Nil）
}
```

配 `alloc_exception_class(name) -> Object::Ref`、`alloc_exception(class_name, message, traceback, cause) -> Object::Ref`、`read_exception(ptr) -> &MsException`、`read_exception_mut(ptr) -> &mut MsException`、`read_exception_class(ptr) -> &MsExceptionClass`（参照 task 22 集合类型的 alloc/read 模式）。

**异常层级注册**（`VM::new` 调用 `init_exception_classes`）：为 12 个内置类名各 `alloc_exception_class` 并 `globals.insert(name, cls)`（`Error` + ValueError/TypeError/IndexError/KeyError/AttributeError/NameError/RuntimeError/IOError/ZeroDivisionError/OverflowError/StopIteration/GeneratorExit）。父类关系**不**在对象内表达，而由 §5 的静态 MRO 表查表（避免 Phase 5 的 Class.parent 指针）。

**异常构造**（`throw ValueError("msg")`）：`ValueError("msg")` 是普通 Call 表达式（callee = 全局 `ValueError` = EXCEPTION_CLASS 对象）。CALL handler 新增分支——

```rust
// 在 OpCode::Call 的 callee 类型分派中（mod.rs:1156 CLOSURE 分支之后）新增：
Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::EXCEPTION_CLASS as u8 => {
    let cls_name = unsafe { read_exception_class(*ptr) }.name.clone();
    // 参数约定：第 1 个实参为 message（无参则 message = nil）。
    let message = if argc >= 1 { self.stack[callee_idx + 1].clone() } else { Object::Nil };
    // 多余实参（如自定义异常的 code）暂忽略；Phase 5 经 __init__ 处理。
    self.stack.truncate(callee_idx);          // 弹出 callee + args
    self.push(alloc_exception(&cls_name, message, alloc_string(""), Object::Nil))?;
}
```

——这样 `e = ValueError("x")`（不仅是 `throw`）也能构造异常对象，语义统一，且 Phase 5 可平滑替换为正式 Class + CALL-on-class。

**属性访问**（`e.message` / `e.type` / `e.traceback` / `e.__cause__`）：GET_ATTR handler 此前为 `unimplemented`（mod.rs:1370 兜底）。本 task 为 GET_ATTR 增加一个**仅处理 EXCEPTION** 的分支（其余类型仍留待 task 41/43）：

```rust
OpCode::GetAttr => {
    let name_idx = self.read_u16()?;
    let obj = self.pop()?;
    let attr = read_string_constant(self.call_stack.last()?.closure, name_idx);
    match &obj {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::EXCEPTION as u8 => {
            let exc = unsafe { read_exception(*ptr) };
            let val = match attr.as_str() {
                "message" => exc.message.clone(),
                "type"    => alloc_string(&exc.class_name),
                "traceback" => exc.traceback.clone(),
                "__cause__" => exc.cause.clone(),
                _ => return Err(format!("AttributeError: 'Error' has no attribute '{}'", attr)),
            };
            self.push(val)?;
        }
        _ => return Err("GET_ATTR for non-exception types: not yet implemented (task 41/43)".into()),
    }
}
```

### 2. 编译 try/except/finally

`src/compiler/statement.rs`。关键点：(a) CATCH 用**类名字符串常量**（不是 Class 对象、不重复压类型）；(b) try body 内所有 early-exit（return/break/continue）出口均须 emit TRY_EXIT，避免 handler 栈泄漏；(c) finally 块末尾 emit FINALLY_END，支持「异常未匹配 → 跑 finally → 重抛」。

```
编译 try { body } except T1 as e { h1 } except { h2 } finally { fin }:

1. emit TRY_ENTER handler_offset       → handler_offset = (except_dispatcher − TRY_ENTER 地址)
2. 编译 try body
   （body 内任何 return/break/continue 出口前：编译器插入 emit TRY_EXIT）
3. emit TRY_EXIT                       → body 正常完成，注销 handler
4. emit JUMP finally_start             → 正常路径跳过 dispatcher

5. except_dispatcher:                  → 异常入口（throw 跳到这里；栈顶为异常对象）
   // —— except T1 as e ——
6. emit DUP                            → 复制异常供后续绑定 / 不匹配时重抛
7. emit CATCH name_idx("T1")           → 弹出 DUP 副本比对，压 bool
8. emit JUMP_IF_FALSE next_except
9. emit POP                            → 弹 bool
10. emit STORE_LOCAL e                 → 绑定异常变量（栈顶是步骤 6 的副本）
11. emit POP_CLEAR_CURRENT_EXC         → 见注：本 except 命中，清 frame.current_exc（已处理）
    （实现上：用一个专用标志，或在 STORE 后由编译器知道「已处理」；最简：GET/SET 通过
      FINALLY_END 检测 current_exc —— 故此处须把 current_exc 置空。新增微操作或用 STORE+辅助）
12. 编译 h1
13. emit JUMP finally_start

14. next_except:                       // 裸 except（匹配所有）
15. emit POP                           → 弹步骤 6 的 DUP 副本（裸 except 不需要类型比对）
16. emit STORE_LOCAL _（或 POP）       → 裸 except 无 as：直接 POP；有 as 则绑定
17. （命中，清 current_exc —— 同步骤 11）
18. 编译 h2
19. emit JUMP finally_start

20. no_match:                          // 所有 except 均不匹配（dispatcher 走到这里）
    // current_exc 仍为 Some(原异常)（throw 进入 dispatcher 时设置，未被任何命中分支清除）
21. emit POP                           → 弹步骤 6 的 DUP 副本（异常本体已在 throw 时存入 frame.current_exc）
22. emit JUMP finally_start            // finally 末尾 FINALLY_END 见 current_exc 非空 → 重抛

23. finally_start:
24. 编译 fin
25. emit FINALLY_END                   → current_exc 非空则重抛，否则继续（正常/命中路径）
26. 结束
```

> **无 finally 时**：省略 finally_start 的代码，except 命中分支末尾直接 JUMP 到 try 语句结束；`no_match` 分支 emit `RETHROW`（重抛 current_exc，重新进入 throw 找外层 handler）。
> **无 except 仅 finally 时**：except_dispatcher 直接 = no_match 路径（POP + JUMP finally_start），finally 跑完 FINALLY_END 重抛。
> **`current_exc` 清除**：except 命中分支须把 frame.current_exc 置 None（异常已处理）；这是 FINALLY_END 区分「正常/命中后跑 finally」（不重抛）与「未匹配跑 finally」（重抛）的依据。用一个专用 opcode `CLEAR_CURRENT_EXC`（无操作数，spec 回写）或复用 STORE_LOCAL 后由编译器在命中分支插入。本 task 采用新增 `CLEAR_CURRENT_EXC`（共 3 条新 opcode：RETHROW / FINALLY_END / CLEAR_CURRENT_EXC）。

### 3. 异常处理器结构 + TRY_ENTER 指令

VM 新增字段 `exception_handlers: Vec<ExceptionHandler>`（`mod.rs` VM struct）与 CallFrame 新增 `current_exc: Option<Object>`（裸 throw 重抛 + finally-on-propagation 共用，见 §6/§7）。`exception_handlers` 与 `defer_stack` 一样按帧分区，但分区用 `frame_stack_base`（值栈基址）判定所属帧。

```rust
struct ExceptionHandler {
    catch_address: usize,       // except 分派器入口（throw 跳转点）
    has_finally: bool,          // 是否有 finally 块
    finally_address: Option<usize>, // finally 块入口
    frame_stack_base: usize,    // 所属帧的值栈基址（跨帧判定）
    scope_stack_base: usize,    // 进入 try 时值栈长度（unwind 时恢复栈平衡）
}
```

```rust
OpCode::TryEnter => {
    let handler_offset = i16::from(self.read_u16()?) as usize;  // 参照 task 24 JUMP 偏移读取
    let frame = self.call_stack.last().ok_or("no frame".to_string())?;
    self.exception_handlers.push(ExceptionHandler {
        catch_address: frame.ip + handler_offset,
        has_finally: self.current_unit_has_finally(),  // 编译期已知，可由操作数承载（见下）
        finally_address: Some(frame.ip + finally_offset), // finally 偏移亦由操作数传入
        frame_stack_base: frame.stack_base,
        scope_stack_base: self.stack.len(),
    });
}
```

> **操作数编码注**：TRY_ENTER 标准操作数为 `handler_offset(2)`。本 task 需额外知道 finally 地址与是否有 finally。为不破坏标准编码，建议让编译器对「有 finally」与「无 finally」分别 emit，finally_address 通过 `TRY_ENTER` 后紧跟一条伪记录或由编译器在 dispatcher 内嵌 finally 偏移；最简方案是**扩展 TRY_ENTER 操作数为 `handler_offset(2) finally_offset(2)`**（spec 回写 §11）。实现时统一为 4 字节操作数，无 finally 时 finally_offset = 0xFFFF（哨兵）。

### 4. TRY_EXIT 指令

```rust
OpCode::TryExit => {
    // try body 正常完成（或 early-exit 出口）注销本 try 的 handler。
    self.exception_handlers.pop();
}
```

> TRY_EXIT 必须在 try body 的**每一个出口** emit：正常末尾、`return`、`break`、`continue`。否则 `exception_handlers` 残留陈旧 handler，后续异常会误命中（见审核 RISK）。编译器在 §2 步骤 2 负责在 body 内 early-exit 前插入 TRY_EXIT。

### 5. CATCH 指令 + 异常类型匹配

```rust
OpCode::Catch => {
    let name_idx = self.read_u16()?;
    let target_name = read_string_constant(self.call_stack.last()?.closure, name_idx)?;
    let exception = self.peek(0)?;   // 不弹出，供后续绑定 / 重抛
    let matches = exception_matches(&exception, &target_name);
    self.push(Object::Bool(matches))?;
}
```

异常类型匹配（含父类链，查**静态 MRO 表**，不依赖 Class 对象）：

```rust
/// 内置异常层级（父类链）。父类一律为 Error。Phase 5 升级为正式 Class 后此表废弃。
const EXCEPTION_PARENTS: &[(&str, &str)] = &[
    ("ValueError", "Error"), ("TypeError", "Error"), ("IndexError", "Error"),
    ("KeyError", "Error"), ("AttributeError", "Error"), ("NameError", "Error"),
    ("RuntimeError", "Error"), ("IOError", "Error"), ("ZeroDivisionError", "Error"),
    ("OverflowError", "Error"), ("StopIteration", "Error"), ("GeneratorExit", "Error"),
    // 用户自定义异常类（< Error）在 Phase 5 之后才存在；本阶段无。
];

/// exception 在 mro 上是否为 target_name 或其子孙。
fn exception_matches(exception: &Object, target_name: &str) -> bool {
    let class_name = match exception {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::EXCEPTION as u8 => {
            unsafe { read_exception(*ptr) }.class_name.clone()
        }
        _ => return false,   // 非 EXCEPTION 不能被 except 捕获
    };
    // GeneratorExit 不可被用户 except 捕获（05-control-flow.md:238）——
    // 仅 CLOSE_GENERATOR（task 39）内部流程可处理。任何用户 CATCH 都不匹配。
    if class_name == "GeneratorExit" {
        return false;
    }
    let mut cur = class_name.as_str();
    loop {
        if cur == target_name { return true; }
        match EXCEPTION_PARENTS.iter().find(|(c, _)| *c == cur).map(|(_, p)| *p) {
            Some(parent) => cur = parent,
            None => return false,
        }
    }
}
```

### 6. THROW / RETHROW / FINALLY_END / CLEAR_CURRENT_EXC 指令

`throw <expr>`：编译端先求值 expr 并压栈，再 emit THROW。`throw "string"`：编译端检测到 expr 为 string 字面量/表达式时，THROW 前压栈的仍是字符串；THROW handler 把 string 自动包装为 `RuntimeError(message)`（05-control-flow.md:278）。裸 `throw`：编译端 emit RETHROW（无 expr 压栈）。

```rust
// throw <expr>（含 string 自动包装）
OpCode::Throw => {
    let val = self.pop()?;
    let err = match &val {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::EXCEPTION as u8 => val,
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::STRING as u8 => {
            // throw "string" → RuntimeError(message)
            let msg = unsafe { read_str(*ptr) }.to_string();
            alloc_exception("RuntimeError", alloc_string(&msg), alloc_string(""), Object::Nil)
        }
        _ => return Err("TypeError: exceptions must derive from Error or be a string".into()),
    };
    self.throw(err)?;
}

// 裸 throw（重抛）—— 05-control-flow.md:279
OpCode::Rethrow => {
    let frame = self.call_stack.last().ok_or("no frame".to_string())?;
    match frame.current_exc.clone() {
        Some(err) => self.throw(err)?,
        None => {
            // except 块外裸 throw → RuntimeError("nothing to rethrow")
            let e = alloc_exception(
                "RuntimeError", alloc_string("nothing to rethrow"),
                alloc_string(""), Object::Nil,
            );
            self.throw(e)?;
        }
    }
}

// finally 块末尾：current_exc 非空则重抛（finally-on-propagation），否则继续
OpCode::FinallyEnd => {
    let frame = self.call_stack.last_mut().ok_or("no frame".to_string())?;
    let pending = frame.current_exc.take();
    if let Some(err) = pending {
        // finally 自身若抛异常，会先经 THROW 进入 throw()，不会到这里；这里只处理
        // 「finally 正常结束 + 进入时带未处理异常」的重抛（05-control-flow.md:71）。
        drop(frame);   // 释放借用
        self.throw(err)?;
    }
}

// except 命中分支：异常已处理，清除 current_exc（使 FINALLY_END 不重抛）
OpCode::ClearCurrentExc => {
    self.call_stack.last_mut().ok_or("no frame".to_string())?.current_exc = None;
}
```

> **finally 内抛异常**（05-control-flow.md:207「finally 块中的异常会覆盖之前的异常」）：finally 块里 `throw` 走正常 THROW → `throw()`。此时 `frame.current_exc` 仍持有进入 finally 时的原异常；`throw()` 在跑该帧 defer 时按规则 1/4 把 current_exc 挂为新异常的 `__cause__`（见 §7）。即 finally 新异常覆盖原异常，原异常作为 cause 保留。

### 7. 异常传播 `fn throw(&mut self, mut err: Object) -> Result<(), String>`

返回类型为 `Result<(), String>`（**与 VM 现有错误路径一致**，非 `MspError`）。核心职责：(a) 该帧首次进入时跑 defer，按规则 1/3/4 构建 `__cause__` 链；(b) 自顶向下扫描 `exception_handlers`，找到属于当前帧的 handler 则跳到 `catch_address`（设 `current_exc`），无 handler 则 pop frame 递归；(c) 顶层未捕获返回 `Err(String)`。

```rust
fn throw(&mut self, mut err: Object) -> Result<(), String> {
    loop {
        let frame_stack_base = self.call_stack.last().ok_or("no frame".to_string())?.stack_base;

        // (a) 跑当前帧的 defer（规则 1/3/4）。仅在该帧首次进入时跑：用 per-frame
        //     `defer_flushing` 标志避免递归 throw 重复跑 defer。
        let already_flushed = self.call_stack.last().unwrap().defer_flushing;
        if !already_flushed {
            // exec_defers_for_unwind 跑完该帧全部 defer（LIFO）；若某个 defer 抛异常，
            // 把原 err 挂为新异常的 __cause__（规则 1/4），err 更新为新异常，继续传播。
            // 返回 Some(new_err) 表示 defer 抛了新异常。
            if let Some(new_err) = self.exec_defers_for_unwind(frame_stack_base)? {
                self.set_cause(&new_err, err);
                err = new_err;
            }
            self.call_stack.last_mut().unwrap().defer_flushing = true;
        }

        // (b) 扫描 exception_handlers，弹出不属于当前帧或已失效的 handler。
        let handler = loop {
            match self.exception_handlers.last() {
                None => break None,
                Some(h) if h.frame_stack_base < frame_stack_base => {
                    // 属于更深的、已弹出的帧 —— 不可能（pop frame 时会清干净）；防御性 pop。
                    self.exception_handlers.pop();
                }
                Some(_) => break self.exception_handlers.pop(),
            }
        };

        if let Some(h) = handler {
            // 恢复栈到 try 入口长度（丢弃 try body 内临时值）。
            self.stack.truncate(h.scope_stack_base);
            // 设 current_exc = err（供 except 绑定 / 裸 throw 重抛 / FINALLY_END 重抛判定）。
            self.call_stack.last_mut().unwrap().current_exc = Some(err.clone());
            // 把 err 压栈供 except 分派器 CATCH 比对；ip 跳到分派器。
            self.call_stack.last_mut().unwrap().ip = h.catch_address;
            self.push(err)?;
            return Ok(());
        }

        // (c) 当前帧无 handler：跑完 defer（已跑），关闭 upvalue，pop frame。
        self.close_upvalues_from(frame_stack_base);
        if self.call_stack.len() > 1 {
            self.stack.truncate(frame_stack_base);
            self.call_stack.pop();
            // 继续外层帧的 throw（err 携带，可能已带 __cause__）。
            continue;
        }
        // 顶层未捕获：返回 String（VM run() 把它作为程序错误输出）。
        return Err(format_uncaught_error(&self, &err));
    }
}
```

**defer 异常链 helper**（task 36 仅实现 EXEC_DEFER opcode handler；本 task 抽出可复用的「刷新当前帧 defer」函数，供 RETURN 前的 EXEC_DEFER 与此处 unwind 路径共用）：

```rust
/// LIFO 跑完 frame_stack_base 对应帧的 defer。返回 Some(exc) 表示有 defer 抛了异常
/// （规则 1：后续 defer 仍跑；最后一个异常返回，之前的逐级挂 __cause__）。
/// 复用 OpCode::ExecDefer 的 ip-rewind 蹦床逻辑（task 36），但这里是同步遍历：
/// 每条 defer 用 run_defer_entry 执行（builtin 同步、closure 经 call_value 推帧）；
/// closure callee 的异步完成由主循环驱动——故 unwind 期间须确保 defer closure 已 RETURN
/// 后再处理下一条（与 EXEC_DEFER 一致，依赖 per-frame defer_flushing + ip 回跳）。
fn exec_defers_for_unwind(&mut self, frame_stack_base: usize) -> Result<Option<Object>, String> {
    let base = self.call_stack.last().unwrap().defer_stack_base;
    let mut thrown: Option<Object> = None;
    while self.defer_stack.len() > base {
        let entry = self.defer_stack.pop().unwrap();
        match self.run_defer_entry(entry.call_tuple) {
            Ok(()) => {}
            // run_defer_entry 在 closure callee 抛异常时返回 Err —— 但本 VM 错误是 String；
            // 异常对象经 THROW→throw 走专门路径，不应混入 String Err。故 closure defer 抛异常
            // 的传递需 throw() 主动捕获（详见实现注）。此处简化：defer 抛 mslang 异常时，
            // 由 throw() 在递归中把新异常挂 __cause__；此处返回最新异常。
            Err(_) => { /* 见下方实现注：异常对象经 throw() 路径流转，本函数仅驱动 */ }
        }
    }
    Ok(thrown)
}
```

> **closure-defer 抛异常的实现注**：本 VM 错误流有两种——`Result<_, String>`（VM 机制错误）与 mslang 异常对象（经 `throw()` 流转，**不**通过 `Err`）。`exec_defers_for_unwind` 在跑 closure defer 时，若 defer 体 `throw`，会再次进入 `throw(err)`；此时 per-frame `defer_flushing=true`（已设），故不会重复跑 defer，而是直接找 handler / pop frame。为保证「规则 1（后续 defer 仍跑）」，`exec_defers_for_unwind` 须改用**显式栈驱动**（不依赖递归 throw 的副作用）：手动遍历 defer 栈，每条 defer 的执行若产生异常对象则记入 `thrown` 链（挂 __cause__），不中断循环。实现时建议 `run_defer_entry` 增加一个「捕获被抛异常对象」的回调路径，而非走 `Err`。**这是本 task 最复杂的实现点**，须在编码时重点设计并与 §6 throw() 的 cause 链对齐。

**`set_cause` / `format_uncaught_error`**（用 MsException 字段，不依赖 Instance）：

```rust
fn set_cause(&mut self, exc: &Object, cause: Object) {
    if let Object::Ref(ptr) = exc {
        if unsafe { (**ptr).type_tag } == TypeTag::EXCEPTION as u8 {
            unsafe { read_exception_mut(*ptr) }.cause = cause;
        }
        // 非 EXCEPTION（理论上不会到这里，throw 只产生 EXCEPTION）静默忽略
    }
}

fn format_uncaught_error(&self, err: &Object) -> String {
    match err {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::EXCEPTION as u8 => {
            let e = unsafe { read_exception(*ptr) };
            let msg = string_value(&e.message).unwrap_or_default();
            // 堆栈跟踪格式参照 11-bytecode-vm.md:395-400
            format!("{}: {}", e.class_name, msg)
        }
        _ => "Error: <non-exception thrown>".to_string(),
    }
}
```

### 8. GC 根集与类型描述表

- **根集**：`exception_handlers`（持 `current_exc` 不在栈上时也须根扫描——`current_exc` 存于 CallFrame，是潜在的 GC 根）与 `CallFrame.current_exc` 须加入 `src/vm/gc.rs` 的根转发段（与 `defer_stack` 同处，gc.rs:723 附近）：遍历 `exception_handlers`、各帧的 `current_exc`，逐个 `forward_slot`。
- **trace 函数**：为 `TypeTag::EXCEPTION`（18）与 `TypeTag::EXCEPTION_CLASS`（19）在类型描述表注册 `forward_fields`：EXCEPTION 转发 `message`/`traceback`/`cause` 三个 Object 槽；EXCEPTION_CLASS 无 Object 槽（仅 `name: String`，非 GC 对象）。无 finalizer。

### 9. traceback 捕获与格式

参照 `11-bytecode-vm.md:393-400` 的堆栈跟踪格式。**捕获点**：在 `throw()` 顶层未捕获分支（`format_uncaught_error`）按 `call_stack` 自顶向下格式化 `at <fn> (file:line)`；行号经编译单元的 `lines` 表（`11-bytecode-vm.md:386-389`）由 ip 反查。**异常对象的 `traceback` 字段**：构造时（CALL-on-EXCEPTION_CLASS）暂为空串，在 `throw()` 跳入 except 分派器前填充当前调用栈快照（仅在该异常**将被传播**时填充，避免构造即捕获的开销）。本 task 至少实现「未捕获时打印多行 traceback」；异常对象 traceback 字段的运行时填充可作基础项。

### 10. Phase 5（OOP）升级路径

Phase 5 完成 Class/Instance（task 40-43）后：
- `MsException`/`MsExceptionClass` 升级为正式 `Instance`（`TypeTag::INSTANCE`）+ `Class`（`TypeTag::CLASS`）；`EXCEPTION`/`EXCEPTION_CLASS` TypeTag 与 `EXCEPTION_PARENTS` 静态表废弃。
- CALL-on-EXCEPTION_CLASS 分支替换为正式 CALL-on-class（`__init__`）；GET_ATTR 的 EXCEPTION 特例分支删除（统一走 Instance 属性 task 41）。
- 用户自定义异常类（`class MyError < ValueError`，05-control-flow.md:246）自此可用，`exception_matches` 改走 Class.parent 链。
- 本 task 的 `current_exc`/`RETHROW`/`FINALLY_END`/`CLEAR_CURRENT_EXC` 机制保留（与 OOP 正交）。

### 11. 设计规格回写（spec writeback）

本 task 对设计文档的扩展（参照 task 28 的 TypeTag 回写惯例，需同步更新对应文档）：

- **`14-gc.md` TypeTag 表**：新增 `EXCEPTION = 18`、`EXCEPTION_CLASS = 19`（在 `UPVALUE = 17` 与 `LARGE_OBJECT = 0xFF` 之间）。
- **`11-bytecode-vm.md` 异常 opcode 表**：新增 `RETHROW`（—）、`FINALLY_END`（—）、`CLEAR_CURRENT_EXC`（—）三行；`CATCH` 操作数说明改为 `name_idx(2)`（字符串类名常量）；`TRY_ENTER` 操作数扩展为 `handler_offset(2) finally_offset(2)`（无 finally 时 finally_offset=0xFFFF）。
- **`05-control-flow.md`**：无需改动（语义未变，仅实现策略）。

## 验证标准

1. try/except 正确捕获指定类型异常
2. 无类型 except 捕获所有异常
3. `as` 绑定正确将异常对象赋给变量
4. finally 块总是执行：正常路径、except 命中路径、**异常未匹配传播路径**（finally-on-propagation）
5. 异常沿调用栈传播（跨帧）
6. 子类异常匹配父类型 except（`except Error` 捕获 `ValueError`）
7. `throw <expr>` 正确创建和抛出异常；`throw "string"` 自动包装为 `RuntimeError`
8. 裸 `throw` 在 except 块内重抛当前异常；在 except 块外抛 `RuntimeError("nothing to rethrow")`
9. 未捕获异常终止程序并打印多行堆栈跟踪
10. 异常对象属性 `message`/`type`/`traceback`/`__cause__` 经 GET_ATTR 可读
11. `__cause__` 链：defer 抛异常时原异常挂为 `__cause__`（规则 1/4）；正常时为 nil
12. finally 块内抛异常覆盖原异常（原异常挂 `__cause__`）
13. GeneratorExit 不可被用户 except 捕获
14. finally/except 命中后 frame.current_exc 清空（FINALLY_END 不误重抛）

> 异常类型注册：12 个内置类型（Error + 11 子类）作为 EXCEPTION_CLASS 全局可用；`OverflowError`/`ZeroDivisionError` 等由算术运算触发为正式异常对象的接线（当前算术返回 String 错误）属跨 task 集成，本 task 仅完成类型注册与 throw/catch 机制。

## 测试用例

```ms
// test_try_except.ms — try/except/finally 异常处理

// 基本捕获
try {
    throw ValueError("test error")
} except ValueError as e {
    print("caught: " + e.message)
}

// finally 执行（用 throw 直接抛 ZeroDivisionError，避免依赖算术接线）
try {
    throw ZeroDivisionError("divide by zero")
} except ZeroDivisionError as e {
    print("division error")
} finally {
    print("cleanup")
}

// 捕获所有
try {
    throw TypeError("type!")
} except {
    print("caught all")
}

// 多 except 子句
try {
    throw KeyError("missing")
} except ValueError as e {
    print("value error")
} except KeyError as e {
    print("key error: " + e.message)
}

// finally 在正常路径也执行
try {
    x = 42
} finally {
    print("always runs")
}

// 异常传播
fn inner() {
    throw RuntimeError("from inner")
}

fn outer() {
    inner()
}

try {
    outer()
} except RuntimeError as e {
    print("propagated: " + e.message)
}

// try/except/finally 组合
try {
    throw ValueError("combo")
} except ValueError as e {
    print("handled: " + e.message)
} finally {
    print("final cleanup")
}

// finally-on-propagation：异常未被本层 except 匹配，finally 仍执行后向上传播
fn boom() {
    try {
        throw ValueError("boom")
    } finally {
        print("inner finally")
    }
}
try {
    boom()
} except ValueError as e {
    print("outer caught: " + e.message)
}

// throw "string" 自动包装为 RuntimeError
try {
    throw "oops"
} except RuntimeError as e {
    print("wrapped: " + e.message)
}

// 裸 throw 在 except 内重抛
try {
    try {
        throw ValueError("first")
    } except ValueError as e {
        throw
    }
} except ValueError as e {
    print("rethrown: " + e.message)
}

// __cause__ 链：defer 抛异常时原异常挂为 __cause__（规则 1/4）
fn defer_throw() {
    throw KeyError("defer err")
}
fn with_defer() {
    defer defer_throw()
    throw ValueError("orig")
}
try {
    with_defer()
} except KeyError as e {
    cause = e.__cause__
    if cause != nil {
        print("cause type: " + cause.type)
    }
    print("caught: " + e.message)
}
```

预期输出：

```
caught: test error
division error
cleanup
caught all
key error: missing
always runs
propagated: from inner
handled: combo
final cleanup
inner finally
outer caught: boom
wrapped: oops
rethrown: first
cause type: ValueError
caught: defer err
```

> **子类匹配**（验证标准 6）：`throw ValueError(...)` 被 `except Error as e` 捕获——单独用例断言即可。
> **裸 throw 在 except 块外**（验证标准 8）：抛 `RuntimeError("nothing to rethrow")`，单独用例（预期程序错误终止）。
> **GeneratorExit 不可捕获**（验证标准 13）：`throw GeneratorExit("ge")` 不被 `except` / `except Error` 捕获，单独用例验证程序以 `GeneratorExit: ge` 终止（不在上述序列内，避免中断主序列）。
> **`10 / 0` 触发 ZeroDivisionError**：当前算术返回 String 错误，升级为正式异常对象的接线属跨 task 集成（见验证标准末注），故本 task 测试用 `throw ZeroDivisionError(...)` 直接构造。
