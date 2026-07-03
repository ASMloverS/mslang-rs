# with 语句（上下文管理器）

## 所属阶段
Phase 4.6 - 控制流 + 高级语法

## 前置任务
37-try-except-finally

## 目标
实现 with 语句，支持上下文管理器协议（`__enter__` / `__exit__`），包括异常传递和异常抑制。

## 设计规格

参照 [05-control-flow](../05-control-flow.md) § with 语句：

### 语法

```
with_stmt = "with" expression ("as" IDENTIFIER)? block
```

### 上下文管理器协议

```ms
fn __enter__(self) -> value
fn __exit__(self, err_type, err_msg, traceback) -> bool
```

### with 执行流程

1. 求值 `with` 后的表达式，得到上下文管理器对象
2. 调用 `__enter__()`，返回值绑定到 `as` 变量（如有）
3. 执行块体
4. 离开块时（正常或异常），调用 `__exit__(err_type, err_msg, traceback)`
5. 若块内有异常，异常信息传递给 `__exit__`
6. 若 `__exit__` 返回 `true`，异常被抑制；返回 `false` 或 `nil` 则异常继续传播

### 嵌套 with

```ms
with ctx1 as a {
    with ctx2 as b {
        // ...
    }
}
```

## 前置修复（task 37 遗留）

**R2 — `CallFrame.current_exc` GC 根集遗漏**。`src/vm/gc.rs:474` 标注 `TODO task 37: EXCEPTION/EXCEPTION_CLASS root forwarding（exception_handlers/current_exc）` 未实现：`minor_gc` / `major_gc` / `maybe_gc` 当前仅扫描 stack + globals + defer_stack，未扫 `call_stack[*].current_exc`。task 38 的 with 语句会在 `current_exc` 持有异常对象期间调用用户代码（`__enter__`/`__exit__`），CALL 安全点触发 GC 时异常对象可能被误回收。task 38 实装前须先修复：

- `src/vm/gc.rs`：为 `minor_gc` / `major_gc` / `maybe_gc` 增加 `call_stack: &mut [&mut [CallFrame]]`（或等价形式）参数；遍历每帧把 `current_exc` 作根转发（minor）/ 标记（major）。
- `src/vm/mod.rs:871` 调用点同步更新。
- 注：`exception_handlers`（`src/vm/mod.rs:94`）只持元数据（catch_address 等整数），**无 Object 引用**，不需扫描。
- 该修复属 task 37 范畴的回填；task 38 验证标准不重复列。

## 实现细节

### 1. 临时支持 GET_ATTR on Dict（Phase 5 由 Instance 接管）

task 38 排在 Phase 4.6，但 `__enter__`/`__exit__` 在 `06-oop.md:247-252` 被归为魔术方法，正式实装依赖 Phase 5 Instance + GET_ATTR on Instance（task 41/43）。为使本 task 可端到端验证，**在 task 38 范围内为 `TypeTag::DICT` 增加一条 GET_ATTR 分支**作为临时机制；Phase 5 task 41/43 完成后由 Instance 接管并删除本分支。

`src/vm/mod.rs` GET_ATTR handler（`mod.rs:1733`）增加 DICT 分支（与 EXCEPTION 分支并列）：

```rust
OpCode::GetAttr => {
    let name_idx = self.read_u16()? as usize;
    let attr = self.read_string_constant(name_idx)?;
    let obj = self.pop()?;
    match &obj {
        // [task 38 临时] Dict 属性访问：等价于 dict[attr]，键不存在返回 nil。
        // Phase 5 task 41/43 由 Instance 接管，本分支删除。
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::DICT as u8 => {
            let map = unsafe { read_dict(*ptr) };
            let key = alloc_string(&attr);
            let val = map_get(map, &key).cloned().unwrap_or(Object::Nil);
            self.push(val)?;
        }
        // [task 37] EXCEPTION 属性访问（message/type/traceback/__cause__）
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::EXCEPTION as u8 => {
            // …（task 37 既有实现，不变）
        }
        _ => return Err("GET_ATTR for non-exception/non-dict types: not yet implemented (task 41/43)".into()),
    }
}
```

> **设计注**：mslang 的 `.` 是**属性访问**运算符（`01-lexical.md:207`、`03-syntax.md:475`），与下标 `dict[key]`（`02-types.md:189`）语义不同。本临时分支把「dict.attr」等价为「dict["attr"]」仅为 with 语句铺路；正式 Instance（Phase 5）将恢复「`.` 仅访问对象属性、dict[key] 访问字典键」的语义二分。

### 2. 编译 with 语句（修订版，对齐 task 37 实装）

`src/compiler/statement.rs`。关键修正：
- (a) **CALL 约定**：callee 在底、args 在顶（`src/compiler/expression.rs:379-389`），`__enter__` 调用须 CALL 1（传 self）、`__exit__` 须 CALL 4（self + 三异常参数）。
- (b) **TRY_ENTER 操作数**：4 字节 `handler_offset(2) finally_offset(2)`，无 finally 用 `0xFFFF` 哨兵（`docs/mslang/11-bytecode-vm.md:169`）。
- (c) **handler 内不 emit TRY_EXIT**：task 37 的 `drive_unwind`（`src/vm/mod.rs:826-827`）命中时已自动 `exception_handlers.pop()`，handler 内再 emit TRY_EXIT 会空栈 pop。
- (d) **异常对象经 per-frame `current_exc` 管理**（`src/vm/frame.rs:18`），不再开 `_exc` 局部；本 task 新增 `LOAD_CURRENT_EXC` opcode 读取（见 §3）。
- (e) **with 块不创建新作用域**（`03-syntax.md:595`）：`as name` 在**外围函数作用域**注册（`declare_local`），与 Python 一致——name 在 with 块外仍可见。

完整字节码序列（用临时局部 `_tmp_ctx` 中转，避免依赖不存在的 SWAP/ROT 指令）：

```
编译 with expr as name { body }：

—— 入口：求值 expr，保存管理器 ——
1. 编译 expr                  → [ctx]
2. STORE_LOCAL _tmp_ctx       → []           ; declare_local 临时槽
3. LOAD_LOCAL _tmp_ctx        → [ctx]        ; callee 垫底
4. DUP                        → [ctx, ctx]   ; 顶 ctx 作 self arg
5. GET_ATTR "__enter__"       → [ctx, enter_fn]  ; callee=enter_fn, arg=ctx(下方)
6. CALL 1                     → [enter_result]   ; 调用 enter_fn(ctx)
7. if name: STORE_LOCAL name  ; declare_local 外围作用域
   else: POP                  → []

—— 注册异常处理器（handler=cleanup_exc，无 finally → 0xFFFF）——
8. TRY_ENTER handler_off cleanup_exc_off  ; 4 字节操作数
   ; cleanup_exc_off = (cleanup_exc 地址 − TRY_ENTER 后 ip)，无 finally 时
   ;   本 task 把整个 cleanup 当作「catch + pseudo-finally」，
   ;   finally_offset = 0xFFFF（task 37 drive_unwind 在 handler 处统一处理）。

—— 编译 body（与 task 37 同样处理 early-exit）——
9. self.try_depth += 1        ; 提示 return/break/continue 编译器插入 TRY_EXIT
   编译 body
   self.try_depth -= 1
10. TRY_EXIT                  ; body 正常完成，注销 handler
11. JUMP cleanup_normal       ; 跳到正常 cleanup

—— cleanup_exc：异常路径入口（throw 注入：栈顶为异常、current_exc 已设）——
12. cleanup_exc:
    ; 栈顶为异常对象（drive_unwind 在跳转前 push）。POP 之（current_exc 仍持有）。
    POP

—— cleanup_normal / cleanup_exc 汇合于 cleanup ——
13. cleanup:
    LOAD_LOCAL _tmp_ctx       → [ctx]
14. DUP                       → [ctx, ctx]
15. GET_ATTR "__exit__"       → [ctx, exit_fn]   ; callee=exit_fn
16. LOAD_LOCAL _tmp_ctx       → [ctx, exit_fn, ctx]   ; self arg

—— 三异常参数入栈（运行期据 current_exc 是否 Some 区分）——
17. LOAD_EXC_TYPE             → [ctx, exit_fn, ctx, err_type_or_nil]
18. LOAD_EXC_MSG              → [ctx, exit_fn, ctx, err_type, err_msg_or_nil]
19. LOAD_EXC_TB               → [ctx, exit_fn, ctx, err_type, err_msg, tb_or_nil]
    ; 上述三条均从 current_exc 派生（无异常时压 nil）。实现上可用 1 条
    ; LOAD_CURRENT_EXC 压异常对象后 GET_ATTR 拆字段，或直接新增 3 条专用 opcode。
    ; 本 task 采用 LOAD_CURRENT_EXC + GET_ATTR 方案（见 §3）。

20. CALL 4                    → [exit_result]   ; 调用 exit_fn(ctx, err, msg, tb)

—— 检查 __exit__ 返回值，决定抑制或重抛 ——
21. ; 若 current_exc 为 None（正常路径）：直接 POP exit_result，JUMP end
    ; 若 current_exc 为 Some（异常路径）：
    ;   truthy(exit_result) → 抑制：CLEAR_CURRENT_EXC + POP exit_result + JUMP end
    ;   falsy(exit_result)  → 重抛：POP exit_result + LOAD_CURRENT_EXC + THROW
    ;
    ; 编译期不知是否异常，故统一生成下面的运行期判定字节码：
    LOAD_CURRENT_EXC          → [exit_result, exc_or_nil]
22. JUMP_IF_NULL suppress     ; exc 为 nil（正常路径）→ 跳 suppress
    ; 注：若不新增 JUMP_IF_NULL，可改为：DUP、LOAD_NIL、EQ、JUMP_IF_TRUE suppress
    POP                       ; 弹 exc（异常路径）
    ; truthy 判定（02-types.md 真值规则）
    JUMP_IF_FALSE rethrow     ; exit_result falsy → 重抛
    POP                       ; 弹 falsy exit_result
    CLEAR_CURRENT_EXC         ; 抑制：清 current_exc
    JUMP end
23. rethrow:
    POP                       ; 弹 exit_result
    LOAD_CURRENT_EXC          → [exc]
    THROW                     ; 重抛
24. suppress:
    POP                       ; 弹 nil（步骤 22 压的 exc）
    POP                       ; 弹 exit_result
25. end:
```

> **opcode 命名约定**：所有 opcode 在源码中用 **CamelCase**（`OpCode::TryEnter`、`OpCode::Call`、`OpCode::GetAttr`、`OpCode::StoreLocal` 等），与 `src/compiler/opcode.rs` 一致；上方伪代码用大写仅为可读性。
> **JUMP_IF_NULL** 与 **LOAD_NIL** 为本 task 新增微操作（spec 回写见 §6）。若不愿新增，可用 `LOAD_CURRENT_EXC + DUP + CONSTANT(nil) + EQ + JUMP_IF_TRUE suppress` 替代（5 条指令换 2 条）。

### 3. 新增 opcode：LOAD_CURRENT_EXC

为支持「在 with cleanup 块中读取 task 37 的 per-frame `current_exc`」（M4/M7），本 task 新增 1 条无操作数字节码：

| OpCode | 操作数 | 说明 |
|---|---|---|
| `LOAD_CURRENT_EXC` | — | 压当前帧 `current_exc`（无异常时压 nil） |

```rust
OpCode::LoadCurrentExc => {
    let exc = self.call_stack.last()
        .ok_or("no frame".to_string())?
        .current_exc.clone()
        .unwrap_or(Object::Nil);
    self.push(exc)?;
}
```

> 不在 `vm.exception_handlers`（仅持元数据）中存异常对象；task 37 的 `current_exc` 是异常对象的唯一非栈根。

### 4. with body 内含 defer 的交互

按 task 36/37 规则：异常传播时，**defer 先于 except/finally 跑**（规则 1/3/4）。with 把 cleanup 当作 catch 块（task 37 drive_unwind 命中 handler 时 pop handler、跳 catch_address、设 current_exc、push 异常）——但 **with 的 cleanup 不是 except**（它不消费异常，只是调 `__exit__`）。

**执行顺序**（异常路径）：
1. body 内 `throw E` → `throw()` 进入 `drive_unwind`
2. drive_unwind 先跑当前帧 defer（LIFO）；defer 抛新异常则挂 `E.__cause__`（规则 1/4）
3. defer 跑完，drive_unwind 找到 with 的 handler，跳 `cleanup_exc`，设 `current_exc=E`，栈顶压 E
4. cleanup_exc 执行 §2 步骤 12-25：POP E（栈顶，因 current_exc 已存）→ 调 `__exit__` → 检查返回值

**关键不变量**：defer 在 drive_unwind 内（步骤 2）跑完，**先于** cleanup_exc（步骤 4）——即「先 defer、后 `__exit__`」。这与 `05-control-flow.md` defer 规则一致。

### 5. 异常信息传递（与 task 37 异常属性对齐）

当块内发生异常时，三参数从 task 37 的 `MsException` 字段派生（`src/vm/object.rs:680-714`）：

| 参数 | 正常时 | 异常时 | 来源 |
|---|---|---|---|
| `err_type` | `nil` | 异常类名字符串（如 `"ValueError"`） | `MsException.class_name`（GET_ATTR `"type"` 经 task 37 EXC 分支返回 `alloc_string(class_name)`） |
| `err_msg` | `nil` | 异常的 message 字段 | `MsException.message`（GET_ATTR `"message"`） |
| `traceback` | `nil` | 堆栈跟踪字符串 | `MsException.traceback`（GET_ATTR `"traceback"`） |

**实现**：cleanup 块压三参数时用 `LOAD_CURRENT_EXC` 把 `current_exc`（或 nil）压栈，再用 GET_ATTR 拆字段。若 `current_exc` 为 nil，GET_ATTR 失败——故编译期分两条路径，或新增 `LOAD_EXC_TYPE/MSG/TB` 三条专用 opcode（无异常时压 nil）。本 task 采用**条件跳转分支**方案（§2 步骤 17-19 的等价展开）：

```
; 等价于 §2 步骤 17-19 的展开（不引入新 opcode）
LOAD_CURRENT_EXC                  → [exc_or_nil]
DUP
JUMP_IF_NULL push_three_nils      ; nil 路径
; 非 nil 路径：拆字段（exc 在栈顶下两处）
GET_ATTR "type"                   → [exc_or_nil, type_str]
SWAP_NIL_HACK...                  ; 需要保留 exc 给后续两次 GET_ATTR
```

> **实现注**：mslang 无 SWAP/ROT 指令。最干净的做法是**新增 3 条专用 opcode** `LOAD_EXC_TYPE` / `LOAD_EXC_MSG` / `LOAD_EXC_TB`（无操作数；从 `current_exc` 直接派生字段、无异常时压 nil）。spec 回写 §6。本 task 推荐此方案，避免复杂的栈操作。

### 6. `__exit__` 返回值的真值判定

按 `02-types.md` 真值规则：`nil` / `false` / `0` / `""` / 空集合视为假（异常继续传播），其他视为真（异常被抑制）。**实现复用现有 truthy 函数**（task 24/25 已有），不引入新逻辑。

### 7. `__exit__` 自身抛异常的行为

参照 task 37 §6（finally 块内 throw 把原异常挂为新异常的 `__cause__`），with 的 `__exit__` 内 throw 同样：
- 新异常覆盖原异常（current_exc 被新异常替换）；
- 原异常挂为新异常的 `__cause__`（通过 `throw()` 内的 defer-style cause 链构建，task 37 已实现）；
- 若 `__exit__` 在**正常路径**（无原异常）抛异常，则按普通 throw 传播，无 `__cause__`。

### 8. 缺失 `__enter__`/`__exit__` 的错误处理

若 ctx 对象经 GET_ATTR 取不到 `__enter__`/`__exit__`：
- **DICT 临时分支**（§1）：返回 nil（dict 键不存在 → nil）。
- **CALL nil**：CALL 调用 nil 触发现有「not callable」错误（task 27 CALL handler 已检查 callee 类型）。

为提供更友好的提示，编译期不做特殊处理；运行期在 CALL nil 时返回 `TypeError: 'nil' is not callable`（或类似）。本 task 不引入「not a context manager」专用错误类型。

## 设计规格回写（spec writeback）

参照 task 37 §11 回写惯例，本 task 对设计文档的扩展：

- **`docs/mslang/11-bytecode-vm.md`**：opcode 表新增 `LOAD_CURRENT_EXC`（—）、`LOAD_EXC_TYPE`（—）、`LOAD_EXC_MSG`（—）、`LOAD_EXC_TB`（—）四行（如采用专用 opcode 方案）；或仅 `LOAD_CURRENT_EXC`（如采用 GET_ATTR 拆字段方案）。若新增 `JUMP_IF_NULL`/`LOAD_NIL` 微操作亦相应补表。
- **`docs/mslang/05-control-flow.md`**：无需改动（语义未变，仅实现策略）。
- **`docs/mslang/06-oop.md`**：在上下文管理器一节加注「Phase 4.6 期间用 dict 临时模拟（task 38），Phase 5 task 41/43 由 Instance 接管」。

## 验证标准

1. with 正确调用 `__enter__` 和 `__exit__`（CALL 1 / CALL 4，含 self）
2. `as` 变量绑定 `__enter__` 返回值（外围函数作用域，with 块外仍可见）
3. 正常退出时 `__exit__` 三异常参数全为 nil
4. 异常退出时 `__exit__` 接收 `err_type`（类名字符串）、`err_msg`、`traceback`
5. `__exit__` 返回真值（true / 非零 / 非空）时异常被抑制
6. `__exit__` 返回假值（false / nil / 0 / ""）时异常继续传播
7. 嵌套 with 正确工作（LIFO 顺序：内层 `__exit__` 先于外层）
8. **内层抛异常 + 内层 `__exit__` 不抑制 → 外层 `__exit__` 被调用且收到异常信息**
9. **内层 `__exit__` 抑制异常 → 外层 `__exit__` 被调用且收到 nil（异常未传播到外层）**
10. **`__exit__` 自身抛异常：原异常挂为新异常的 `__cause__`（与 task 37 §6 一致）**
11. **with body 内 defer：异常路径下 defer 先于 `__exit__` 跑（与 `05-control-flow.md` defer 规则一致）**
12. **`__enter__` 抛异常：`__exit__` 不被调用（与 Python 一致）**
13. **`try_depth` 正确传递**：with body 内 `return`/`break`/`continue` 插入 TRY_EXIT，避免 handler 栈泄漏

> **前置项 R2 修复验证**：task 37 遗留的 `current_exc` GC 根集遗漏须先修复（见「前置修复」节）。验证方式：构造在 with cleanup 期间触发 GC 的场景（如 body 内大量分配），断言 `__exit__` 收到的 err_type/msg 不被误回收。

## 测试用例

```ms
// test_with.ms — with 语句
// 注：本阶段用 dict 模拟上下文管理器（task 38 §1 临时实装 GET_ATTR on Dict）。
//     Phase 5 task 41/43 完成后改用正式 class + Instance。

// 基本上下文管理器
fn test_basic() {
    ctx = {
        "__enter__": fn(self) { print("enter"); return self },
        "__exit__": fn(self, err, msg, tb) { print("exit"); return false }
    }
    with ctx as c {
        print("body")
    }
}
test_basic()

// with 中发生异常（err_type 为类名字符串）
fn test_exception() {
    ctx = {
        "__enter__": fn(self) { print("enter"); return self },
        "__exit__": fn(self, err, msg, tb) {
            print("exit with: " + str(err))
            return false
        }
    }
    try {
        with ctx as c {
            print("before error")
            throw ValueError("oops")
            print("unreachable")
        }
    } except ValueError as e {
        print("caught: " + e.message)
    }
}
test_exception()

// __exit__ 抑制异常（返回 true）
fn test_suppress() {
    ctx = {
        "__enter__": fn(self) { return self },
        "__exit__": fn(self, err, msg, tb) {
            print("suppressing: " + str(err))
            return true
        }
    }
    with ctx as c {
        throw ValueError("suppressed")
    }
    print("after with")
}
test_suppress()

// 嵌套 with（正常路径，LIFO 顺序）
fn test_nested() {
    ctx1 = {
        "__enter__": fn(self) { print("enter1"); return self },
        "__exit__": fn(self, err, msg, tb) { print("exit1"); return false }
    }
    ctx2 = {
        "__enter__": fn(self) { print("enter2"); return self },
        "__exit__": fn(self, err, msg, tb) { print("exit2"); return false }
    }
    with ctx1 as a {
        with ctx2 as b {
            print("nested body")
        }
    }
}
test_nested()
```

预期输出（test_basic / test_exception / test_suppress / test_nested 顺序执行）：

```
enter
body
exit
enter
before error
exit with: ValueError
caught: oops
suppressing: ValueError
after with
enter1
enter2
nested body
exit2
exit1
```

> 下列场景须**单独用例**断言（避免与主序列夹杂）：
>
> - **验证标准 8**（异常跨 with 传播）：内层 with body 抛异常、内层 `__exit__` 返回 false → 外层 `__exit__` 收到 err_type 非空、msg 非空。断言输出顺序：`enter1, enter2, exit2(err_type="ValueError"), exit1(err_type="ValueError")`。
> - **验证标准 9**（内层抑制）：内层 `__exit__` 返回 true → 外层 `__exit__` 收到 err_type=nil。断言：`exit2(err_type="ValueError"), exit1(err_type=nil)`。
> - **验证标准 10**（`__exit__` 抛异常）：`__exit__` 内 `throw RuntimeError("from exit")`，原异常挂为 `__cause__`。断言外层捕获 RuntimeError 且 `e.__cause__.type == "ValueError"`。
> - **验证标准 11**（defer 与 with 交互）：with body 内 `defer cleanup()`，body 抛异常 → 输出顺序 `cleanup, exit(err_type=...)`（defer 先于 `__exit__`）。
> - **验证标准 12**（`__enter__` 抛异常）：`__enter__` 内 throw → `__exit__` 不被调用（断言无 "exit" 输出）。
> - **验证标准 13**（early-exit TRY_EXIT）：with body 内 `return`/`break`/`continue` → 不泄漏 handler（用后续 throw 验证不被误捕获）。
