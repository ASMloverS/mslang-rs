# select 语句（多 channel 复用）

## 所属阶段
Phase 7.3 - 并发（见 `12-implementation-plan.md:504-512`）

## 前置任务
54-channel, 53-async-await, 55-go-concurrency

> **依赖说明**：测试用例使用 `go fn() { ... }()` 启动发送方协程（task 55）；select 暂停/恢复机制沿用 task 53 可暂停帧设计；channel 等待列表与 try_send/try_recv 非阻塞语义由 task 54 提供。

## 目标

实现 `select` 语句，支持同时等待多个 channel 操作，任一分支就绪即执行。

## 设计规格

参照 [08-concurrency](../08-concurrency.md) § select：

### 语法

```
select_stmt = "select" "{" select_case+ ("default" block)? "}"
select_case = "case" channel_op block
channel_op  = IDENTIFIER "=" "<-" IDENTIFIER    // 接收
            | IDENTIFIER "<-" expression          // 发送
```

`select`、`case`、`default` 为保留字（见 [01-lexical](../01-lexical.md) § 保留字），不可用作变量名。

### 语义

- 多个 `case` 分支同时就绪时，**随机选择一个**执行（避免饥饿）
- `default` 分支在所有 channel 操作均未就绪时立即执行（非阻塞）
- 无 `default` 时，`select` 阻塞直到某个 case 就绪
- 空 `select {}`（无任何 case）永久阻塞当前协程

## 实现细节

> **同步设计文档更新**：本 task 同时修订两份标准文档：
> - `01-lexical.md`：将 `select`/`case`/`default` 从保留字升级为正式关键字（关键字总数 36 → 39），新增 `TokenKind::Select` / `Case` / `Default`。
> - `11-bytecode-vm.md`：在「其他」OpCode 表（line 180-191）追加 `SELECT` 指令。

### 0. 词法器升级（Phase 7 启用）

`01-lexical.md:28` 预告「后续 Phase 启用时升级为正式关键字」——本 task 完成该升级。

`src/lexer/token.rs`：

```rust
// TokenKind 新增变体
Select,
Case,
Default,
```

`src/lexer/mod.rs`：

- 从 `RESERVED_WORDS` 列表（见 `lexer/mod.rs:862` 测试 `test_all_reserved_words_error`）移除 `select`/`case`/`default`（保留 `export`/`match`）
- 关键字查找表新增三条映射：`"select" -> TokenKind::Select`、`"case" -> TokenKind::Case`、`"default" -> TokenKind::Default`
- 删除 lexer 中针对 `select`/`case`/`default` 的「保留字报错」分支
- 更新 `test_all_reserved_words_error` 仅保留 `export`/`match`

> **`01-lexical.md` 配套修改**：关键字章节（line 11-26）补入 `select case default` 三行；保留字表（line 328-332）删除这三条；总数描述（line 28）改为「共 39 个关键字」。

### 文件位置

- `src/ast/node.rs` — AST 节点
- `src/parser/advanced_statement.rs` — 解析
- `src/compiler/statement.rs` — 编译
- `src/vm/mod.rs` — VM 执行
- `src/vm/select.rs` — select 实现

### AST 节点

```rust
#[derive(Debug, Clone)]
pub struct SelectCase {
    pub operation: SelectOp,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone)]
pub enum SelectOp {
    Receive {
        channel: String,
        target: String,
    },
    Send {
        channel: String,
        value: Expr,
    },
}
```

在 `Stmt` 枚举中添加：

```rust
Select {
    cases: Vec<SelectCase>,
    default_block: Option<Vec<Stmt>>,
},
```

### 解析

词法器升级后（见上方「词法器升级」），`select`/`case`/`default` 已变为正规关键字 Token。在 `parse_statement()` 中，当遇到 `TokenKind::Select` 时调用 `parse_select()`：

```rust
fn parse_select(&mut self) -> Result<Stmt> {
    let start_span = self.peek().span;            // 记录起始位置供错误回退
    self.advance(); // consume TokenKind::Select
    self.expect(TokenKind::LeftBrace, "expected '{' after 'select'")?;
    self.skip_newlines();

    let mut cases = Vec::new();
    let mut default_block = None;

    while !self.check(&TokenKind::RightBrace) {
        let tok = self.peek();
        match &tok.kind {
            TokenKind::Case => {
                self.advance();
                let op = self.parse_select_op()?;
                let body = self.parse_block()?;
                cases.push(SelectCase { operation: op, body });
            }
            TokenKind::Default => {
                self.advance();
                default_block = Some(self.parse_block()?);
            }
            _ => {
                return Err(MspError::ParseError {
                    line: tok.span.start.line,
                    column: tok.span.start.column,
                    message: "expected 'case' or 'default' in select".into(),
                });
            }
        }
        self.skip_newlines();
    }

    self.expect(TokenKind::RightBrace, "expected '}' after select")?;
    Ok(Stmt::Select { cases, default_block })
}

fn parse_select_op(&mut self) -> Result<SelectOp> {
    let name_tok = self.peek();
    let name = self.expect_identifier("expected identifier in case")?;
    let op_start = name_tok.span;

    if self.match_token(&[TokenKind::Equal]) {
        self.expect(TokenKind::LeftArrow, "expected '<-' in receive case")?;
        let channel = self.expect_identifier("expected channel name")?;
        Ok(SelectOp::Receive { channel, target: name })
    } else if self.match_token(&[TokenKind::LeftArrow]) {
        let value = self.parse_expression()?;
        Ok(SelectOp::Send { channel: name, value })
    } else {
        Err(MspError::ParseError {
            line: op_start.start.line,
            column: op_start.start.column,
            message: "expected '=' or '<-' in select case".into(),
        })
    }
}
```

> **空 select{} 校验**：`Stmt::Select { cases: vec![], default_block: None }` 合法——由 VM 在运行时实现永久阻塞语义（见 VM 章节）。parser 不报错。
>
> **语法限制**：`channel_op` 产生式中 channel 仅允许 IDENTIFIER（`03-syntax.md` select 产生式），不支持 `obj.ch` 等成员表达式。若需动态 channel，先用 `c = obj.ch` 提取到局部变量再 select。

### 编译

`src/compiler/statement.rs` 中的 `compile_select`：

**求值时机规则**（与 Go 一致）：
- 进入 select 时按 case 源代码顺序**一次性求值**所有 channel 引用与 send value 表达式
- 求值结果暂存到临时局部变量槽
- select 重试期间不重新求值（即使带副作用的表达式如 `case ch <- pop()` 也只调用一次 `pop()`）

**case_count 编译期校验**：`case_count` 操作数为单字节无符号整数（0-255，与 `BUILD_LIST` 等一致）。若 cases.len() > 255，编译器报错："`select' case count exceeds 255"。

**字节码布局**：

```
// 1. 预求值阶段：按顺序求值所有 channel + send value，存入临时槽
LOAD_LOCAL ch1           // case 0 channel
STORE_LOCAL tmp_ch0
LOAD_LOCAL ch2           // case 1 channel
STORE_LOCAL tmp_ch1
<eval send_value for case 1>
STORE_LOCAL tmp_val1
...

// 2. SELECT 指令
SELECT
  case_count(1)          // N
  has_default(1)         // 0 或 1
  // case 描述表：每条 6 字节
  //   kind(1)      : 0=Receive, 1=Send
  //   channel_slot(1) : 临时槽索引（LOAD_LOCAL 用）
  //   value_slot(1)   : Send 的值槽（Receive 时为 0xFF 占位）
  //   target_slot(1)  : Receive 的目标槽（Send 时为 0xFF 占位）
  //   body_offset(2)  : 该 case body 的字节码起始偏移（相对 SELECT 起点，有符号 16 位）
  // 共 N 条
  default_offset(2)      : default body 偏移（has_default=0 时为 0x0000）

// 3. 各 case body 顺序排列
case0_body:
    <body 字节码>
    JUMP end_select       // 跳到 SELECT 之后
case1_body:
    ...
default_body:
    ...
end_select:
    <下一条语句>
```

> **`body_offset(2)` 限制**：单次跳转 ±32KB（`11-bytecode-vm.md:105`）。select body 跨度过大时编译器报错"`select' body offset exceeds 32KB"，提示拆分。

**编译器骨架**：

```rust
fn compile_select(&mut self, cases: &[SelectCase], default_block: &Option<Vec<Stmt>>) {
    // 1. 分配临时槽并预求值
    let mut case_descs = Vec::with_capacity(cases.len());
    for case in cases {
        match &case.operation {
            SelectOp::Receive { channel, target } => {
                let ch_slot = self.resolve_or_emit_load(channel);
                let target_slot = self.declare_local(&format!("__sel_tgt_{}", case_descs.len()));
                case_descs.push(CaseDesc {
                    kind: 0,
                    channel_slot: ch_slot,
                    value_slot: 0xFF,
                    target_slot,
                });
            }
            SelectOp::Send { channel, value } => {
                let ch_slot = self.resolve_or_emit_load(channel);
                self.compile_expression(value);
                let val_slot = self.alloc_temp();
                self.emit_store_local(val_slot);
                case_descs.push(CaseDesc {
                    kind: 1,
                    channel_slot: ch_slot,
                    value_slot: val_slot,
                    target_slot: 0xFF,
                });
            }
        }
    }

    // 2. case_count 校验
    if case_descs.len() > 255 {
        return Err(compile_error("`select' case count exceeds 255"));
    }

    // 3. 预留 SELECT 指令位置（先发占位，body 编完后再回填偏移）
    let select_pc = self.emit(OpCode::Select);
    self.emit_byte(case_descs.len() as u8);
    self.emit_byte(if default_block.is_some() { 1 } else { 0 });
    let table_pc = self.emit_placeholder(case_descs.len() * 6);
    self.emit_placeholder(2); // default_offset

    // 4. 依次编译每个 body，回填 body_offset
    for (i, case) in cases.iter().enumerate() {
        let body_pc = self.current_pc();
        let offset = body_pc as i64 - select_pc as i64;
        if !(-32768..=32767).contains(&offset) {
            return Err(compile_error("`select' body offset exceeds 32KB"));
        }
        self.patch_u16(table_pc + i * 6 + 3, offset as u16);
        self.compile_block(&case.body)?;
        self.emit_jump(OpCode::Jump, "end_select");
    }

    // 5. default body
    if let Some(db) = default_block {
        let d_pc = self.current_pc();
        let offset = d_pc as i64 - select_pc as i64;
        self.patch_u16(table_pc + case_descs.len() * 6, offset as u16);
        self.compile_block(db)?;
    }

    // 6. end_select 标签
    self.patch_label("end_select");
}
```

> **channel 名解析**：`resolve_or_emit_load(name)` 先查局部变量表（含 upvalue），命中则返回 slot；否则发 `LOAD_GLOBAL name_idx` 并分配临时槽。

### VM SELECT 指令

| OpCode | 操作数 | 说明 |
|---|---|---|
| `SELECT` | `case_count(1)`, `has_default(1)`, `case_table(6*N)`, `default_offset(2)` | 执行 select 多路复用 |

每条 case 描述符 6 字节：

```
kind(1) | channel_slot(1) | value_slot(1) | target_slot(1) | body_offset(2)
```

- `kind`：0 = Receive，1 = Send
- `channel_slot`：channel 引用所在的局部变量槽
- `value_slot`：Send case 的待发送值所在槽（Receive 时为 0xFF）
- `target_slot`：Receive case 写入的目标局部变量槽（Send 时为 0xFF）
- `body_offset`：该 case body 字节码起始位置相对 SELECT 起点的有符号 16 位偏移

```rust
OpCode::Select => {
    // 安全点检查（SELECT 是 GC 安全点，见 14-gc.md 安全点位置表）
    self.check_safepoint();

    let select_pc = self.ip - 1;  // SELECT opcode 起始
    let case_count = self.read_byte() as usize;
    let has_default = self.read_byte() != 0;

    // 读取 case 描述表
    let mut cases = Vec::with_capacity(case_count);
    for _ in 0..case_count {
        let kind = self.read_byte();
        let channel_slot = self.read_byte() as usize;
        let value_slot = self.read_byte();
        let target_slot = self.read_byte();
        let body_offset = self.read_i16();
        cases.push(CaseEntry { kind, channel_slot, value_slot, target_slot, body_offset });
    }
    let default_offset = self.read_i16();

    // 空 select{} 永久阻塞：case_count=0 且无 default
    // 直接暂停协程且不挂到任何 channel 等待列表（标记为 "empty_select"）
    // EventLoop 死锁检测时排除此类协程
    if case_count == 0 {
        if has_default {
            self.ip = (select_pc as i64 + default_offset as i64) as usize;
            return RunOutcome::Continue;
        }
        return RunOutcome::Yield(ChannelYield::EmptySelect);
    }

    // 非阻塞扫描：按 case 顺序尝试 try_send / try_recv
    // 任何 borrow_mut() guard 在下一次 case 评估前必须释放
    let mut ready_indices = Vec::new();
    for (i, entry) in cases.iter().enumerate() {
        let channel_obj = self.load_local(entry.channel_slot);
        let channel_ptr = expect_channel(&channel_obj)?;
        let channel = unsafe { &*channel_ptr };

        if entry.kind == 0 {
            // Receive case：可立即取值则就绪
            // 已关闭且缓冲区空的 channel 视为就绪（返回 nil）
            let ready = {
                let _buffer_guard = channel.buffer.try_borrow();
                let _senders_guard = channel.waiting_senders.try_borrow();
                channel.buffer.borrow().front().is_some()
                    || matches!(channel.state.borrow(), ChannelState::Closed)
                    || !channel.waiting_senders.borrow().is_empty()
            }; // guard 释放
            if ready { ready_indices.push(i); }
        } else {
            // Send case：可立即投递则就绪
            // 目标 channel 已关闭 → 永远不就绪（不抛错，静默跳过）
            if matches!(channel.state.borrow(), ChannelState::Closed) {
                continue;
            }
            let ready = {
                let _guard = channel.waiting_receivers.try_borrow();
                !channel.waiting_receivers.borrow().is_empty()
                    || channel.buffer.borrow().len() < channel.capacity
            }; // guard 释放
            if ready { ready_indices.push(i); }
        }
    }

    if !ready_indices.is_empty() {
        // 无偏随机选择一个就绪 case（避免饥饿，08-concurrency.md:259）
        let chosen_idx = self.rng.gen_range(0..ready_indices.len());
        let entry = &cases[ready_indices[chosen_idx]];

        if entry.kind == 0 {
            // 执行 RECEIVE（复用 RECEIVE 指令逻辑的非阻塞路径）
            let val = self.try_receive(entry.channel_slot)?;
            self.store_local(entry.target_slot as usize, val);
        } else {
            // 执行 SEND（复用 SEND 指令逻辑的非阻塞路径）
            let val = self.load_local(entry.value_slot as usize);
            self.try_send(entry.channel_slot, val)?;
        }

        // 跳转到 body
        self.ip = (select_pc as i64 + entry.body_offset as i64) as usize;
        return RunOutcome::Continue;
    }

    if has_default {
        self.ip = (select_pc as i64 + default_offset as i64) as usize;
        return RunOutcome::Continue;
    }

    // 全部未就绪且无 default：挂到所有 channel 的等待列表
    // 通过 SelectToken 标识本次 select 实例，防止多重唤醒
    let token = self.next_select_token();
    let channels: Vec<(*mut MsObjHeader, SelectOpKind)> = cases.iter()
        .map(|e| (
            expect_channel(&self.load_local(e.channel_slot))?,
            if e.kind == 0 { SelectOpKind::Recv } else { SelectOpKind::Send { value_slot: e.value_slot } },
        ))
        .collect();

    return RunOutcome::Yield(ChannelYield::Select { channels, token });
}
```

> **关键修正点**（针对原伪代码漏洞）：
> 1. **`gen_range` 取代取模**：`rand::thread_rng().gen_range(0..n)` 无偏均匀采样，满足 `08-concurrency.md:259` 「随机」语义。`rand` crate 依赖需在 `Cargo.toml` 声明。
> 2. **空 ready_indices 防御**：取模前先判 `is_empty()`；case_count=0 单独走 `EmptySelect` 分支，永不到达取模。
> 3. **`check_safepoint()` 入口**：SELECT 是阻塞指令，按 `14-gc.md:586` 必须作为 safepoint。
> 4. **RefCell guard 边界**：每次 case 评估用 `{ ... }` 块限定 `try_borrow`/`borrow` 生命周期，下一 case 评估前必释放。
> 5. **send-on-closed 静默跳过**：send case 目标 channel 已关闭时该 case 不参与就绪判定，不抛错（select 选择性语义）。

### EventLoop 集成

本 task 扩展 task 54 的 `ChannelYield` 枚举（`src/vm/mod.rs:109`）新增两个 select 专用变体：

```rust
enum ChannelYield {
    Send { channel: *mut MsObjHeader, value: Object },         // task 54
    Recv { channel: *mut MsObjHeader },                        // task 54
    // 本 task 新增：
    Select {
        channels: Vec<(*mut MsObjHeader, SelectOpKind)>,
        token: u64,                                            // 全局递增，标识本次 select 实例
    },
    EmptySelect,                                               // 空 select{} 永久阻塞
}

enum SelectOpKind {
    Recv,
    Send { value_slot: u8 },   // value 已存于局部变量槽，恢复时按槽读取
}
```

> **为什么不复用 Send/Recv 单 channel 变体**：select 暂停的协程需同时挂到 N 个 channel 的等待列表上，且任一 channel 唤醒后须从其他列表清除。单 channel 变体无法表达这种「一处被唤醒、N-1 处需清理」的扇出/扇入关系。

### SelectToken 与多重唤醒防护

`SelectToken` 是全局递增的 `u64`，每次进入 select 暂停分支时由 VM 分配。等待列表条目扩展：

```rust
// 扩展 task 54 的 WaitingSender / WaitingReceiver
struct WaitingSender {
    coroutine: Coroutine,
    value: Object,
    select_token: Option<u64>,   // None = 普通 SEND；Some = select SEND
}
struct WaitingReceiver {
    coroutine: Coroutine,
    select_token: Option<u64>,   // None = 普通 RECV；Some = select RECV
}
```

唤醒方（SEND/RECEIVE/close 的 pop_front 路径）逻辑：

```rust
fn wake_one_waiting_receiver(channel: &MsChannel, ready_value: Object, loop: &mut EventLoop) {
    let mut receivers = channel.waiting_receivers.borrow_mut();
    while let Some(r) = receivers.pop_front() {
        if let Some(token) = r.select_token {
            // select 协程：CAS 检查 token 是否已被另一 channel 唤醒
            if loop.is_select_already_woken(token) {
                continue;   // 已被唤醒，跳过此条目（漏唤醒安全）
            }
            loop.mark_select_woken(token);
            // 从其他 channel 的等待列表中清除同 token 的条目
            loop.cleanup_select_entries(token, except_channel=channel);
        }
        // 唤醒协程（压入值、push 到 ready_queue）
        r.coroutine.value_stack.push(ready_value);
        loop.ready_queue.push_back(r.coroutine);
        return;
    }
}
```

> **`is_select_already_woken` / `mark_select_woken`**：EventLoop 维护 `HashMap<u64, WokenChannel>` 记录每个 select token 的唤醒状态。即使多个 channel 在同一调度周期内尝试唤醒同一协程，只有第一个成功；后续的 wake 请求被丢弃，待发送值/待接收槽位留在原 channel 中（不丢失数据）。

### EventLoop 处理 select 暂停

在 task 53 `EventLoop::run`（`53-async-await.md:243-275`）的 `match result` 中追加两个分支：

```rust
ChannelYield::Select { channels, token } => {
    // 快照协程状态
    coroutine.call_stack = vm.call_stack.split_off(0);
    coroutine.stack = vm.snapshot_value_stack();
    coroutine.defer_stack = std::mem::take(&mut vm.defer_stack);
    coroutine.tlab = vm.tlab.take();
    // 标记 select 暂停状态
    coroutine.select_state = Some(SelectState {
        token,
        channels: channels.iter().map(|(p, _)| *p).collect(),
    });

    // 挂到每个 channel 的对应等待列表
    for (ch_ptr, kind) in channels {
        let ch = unsafe { &*ch_ptr };
        match kind {
            SelectOpKind::Recv => ch.waiting_receivers.borrow_mut().push_back(
                WaitingReceiver { coroutine_snapshot: coroutine.clone_shallow(), select_token: Some(token) }
            ),
            SelectOpKind::Send { value_slot } => {
                let val = vm.load_local(value_slot as usize);
                ch.waiting_senders.borrow_mut().push_back(
                    WaitingSender { coroutine_snapshot: coroutine.clone_shallow(), value: val, select_token: Some(token) }
                )
            }
        }
    }

    // 协程本体存入 paused，任一 channel 唤醒时从这里取出
    self.paused.push(PausedCoroutine {
        coroutine,
        waiting_on: SELECT_MARKER,  // 特殊 sentinel，区分单 Future 暂停
    });
}

ChannelYield::EmptySelect => {
    // 空 select{}：永久阻塞。协程存入 paused 但不挂到任何 channel。
    // EventLoop 的死锁检测须排除此类协程（见下方「死锁检测调整」）。
    coroutine.call_stack = vm.call_stack.split_off(0);
    coroutine.stack = vm.snapshot_value_stack();
    coroutine.defer_stack = std::mem::take(&mut vm.defer_stack);
    coroutine.tlab = vm.tlab.take();
    coroutine.select_state = Some(SelectState { token: 0, channels: vec![] });
    self.paused.push(PausedCoroutine {
        coroutine,
        waiting_on: EMPTY_SELECT_MARKER,
    });
}
```

### 死锁检测调整

task 53 `EventLoop::run` 的死锁检测（`53-async-await.md:276-281`）须修改：

```rust
} else {
    // 无就绪协程但有暂停协程
    let all_empty_select = self.paused.iter().all(|p|
        p.waiting_on == EMPTY_SELECT_MARKER
        || (p.coroutine.select_state.as_ref().map_or(false, |s| s.channels.is_empty()))
    );
    if all_empty_select {
        // 全部是空 select{}：合法的永久阻塞，但程序不会前进
        // 选择：1) 永久挂起（标准 Go 语义）；2) 报 "program suspended in empty select" 错误
        // 推荐方案 1：保持主协程可被外部信号/IO 唤醒的语义
        std::thread::park();   // 或返回 SuspendResult::BlockedForever
        return Ok(Object::Nil);
    }
    return Err(MspError::RuntimeError("deadlock: all coroutines paused".into()));
}
```

> **`08-concurrency.md:262` 语义落实**：空 select{} 不触发 deadlock 错误，而是永久阻塞。本实现选择「永久挂起」（与 Go 一致）。`08-concurrency.md` 与 task 53 同步增补此说明（见本 task 「同步设计文档更新」章节）。

## GC 安全

本 task 引入 select 暂停状态——同一协程被多个 channel 的等待列表同时引用。GC 必须正确扫描所有可达对象，且不重复 trace（性能）。

### 根集扩展

新增根集来源（参照 `14-gc.md:606-626`、task 53 `53-async-await.md:340-349`、task 54 `54-channel.md:384-394`）：

| 新增根集来源 | 扫描内容 |
|---|---|
| `EventLoop.paused` 中 select 暂停协程 | 协程 `stack` 中的所有 `Object::Ref` + `call_stack.top().closure` + `defer_stack` Ref + `select_state.channels` 中的 channel 指针 |
| 各 channel `waiting_senders`/`waiting_receivers` 中带 `select_token` 的条目 | **不重复 trace**：channel trace 函数遍历到带 select_token 的条目时跳过协程本体（已由 EventLoop.paused 扫描），仅 trace `WaitingSender.value` 中的 Ref |
| 协程的临时槽（`tmp_chN`、`tmp_valN`） | 已包含在协程 `stack` 扫描中 |

> **关键不变量**：select 暂停的协程**唯一拥有权**在 `EventLoop.paused`。channel 等待列表中的 `WaitingSender.coroutine_snapshot` / `WaitingReceiver.coroutine_snapshot` 为弱引用快照（仅用于 EventLoop 唤醒时定位），GC 不应通过它们 trace 协程本体——否则同一对象会被 trace N 次。

### CHANNEL trace 调整

task 54 的 `trace_channel`（`54-channel.md:342-379`）须修改以识别 select 条目：

```rust
fn trace_channel(header: *mut MsObjHeader, callback: &mut dyn FnMut(*mut MsObjHeader)) {
    let channel = unsafe { &*(header as *const MsChannel) };

    // 1. buffer（不变）
    for obj in channel.buffer.borrow().iter() {
        if let Object::Ref(ptr) = obj { callback(ptr); }
    }

    // 2. waiting_senders：select 条目仅 trace value，不 trace coroutine_snapshot
    for sender in channel.waiting_senders.borrow().iter() {
        if let Object::Ref(ptr) = &sender.value { callback(ptr); }
        if sender.select_token.is_none() {
            // 普通 SEND 条目：trace 协程本体（task 54 行为）
            trace_coroutine(&sender.coroutine_snapshot, callback);
        }
        // select 条目：协程本体由 EventLoop.paused 统一 trace，跳过
    }

    // 3. waiting_receivers：select 条目不 trace 协程本体
    for receiver in channel.waiting_receivers.borrow().iter() {
        if receiver.select_token.is_none() {
            trace_coroutine(&receiver.coroutine_snapshot, callback);
        }
    }
}
```

### SELECT 安全点

SELECT 入口的 `check_safepoint()`（见 VM 伪代码）确保 STW 期间协程不在 case 评估中。任一 case 评估循环内的隐式 safepoint 由 `try_borrow` 失败时退化为标灰策略保证（参照 `54-channel.md:413`）。

### 写屏障

select 选中 send case 时通过 `try_send` 写入对端 channel.buffer，需触发混合写屏障（`14-gc.md:546`）。`try_send` 复用 task 54 `SEND` 指令的写屏障逻辑（`54-channel.md:151`）：

```rust
fn try_send(&mut self, channel_slot: usize, value: Object) -> Result<()> {
    // ... 同 task 54 SEND 非阻塞路径 ...
    self.write_barrier(&mut buffer, value.clone());
    buffer.push_back(value);
    // ...
}
```

### GC 移动对象的指针更新

Minor GC 半空间复制移动 Young 代对象时（`14-gc.md:351-359`），以下指针需 forwarding 更新：

- select 暂停协程 `stack` 中所有 `Object::Ref`
- `select_state.channels` Vec 中的 channel 指针
- 临时槽 `tmp_chN`、`tmp_valN` 中的 `Object::Ref`
- `WaitingSender.value` 中的 `Object::Ref`（select 与普通 SEND 一致）

### RefCell borrow 约束

select 评估多 case 时严格遵循 task 54 `54-channel.md:408-413` 的约束：

- 每个 case 的 `channel.buffer.borrow()` / `waiting_senders.borrow()` / `waiting_receivers.borrow()` guard 在下一次 case 评估前必须释放（用 `{ ... }` 块限定）
- 评估期间禁止 `return RunOutcome::Yield(...)`——yield 前所有 guard 必须已释放
- 同一 channel 不允许在多个 case 中嵌套 borrow（语法上 select 允许两 case 指向同 channel；本实现要求按 case 顺序串行评估，guard 不跨 case）
- GC trace 函数使用 `try_borrow()`，失败时标灰待重扫

## cancel / close 交互

### cancel 处理

task 55 `55-go-concurrency.md:219-239` 定义 `handle.cancel()` 在 safepoint 终止协程。select 暂停的协程被 cancel 时：

1. EventLoop 检测 `coroutine.handle.cancel_requested == true`
2. 遍历 `select_state.channels`，从每个 channel 的 `waiting_senders`/`waiting_receivers` 中删除 `select_token == token` 的条目
3. 唤醒协程，注入 `RuntimeError("coroutine cancelled")`
4. 协程 defer 栈正常 LIFO 执行

### channel close 交互

select 暂停期间，若被监听的某个 channel 被关闭（task 54 `54-channel.md:235-258` close 逻辑会唤醒所有等待者）：

- close 路径遍历 `waiting_receivers` 唤醒所有协程——select 协程被唤醒后重新执行 SELECT 指令
- 重扫描时已关闭 channel 的 receive case 视为就绪（取值 nil），send case 静默跳过
- 若 close 后所有 case 都不就绪且无 default，协程再次暂停（重新挂到剩余未关闭 channel）
- close 路径遍历 `waiting_senders` 时，select send 条目按「send-on-closed 错误」处理：标记 token 为「已被 wake with error」，唤醒协程后由 SELECT handler 检测并选择下一就绪 case（若没有则整体抛 "send on closed channel"）

## 验证标准

1. 单 case 接收正确执行
2. 多 case 同时就绪时**无偏随机**选择（多次运行统计分布近似均匀）
3. send case 在目标 channel 缓冲区有空位时立即就绪并执行其 body
4. `default` 在无就绪分支时立即执行（非阻塞）
5. 无 `default` 时阻塞直到有分支就绪
6. 空 `select {}` 永久阻塞（不触发 deadlock 错误）
7. send case 目标 channel 已关闭时该 case 静默跳过（不抛错）
8. receive case 目标 channel 已关闭且缓冲区空时立即返回 nil（视为就绪）
9. select 暂停期间被 cancel 时从所有 channel 等待列表清除并执行 defer
10. select 暂停期间被监听的 channel 关闭时正确唤醒并重新评估
11. send value 表达式仅求值一次（即使 select 重试多次）
12. 多个 case 指向同一 channel 时不触发 RefCell panic（borrow guard 正确释放）
13. select body 偏移超过 32KB 时编译器报错
14. case 数超过 255 时编译器报错
15. select 关键字升级后不再被词法器拒绝（`select`/`case`/`default` 可正常解析）

## 测试用例

### test_select_basic.ms

```ms
ch1 = channel(1)
ch2 = channel(1)

go fn() {
    ch1 <- "hello"
}()

select {
    case val = <-ch1 {
        print("from ch1: " + val)
    }
    case ch2 <- data {
        print("sent to ch2")
    }
    default {
        print("no activity")
    }
}
```

预期输出：`from ch1: hello`

### test_select_default.ms

> 验证 default 分支在所有 case 未就绪时立即执行。

```ms
ch1 = channel(1)
ch2 = channel(1)

select {
    case v = <-ch1 {
        print("recv: " + str(v))
    }
    case ch2 <- "x" {
        print("sent")
    }
    default {
        print("no activity")
    }
}
```

预期输出：`no activity`

### test_select_send.ms

> 验证 send case 在缓冲区有空位时立即就绪。

```ms
ch = channel(1)

select {
    case ch <- 42 {
        print("sent successfully")
    }
}

print(<-ch)
```

预期输出：
```
sent successfully
42
```

### test_select_random.ms

> 验证多 case 同时就绪时无偏随机选择（统计分布）。

```ms
counts = {"a": 0, "b": 0}

for i in range(100) {
    ch_a = channel(1)
    ch_b = channel(1)
    ch_a <- "a"
    ch_b <- "b"

    select {
        case v = <-ch_a {
            counts["a"] = counts["a"] + 1
        }
        case v = <-ch_b {
            counts["b"] = counts["b"] + 1
        }
    }
}

print(counts)
```

预期输出：counts.a 与 counts.b 均接近 50（允许 ±15 偏差），证明无偏采样。

### test_select_block.ms

> 验证无 default 时阻塞直到有 case 就绪。

```ms
ch = channel(1)

go fn() {
    ch <- "delayed"
}()

select {
    case v = <-ch {
        print("got: " + v)
    }
}
```

预期输出：`got: delayed`

### test_select_empty.ms

> 验证空 select{} 永久阻塞（不报 deadlock）。
> 测试需配合超时机制：在独立协程中关闭一个 channel 唤醒主协程。

```ms
done = channel(1)

go fn() {
    # 主协程会卡在空 select{}，此处永远无法到达
    done <- "unreachable"
}

try {
    select {}
} except e {
    print("should not reach")
}
```

预期行为：程序永久挂起（不输出任何内容、不抛 deadlock）。CI 中应通过外部 timeout 终止并视为通过。

### test_select_value_eval_once.ms

> 验证 send value 表达式只求值一次。

```ms
counter = 0

fn next_val() {
    counter = counter + 1
    return counter
}

ch = channel(1)
ch.close()   # 让 send case 永远不就绪

select {
    case ch <- next_val() {
        print("sent")
    }
    default {
        print("default, counter=" + str(counter))
    }
}
```

预期输出：`default, counter=1`（即使 select 重试，`next_val` 也只调用一次）

### test_select_closed_channel.ms

> 验证已关闭 channel 的 receive case 立即返回 nil。

```ms
ch = channel(1)
ch.close()

select {
    case v = <-ch {
        print("got: " + str(v))
    }
}
```

预期输出：`got: nil`

### test_select_send_on_closed.ms

> 验证 send case 目标 channel 已关闭时静默跳过。

```ms
ch1 = channel(1)
ch2 = channel(1)
ch1.close()

select {
    case ch1 <- "x" {
        print("should not happen")
    }
    case v = <-ch2 {
        print("got from ch2: " + str(v))
    }
    default {
        print("default (ch1 closed, ch2 empty)")
    }
}
```

预期输出：`default (ch1 closed, ch2 empty)`

### test_select_cancel.ms

> 验证 select 暂停的协程被 cancel 时正确清理。

```ms
ch1 = channel(1)
ch2 = channel(1)
cleanup = channel(1)

handle = go fn() {
    defer cleanup <- "cleaned"
    select {
        case v = <-ch1 {
            print("ch1: " + str(v))
        }
        case v = <-ch2 {
            print("ch2: " + str(v))
        }
    }
}()

# 给协程时间进入 select 暂停
await async.sleep(0)

handle.cancel()

print(await cleanup.join())
```

预期输出：`cleaned`（defer 正常执行，select 未输出）
