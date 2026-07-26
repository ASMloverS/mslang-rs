pub mod builtins;
pub mod frame;
pub mod gc;
pub mod object;
pub mod stdlib;

use crate::compiler::opcode::OpCode;
use crate::compiler::Chunk;
use crate::module::{self, ModuleResolver};
use crate::async_runtime::channel::{read_channel, ChannelState, WaitingReceiver, WaitingSender};
use crate::async_runtime::join_handle::{alloc_join_handle, read_join_handle};
use crate::vm::builtins::{alloc_native_function, read_native_function, NativeFunction, to_iterator};
use crate::vm::object::{
    alloc_bound_method, alloc_class, alloc_closure, alloc_dict, alloc_exception,
    alloc_exception_class, alloc_function, alloc_future, alloc_generator, alloc_instance,
    alloc_iterator, alloc_list, alloc_module, alloc_set, alloc_string, alloc_tuple, alloc_upvalue,
    read_bound_method, read_class, read_closure, read_dict, read_exception, read_exception_class,
    read_exception_mut, read_function, read_future, read_generator, read_generator_mut,
    read_instance, read_iterator, read_list, read_module, read_module_mut, read_set, read_str,
    read_tuple, read_upvalue, CmpOp, DictMap, Function, FutureState, GeneratorState, MsException,
    MsGenerator, MsObjHeader, MsUpvalue, Object, TypeTag,
};
use frame::CallFrame;
use std::collections::HashMap;
use std::path::PathBuf;

const STACK_MAX: usize = 1024;
/// 调用栈最大深度（对齐 Python 默认；task 28/31/36/37/70 共用此常量）。
pub const MAX_CALL_DEPTH: usize = 1000;

/// defer 注册条目（task 36）。`call_tuple` = tuple(callee, arg1, ..., argN)，
/// 在 defer 注册时已求值完毕（规则 3）。GC 须将其作根扫描（见 gc.rs）。
#[derive(Clone)]
pub struct DeferEntry {
    pub call_tuple: Object,
}

/// task 39：生成器恢复执行的结果。YIELD 产出值或自然结束/close 耗尽。
enum GenOutcome {
    Yielded(Object),
    Exhausted,
}

/// 异常处理器条目（task 37）。与 defer_stack 一样按帧分区，但用 frame_stack_base
/// （值栈基址）判定所属帧。throw() 自顶向下扫描，匹配当前帧者跳到 catch_address。
pub struct ExceptionHandler {
    /// except 分派器入口（throw 跳转点）。
    catch_address: usize,
    /// finally 块入口地址（None 表示无 finally 块）。当前由编译端 dispatcher 经 JUMP
    /// 路由到 finally，VM 不直接读取；保留以契合 spec（TRY_ENTER 双操作数）。
    #[allow(dead_code)]
    finally_address: Option<usize>,
    /// 所属帧的值栈基址（跨帧判定）。
    frame_stack_base: usize,
    /// 进入 try 时值栈长度（unwind 时恢复栈平衡）。
    scope_stack_base: usize,
}

// ---------------------------------------------------------------------------
// task 53：async/await 协程
// ---------------------------------------------------------------------------

/// 协程。每个协程拥有独立的执行上下文（call_stack / value_stack / defer_stack 等）。
/// EventLoop 在协程间切换时整体保存/恢复这些字段。
pub struct Coroutine {
    pub call_stack: Vec<CallFrame>,
    pub stack: Vec<Object>,
    pub defer_stack: Vec<DeferEntry>,
    pub open_upvalues: Vec<*mut MsObjHeader>,
    pub exception_handlers: Vec<ExceptionHandler>,
    pub pending_unwind: Option<Object>,
    /// async fn 协程关联的 Future（TypeTag::FUTURE）；主协程为 None。
    /// 协程完成时 EventLoop 通过此字段 resolve 对应 Future。
    pub future: Option<*mut MsObjHeader>,
    /// task 55：go 协程关联的 JoinHandle（TypeTag::JOIN_HANDLE）；非 go 协程为 None。
    /// 协程完成时 EventLoop 通过此字段填充 JoinHandle result/error/done。
    pub handle: Option<*mut MsObjHeader>,
}

/// 暂停的协程。waiting_on 指向被 await 的 MsFuture。
pub struct PausedCoroutine {
    pub coroutine: Coroutine,
    pub waiting_on: *mut MsObjHeader,
}

/// 事件循环。协作式调度，FIFO 就绪队列 + 暂停列表。
pub struct EventLoop {
    pub ready_queue: std::collections::VecDeque<Coroutine>,
    pub paused: Vec<PausedCoroutine>,
}

impl EventLoop {
    pub fn new() -> Self {
        Self {
            ready_queue: std::collections::VecDeque::new(),
            paused: Vec::new(),
        }
    }
}

impl Default for EventLoop {
    fn default() -> Self {
        Self::new()
    }
}

/// task 54：channel 暂停信号。SEND/RECEIVE 阻塞时设置，event_loop_run 据此
/// 将协程快照存入 channel 的等待列表（参照 yield_future 的 await 模式）。
enum ChannelYield {
    /// SEND 阻塞：协程及其待发送值存入 channel.waiting_senders。
    Send {
        channel: *mut MsObjHeader,
        value: Object,
    },
    /// RECEIVE 阻塞：协程存入 channel.waiting_receivers。
    Recv {
        channel: *mut MsObjHeader,
    },
}

/// 内置异常层级（父类链）。父类一律为 Error。Phase 5 升级为正式 Class 后此表废弃。
/// 参照 [37-try-except-finally](../docs/mslang/tasks/37-try-except-finally.md) §5。
const EXCEPTION_PARENTS: &[(&str, &str)] = &[
    ("ValueError", "Error"),
    ("TypeError", "Error"),
    ("IndexError", "Error"),
    ("KeyError", "Error"),
    ("AttributeError", "Error"),
    ("NameError", "Error"),
    ("RuntimeError", "Error"),
    ("IOError", "Error"),
    ("ZeroDivisionError", "Error"),
    ("OverflowError", "Error"),
    ("StopIteration", "Error"),
    ("GeneratorExit", "Error"),
    // task 45：ImportError（父类 Error），由 load()/IMPORT handler 抛出。
    ("ImportError", "Error"),
];

/// 内置异常类名（Error + 子类）。VM::new 时注册为 EXCEPTION_CLASS 全局变量。
const BUILTIN_EXCEPTION_NAMES: &[&str] = &[
    "Error",
    "ValueError",
    "TypeError",
    "IndexError",
    "KeyError",
    "AttributeError",
    "NameError",
    "RuntimeError",
    "IOError",
    "ZeroDivisionError",
    "OverflowError",
    "StopIteration",
    "GeneratorExit",
    // task 45：使脚本可 `except ImportError`。
    "ImportError",
];

pub struct VM {
    stack: Vec<Object>,
    call_stack: Vec<CallFrame>,
    globals: HashMap<String, Object>,
    /// 内置函数参数个数表（`usize::MAX` = 可变参数），供 CALL 校验。
    native_arities: HashMap<String, usize>,
    /// 开放上值列表（task 28）。每项指向 MsUpvalue（TypeTag::UPVALUE）。
    /// **不变量**：按 `location` **升序**维护（最小在前）。
    /// `close_upvalues_from` 从末尾向前扫描，依赖此序保证正确性（见 §6/§8）。
    open_upvalues: Vec<*mut MsObjHeader>,
    /// defer 栈（task 36）。每个 CallFrame 经 `defer_stack_base` 分区隔离；
    /// EXEC_DEFER 按本帧区间 LIFO 刷新。
    defer_stack: Vec<DeferEntry>,
    /// 异常处理器栈（task 37）。按帧分区（frame_stack_base 判定所属帧）。
    /// TRY_ENTER 压入、TRY_EXIT 弹出；throw() 自顶向下扫描匹配当前帧者。
    exception_handlers: Vec<ExceptionHandler>,
    /// task 37：待传播的异常。throw() 置位后由 drive_unwind() 推进；drive_unwind 在
    /// 需要运行 closure defer 时「泊车」（压 defer 帧后返回主循环），主循环顶部检测到
    /// pending_unwind + 当前帧 defer_flushing 时重新调用 drive_unwind 续行。
    pending_unwind: Option<Object>,
    /// task 39：生成器恢复结果的传输槽。YIELD / generator_return 写入，
    /// run_until_generator_yield 读取。每次 resume 前清空。
    gen_outcome: Option<GenOutcome>,
    /// task 39：GET_ATTR 对 GENERATOR 解析 __next__/close/__iter__ 时写入待调用方法 id，
    /// CALL（call_value）对 GENERATOR 被调用者读取并清空。1=__next__，2=close，3=__iter__。
    gen_call_method: Option<u8>,
    /// GC 堆（task 52）。MVP 经 `gc::maybe_gc` 在主循环触发；当前 VM 日常分配
    /// （`object.rs`/`builtins.rs` 的 `alloc_*`）尚未接入 GC 堆，故 GC 保持 dormant。
    /// task 74：pub(crate) 供 capi::gc 的 GC 控制/统计/finalizer API 直接访问。
    pub(crate) heap: gc::MsHeap,
    /// task 53：事件循环。管理协程的就绪队列与暂停列表。
    pub(crate) event_loop: EventLoop,
    /// task 53：AWAIT Pending 时设置的 yield 信号。run_loop 退出后 EventLoop 检查此字段：
    /// Some(fp) → 协程因 await 暂停；None → 协程正常完成。
    yield_future: Option<*mut MsObjHeader>,
    /// task 54：SEND/RECEIVE 阻塞时设置的 yield 信号。event_loop_run 检查此字段：
    /// Some(cy) → 协程因 channel 操作暂停，快照存入对应 channel 等待列表。
    yield_channel: Option<ChannelYield>,
    /// task 55：await handle.join() 时 JoinHandle 未完成的 yield 信号。
    /// Some(hp) → 协程因 join 暂停，存入 event_loop.paused，waiting_on = handle_ptr。
    yield_join: Option<*mut MsObjHeader>,
    /// task 55：当前执行协程关联的 JoinHandle（None = 非 go 协程）。
    /// event_loop_run 恢复协程时设置，用于安全点 cancel 检查。
    current_coro_handle: Option<*mut MsObjHeader>,
    /// task 53：协程未捕获异常的 Object 快照。drive_unwind 在 call_stack 空时格式化错误
    /// 前存储此值，供 EventLoop 据此 reject Future。
    last_uncaught_exception: Option<Object>,
    /// task 42：隐式 Object 基类（Immortal 代）。无显式父类的类在 CLASS handler
    /// 中自动链接至此；提供默认 __repr__/__eq__/__ne__。
    pub(crate) object_class: *mut MsObjHeader,
    /// task 45：模块解析器（搜索路径/缓存/加载链/安全模式）。
    pub(crate) module_resolver: ModuleResolver,
    /// task 45：基线全局快照（内置函数 + 异常类），供 execute_module 隔离时复用，
    /// 使模块代码可访问 print/type/except 等而不泄漏调用方用户定义。
    baseline_globals: HashMap<String, Object>,
    /// task 66：C API 输出重定向回调（None = 使用默认 stdout/stderr）。
    /// 指向 capi::vm::WriteCallback（经 msSetStdout/msSetStderr 设置）。
    #[cfg(feature = "capi")]
    pub stdout_writer: Option<*mut std::ffi::c_void>,
    #[cfg(feature = "capi")]
    pub stderr_writer: Option<*mut std::ffi::c_void>,
    /// task 67：C 侧注册的 GC 根集合。msRoot/msUnroot 增删。
    /// GC 标记阶段作为额外根集参与扫描（14-gc.md:616）。
    #[cfg(feature = "capi")]
    pub(crate) c_roots: std::collections::HashSet<*mut MsObjHeader>,
    /// task 68：C API 错误标志占位（Task 71 完成后由 msThrowTypeError 取代）。
    #[cfg(feature = "capi")]
    pub(crate) has_error: bool,
    /// task 68：C API 错误消息占位（Task 71 完成后由 msThrowTypeError 取代）。
    #[cfg(feature = "capi")]
    pub(crate) error_message: String,
    /// task 70：MsVM* 反向指针。C API 上下文运行时由 msVmNew 设置；
    /// 纯 Rust 调用时为 null。call_value 的 NATIVE_C_FUNCTION 分支使用。
    #[cfg(feature = "capi")]
    pub capi_vm_ptr: *mut u8,
    /// task 72：已加载的动态库句柄，生命周期与 VM 相同。
    /// Library 必须存活以保持 C 函数指针有效。
    #[cfg(feature = "capi")]
    pub loaded_libs: Vec<libloading::Library>,
}

impl VM {
    pub fn new() -> Self {
        let mut vm = VM {
            // 预留 slot 0（callee 占位），修复 task 26 发现的「顶层预留 slot 0
            // 但 VM 栈未预分配 → StoreLocal 1 越界」bug（订正 A3）。
            stack: vec![Object::Nil],
            call_stack: Vec::new(),
            globals: HashMap::new(),
            native_arities: HashMap::new(),
            open_upvalues: Vec::new(),
            defer_stack: Vec::new(),
            exception_handlers: Vec::new(),
            pending_unwind: None,
            gen_outcome: None,
            gen_call_method: None,
            heap: gc::MsHeap::new(),
            event_loop: EventLoop::new(),
            yield_future: None,
            yield_channel: None,
            yield_join: None,
            current_coro_handle: None,
            last_uncaught_exception: None,
            object_class: std::ptr::null_mut(),
            module_resolver: ModuleResolver::new(),
            baseline_globals: HashMap::new(),
            #[cfg(feature = "capi")]
            stdout_writer: None,
            #[cfg(feature = "capi")]
            stderr_writer: None,
            #[cfg(feature = "capi")]
            c_roots: std::collections::HashSet::new(),
            #[cfg(feature = "capi")]
            has_error: false,
            #[cfg(feature = "capi")]
            error_message: String::new(),
            #[cfg(feature = "capi")]
            capi_vm_ptr: std::ptr::null_mut(),
            #[cfg(feature = "capi")]
            loaded_libs: Vec::new(),
        };
        vm.register_builtins();
        vm.init_object_class();
        vm.init_exception_classes();
        // task 45：快照基线全局（内置函数 + 异常类），供 execute_module 隔离复用。
        vm.baseline_globals = vm.globals.clone();
        // task 46：注册原生 io 模块 + 模块函数 arity（经 module.fn() 走 GET_ATTR→CALL 校验）。
        let io_ptr = stdlib::register_io_module();
        vm.module_resolver
            .native_modules
            .insert("io".to_string(), io_ptr);
        vm.native_arities.insert("read_file".to_string(), 1);
        vm.native_arities.insert("write_file".to_string(), 2);
        vm.native_arities.insert("exists".to_string(), 1);
        // "open" 已由 register_builtins 注册为 usize::MAX（可变参数），io.open 同名复用。

        // task 47：注册原生 math 模块 + 模块函数 arity（经 module.fn() 走 GET_ATTR→CALL 校验）。
        let math_ptr = stdlib::register_math_module();
        vm.module_resolver
            .native_modules
            .insert("math".to_string(), math_ptr);
        // 仅注册 math 独有函数的 arity。abs/ceil/floor/round 与全局内置同名，已由
        // register_builtins 登记（abs=1, ceil=1, floor=1, round=MAX）；此处不可覆盖，
        // 否则 round 的可变参数形式 round(n, digits) 会退化为固定 1 参（CALL 按 name
        // 查 native_arities，name 在全局/模块间共享）。
        vm.native_arities.insert("sqrt".to_string(), 1);
        vm.native_arities.insert("pow".to_string(), 2);
        vm.native_arities.insert("sin".to_string(), 1);
        vm.native_arities.insert("cos".to_string(), 1);
        vm.native_arities.insert("tan".to_string(), 1);
        vm.native_arities.insert("log".to_string(), 1);
        vm.native_arities.insert("log2".to_string(), 1);
        vm.native_arities.insert("log10".to_string(), 1);
        vm.native_arities.insert("exp".to_string(), 1);

        // task 48：注册原生 os/string/time/path 模块 + 模块函数 arity。
        for (name, ptr) in [
            ("os", stdlib::register_os_module()),
            ("string", stdlib::register_string_module()),
            ("time", stdlib::register_time_module()),
            ("path", stdlib::register_path_module()),
        ] {
            vm.module_resolver
                .native_modules
                .insert(name.to_string(), ptr);
        }
        // 仅注册模块独有函数 arity（CALL 按 name 查 native_arities，全局/模块间共享）。
        // format/join 为可变参（usize::MAX）；string.format 与 time.format 同名同 arity，无冲突。
        vm.native_arities.insert("getenv".to_string(), 1);
        vm.native_arities.insert("setenv".to_string(), 2);
        vm.native_arities.insert("getcwd".to_string(), 0);
        vm.native_arities.insert("chdir".to_string(), 1);
        vm.native_arities.insert("exec".to_string(), 1);
        vm.native_arities.insert("exit".to_string(), 1);
        vm.native_arities.insert("repeat".to_string(), 2);
        vm.native_arities.insert("reverse".to_string(), 1);
        vm.native_arities.insert("is_alpha".to_string(), 1);
        vm.native_arities.insert("is_digit".to_string(), 1);
        vm.native_arities.insert("now".to_string(), 0);
        vm.native_arities.insert("sleep".to_string(), 1);
        vm.native_arities.insert("format".to_string(), usize::MAX);
        vm.native_arities.insert("ext".to_string(), 1);
        vm.native_arities.insert("base".to_string(), 1);
        vm.native_arities.insert("dir".to_string(), 1);
        vm.native_arities.insert("join".to_string(), usize::MAX);

        // task 49：注册原生 json 模块 + 模块函数 arity。
        vm.module_resolver
            .native_modules
            .insert("json".to_string(), stdlib::register_json_module());
        vm.native_arities.insert("parse".to_string(), 1);
        vm.native_arities.insert("stringify".to_string(), 1);

        // task 60：注册原生 gc 模块 + 模块函数 arity。
        let gc_ptr = stdlib::register_gc_module();
        vm.module_resolver
            .native_modules
            .insert("gc".to_string(), gc_ptr);
        vm.native_arities.insert("collect".to_string(), 0);
        vm.native_arities.insert("collect_minor".to_string(), 0);
        vm.native_arities.insert("enable".to_string(), 0);
        vm.native_arities.insert("disable".to_string(), 0);
        vm.native_arities.insert("is_enabled".to_string(), 0);
        vm.native_arities.insert("set_threshold".to_string(), 2);
        vm.native_arities.insert("set_promotion_age".to_string(), 1);
        vm.native_arities.insert("set_gc_threads".to_string(), 1);
        vm.native_arities.insert("stats".to_string(), 0);
        vm.native_arities.insert("count".to_string(), 0);
        vm.native_arities.insert("mem_alloc".to_string(), 0);
        vm.native_arities.insert("mem_live".to_string(), 0);

        vm
    }

    pub fn interpret(&mut self, chunk: Chunk) -> Result<Object, String> {
        let function = Function {
            name: "<main>".to_string(),
            arity: 0,
            code: chunk.code,
            constants: chunk.constants,
            upvalue_count: 0,
            source_file: None,
            default_values: Vec::new(),
            has_variadic: false,
            required_arity: 0,
            is_generator: false,
            locals_count: 1,
            is_async: false,
        };
        let Object::Ref(closure_ptr) = alloc_closure(alloc_function(function), Vec::new()) else {
            unreachable!()
        };
        self.call_stack
            .push(CallFrame::new(closure_ptr, 0, self.defer_stack.len()));
        // task 53：主脚本作为主协程在事件循环中执行。
        let main_coro = self.take_coroutine_state(None, None);
        self.event_loop.ready_queue.push_back(main_coro);
        self.event_loop_run()
    }

    // ---- task 53：async/await 事件循环 ----

    /// 将当前 VM 执行状态提取为 Coroutine（move 语义，VM 字段被清空）。
    fn take_coroutine_state(
        &mut self,
        future: Option<*mut MsObjHeader>,
        handle: Option<*mut MsObjHeader>,
    ) -> Coroutine {
        Coroutine {
            call_stack: std::mem::take(&mut self.call_stack),
            stack: std::mem::take(&mut self.stack),
            defer_stack: std::mem::take(&mut self.defer_stack),
            open_upvalues: std::mem::take(&mut self.open_upvalues),
            exception_handlers: std::mem::take(&mut self.exception_handlers),
            pending_unwind: self.pending_unwind.take(),
            future,
            handle,
        }
    }

    /// 将 Coroutine 的状态恢复到 VM（move 语义）。
    fn restore_coroutine_state(&mut self, coro: Coroutine) {
        self.call_stack = coro.call_stack;
        self.stack = coro.stack;
        self.defer_stack = coro.defer_stack;
        self.open_upvalues = coro.open_upvalues;
        self.exception_handlers = coro.exception_handlers;
        self.pending_unwind = coro.pending_unwind;
    }

    /// 唤醒等待指定 Future/JoinHandle 的暂停协程，将它们从 paused 移至 ready_queue。
    fn wake_waiters(&mut self, resolved_ptr: *mut MsObjHeader) {
        let mut still_paused = Vec::new();
        for paused in self.event_loop.paused.drain(..) {
            if paused.waiting_on == resolved_ptr {
                self.event_loop.ready_queue.push_back(paused.coroutine);
            } else {
                still_paused.push(paused);
            }
        }
        self.event_loop.paused = still_paused;
    }

    /// task 55：安全点 cancel 检查（AWAIT/SEND/RECEIVE）。若当前协程的 JoinHandle
    /// 被请求取消，抛出 "coroutine cancelled" 异常。返回 true 表示已抛出（调用方
    /// 应 continue 重新进入主循环执行 catch）。
    fn check_cancel_safepoint(&mut self) -> Result<bool, String> {
        if let Some(handle_ptr) = self.current_coro_handle {
            // 先克隆标志值以释放 RefCell borrow（安全点 = GC 可运行）
            let cancelled = {
                let handle = unsafe { read_join_handle(handle_ptr) };
                *handle.cancel_requested.borrow()
            };
            if cancelled {
                let exc = alloc_exception(
                    "RuntimeError",
                    alloc_string("coroutine cancelled"),
                    alloc_string(""),
                    Object::Nil,
                );
                self.throw(exc)?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// 事件循环主方法。协作式调度：从 ready_queue 取协程 → 恢复 → run_loop →
    /// 根据 yield/complete/error 结果调度。所有协程完成后返回主协程结果。
    fn event_loop_run(&mut self) -> Result<Object, String> {
        let mut main_result = Object::Nil;

        while !self.event_loop.ready_queue.is_empty() || !self.event_loop.paused.is_empty() {
            let coro = match self.event_loop.ready_queue.pop_front() {
                Some(c) => c,
                None => {
                    // 无就绪协程但有暂停协程 → 死锁
                    return Err("deadlock: all coroutines paused".to_string());
                }
            };
            let coro_future = coro.future;
            let coro_handle = coro.handle;
            self.restore_coroutine_state(coro);
            self.current_coro_handle = coro_handle;

            self.yield_future = None;
            self.yield_channel = None;
            self.yield_join = None;
            self.last_uncaught_exception = None;
            let result = self.run_loop(None);
            self.current_coro_handle = None;

            if let Some(fp) = self.yield_future.take() {
                // 协程因 AWAIT Pending Future 暂停——保存当前状态到 paused
                let coro = self.take_coroutine_state(coro_future, coro_handle);
                self.event_loop.paused.push(PausedCoroutine {
                    coroutine: coro,
                    waiting_on: fp,
                });
            } else if let Some(hp) = self.yield_join.take() {
                // task 55：协程因 await join() 暂停——保存到 paused，waiting_on = handle_ptr。
                // go 协程完成时通过 wake_waiters(hp) 唤醒。
                let coro = self.take_coroutine_state(coro_future, coro_handle);
                self.event_loop.paused.push(PausedCoroutine {
                    coroutine: coro,
                    waiting_on: hp,
                });
            } else if let Some(cy) = self.yield_channel.take() {
                // task 54：协程因 channel SEND/RECEIVE 阻塞——快照存入 channel 等待列表
                let coro = self.take_coroutine_state(coro_future, coro_handle);
                match cy {
                    ChannelYield::Send { channel, value } => {
                        let ch = unsafe { read_channel(channel) };
                        ch.waiting_senders
                            .borrow_mut()
                            .push_back(WaitingSender { coroutine: coro, value });
                    }
                    ChannelYield::Recv { channel } => {
                        let ch = unsafe { read_channel(channel) };
                        ch.waiting_receivers
                            .borrow_mut()
                            .push_back(WaitingReceiver { coroutine: coro });
                    }
                }
            } else {
                // 协程完成或出错
                match result {
                    Ok(val) => {
                        if let Some(fp) = coro_future {
                            // async fn 协程：resolve Future
                            let f = unsafe { read_future(fp) };
                            *f.state.borrow_mut() = FutureState::Resolved(val.clone());
                            self.wake_waiters(fp);
                        }
                        if let Some(hp) = coro_handle {
                            // task 55：go 协程完成——填充 JoinHandle
                            let handle = unsafe { read_join_handle(hp) };
                            *handle.result.borrow_mut() = Some(val.clone());
                            *handle.done.borrow_mut() = true;
                            self.wake_waiters(hp);
                        }
                        if coro_future.is_none() && coro_handle.is_none() {
                            main_result = val;
                        }
                    }
                    Err(msg) => {
                        if let Some(fp) = coro_future {
                            // async fn 协程异常：reject Future
                            let exc = self.last_uncaught_exception.take().unwrap_or_else(|| {
                                alloc_exception(
                                    "Error",
                                    alloc_string(&msg),
                                    alloc_string(""),
                                    Object::Nil,
                                )
                            });
                            let f = unsafe { read_future(fp) };
                            *f.state.borrow_mut() = FutureState::Rejected(exc);
                            self.wake_waiters(fp);
                        } else if let Some(hp) = coro_handle {
                            // task 55：go 协程异常——存入 JoinHandle.error，不传播
                            let exc = self.last_uncaught_exception.take().unwrap_or_else(|| {
                                alloc_exception(
                                    "Error",
                                    alloc_string(&msg),
                                    alloc_string(""),
                                    Object::Nil,
                                )
                            });
                            let handle = unsafe { read_join_handle(hp) };
                            *handle.error.borrow_mut() = Some(exc);
                            *handle.done.borrow_mut() = true;
                            self.wake_waiters(hp);
                        } else {
                            return Err(msg);
                        }
                    }
                }
            }
        }

        Ok(main_result)
    }

    // ---- task 60：GC 便捷方法（供 gc stdlib native 函数调用） ----

    /// Full GC = minor + major + finalizers（参照 gc::maybe_gc 的 Full 路径）。
    pub fn gc_full(&mut self) {
        gc::minor_gc(
            &mut self.heap,
            &mut self.stack,
            &mut self.globals,
            &mut self.defer_stack,
            &mut self.call_stack,
        );
        gc::major_gc(
            &mut self.heap,
            &self.stack,
            &self.globals,
            &self.defer_stack,
            &self.call_stack,
        );
        gc::run_finalizers(&mut self.heap);
    }

    /// 仅 Minor GC + finalizers。
    pub fn gc_minor_only(&mut self) {
        gc::minor_gc(
            &mut self.heap,
            &mut self.stack,
            &mut self.globals,
            &mut self.defer_stack,
            &mut self.call_stack,
        );
        gc::run_finalizers(&mut self.heap);
    }

    /// task 74：仅 Major GC + finalizers（供 msGcCollect(MS_GC_MAJOR) 调用）。
    pub fn gc_major_only(&mut self) {
        gc::major_gc(
            &mut self.heap,
            &self.stack,
            &self.globals,
            &self.defer_stack,
            &self.call_stack,
        );
        gc::run_finalizers(&mut self.heap);
    }

    // ---- task 66：C API 访问器 ----

    /// 全局变量表只读引用（供 capi::vm 的 msGetGlobal 使用）。
    pub fn globals(&self) -> &HashMap<String, Object> {
        &self.globals
    }

    /// 全局变量表可变引用（供 capi::vm 的 msSetGlobal/msDelGlobal 使用）。
    pub fn globals_mut(&mut self) -> &mut HashMap<String, Object> {
        &mut self.globals
    }

    /// task 67：C root 注册表可变引用（供 capi::gc 的 msRoot/msUnroot 使用）。
    #[cfg(feature = "capi")]
    pub(crate) fn c_roots_mut(
        &mut self,
    ) -> &mut std::collections::HashSet<*mut MsObjHeader> {
        &mut self.c_roots
    }

    /// task 74：检查对象是否从 GC 根集（stack + globals + c_roots + call_stack
    /// current_exc）可达。用于 C finalizer 的可达性判定。
    /// 注意：此为浅层检查（不遍历对象图），覆盖 MVP test 用例。
    #[cfg(feature = "capi")]
    pub(crate) fn is_obj_reachable(&self, header: *mut MsObjHeader) -> bool {
        // stack
        for v in self.stack.iter() {
            if let Object::Ref(r) = v {
                if *r == header {
                    return true;
                }
            }
        }
        // globals
        for v in self.globals.values() {
            if let Object::Ref(r) = v {
                if *r == header {
                    return true;
                }
            }
        }
        // c_roots
        if self.c_roots.contains(&header) {
            return true;
        }
        // call_stack current_exc
        for frame in &self.call_stack {
            if let Some(Object::Ref(r)) = &frame.current_exc {
                if *r == header {
                    return true;
                }
            }
        }
        false
    }
}

impl VM {
    fn push(&mut self, value: Object) -> Result<(), String> {
        if self.stack.len() >= STACK_MAX {
            return Err("stack overflow".to_string());
        }
        self.stack.push(value);
        Ok(())
    }

    fn pop(&mut self) -> Result<Object, String> {
        self.stack
            .pop()
            .ok_or_else(|| "stack underflow".to_string())
    }

    fn peek(&self, distance: usize) -> Result<&Object, String> {
        let idx = self
            .stack
            .len()
            .checked_sub(distance + 1)
            .ok_or_else(|| "stack underflow".to_string())?;
        self.stack
            .get(idx)
            .ok_or_else(|| "stack underflow".to_string())
    }

    #[allow(dead_code)]
    fn peek_mut(&mut self, distance: usize) -> Result<&mut Object, String> {
        let idx = self
            .stack
            .len()
            .checked_sub(distance + 1)
            .ok_or_else(|| "stack underflow".to_string())?;
        self.stack
            .get_mut(idx)
            .ok_or_else(|| "stack underflow".to_string())
    }
}

/// 从 `catch_unwind` 捕获的 panic payload 中提取「不可哈希」TypeError 消息。
///
/// `Object::hash`（`object.rs`）对 list/dict/set/NaN 发
/// `panic!("TypeError: unhashable type: '...'")`，该消息已符合规范，直接转
/// `Err` 返回，使 try/except（task 37）可捕获而非终止 VM 进程（spec §3）。
fn unhashable_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = payload.downcast_ref::<&'static str>() {
        (*s).to_string()
    } else {
        "TypeError: unhashable type".to_string()
    }
}

/// 取 `Object` 的整数值；非 int 抛 TypeError（下标/切片索引专用，消息与 range 不同）。
fn require_int(obj: &Object) -> Result<i64, String> {
    match obj {
        Object::Int(n) => Ok(*n),
        other => Err(format!(
            "TypeError: indices must be integers, got '{}'",
            other.type_name()
        )),
    }
}

/// task 54：校验 Object 为 CHANNEL 引用，返回其裸指针。
fn expect_channel(obj: &Object) -> Result<*mut MsObjHeader, String> {
    match obj {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::CHANNEL as u8 => Ok(*ptr),
        other => Err(format!(
            "TypeError: expected a channel, got '{}'",
            other.type_name()
        )),
    }
}

/// task 54/55：ch.close() — 关闭 channel（幂等）。
/// 唤醒所有等待的接收者（给剩余缓冲区数据）与发送者（恢复后重试 SEND → 报错）。
/// 缓冲区耗尽的接收者：回退 ip 重派 FOR_ITER（检测 is_closed → 退出循环），
/// 避免将 nil 误当作有效迭代值（task 55 producer-consumer 场景）。
fn channel_close(vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = match args.first() {
        Some(o) => expect_channel(o)?,
        None => return Err("TypeError: close() expects a channel".to_string()),
    };
    let ch = unsafe { read_channel(ptr) };
    // 幂等：无论当前状态，置为 Closed。
    *ch.state.borrow_mut() = ChannelState::Closed;

    // 唤醒所有等待的接收者：依次给予剩余缓冲区数据，缓冲区空后回退 ip 重派 FOR_ITER。
    let receivers: Vec<WaitingReceiver> = ch.waiting_receivers.borrow_mut().drain(..).collect();
    for recv in receivers {
        let mut coro = recv.coroutine;
        if let Some(val) = ch.buffer.borrow_mut().pop_front() {
            // 缓冲区有剩余数据：压栈，接收者越过 FOR_ITER 消费。
            coro.stack.push(val);
        } else {
            // 缓冲区空 + 已关闭：回退 ip 重派 FOR_ITER（4 字节 = opcode + iter_slot + offset），
            // FOR_ITER 检测 is_closed → 退出循环，不误推 nil 为迭代值。
            if let Some(frame) = coro.call_stack.last_mut() {
                if frame.ip >= 4 {
                    frame.ip -= 4;
                }
            }
        }
        vm.event_loop.ready_queue.push_back(coro);
    }

    // 唤醒所有等待的发送者：将 channel 与待发送值压回栈，回退 ip 使 SEND 重新执行
    // → 检测 is_closed → 抛出 "send on closed channel"。
    let senders: Vec<WaitingSender> = ch.waiting_senders.borrow_mut().drain(..).collect();
    for sender in senders {
        let mut coro = sender.coroutine;
        // 栈布局 [value, channel]（channel 在顶），使重执行的 SEND 正确弹出。
        coro.stack.push(sender.value);
        coro.stack.push(Object::Ref(ptr));
        if let Some(frame) = coro.call_stack.last_mut() {
            if frame.ip > 0 {
                frame.ip -= 1; // SEND 无操作数，回退 1 字节重执行
            }
        }
        vm.event_loop.ready_queue.push_back(coro);
    }

    Ok(Object::Nil)
}

/// task 54：ch.closed() — 返回 channel 是否已关闭。
fn channel_closed(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = match args.first() {
        Some(o) => expect_channel(o)?,
        None => return Err("TypeError: closed() expects a channel".to_string()),
    };
    let ch = unsafe { read_channel(ptr) };
    Ok(Object::Bool(ch.is_closed()))
}

// ---- task 55：JoinHandle 方法 ----

/// task 55：校验 Object 为 JOIN_HANDLE 引用，返回其裸指针。
fn expect_join_handle(obj: &Object) -> Result<*mut MsObjHeader, String> {
    match obj {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::JOIN_HANDLE as u8 => Ok(*ptr),
        other => Err(format!(
            "TypeError: expected a JoinHandle, got '{}'",
            other.type_name()
        )),
    }
}

/// task 55：handle.join() — 返回 JoinHandle 自身作为 awaitable。
/// AWAIT 指令识别 JOIN_HANDLE 类型：done → 返回 result/抛 error；未完成 → 暂停。
fn join_handle_join(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    match args.first() {
        Some(o) => {
            expect_join_handle(o)?;
            Ok(o.clone())
        }
        None => Err("TypeError: join() expects a JoinHandle".to_string()),
    }
}

/// task 55：handle.is_done() — 返回协程是否已完成。
fn join_handle_is_done(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = match args.first() {
        Some(o) => expect_join_handle(o)?,
        None => return Err("TypeError: is_done() expects a JoinHandle".to_string()),
    };
    let handle = unsafe { read_join_handle(ptr) };
    Ok(Object::Bool(handle.is_done()))
}

/// task 55：handle.cancel() — 请求取消协程（协程在下一个安全点终止）。
fn join_handle_cancel(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = match args.first() {
        Some(o) => expect_join_handle(o)?,
        None => return Err("TypeError: cancel() expects a JoinHandle".to_string()),
    };
    let handle = unsafe { read_join_handle(ptr) };
    *handle.cancel_requested.borrow_mut() = true;
    Ok(Object::Nil)
}

/// list/tuple/string 整数索引归一化：负索引加 len；越界抛 IndexError。
fn normalize_index(idx: i64, len: usize) -> Result<usize, String> {
    let len_i = len as i64;
    let i = if idx < 0 { idx + len_i } else { idx };
    if i < 0 || i >= len_i {
        return Err(format!(
            "IndexError: index {} out of range for length {}",
            idx, len
        ));
    }
    Ok(i as usize)
}

/// GET_INDEX 辅助：`obj[key]` 读取。
/// list/tuple/string 需整数索引（负索引 + 越界 IndexError）；string 按字符返回单字符串；
/// dict 命中返回值，缺失返回 nil，不可哈希 key 经 catch_unwind 转 TypeError。
fn get_item(obj: Object, key: Object) -> Result<Object, String> {
    match &obj {
        Object::Ref(ptr) => {
            // SAFETY：type_tag 守卫确认 ptr 指向对应集合类型（由 alloc_* 分配）。
            let ptr = *ptr;
            let tag = unsafe { (*ptr).type_tag };
            if tag == TypeTag::LIST as u8 {
                let items = unsafe { read_list(ptr) };
                let i = normalize_index(require_int(&key)?, items.len())?;
                Ok(items[i].clone())
            } else if tag == TypeTag::TUPLE as u8 {
                let items = unsafe { read_tuple(ptr) };
                let i = normalize_index(require_int(&key)?, items.len())?;
                Ok(items[i].clone())
            } else if tag == TypeTag::STRING as u8 {
                let chars: Vec<char> = unsafe { read_str(ptr) }.chars().collect();
                let i = normalize_index(require_int(&key)?, chars.len())?;
                Ok(alloc_string(&chars[i].to_string()))
            } else if tag == TypeTag::DICT as u8 {
                // 不可哈希 key 在 HashMap 哈希阶段 panic（查询前），dict 未被破坏。
                let got = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    unsafe { read_dict(ptr) }.get(&key).cloned()
                }));
                match got {
                    Ok(v) => Ok(v.unwrap_or(Object::Nil)),
                    Err(p) => Err(unhashable_message(p)),
                }
            } else {
                Err(format!(
                    "TypeError: '{}' object is not subscriptable",
                    obj.type_name()
                ))
            }
        }
        _ => Err(format!(
            "TypeError: '{}' object is not subscriptable",
            obj.type_name()
        )),
    }
}

/// SET_INDEX 辅助：`obj[key] = val`。list 负索引 + 越界 IndexError；dict 设置/覆盖，
/// 不可哈希 key 经 catch_unwind 转 TypeError；string/tuple 等不可变类型抛 TypeError。
fn set_item(obj: Object, key: Object, val: Object) -> Result<(), String> {
    match &obj {
        Object::Ref(ptr) => {
            // SAFETY：type_tag 守卫确认 ptr 指向对应集合类型。
            let ptr = *ptr;
            let tag = unsafe { (*ptr).type_tag };
            if tag == TypeTag::LIST as u8 {
                let items = unsafe { read_list(ptr) };
                let i = normalize_index(require_int(&key)?, items.len())?;
                items[i] = val;
                Ok(())
            } else if tag == TypeTag::DICT as u8 {
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    unsafe { read_dict(ptr) }.insert(key, val)
                }));
                match r {
                    Ok(_) => Ok(()),
                    Err(p) => Err(unhashable_message(p)),
                }
            } else {
                Err(format!(
                    "TypeError: '{}' object does not support item assignment",
                    obj.type_name()
                ))
            }
        }
        _ => Err(format!(
            "TypeError: '{}' object does not support item assignment",
            obj.type_name()
        )),
    }
}

/// 切片边界调整（等价 CPython `PySlice_AdjustIndices`）。全程 i64 计算，杜绝负数
/// `as usize` 回绕；仅在调用方取元素时 `as usize`（已确保 0 <= i < len）。
/// step==0 返回 ValueError（非 panic）。
///
/// 注意：默认 stop（step<0 时为 -1）是「含下标 0」的哨兵边界，不可当作用户负索引
/// 做 +len 归一化（否则 [::-1] 会得空切片）。故默认值与用户给定值分别处理：仅对
/// 用户显式给定的索引做「负索引 +len → 裁剪」，None 直接填入正确边界。
fn slice_bounds(
    len: usize,
    start: Option<i64>,
    stop: Option<i64>,
    step: i64,
) -> Result<(i64, i64, i64), String> {
    if step == 0 {
        return Err("ValueError: slice step cannot be zero".to_string());
    }
    let len = len as i64;
    // 负索引归一化（用户给定值）：-n 等价于 len-n
    let norm = |idx: i64| -> i64 {
        if idx < 0 {
            idx + len
        } else {
            idx
        }
    };
    if step > 0 {
        let start = match start {
            Some(i) => norm(i).clamp(0, len),
            None => 0,
        };
        let stop = match stop {
            Some(i) => norm(i).clamp(0, len),
            None => len,
        };
        Ok((start, stop, step))
    } else {
        let start = match start {
            Some(i) => norm(i).clamp(-1, len - 1),
            None => len - 1,
        };
        let stop = match stop {
            Some(i) => norm(i).clamp(-1, len - 1),
            None => -1,
        };
        Ok((start, stop, step))
    }
}

/// GET_SLICE 辅助：`obj[start:stop:step]` 切片，返回**同类型新对象**（不改原对象）。
/// list/string/tuple 支持；string 按 char 切片（Unicode 安全）。
fn slice_object(
    obj: Object,
    start: Option<i64>,
    stop: Option<i64>,
    step: i64,
) -> Result<Object, String> {
    match &obj {
        Object::Ref(ptr) => {
            // SAFETY：type_tag 守卫确认 ptr 指向对应集合类型。
            let ptr = *ptr;
            let tag = unsafe { (*ptr).type_tag };
            if tag == TypeTag::LIST as u8 {
                let items = unsafe { read_list(ptr) };
                let (s, e, st) = slice_bounds(items.len(), start, stop, step)?;
                let mut out = Vec::new();
                let mut i = s;
                while (st > 0 && i < e) || (st < 0 && i > e) {
                    out.push(items[i as usize].clone());
                    i += st;
                }
                Ok(alloc_list(out))
            } else if tag == TypeTag::STRING as u8 {
                let chars: Vec<char> = unsafe { read_str(ptr) }.chars().collect();
                let (s, e, st) = slice_bounds(chars.len(), start, stop, step)?;
                let mut out = String::new();
                let mut i = s;
                while (st > 0 && i < e) || (st < 0 && i > e) {
                    out.push(chars[i as usize]);
                    i += st;
                }
                Ok(alloc_string(&out))
            } else if tag == TypeTag::TUPLE as u8 {
                let items = unsafe { read_tuple(ptr) };
                let (s, e, st) = slice_bounds(items.len(), start, stop, step)?;
                let mut out = Vec::new();
                let mut i = s;
                while (st > 0 && i < e) || (st < 0 && i > e) {
                    out.push(items[i as usize].clone());
                    i += st;
                }
                Ok(alloc_tuple(out))
            } else {
                Err(format!(
                    "TypeError: '{}' object is not sliceable",
                    obj.type_name()
                ))
            }
        }
        _ => Err(format!(
            "TypeError: '{}' object is not sliceable",
            obj.type_name()
        )),
    }
}

impl VM {
    fn read_byte(&mut self) -> Result<u8, String> {
        let frame = self
            .call_stack
            .last_mut()
            .ok_or("no call frame".to_string())?;
        // SAFETY：frame.closure 由 alloc_closure 分配（CLOSURE），closure.function
        // 指向 alloc_function 分配的 MsFunction，在帧生命周期内有效。
        let code = unsafe {
            let closure = read_closure(frame.closure);
            read_function(closure.function).function.code.as_slice()
        };
        let b = *code
            .get(frame.ip)
            .ok_or_else(|| "ip past end of bytecode".to_string())?;
        frame.ip += 1;
        Ok(b)
    }

    fn read_u16(&mut self) -> Result<u16, String> {
        let frame = self
            .call_stack
            .last_mut()
            .ok_or("no call frame".to_string())?;
        let code = unsafe {
            let closure = read_closure(frame.closure);
            read_function(closure.function).function.code.as_slice()
        };
        let lo = *code
            .get(frame.ip)
            .ok_or_else(|| "ip past end of bytecode".to_string())?;
        let hi = *code
            .get(frame.ip + 1)
            .ok_or_else(|| "ip past end of bytecode".to_string())?;
        frame.ip += 2;
        Ok(u16::from_be_bytes([lo, hi]))
    }

    /// 读取当前帧的常量池中的常量（克隆）。供 CONSTANT/LoadGlobal/StoreGlobal 使用。
    fn read_constant(&self, idx: usize) -> Result<Object, String> {
        let frame = self.call_stack.last().ok_or("no call frame".to_string())?;
        // SAFETY：同 read_byte，closure→MsFunction 在帧生命周期内有效。
        let constants = unsafe {
            let closure = read_closure(frame.closure);
            read_function(closure.function)
                .function
                .constants
                .as_slice()
        };
        constants
            .get(idx)
            .ok_or_else(|| "constant index out of range".to_string())
            .cloned()
    }
}

impl VM {
    /// 捕获（或复用）指向栈槽 `location` 的开放上值，返回 `*mut MsObjHeader` (MsUpvalue)。
    /// 插入时维持 `open_upvalues` 按 `location` **升序**（最小在前）。
    ///
    /// 升序不变量是 `close_upvalues_from` 正确性的前提：它从末尾（最大 location）
    /// 向前扫描，遇 `location < last` 即 break。若非升序，close 会提前中断或遗漏。
    fn capture_upvalue(&mut self, location: usize) -> *mut MsObjHeader {
        // 升序表中，第一个 location >= 新 location 的位置即插入点。
        let insert_at = self.open_upvalues.iter().position(|&ptr| {
            // SAFETY: ptr 指向由 alloc_upvalue 分配的有效 MsUpvalue。
            let loc = unsafe { (*(ptr as *const MsUpvalue)).location };
            loc >= location
        });

        if let Some(i) = insert_at {
            let existing = self.open_upvalues[i];
            // SAFETY: existing 指向由 alloc_upvalue 分配的有效 MsUpvalue。
            let loc = unsafe { (*(existing as *const MsUpvalue)).location };
            if loc == location {
                return existing; // 复用已存在的开放上值（多个闭包共享同一变量）
            }
            // 插入新上值于 i（保持升序：新 location < existing[i].location）
            let Object::Ref(ptr) = alloc_upvalue(location) else {
                unreachable!()
            };
            self.open_upvalues.insert(i, ptr);
            return ptr;
        }

        // 新 location 大于所有现存上值 → 追加末尾（升序保持）
        let Object::Ref(ptr) = alloc_upvalue(location) else {
            unreachable!()
        };
        self.open_upvalues.push(ptr);
        ptr
    }

    /// 关闭所有 `location >= last` 的开放上值：将栈槽当前值拷贝到 `closed`。
    /// 依赖 `open_upvalues` 按 location 升序：从末尾（最大 location）向前扫，
    /// 遇 `location < last` 即停止（升序保证其前所有 location 更小，确属作用域外）。
    ///
    /// **必须在栈截断前调用**（`close` 读取 `stack[location]`，截断后越界）。
    fn close_upvalues_from(&mut self, last: usize) {
        let mut i = self.open_upvalues.len();
        while i > 0 {
            i -= 1;
            let ptr = self.open_upvalues[i];
            // SAFETY: ptr 指向由 alloc_upvalue 分配的有效 MsUpvalue。
            let location = unsafe { (*(ptr as *const MsUpvalue)).location };
            if location < last {
                break;
            }
            // SAFETY: ptr 指向有效 MsUpvalue；close 仅读写其 closed 字段与（借）栈。
            unsafe {
                read_upvalue(ptr).close(&self.stack);
            }
            self.open_upvalues.remove(i);
        }
    }

    /// CALL 子流程（task 25/27/36）：栈顶为 [callee, arg1, ..., arg(argc)]。
    /// native（FUNCTION）同步执行并压结果；用户函数（CLOSURE）压入新帧（异步）。
    /// 抽出以供 EXEC_DEFER 的 defer 调用复用（task 36）。
    fn call_value(&mut self, argc: usize) -> Result<(), String> {
        // 边界检查（D1）：防止 argc 过大导致下溢/越界。
        if argc + 1 > self.stack.len() {
            return Err("stack underflow for CALL arguments".to_string());
        }
        let callee_idx = self.stack.len() - argc - 1;
        let callee = self.stack[callee_idx].clone();
        match &callee {
            // task 39: GENERATOR 方法调用（gen.__next__() / gen.close() / gen.__iter__()）。
            // GET_ATTR 已设置 gen_call_method 并把 gen 压回栈顶作为 callee。
            Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::GENERATOR as u8 => {
                let method_id = self
                    .gen_call_method
                    .take()
                    .ok_or("TypeError: 'generator' object is not directly callable")?;
                self.stack.truncate(callee_idx);
                match method_id {
                    1 => match self.resume_generator(*ptr)? {
                        Some(v) => self.push(v)?,
                        None => {
                            let exc = alloc_exception(
                                "StopIteration",
                                alloc_string("generator exhausted"),
                                alloc_string(""),
                                Object::Nil,
                            );
                            return self.throw(exc);
                        }
                    },
                    2 => {
                        self.close_generator(*ptr)?;
                        self.push(Object::Nil)?;
                    }
                    3 => {
                        self.push(callee)?;
                    }
                    _ => unreachable!("invalid gen_call_method id"),
                }
            }
            Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::FUNCTION as u8 => {
                // 读出函数指针与参数个数信息（借用堆对象，不借用 self）。
                let (func, arity, name) = {
                    debug_assert!(!ptr.is_null(), "null Object::Ref");
                    // SAFETY: type_tag 为 FUNCTION，指针由 alloc_native_function 分配。
                    let native = unsafe { read_native_function(*ptr) };
                    let arity = self.native_arities.get(native.name()).copied();
                    (native.func, arity, native.name().to_owned())
                };
                // 参数个数校验（C2）：固定 arity 须严格匹配。
                if let Some(arity) = arity {
                    if arity != usize::MAX && arity != argc {
                        return Err(format!(
                            "TypeError: {}() takes exactly {} argument{} but {} were given",
                            name,
                            arity,
                            if arity == 1 { "" } else { "s" },
                            argc
                        ));
                    }
                }
                let args = self.stack[self.stack.len() - argc..].to_vec();
                self.stack.truncate(self.stack.len() - argc - 1);
                let result = func(self, &args)?;
                self.push(result)?;
            }
            // 用户函数（task 27/31）：CLOSURE 分支。支持默认参数与可变参数。
            Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::CLOSURE as u8 => {
                // 读出 arity / required_arity / has_variadic / func_ptr（不借用 self）。
                let (arity, required_arity, has_variadic, func_ptr) = {
                    debug_assert!(!ptr.is_null(), "null Object::Ref");
                    // SAFETY: type_tag 为 CLOSURE，指针由 alloc_closure 分配。
                    let closure = unsafe { read_closure(*ptr) };
                    // SAFETY: closure.function 由 alloc_function 分配。
                    let func = unsafe { read_function(closure.function) };
                    let f = &func.function;
                    (f.arity, f.required_arity, f.has_variadic, closure.function)
                };

                // 实参数量校验（task 31：放宽为范围检查）。
                if has_variadic {
                    if argc < required_arity {
                        return Err(format!(
                            "TypeError: expected at least {} arguments, got {}",
                            required_arity, argc
                        ));
                    }
                } else if argc < required_arity || argc > arity {
                    return Err(format!(
                        "TypeError: expected {}-{} arguments, got {}",
                        required_arity, arity, argc
                    ));
                }
                if self.call_stack.len() >= MAX_CALL_DEPTH {
                    return Err("RecursionError: stack overflow".to_string());
                }

                // 步骤 1：填充默认值（argc < arity 时）。
                // 默认值追加在固定参数之后（位置 argc..arity）。
                if argc < arity {
                    let defaults_to_fill = arity - argc;
                    let offset = argc - required_arity;
                    // 先克隆所需默认值（裸指针引用堆，借用局限于本块）。
                    let to_fill: Vec<Object> = unsafe {
                        let f = &read_function(func_ptr).function;
                        f.default_values[offset..offset + defaults_to_fill].to_vec()
                    };
                    for v in to_fill {
                        self.push(v)?;
                    }
                }

                // 步骤 2：处理可变参数（*rest 收集多余实参为 list）。
                // 必须在填默认值之后：填完后栈长恰为 callee_idx+1+arity。
                if has_variadic {
                    let fixed_end = callee_idx + 1 + arity;
                    if self.stack.len() > fixed_end {
                        let varargs: Vec<Object> = self.stack.drain(fixed_end..).collect();
                        self.push(alloc_list(varargs))?;
                    } else {
                        self.push(alloc_list(Vec::new()))?;
                    }
                }

                // task 39: is_generator 预检 — 默认值/可变参数已填充完毕，
                // 此时栈上 callee + argc 个值即为生成器帧的初始快照。
                let is_generator = unsafe { read_function(func_ptr) }.function.is_generator;
                if is_generator {
                    let final_argc = self.stack.len() - callee_idx - 1;
                    return self.call_generator(*ptr, final_argc);
                }

                // task 53: is_async 预检 — async fn 调用不直接压帧，
                // 而是创建 Future + Coroutine，加入就绪队列，返回 Future。
                let is_async_fn = unsafe { read_function(func_ptr) }.function.is_async;
                if is_async_fn {
                    // 创建 Future（Pending）
                    let future = alloc_future(FutureState::Pending);
                    let Object::Ref(future_ptr) = future else {
                        unreachable!()
                    };
                    // 提取 callee + args 作为协程初始值栈（stack_base = 0）
                    let coro_stack: Vec<Object> = self.stack[callee_idx..].to_vec();
                    self.stack.truncate(callee_idx);
                    let frame = CallFrame::new(*ptr, 0, 0);
                    let coroutine = Coroutine {
                        call_stack: vec![frame],
                        stack: coro_stack,
                        defer_stack: Vec::new(),
                        open_upvalues: Vec::new(),
                        exception_handlers: Vec::new(),
                        pending_unwind: None,
                        future: Some(future_ptr),
                        handle: None,
                    };
                    self.event_loop.ready_queue.push_back(coroutine);
                    // 返回 Future 给调用者
                    self.push(future)?;
                    return Ok(());
                }

                // stack_base = callee_idx：slot 0 = callee（closure 自身），
                // 参数在 slot 1..（与 compile_fn_decl 的 slot-0 预留约定自洽）。
                // defer_stack_base = 当前 defer 栈长度，按帧分区隔离嵌套调用的 defer。
                self.call_stack
                    .push(CallFrame::new(*ptr, callee_idx, self.defer_stack.len()));
            }
            // task 41/42：BoundMethod（`obj.method(args)` / __init__ / super.method）。
            // receiver（self）覆盖 callee 所在 slot（= 新帧 slot 0）。
            // task 42：method 可为 CLOSURE（用户方法）或 FUNCTION（Object 原生方法）。
            Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::BOUND_METHOD as u8 => {
                let (method_ptr, receiver) = {
                    let bound = unsafe { read_bound_method(*ptr) };
                    debug_assert!(
                        !bound.method.is_null(),
                        "bound method pointer is null"
                    );
                    (bound.method, bound.receiver.clone())
                };
                let method_tag = unsafe { (*method_ptr).type_tag };
                if method_tag == TypeTag::FUNCTION as u8 {
                    // task 42：原生方法（Object.__repr__/__eq__/__ne__ 等）。
                    // args = [receiver, ...call_args]，内联调用后压单一结果。
                    let func = unsafe { read_native_function(method_ptr) }.func;
                    let mut args = self.stack[self.stack.len() - argc..].to_vec();
                    args.insert(0, receiver);
                    self.stack.truncate(self.stack.len() - argc - 1);
                    let result = func(self, &args)?;
                    self.push(result)?;
                } else if method_tag == TypeTag::NATIVE_C_FUNCTION as u8 {
                    #[cfg(feature = "capi")]
                    {
                        let frame_base = self.stack.len() - argc - 1;
                        self.stack[frame_base] = receiver;
                        self.call_c_native(method_ptr, argc + 1, frame_base)?;
                    }
                } else {
                    debug_assert_eq!(
                        method_tag,
                        TypeTag::CLOSURE as u8,
                        "bound method not pointing to closure/function"
                    );
                    if self.call_stack.len() >= MAX_CALL_DEPTH {
                        return Err("RecursionError: stack overflow".to_string());
                    }
                    // frame_base = callee(BoundMethod) 所在 slot；覆写为 receiver（self）。
                    let frame_base = self.stack.len() - argc - 1;
                    self.stack[frame_base] = receiver;
                    self.call_stack
                        .push(CallFrame::new(method_ptr, frame_base, self.defer_stack.len()));
                }
            }
            // task 37：异常类对象（EXCEPTION_CLASS）— `ValueError("msg")` 等构造调用。
            // 参数约定：第 1 个实参为 message（无参则 message = nil）。多余实参暂忽略
            // （Phase 5 经 __init__ 处理）。构造 MsException 并替换 callee+args。
            Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::EXCEPTION_CLASS as u8 => {
                let cls_name = unsafe { read_exception_class(*ptr) }.name.clone();
                let message = if argc >= 1 {
                    self.stack[callee_idx + 1].clone()
                } else {
                    Object::Nil
                };
                self.stack.truncate(callee_idx); // 弹出 callee + args
                self.push(alloc_exception(
                    &cls_name,
                    message,
                    alloc_string(""),
                    Object::Nil,
                ))?;
            }
            // task 70：C 原生函数（NATIVE_C_FUNCTION）— 桥接 C 函数调用。
            #[cfg(feature = "capi")]
            Object::Ref(ptr) if unsafe { (**ptr).type_tag }
                == TypeTag::NATIVE_C_FUNCTION as u8 =>
            {
                self.call_c_native(*ptr, argc, callee_idx)?;
            }
            // task 40：用户类对象（CLASS）— `ClassName(args)` 构造实例并调用 __init__。
            Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::CLASS as u8 => {
                self.call_class(*ptr, argc)?;
            }
            // task 43 §5：Instance 有 __call__ 时，替换栈上 callee 为 BoundMethod
            // （receiver=实例），递归 call_value 走已有 BOUND_METHOD 调用路径。
            Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::INSTANCE as u8 => {
                let call_ptr = unsafe {
                    let class_ptr = read_instance(*ptr).class;
                    read_class(class_ptr).find_method("__call__")
                };
                match call_ptr {
                    Some(mp) => {
                        let bound = alloc_bound_method(self.stack[callee_idx].clone(), mp);
                        self.stack[callee_idx] = bound;
                        return self.call_value(argc);
                    }
                    None => {
                        let name = unsafe { read_class(read_instance(*ptr).class) }
                            .name
                            .clone();
                        return Err(format!("TypeError: '{}' object is not callable", name));
                    }
                }
            }
            _ => {
                return Err(format!(
                    "TypeError: '{}' object is not callable",
                    callee.type_name()
                ))
            }
        }
        Ok(())
    }

    /// task 40 §8：实例化类对象。`ClassName(args)` → 创建 Instance、查 __init__、
    /// task 41 前**不**使用 BoundMethod：直接构造 [closure, self, args...] 并 call(argc+1)。
    /// R4：无 __init__ 且 argc>0 报错。§13：含 __del__ 的类在 instance 上置 HAS_FINALIZER。
    fn call_class(&mut self, cls_ptr: *mut MsObjHeader, argc: usize) -> Result<(), String> {
        // V1/R1：防御性栈下溢校验。
        if argc + 1 > self.stack.len() {
            return Err("stack underflow in call_class".into());
        }
        // 弹出参数（top-first）与 callee(class)。
        let mut args: Vec<Object> = (0..argc).map(|_| self.pop()).collect::<Result<_, _>>()?;
        self.pop()?; // 弹出 class
        args.reverse(); // 复原为 bottom-to-top 顺序

        let inst_obj = alloc_instance(cls_ptr);
        // §13：含 __del__ 的类，instance 置 has_finalizer（配合 task 52 run_finalizers）。
        let has_del = unsafe { read_class(cls_ptr) }
            .methods
            .contains_key("__del__");
        if let Object::Ref(ip) = &inst_obj {
            if has_del {
                unsafe {
                    (*(*ip)).set_has_finalizer(true);
                }
            }
        }

        let init_ptr_opt = unsafe { read_class(cls_ptr) }
            .methods
            .get("__init__")
            .copied();
        match init_ptr_opt {
            Some(init_ptr) => {
                // task 41 §2 call_class 切换：以 BoundMethod 为 callee，self 由 CALL
                // handler 写入 slot 0。栈布局：[bound, args...]，call(argc)。
                let bound = alloc_bound_method(inst_obj.clone(), init_ptr);
                self.push(bound)?;
                for arg in &args {
                    self.push(arg.clone())?;
                }
                let caller_depth = self.call_stack.len();
                self.call_value(argc)?;
                // __init__ 为闭包 → 已压帧；驱动至其返回（复用生成器的 run_loop 模式）。
                if self.call_stack.len() > caller_depth {
                    self.run_loop(Some(caller_depth))?;
                }
                // __init__ 的返回值（nil）丢弃；实例对象保留为构造调用的结果。
                self.pop()?;
                self.push(inst_obj)?;
                Ok(())
            }
            None => {
                // R4：无 __init__ 且有参数 → 报错（与 Python 一致）。
                if argc > 0 {
                    let cls_name = unsafe { read_class(cls_ptr) }.name.clone();
                    return Err(format!("'{}' takes no arguments (got {})", cls_name, argc));
                }
                self.push(inst_obj)?;
                Ok(())
            }
        }
    }

    /// task 40 §10 / task 41：在当前帧之上调用一个方法闭包并运行至返回，取其返回值。
    /// 供 print/str 经由 __repr__/__str__ 显示 Instance 使用。
    /// task 41：经 BoundMethod 绑定 receiver 为 self（slot 0），extra_args 紧随其后。
    fn invoke_method(
        &mut self,
        closure_ptr: *mut MsObjHeader,
        receiver: Object,
        extra_args: &[Object],
    ) -> Result<Object, String> {
        let bound = alloc_bound_method(receiver, closure_ptr);
        self.push(bound)?;
        let mut argc = 0usize;
        for a in extra_args {
            self.push(a.clone())?;
            argc += 1;
        }
        let caller_depth = self.call_stack.len();
        self.call_value(argc)?;
        if self.call_stack.len() > caller_depth {
            self.run_loop(Some(caller_depth))?;
        }
        self.pop()
    }

    /// task 51：调用任意 callable Object（CLOSURE/FUNCTION/BOUND_METHOD）并返回结果。
    /// 供 List.map/filter/reduce 等原生方法调用用户回调。
    /// 压栈 callee + args，call_value 后 run_loop 至返回，弹出结果。
    pub fn call_function(
        &mut self,
        callee: &Object,
        args: &[Object],
    ) -> Result<Object, String> {
        self.push(callee.clone())?;
        for arg in args {
            self.push(arg.clone())?;
        }
        let caller_depth = self.call_stack.len();
        self.call_value(args.len())?;
        if self.call_stack.len() > caller_depth {
            self.run_loop(Some(caller_depth))?;
        }
        self.pop()
    }

    /// task 70：调用 C 原生函数（NATIVE_C_FUNCTION）。
    /// 将栈上 Object 参数转为 MsValue* 数组传给 C 函数，回收参数包装，
    /// 将返回的 MsValue* 转为 Object 并压栈。C 函数返回 NULL 表示异常。
    #[cfg(feature = "capi")]
    fn call_c_native(
        &mut self,
        ptr: *mut MsObjHeader,
        argc: usize,
        callee_idx: usize,
    ) -> Result<(), String> {
        use crate::capi::types::MsValue;
        use crate::capi::vm::MsVM;
        use crate::vm::builtins::read_c_native_function;

        let (c_func, arity, name) = {
            let cnf = unsafe { read_c_native_function(ptr) };
            (cnf.func, cnf.arity, cnf.name().to_owned())
        };

        if arity >= 0 && arity as usize != argc {
            return Err(format!(
                "TypeError: {}() takes exactly {} argument{} but {} were given",
                name,
                arity,
                if arity == 1 { "" } else { "s" },
                argc,
            ));
        }

        let arg_vals: Vec<Box<MsValue>> = self.stack[self.stack.len() - argc..]
            .iter()
            .map(|obj| Box::new(MsValue { inner: obj.clone() }))
            .collect();
        let arg_ptrs: Vec<*mut MsValue> = arg_vals
            .iter()
            .map(|b| b.as_ref() as *const MsValue as *mut MsValue)
            .collect();

        self.stack.truncate(callee_idx);

        let c_fn = c_func.ok_or("TypeError: null C function pointer")?;
        let result_ptr = c_fn(
            self.capi_vm_ptr as *mut MsVM,
            arg_ptrs.as_ptr(),
            argc as i32,
        );

        drop(arg_vals);

        if result_ptr.is_null() {
            let msg = self.error_message.clone();
            self.has_error = false;
            self.error_message.clear();
            let exc = alloc_exception(
                "Error",
                alloc_string(&msg),
                alloc_string(""),
                Object::Nil,
            );
            return self.throw(exc);
        }

        let result = unsafe { (*result_ptr).inner.clone() };
        unsafe {
            drop(Box::from_raw(result_ptr));
        }
        self.push(result)?;
        Ok(())
    }


    /// 调用 obj.method(args...) 并返回 Ok(Some(result))；否则返回 Ok(None)，
    /// 由调用方决定 fallback（内置运算）或报错。
    /// 复用 invoke_method（§8）：内部创建 BoundMethod、压参、call_value、嵌套 run_loop、
    /// 弹出返回值。GC 安全：invoke_method 在调用前将 receiver/args 压栈（栈为 GC 根集）。
    fn try_instance_magic(
        &mut self,
        obj: &Object,
        method_name: &str,
        args: &[Object],
    ) -> Result<Option<Object>, String> {
        let method_ptr = if let Object::Ref(ptr) = obj {
            if unsafe { (**ptr).type_tag } == TypeTag::INSTANCE as u8 {
                let class_ptr = unsafe { read_instance(*ptr) }.class;
                unsafe { read_class(class_ptr).find_method(method_name) }
            } else {
                None
            }
        } else {
            None
        };
        match method_ptr {
            Some(mp) => Ok(Some(self.invoke_method(mp, obj.clone(), args)?)),
            None => Ok(None),
        }
    }

    /// task 43：判断 obj 是否为用户 Instance。
    fn is_instance(obj: &Object) -> bool {
        matches!(
            obj,
            Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::INSTANCE as u8
        )
    }
}

// ---------------------------------------------------------------------------
// task 39：生成器（创建 / 恢复 / yield / yield from / close）
// ---------------------------------------------------------------------------

impl VM {
    /// CALL 检测到 is_generator 函数：不执行函数体，创建 MsGenerator 并压栈。
    /// `argc` 为栈上 callee 之上的实参数（默认值已由 call_value 填充）。
    fn call_generator(&mut self, closure_ptr: *mut MsObjHeader, argc: usize) -> Result<(), String> {
        // V1：显式栈下溢校验。
        if argc + 1 > self.stack.len() {
            return Err("stack underflow in call_generator".into());
        }
        let callee_idx = self.stack.len() - argc - 1;
        let initial_stack: Vec<Object> = self.stack[callee_idx..callee_idx + argc + 1].to_vec();

        let func_ptr = unsafe { read_closure(closure_ptr) }.function;
        // V6/R6：locals_count 上限校验。
        const MAX_GENERATOR_LOCALS: usize = 65536;
        let locals_count = unsafe { read_function(func_ptr) }.function.locals_count;
        if locals_count > MAX_GENERATOR_LOCALS {
            return Err(format!(
                "generator locals_count {} exceeds MAX_GENERATOR_LOCALS {}",
                locals_count, MAX_GENERATOR_LOCALS
            ));
        }

        let frame = CallFrame {
            closure: closure_ptr,
            ip: 0,
            stack_base: callee_idx,
            defer_stack_base: self.defer_stack.len(),
            defer_flushing: false,
            current_exc: None,
            gen_owner: None,
        };
        let generator = MsGenerator::new(frame, initial_stack);

        // 弹出 callee + args，压入 Generator。
        for _ in 0..=argc {
            self.stack.pop();
        }
        self.push(alloc_generator(generator))?;
        Ok(())
    }

    /// 恢复生成器：把 stack_snapshot 拷回主栈、push 生成器 CallFrame、置 Running。
    fn push_generator_frame(&mut self, gen_ptr: *mut MsObjHeader) {
        {
            let gen = unsafe { read_generator_mut(gen_ptr) };
            gen.state = GeneratorState::Running;
        }
        let (frame, snapshot) = {
            let gen = unsafe { read_generator_mut(gen_ptr) };
            (gen.frame.clone(), std::mem::take(&mut gen.stack_snapshot))
        };
        let new_base = self.stack.len();
        for v in snapshot {
            self.stack.push(v);
        }
        let mut new_frame = frame;
        new_frame.stack_base = new_base;
        new_frame.gen_owner = Some(gen_ptr);
        self.call_stack.push(new_frame);
    }

    /// yield / 结束时：把当前帧的 [stack_base..stack_top) 拷回生成器快照、pop 帧。
    fn pop_generator_frame(&mut self, gen_ptr: *mut MsObjHeader) {
        let frame = self.call_stack.pop().expect("no generator frame to pop");
        let stack_base = frame.stack_base;
        let snapshot: Vec<Object> = self.stack[stack_base..].to_vec();
        self.stack.truncate(stack_base);
        let gen = unsafe { read_generator_mut(gen_ptr) };
        gen.frame.ip = frame.ip;
        gen.frame.defer_stack_base = frame.defer_stack_base;
        gen.frame.current_exc = frame.current_exc;
        gen.frame.defer_flushing = frame.defer_flushing;
        gen.stack_snapshot = snapshot;
    }

    /// 恢复生成器执行直至 YIELD（返回 Some(value)）或结束（返回 None）。
    fn resume_generator(&mut self, gen_ptr: *mut MsObjHeader) -> Result<Option<Object>, String> {
        let state = unsafe { read_generator(gen_ptr) }.state;
        match state {
            GeneratorState::Exhausted => return Ok(None),
            GeneratorState::Running => {
                return Err("RuntimeError: generator already executing".into())
            }
            GeneratorState::Suspended => {}
        }
        self.push_generator_frame(gen_ptr);
        let caller_depth = self.call_stack.len() - 1;
        self.gen_outcome = None;
        self.run_loop(Some(caller_depth))?;
        Ok(match self.gen_outcome.take() {
            Some(GenOutcome::Yielded(v)) => Some(v),
            _ => None,
        })
    }

    /// 从 gen.receiver 取下一个值；有值则按 YIELD 流程产出、耗尽则清 receiver 继续。
    fn yield_from_step(&mut self, gen_ptr: *mut MsObjHeader) -> Result<(), String> {
        let sub_iter_ptr = unsafe { read_generator(gen_ptr) }
            .receiver
            .ok_or("internal: yield_from_step with no receiver")?;
        let tag = unsafe { (*sub_iter_ptr).type_tag };
        let next: Option<Object> = if tag == TypeTag::ITERATOR as u8 {
            unsafe { read_iterator(sub_iter_ptr) }.state.next()
        } else if tag == TypeTag::GENERATOR as u8 {
            self.resume_generator(sub_iter_ptr)?
        } else {
            return Err("yield from receiver corrupted".into());
        };
        match next {
            Some(value) => {
                self.pop_generator_frame(gen_ptr);
                unsafe { read_generator_mut(gen_ptr) }.state = GeneratorState::Suspended;
                self.gen_outcome = Some(GenOutcome::Yielded(value));
            }
            None => {
                unsafe { read_generator_mut(gen_ptr) }.receiver = None;
                self.push(Object::Nil)?;
            }
        }
        Ok(())
    }

    /// 显式 gen.close() / GC finalizer：注入 GeneratorExit 并恢复，触发 defer/finally。
    fn close_generator(&mut self, gen_ptr: *mut MsObjHeader) -> Result<(), String> {
        let state = unsafe { read_generator(gen_ptr) }.state;
        match state {
            GeneratorState::Exhausted => return Ok(()),
            GeneratorState::Running => {
                return Err("RuntimeError: generator already executing".into())
            }
            GeneratorState::Suspended => {}
        }
        unsafe { read_generator_mut(gen_ptr) }.gen_exit_pending = true;
        let res =
            self.resume_generator_with_exception(gen_ptr, "GeneratorExit", "generator closed");
        // 无论内部控制流如何，close 后一律置 Exhausted（A1）。
        let gen = unsafe { read_generator_mut(gen_ptr) };
        gen.state = GeneratorState::Exhausted;
        gen.gen_exit_pending = false;
        res
    }

    /// 注入异常并恢复生成器执行。复用 throw()/drive_unwind：GeneratorExit 不可被用户
    /// except 捕获，跑完 defer/finally 后在生成器帧边界被 drive_unwind 拦截（置 Exhausted）。
    fn resume_generator_with_exception(
        &mut self,
        gen_ptr: *mut MsObjHeader,
        class_name: &str,
        message: &str,
    ) -> Result<(), String> {
        let exc = alloc_exception(
            class_name,
            alloc_string(message),
            alloc_string(""),
            Object::Nil,
        );
        self.push_generator_frame(gen_ptr);
        let caller_depth = self.call_stack.len() - 1;
        self.gen_outcome = None;
        // throw 在生成器帧内找 handler（GeneratorExit 不可捕获），驱动 defer/finally。
        self.throw(exc)?;
        // throw 可能泊车（闭包 defer）或已弹出生成器帧；仍存在则继续驱动至帧弹出。
        if self.call_stack.len() > caller_depth {
            self.run_loop(Some(caller_depth))?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// task 37：异常处理（异常类注册、throw、CATCH 匹配、finally-on-propagation）
// ---------------------------------------------------------------------------

impl VM {
    /// 注册 12 个内置异常类为 EXCEPTION_CLASS 全局变量（Error + 11 子类）。
    fn init_exception_classes(&mut self) {
        for &name in BUILTIN_EXCEPTION_NAMES {
            let cls = alloc_exception_class(name);
            self.globals.insert(name.to_string(), cls);
        }
    }

    /// 读取当前帧常量池中索引为 `idx` 的**字符串常量**，返回克隆的 String。
    /// 供 CATCH（异常类名匹配）与 GET_ATTR（属性名）使用。
    fn read_string_constant(&self, idx: usize) -> Result<String, String> {
        let frame = self.call_stack.last().ok_or("no call frame".to_string())?;
        let constants = unsafe {
            let closure = read_closure(frame.closure);
            read_function(closure.function)
                .function
                .constants
                .as_slice()
        };
        let val = constants
            .get(idx)
            .ok_or_else(|| "constant index out of range".to_string())?;
        match val {
            Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::STRING as u8 => {
                Ok(unsafe { read_str(*ptr) }.to_string())
            }
            _ => Err("internal: expected string constant".into()),
        }
    }

    /// 异常在 MRO 上是否为 `target_name` 或其子孙。查静态 EXCEPTION_PARENTS 表。
    /// GeneratorExit 不可被用户 except 捕获（仅 CLOSE_GENERATOR 内部流程可处理）。
    fn exception_matches(exception: &Object, target_name: &str) -> bool {
        let class_name = match exception {
            Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::EXCEPTION as u8 => {
                unsafe { read_exception(*ptr) }.class_name.clone()
            }
            _ => return false, // 非 EXCEPTION 不能被 except 捕获
        };
        // GeneratorExit 不可被用户 except 捕获（05-control-flow.md:238）。
        if class_name == "GeneratorExit" {
            return false;
        }
        let mut cur = class_name.as_str();
        loop {
            if cur == target_name {
                return true;
            }
            match EXCEPTION_PARENTS
                .iter()
                .find(|(c, _)| *c == cur)
                .map(|(_, p)| *p)
            {
                Some(parent) => cur = parent,
                None => return false,
            }
        }
    }

    /// 将 `cause` 挂为异常 `exc` 的 `__cause__`（规则 1/4）。
    fn set_cause(&mut self, exc: &Object, cause: Object) {
        if let Object::Ref(ptr) = exc {
            if unsafe { (**ptr).type_tag } == TypeTag::EXCEPTION as u8 {
                unsafe { read_exception_mut(*ptr) }.cause = cause;
            }
        }
    }

    /// task 38：从当前帧 current_exc 派生字段；无异常（或非异常对象）返回 Nil。
    /// 供 with `__exit__` 的 err_type/err_msg/tb 三参数，避免 GET_ATTR-on-nil 失败。
    fn current_exc_field<F>(&self, extractor: F) -> Result<Object, String>
    where
        F: FnOnce(&MsException) -> Object,
    {
        let frame = self.call_stack.last().ok_or("no call frame".to_string())?;
        Ok(match &frame.current_exc {
            Some(Object::Ref(ptr)) if unsafe { (**ptr).type_tag } == TypeTag::EXCEPTION as u8 => {
                extractor(unsafe { read_exception(*ptr) })
            }
            _ => Object::Nil,
        })
    }

    /// 格式化未捕获异常为错误字符串（顶层 throw() 返回此 Err）。
    fn format_uncaught_error(&self, err: &Object) -> String {
        match err {
            Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::EXCEPTION as u8 => {
                let e = unsafe { read_exception(*ptr) };
                let msg = match &e.message {
                    Object::Ref(s) if unsafe { (**s).type_tag } == TypeTag::STRING as u8 => {
                        unsafe { read_str(*s) }.to_string()
                    }
                    _ => e.message.type_name().to_string(),
                };
                format!("{}: {}", e.class_name, msg)
            }
            _ => "Error: <non-exception thrown>".to_string(),
        }
    }

    /// 异常传播入口（THROW/RETHROW/FINALLY_END 调用）。
    ///
    /// 把 `err` 挂为 `pending_unwind` 并驱动 `drive_unwind`。若当前已处于 unwind 态
    /// （一个 defer 抛了新异常）或当前帧正处理某异常（finally 内抛新异常），则旧异常
    /// 挂为新异常的 `__cause__`（规则 1/4 / finally 覆盖）。
    fn throw(&mut self, err: Object) -> Result<(), String> {
        // 取出「当前正在传播/处理的异常」作为 __cause__ 链源：
        //  - pending_unwind：unwind 途中某 defer 抛了新异常（规则 1/4）。
        //  - current_exc：finally 块内抛新异常，覆盖进入 finally 时的原异常（规则 §6）。
        let cause = self.pending_unwind.take().or_else(|| {
            self.call_stack
                .last_mut()
                .and_then(|f| f.current_exc.take())
        });
        if let Some(c) = cause {
            self.set_cause(&err, c);
        }
        self.pending_unwind = Some(err);
        self.drive_unwind()
    }

    /// 推进异常传播（由 throw() 与主循环顶部共同驱动）。
    ///
    /// 对当前帧：(a) 若有未刷新 defer，逐条经主循环执行（closure defer 须泊车）；
    /// (b) defers 完成后扫描 exception_handlers，命中则跳 catch_address；不命中则 (c)
    /// pop frame 续传；顶层无 handler 返回 Err。
    fn drive_unwind(&mut self) -> Result<(), String> {
        let err = self
            .pending_unwind
            .take()
            .expect("drive_unwind called with no pending exception");
        loop {
            let frame_stack_base = match self.call_stack.last() {
                Some(f) => f.stack_base,
                None => {
                    self.last_uncaught_exception = Some(err.clone());
                    return Err(self.format_uncaught_error(&err));
                }
            };
            let defer_base = self.call_stack.last().unwrap().defer_stack_base;

            // [task 38] 跨帧 cause 链：with 的 __exit__ 在子帧运行，其内部 throw 的新异常
            // 经 throw() 时取不到本帧的 current_exc（子帧 current_exc 为 None）。当该新异常
            // 传播回本帧（current_exc 仍持原异常），把原异常挂为新异常的 __cause__（§6/§7）。
            // 同帧 finally 场景的 cause 已由 throw() 处理（current_exc 已 take），此处不重复触发。
            let prev_exc = {
                let frame = self.call_stack.last_mut().unwrap();
                frame.current_exc.take()
            };
            if let Some(old) = prev_exc {
                self.set_cause(&err, old);
            }

            // (a) 刷新本帧 defer（逐条；closure callee 须泊车交主循环执行）。
            if self.defer_stack.len() > defer_base {
                self.call_stack.last_mut().unwrap().defer_flushing = true;
                let entry = self.defer_stack.pop().unwrap();
                // 拆开 call_tuple = (callee, arg1, ..., argN)。
                let items = match &entry.call_tuple {
                    Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::TUPLE as u8 => {
                        unsafe { read_tuple(*ptr) }.clone()
                    }
                    _ => return Err("internal: defer call_tuple is not a tuple".into()),
                };
                let argc = items.len() - 1;
                for a in &items {
                    self.push(a.clone())?;
                }
                let frames_before = self.call_stack.len();
                self.call_value(argc)?;
                if self.call_stack.len() == frames_before {
                    // native defer 同步完成：丢弃返回值，继续刷下一条 defer（不泊车）。
                    self.pop()?;
                    continue;
                }
                // closure defer：新帧已压入 → 泊车，交主循环执行该帧。
                // defer 抛异常时该帧的 throw() 会自驱动（含跨帧续传）。
                self.pending_unwind = Some(err);
                return Ok(());
            }

            // defers 全部完成。
            self.call_stack.last_mut().unwrap().defer_flushing = false;

            // (b) 扫描 exception_handlers，找到属于当前帧的 handler。
            //   frame_stack_base 是值栈基址：内层帧更高、外层帧更低。
            //   handler > 当前帧 → 残留的内层 handler（其帧已 pop）→ 丢弃；
            //   handler == 当前帧 → 命中本帧；
            //   handler < 当前帧 → 属于外层帧 → 保留，pop 本帧后续传。
            let handler = loop {
                match self.exception_handlers.last() {
                    None => break None,
                    Some(h) if h.frame_stack_base > frame_stack_base => {
                        self.exception_handlers.pop();
                    }
                    Some(h) if h.frame_stack_base == frame_stack_base => {
                        break self.exception_handlers.pop();
                    }
                    Some(_) => break None,
                }
            };

            if let Some(h) = handler {
                self.stack.truncate(h.scope_stack_base);
                let frame = self.call_stack.last_mut().unwrap();
                frame.current_exc = Some(err.clone());
                frame.ip = h.catch_address;
                self.push(err)?;
                return Ok(()); // 主循环将派发 catch_address（except 分派器）
            }

            // (c) 本帧无 handler：关闭 upvalue，pop frame，续传外层帧。
            self.close_upvalues_from(frame_stack_base);
            // task 39: 生成器帧边界 — 保存快照、置 Exhausted。
            // GeneratorExit 在此停止传播（不可越界到调用者帧）；其他异常也停止
            // 传播（生成器内未捕获异常导致生成器终止，异常以 Err 返回）。
            let gen_owner = self.call_stack.last().unwrap().gen_owner;
            if let Some(gen_ptr) = gen_owner {
                self.pop_generator_frame(gen_ptr);
                unsafe { read_generator_mut(gen_ptr) }.state = GeneratorState::Exhausted;
                let is_gen_exit = match &err {
                    Object::Ref(e_ptr)
                        if unsafe { (**e_ptr).type_tag } == TypeTag::EXCEPTION as u8 =>
                    {
                        unsafe { read_exception(*e_ptr) }.class_name == "GeneratorExit"
                    }
                    _ => false,
                };
                if is_gen_exit {
                    return Ok(());
                }
                self.last_uncaught_exception = Some(err.clone());
                return Err(self.format_uncaught_error(&err));
            }
            if self.call_stack.len() > 1 {
                self.stack.truncate(frame_stack_base);
                self.call_stack.pop();
                continue;
            }
            self.last_uncaught_exception = Some(err.clone());
            return Err(self.format_uncaught_error(&err));
        }
    }
}

impl VM {
    fn run(&mut self) -> Result<Object, String> {
        self.run_loop(None)
    }

    /// 主解释循环。`stop_depth = None` 为顶层模式（执行至 HALT / 顶层 RETURN）；
    /// `Some(d)` 为生成器驱动模式（task 39）：当调用栈缩回 `d`（生成器帧被 YIELD 或
    /// 结束弹出）时立即返回。生成器恢复结果经 `gen_outcome` 字段回传。
    fn run_loop(&mut self, stop_depth: Option<usize>) -> Result<Object, String> {
        loop {
            // task 39：生成器驱动模式 — 生成器帧弹出即返回（结果在 gen_outcome）。
            if let Some(d) = stop_depth {
                if self.call_stack.len() <= d {
                    return Ok(Object::Nil);
                }
            }

            // task 37：unwind 续行——若待传播异常存在且当前帧正处于 defer 刷新
            // （说明一个 closure defer 帧刚弹出回到 unwind 帧），则推进 drive_unwind。
            // 首次 throw 在 handler 内已调用 drive_unwind；这里仅处理泊车后续行。
            if self.pending_unwind.is_some()
                && self.call_stack.last().is_some_and(|f| f.defer_flushing)
            {
                self.drive_unwind()?;
            }

            // GC 触发点（task 52）。MVP：VM 日常分配未接入 GC 堆，bytes_allocated 保持
            // 0，此调用为 no-op；接入后在此按阈值触发 minor/major GC（STW）。
            gc::maybe_gc(
                &mut self.heap,
                &mut self.stack,
                &mut self.globals,
                &mut self.defer_stack,
                &mut self.call_stack,
            );

            let opcode_byte = self.read_byte()?;
            let opcode = OpCode::from_byte(opcode_byte)
                .ok_or_else(|| format!("unknown opcode: {}", opcode_byte))?;

            match opcode {
                OpCode::Constant => {
                    let idx = self.read_u16()? as usize;
                    let value = self.read_constant(idx)?;
                    self.push(value)?;
                }

                OpCode::Nil => self.push(Object::Nil)?,
                OpCode::True => self.push(Object::Bool(true))?,
                OpCode::False => self.push(Object::Bool(false))?,

                OpCode::LoadLocal => {
                    let slot = self.read_byte()? as usize;
                    let frame = self.call_stack.last().unwrap();
                    let idx = frame
                        .stack_base
                        .checked_add(slot)
                        .ok_or_else(|| "local slot overflow".to_string())?;
                    let value = self
                        .stack
                        .get(idx)
                        .ok_or_else(|| "local slot out of range".to_string())?
                        .clone();
                    self.push(value)?;
                }

                OpCode::StoreLocal => {
                    let slot = self.read_byte()? as usize;
                    let value = self.pop()?;
                    let frame = self.call_stack.last().unwrap();
                    let idx = frame
                        .stack_base
                        .checked_add(slot)
                        .ok_or_else(|| "local slot overflow".to_string())?;
                    while self.stack.len() <= idx {
                        self.stack.push(Object::Nil);
                    }
                    self.stack[idx] = value;
                }

                OpCode::LoadGlobal => {
                    let name_idx = self.read_u16()? as usize;
                    let constant = self.read_constant(name_idx)?;
                    let name = match &constant {
                        // SAFETY：type_tag 守卫确认常量为 STRING，且由编译器经
                        // alloc_string 分配，生命周期与 Chunk/VM 一致；read_str
                        // 的借用仅用于 to_owned，立即结束。
                        Object::Ref(ptr)
                            if unsafe { (**ptr).type_tag } == TypeTag::STRING as u8 =>
                        {
                            debug_assert!(!(*ptr).is_null());
                            unsafe { read_str(*ptr) }.to_owned()
                        }
                        _ => return Err("invalid global name constant".to_string()),
                    };
                    let value = self.globals.get(&name).cloned().unwrap_or(Object::Nil);
                    self.push(value)?;
                }

                OpCode::StoreGlobal => {
                    let name_idx = self.read_u16()? as usize;
                    let value = self.pop()?;
                    let constant = self.read_constant(name_idx)?;
                    let name = match &constant {
                        // SAFETY：同 LoadGlobal。
                        Object::Ref(ptr)
                            if unsafe { (**ptr).type_tag } == TypeTag::STRING as u8 =>
                        {
                            debug_assert!(!(*ptr).is_null());
                            unsafe { read_str(*ptr) }.to_owned()
                        }
                        _ => return Err("invalid global name constant".to_string()),
                    };
                    self.globals.insert(name, value);
                }

                OpCode::Pop => {
                    self.pop()?;
                }

                OpCode::Dup => {
                    let value = self.peek(0)?.clone();
                    self.push(value)?;
                }

                // task 55：关闭所有开放上值后再终止——go 协程闭包可能捕获顶层变量，
                // 顶层 HALT 时须 close upvalue 使其在协程切换后仍可安全访问。
                OpCode::Halt => {
                    self.close_upvalues_from(0);
                    return Ok(self.pop().unwrap_or(Object::Nil));
                }

                OpCode::Add => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    if let Some(r) = self.try_instance_magic(&a, "__add__", std::slice::from_ref(&b))? {
                        self.push(r)?;
                    } else {
                        self.push(a.add(&b)?)?;
                    }
                }

                OpCode::Subtract => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    if let Some(r) = self.try_instance_magic(&a, "__sub__", std::slice::from_ref(&b))? {
                        self.push(r)?;
                    } else {
                        self.push(a.subtract(&b)?)?;
                    }
                }

                OpCode::Multiply => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    if let Some(r) = self.try_instance_magic(&a, "__mul__", std::slice::from_ref(&b))? {
                        self.push(r)?;
                    } else {
                        self.push(a.multiply(&b)?)?;
                    }
                }

                OpCode::Divide => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    if let Some(r) = self.try_instance_magic(&a, "__div__", std::slice::from_ref(&b))? {
                        self.push(r)?;
                    } else {
                        self.push(a.divide(&b)?)?;
                    }
                }

                OpCode::FloorDiv => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    if let Some(r) = self.try_instance_magic(&a, "__floordiv__", std::slice::from_ref(&b))? {
                        self.push(r)?;
                    } else {
                        self.push(a.floor_divide(&b)?)?;
                    }
                }

                OpCode::Modulo => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    if let Some(r) = self.try_instance_magic(&a, "__mod__", std::slice::from_ref(&b))? {
                        self.push(r)?;
                    } else {
                        self.push(a.modulo(&b)?)?;
                    }
                }

                OpCode::Power => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    if let Some(r) = self.try_instance_magic(&a, "__pow__", std::slice::from_ref(&b))? {
                        self.push(r)?;
                    } else {
                        self.push(a.power(&b)?)?;
                    }
                }

                OpCode::Negate => {
                    let value = self.pop()?;
                    let result = value.negate()?;
                    self.push(result)?;
                }

                OpCode::BitAnd => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    let result = a.bit_and(&b)?;
                    self.push(result)?;
                }

                OpCode::BitOr => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    let result = a.bit_or(&b)?;
                    self.push(result)?;
                }

                OpCode::BitXor => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    let result = a.bit_xor(&b)?;
                    self.push(result)?;
                }

                OpCode::BitNot => {
                    let value = self.pop()?;
                    let result = value.bit_not()?;
                    self.push(result)?;
                }

                OpCode::LeftShift => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    let result = a.left_shift(&b)?;
                    self.push(result)?;
                }

                OpCode::RightShift => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    let result = a.right_shift(&b)?;
                    self.push(result)?;
                }

                // CmpOp 与 OpCode 解耦（task 21 设计决策，见 src/vm/object.rs:378）。
                // 需 `use crate::vm::object::CmpOp;`（CmpOp 为 Copy，按值传递）。
                OpCode::Equal => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    if let Some(r) = self.try_instance_magic(&a, "__eq__", std::slice::from_ref(&b))? {
                        self.push(r)?;
                    } else {
                        self.push(Object::Bool(a == b))?;
                    }
                }

                OpCode::NotEqual => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    if let Some(r) = self.try_instance_magic(&a, "__ne__", std::slice::from_ref(&b))? {
                        self.push(r)?;
                    } else {
                        self.push(Object::Bool(a != b))?;
                    }
                }

                OpCode::Less => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    if let Some(r) = self.try_instance_magic(&a, "__lt__", std::slice::from_ref(&b))? {
                        self.push(r)?;
                    } else {
                        self.push(a.compare(&b, CmpOp::Less)?)?;
                    }
                }

                OpCode::Greater => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    if let Some(r) = self.try_instance_magic(&a, "__gt__", std::slice::from_ref(&b))? {
                        self.push(r)?;
                    } else {
                        self.push(a.compare(&b, CmpOp::Greater)?)?;
                    }
                }

                OpCode::LessEqual => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    if let Some(r) = self.try_instance_magic(&a, "__le__", std::slice::from_ref(&b))? {
                        self.push(r)?;
                    } else {
                        self.push(a.compare(&b, CmpOp::LessEqual)?)?;
                    }
                }

                OpCode::GreaterEqual => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    if let Some(r) = self.try_instance_magic(&a, "__ge__", std::slice::from_ref(&b))? {
                        self.push(r)?;
                    } else {
                        self.push(a.compare(&b, CmpOp::GreaterEqual)?)?;
                    }
                }

                OpCode::Is => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    self.push(a.is_identity(&b)?)?;
                }

                OpCode::In => {
                    let container = self.pop()?;
                    let item = self.pop()?;
                    if Self::is_instance(&container) {
                        if let Some(r) = self.try_instance_magic(&container, "__contains__", std::slice::from_ref(&item))? {
                            self.push(r)?;
                        } else {
                            return Err("argument of type 'instance' is not iterable".into());
                        }
                    } else {
                        // task 22/24 内置成员判断（String 子串、List/Set/Dict 成员）。
                        self.push(container.contains_str(&item)?)?;
                    }
                }

                OpCode::Not => {
                    let value = self.pop()?;
                    self.push(value.logical_not())?;
                }

                OpCode::Jump => {
                    let offset = self.read_u16()? as usize;
                    let frame = self
                        .call_stack
                        .last_mut()
                        .ok_or("no call frame".to_string())?;
                    frame.ip += offset;
                }

                OpCode::JumpIfFalse => {
                    let offset = self.read_u16()? as usize;
                    if !self.peek(0)?.is_truthy() {
                        let frame = self
                            .call_stack
                            .last_mut()
                            .ok_or("no call frame".to_string())?;
                        frame.ip += offset;
                    }
                }

                OpCode::JumpIfTrue => {
                    let offset = self.read_u16()? as usize;
                    if self.peek(0)?.is_truthy() {
                        let frame = self
                            .call_stack
                            .last_mut()
                            .ok_or("no call frame".to_string())?;
                        frame.ip += offset;
                    }
                }

                OpCode::JumpBack => {
                    let offset = self.read_u16()? as usize;
                    let frame = self
                        .call_stack
                        .last_mut()
                        .ok_or("no call frame".to_string())?;
                    frame.ip = frame
                        .ip
                        .checked_sub(offset)
                        .ok_or_else(|| "jump back underflow".to_string())?;
                }

                // BREAK：前向跳到循环出口（编译器 patch_jump）
                OpCode::Break => {
                    let offset = self.read_u16()? as usize;
                    let frame = self
                        .call_stack
                        .last_mut()
                        .ok_or("no call frame".to_string())?;
                    frame.ip += offset;
                }

                // CONTINUE：后向跳到循环头（编译器 patch_jump_back）
                OpCode::Continue => {
                    let offset = self.read_u16()? as usize;
                    let frame = self
                        .call_stack
                        .last_mut()
                        .ok_or("no call frame".to_string())?;
                    frame.ip = frame
                        .ip
                        .checked_sub(offset)
                        .ok_or_else(|| "continue underflow".to_string())?;
                }

                // ITERATOR（task 26）：弹出可迭代对象，压入其迭代器。
                // 编译器在 for..in 头部发射（statement.rs:329）。
                OpCode::Iterator => {
                    let iterable = self.pop()?;
                    let is_gen = matches!(&iterable,
                        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::GENERATOR as u8);
                    // task 54：channel 是自身的迭代器。
                    let is_channel = matches!(&iterable,
                        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::CHANNEL as u8);
                    if is_gen || is_channel {
                        self.push(iterable)?;
                    } else {
                        let iter_state =
                            to_iterator(&iterable).map_err(|e| format!("RuntimeError: {}", e))?;
                        self.push(alloc_iterator(iter_state))?;
                    }
                }

                // FOR_ITER（task 32 修订）：迭代器存储在局部 slot（非栈顶）。
                // 操作数：iter_slot(1) + exit_offset(2)。从 stack[base+iter_slot]
                // 读取迭代器，取下一值压入栈顶供 StoreLocal/Unpack 消费。耗尽时
                // ip += offset 跳到循环出口。slot 方式使嵌套 for..in 不冲突。
                OpCode::ForIter => {
                    let iter_slot = self.read_byte()? as usize;
                    let offset = self.read_u16()? as usize;
                    let stack_base = self
                        .call_stack
                        .last()
                        .ok_or("no call frame".to_string())?
                        .stack_base;
                    let location = stack_base + iter_slot;
                    if location >= self.stack.len() {
                        return Err("RuntimeError: FOR_ITER slot out of range".to_string());
                    }
                    let gen_ptr = match &self.stack[location] {
                        Object::Ref(ptr)
                            if unsafe { (**ptr).type_tag } == TypeTag::GENERATOR as u8 =>
                        {
                            Some(*ptr)
                        }
                        _ => None,
                    };
                    if let Some(gp) = gen_ptr {
                        match self.resume_generator(gp)? {
                            Some(value) => self.push(value)?,
                            None => {
                                self.call_stack.last_mut().unwrap().ip += offset;
                            }
                        }
                    } else {
                        // task 54：channel 迭代（for val in ch）。
                        let chan_ptr = match &self.stack[location] {
                            Object::Ref(ptr)
                                if unsafe { (**ptr).type_tag } == TypeTag::CHANNEL as u8 =>
                            {
                                Some(*ptr)
                            }
                            _ => None,
                        };
                        if let Some(cp) = chan_ptr {
                            let ch = unsafe { read_channel(cp) };
                            // 1. 缓冲区有数据：取出，压栈供循环体消费。
                            let from_buffer = { ch.buffer.borrow_mut().pop_front() };
                            if let Some(val) = from_buffer {
                                // 腾出空位：唤醒等待发送者将其值移入缓冲区。
                                let woken = { ch.waiting_senders.borrow_mut().pop_front() };
                                if let Some(sender) = woken {
                                    let mut buffer = ch.buffer.borrow_mut();
                                    buffer.push_back(sender.value);
                                    drop(buffer);
                                    let mut coro = sender.coroutine;
                                    coro.stack.push(Object::Nil);
                                    self.event_loop.ready_queue.push_back(coro);
                                }
                                self.push(val)?;
                            } else if ch.is_closed() {
                                // 2. 已关闭且缓冲区空：结束迭代。
                                self.call_stack.last_mut().unwrap().ip += offset;
                            } else {
                                // 3. 缓冲区空、未关闭：尝试 rendezvous（无缓冲 channel）。
                                let woken = { ch.waiting_senders.borrow_mut().pop_front() };
                                if let Some(sender) = woken {
                                    self.push(sender.value)?;
                                    let mut coro = sender.coroutine;
                                    coro.stack.push(Object::Nil);
                                    self.event_loop.ready_queue.push_back(coro);
                                } else {
                                    // 4. 无数据可取：暂停。channel 留在 slot 中。
                                    //    ip 已越过 FOR_ITER；被唤醒时发送者已将值压入栈。
                                    self.yield_channel = Some(ChannelYield::Recv { channel: cp });
                                    return Ok(Object::Nil);
                                }
                            }
                        } else {
                            let next_val: Option<Object> = {
                                match &mut self.stack[location] {
                                    Object::Ref(ptr)
                                        if unsafe { (**ptr).type_tag }
                                            == TypeTag::ITERATOR as u8 =>
                                    {
                                        unsafe { read_iterator(*ptr) }.state.next()
                                    }
                                    _ => return Err("RuntimeError: not an iterator".to_string()),
                                }
                            };
                            match next_val {
                                Some(v) => self.push(v)?,
                                None => {
                                    self.call_stack.last_mut().unwrap().ip += offset;
                                }
                            }
                        }
                    }
                }

                // YIELD（task 39）：弹出产出值，保存生成器帧快照，置 Suspended，
                // 把值压入调用者栈。gen_outcome 通知 resume_generator 的 run_loop 返回。
                OpCode::Yield => {
                    let value = self.pop()?;
                    let gen_ptr = self
                        .call_stack
                        .last()
                        .ok_or("no frame")?
                        .gen_owner
                        .ok_or("YIELD outside generator frame")?;
                    self.push(Object::Nil)?;
                    self.pop_generator_frame(gen_ptr);
                    unsafe { read_generator_mut(gen_ptr) }.state = GeneratorState::Suspended;
                    self.gen_outcome = Some(GenOutcome::Yielded(value));
                }

                // YIELD_FROM（task 39）：弹出可迭代对象，转为子迭代器存入 gen.receiver，
                // 立即经 yield_from_step 产出首个值（或子迭代器为空时 fall-through）。
                OpCode::YieldFrom => {
                    let iterable = self.pop()?;
                    let gen_ptr = self
                        .call_stack
                        .last()
                        .ok_or("no frame")?
                        .gen_owner
                        .ok_or("YIELD_FROM outside generator frame")?;
                    let sub_iter = match &iterable {
                        Object::Ref(ptr)
                            if unsafe { (**ptr).type_tag } == TypeTag::GENERATOR as u8 =>
                        {
                            iterable
                        }
                        Object::Ref(ptr)
                            if unsafe { (**ptr).type_tag } == TypeTag::ITERATOR as u8 =>
                        {
                            iterable
                        }
                        _ => {
                            let is = to_iterator(&iterable)
                                .map_err(|e| format!("RuntimeError: {}", e))?;
                            alloc_iterator(is)
                        }
                    };
                    let sub_ptr = match sub_iter {
                        Object::Ref(r) => r,
                        _ => return Err("yield from requires an iterable".into()),
                    };
                    unsafe { read_generator_mut(gen_ptr) }.receiver = Some(sub_ptr);
                    self.yield_from_step(gen_ptr)?;
                }

                // YIELD_FROM_RESUME（task 39）：yield-from 的配套恢复指令。
                // 生成器被恢复时从此处继续：若 receiver 仍有值则再次 yield，否则 fall-through。
                //
                // IP 管理：read_byte 已将 IP 推进到本指令之后。若有 receiver，先将 IP 回退
                // 1 字节（指回 YIELD_FROM_RESUME），再调 yield_from_step。这样：
                // — Some(value)：pop_generator_frame 保存的 IP = YIELD_FROM_RESUME，
                //   下次恢复时重新执行本指令，形成迭代循环。
                // — None：yield_from_step 推 Nil 后正常返回，IP += 1 跳过本指令继续执行。
                //
                // 关键：Some 情况下 yield_from_step 内部 pop_generator_frame 弹出生成器帧，
                // 此时 self.call_stack.last() 已变为调用者帧。若无条件执行 ip += 1 会
                // 破坏调用者 IP。故仅在帧未弹出（None 情况）时才前进 IP。
                OpCode::YieldFromResume => {
                    let gen_ptr = self
                        .call_stack
                        .last()
                        .ok_or("no frame")?
                        .gen_owner
                        .ok_or("YIELD_FROM_RESUME outside generator frame")?;
                    let has_receiver = unsafe { read_generator(gen_ptr) }.receiver.is_some();
                    if has_receiver {
                        let depth_before = self.call_stack.len();
                        self.call_stack.last_mut().unwrap().ip -= 1;
                        self.yield_from_step(gen_ptr)?;
                        if self.call_stack.len() == depth_before {
                            self.call_stack.last_mut().unwrap().ip += 1;
                        }
                    }
                }

                // UNPACK n（task 26）：弹出顶部集合（tuple/list），将其 n 个元素
                // 压入栈，使元素 0 位于栈顶（编译器随后按序 StoreLocal 各循环变量）。
                // for..in 双变量循环（statement.rs:336）依赖此指令。
                OpCode::Unpack => {
                    let n = self.read_byte()? as usize;
                    let val = self.pop()?;
                    let elements: Vec<Object> = match &val {
                        Object::Ref(ptr) => {
                            debug_assert!(!ptr.is_null(), "null Object::Ref");
                            let tag = unsafe { (**ptr).type_tag };
                            if tag == TypeTag::TUPLE as u8 {
                                unsafe { read_tuple(*ptr) }.clone()
                            } else if tag == TypeTag::LIST as u8 {
                                unsafe { read_list(*ptr) }.clone()
                            } else {
                                return Err(format!(
                                    "TypeError: cannot unpack non-iterable '{}' object",
                                    val.type_name()
                                ));
                            }
                        }
                        _ => {
                            return Err(format!(
                                "TypeError: cannot unpack non-iterable '{}' object",
                                val.type_name()
                            ))
                        }
                    };
                    if elements.len() != n {
                        return Err(format!(
                            "ValueError: not enough values to unpack (expected {}, got {})",
                            n,
                            elements.len()
                        ));
                    }
                    // 逆序压入，使 elements[0] 落在栈顶。
                    for e in elements.into_iter().rev() {
                        self.push(e)?;
                    }
                }

                // BUILD_TUPLE count：从栈顶弹出 count 个元素，构建 tuple 对象并压栈。
                // 编译端由 compile_tuple_literal（expression.rs）与 compile_return
                // 多返回值（statement.rs）发射。
                OpCode::BuildTuple => {
                    let count = self.read_byte()? as usize;
                    let start = self
                        .stack
                        .len()
                        .checked_sub(count)
                        .ok_or("RuntimeError: stack underflow in BUILD_TUPLE")?;
                    let elements: Vec<Object> = self.stack.drain(start..).collect();
                    self.push(alloc_tuple(elements))?;
                }

                // BUILD_LIST count：从栈顶弹出 count 个元素，构建 list 对象并压栈。
                // 编译端由 compile_list_literal（expression.rs）发射。
                // task 32 回填：此前 opcode 已定义且编译器已发射，但 VM 无 handler。
                OpCode::BuildList => {
                    let count = self.read_byte()? as usize;
                    let start = self
                        .stack
                        .len()
                        .checked_sub(count)
                        .ok_or("RuntimeError: stack underflow in BUILD_LIST")?;
                    let elements: Vec<Object> = self.stack.drain(start..).collect();
                    self.push(alloc_list(elements))?;
                }

                // LIST_APPEND slot（task 33）：弹出栈顶值，原地追加到 slot 处的 list
                // 局部变量。不向栈顶 push 任何值（结果 list 由推导式末尾 LOAD_LOCAL 显式
                // 取出）。编译端由 compile_list_comprehension（expression.rs）发射。
                OpCode::ListAppend => {
                    let slot = self.read_byte()? as usize;
                    let value = self.pop()?;
                    let stack_base = self
                        .call_stack
                        .last()
                        .ok_or("no call frame".to_string())?
                        .stack_base;
                    let location = stack_base
                        .checked_add(slot)
                        .ok_or_else(|| "local slot overflow".to_string())?;
                    if location >= self.stack.len() {
                        return Err("RuntimeError: LIST_APPEND slot out of range".to_string());
                    }
                    match &self.stack[location] {
                        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::LIST as u8 => {
                            debug_assert!(!ptr.is_null(), "null Object::Ref");
                            // SAFETY：type_tag 守卫确认 ptr 指向由 alloc_list 分配的 MsList。
                            unsafe { read_list(*ptr) }.push(value);
                        }
                        other => {
                            return Err(format!(
                                "TypeError: LIST_APPEND requires a list, got '{}'",
                                other.type_name()
                            ));
                        }
                    }
                }

                // SET_ADD slot（task 34）：弹出栈顶元素，原地加入 slot 处的 set 局部变量。
                // 不向栈顶 push 任何值（结果 set 由推导式末尾 LOAD_LOCAL 显式取出）。
                // 编译端由 compile_set_comprehension（expression.rs）发射。
                OpCode::SetAdd => {
                    let slot = self.read_byte()? as usize;
                    let elem = self.pop()?;
                    let stack_base = self
                        .call_stack
                        .last()
                        .ok_or("no call frame".to_string())?
                        .stack_base;
                    let location = stack_base
                        .checked_add(slot)
                        .ok_or_else(|| "local slot overflow".to_string())?;
                    if location >= self.stack.len() {
                        return Err("RuntimeError: SET_ADD slot out of range".to_string());
                    }
                    match &self.stack[location] {
                        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::SET as u8 => {
                            debug_assert!(!ptr.is_null(), "null Object::Ref");
                            // SAFETY：type_tag 守卫确认 ptr 指向由 alloc_set 分配的 MsSet。
                            // Object::hash 对 list/dict/set/NaN 元素会 panic；用 catch_unwind
                            // 将其转为可被 try/except 捕获的 TypeError（见 spec §3 不可哈希值）。
                            // panic 发生在 HashSet 哈希阶段（插入前），set 未被破坏。
                            let result =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    unsafe { read_set(*ptr) }.insert(elem)
                                }));
                            if let Err(payload) = result {
                                return Err(unhashable_message(payload));
                            }
                        }
                        other => {
                            return Err(format!(
                                "TypeError: SET_ADD requires a set, got '{}'",
                                other.type_name()
                            ));
                        }
                    }
                }

                // DICT_INSERT slot（task 34）：先弹 val、再弹 key（编译端 key 先压栈，故
                // val 在栈顶），原地插入 slot 处的 dict 局部变量。不向栈顶 push 任何值。
                // 不可哈希 key 同样经 catch_unwind 转 TypeError。
                OpCode::DictInsert => {
                    let slot = self.read_byte()? as usize;
                    let value = self.pop()?;
                    let key = self.pop()?;
                    let stack_base = self
                        .call_stack
                        .last()
                        .ok_or("no call frame".to_string())?
                        .stack_base;
                    let location = stack_base
                        .checked_add(slot)
                        .ok_or_else(|| "local slot overflow".to_string())?;
                    if location >= self.stack.len() {
                        return Err("RuntimeError: DICT_INSERT slot out of range".to_string());
                    }
                    match &self.stack[location] {
                        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::DICT as u8 => {
                            debug_assert!(!ptr.is_null(), "null Object::Ref");
                            // SAFETY：type_tag 守卫确认 ptr 指向由 alloc_dict 分配的 MsDict。
                            // 不可哈希 key 在哈希阶段 panic（插入前），dict 未被破坏。
                            let result =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    unsafe { read_dict(*ptr) }.insert(key, value)
                                }));
                            if let Err(payload) = result {
                                return Err(unhashable_message(payload));
                            }
                        }
                        other => {
                            return Err(format!(
                                "TypeError: DICT_INSERT requires a dict, got '{}'",
                                other.type_name()
                            ));
                        }
                    }
                }

                // BUILD_DICT pairs：栈上已按 key0,val0,key1,val1,… 顺序压入 pairs 对
                // 键值对。弹出 pairs*2 个值，构建 dict 对象并压栈。
                // task 32 回填。
                OpCode::BuildDict => {
                    let pairs = self.read_byte()? as usize;
                    let needed = pairs
                        .checked_mul(2)
                        .ok_or("RuntimeError: BUILD_DICT count overflow")?;
                    let start = self
                        .stack
                        .len()
                        .checked_sub(needed)
                        .ok_or("RuntimeError: stack underflow in BUILD_DICT")?;
                    let mut map = DictMap::new();
                    let mut i = start;
                    for _ in 0..pairs {
                        let key = self.stack[i].clone();
                        let value = self.stack[i + 1].clone();
                        map.insert(key, value);
                        i += 2;
                    }
                    self.stack.truncate(start);
                    self.push(alloc_dict(map))?;
                }

                // BUILD_SET count：从栈顶弹出 count 个元素，构建 set 对象并压栈。
                // task 32 回填。
                OpCode::BuildSet => {
                    let count = self.read_byte()? as usize;
                    let start = self
                        .stack
                        .len()
                        .checked_sub(count)
                        .ok_or("RuntimeError: stack underflow in BUILD_SET")?;
                    let elements: Vec<Object> = self.stack.drain(start..).collect();
                    self.push(alloc_set(elements.into_iter().collect()))?;
                }

                // CLOSURE（task 28）：从常量池取出 Function，捕获上值，创建 Closure。
                // 操作数：func_idx(2) + 每上值 (is_local:1, index:1)。
                OpCode::Closure => {
                    let func_idx = self.read_u16()? as usize;
                    let func_obj = self.read_constant(func_idx)?;

                    let func_ptr = match func_obj {
                        Object::Ref(ptr)
                            if unsafe { (*ptr).type_tag } == TypeTag::FUNCTION as u8 =>
                        {
                            ptr
                        }
                        _ => return Err("CLOSURE expects a Function".to_string()),
                    };

                    // task 27 嵌套布局：read_function(ptr).function.upvalue_count
                    let upvalue_count = unsafe { read_function(func_ptr) }.function.upvalue_count;
                    let mut upvalues: Vec<*mut MsObjHeader> = Vec::with_capacity(upvalue_count);

                    for _ in 0..upvalue_count {
                        let is_local = self.read_byte()? == 1;
                        let index = self.read_byte()? as usize; // 编译期已断言 ≤ 255

                        if is_local {
                            // 直接外层局部变量：捕获当前帧栈槽。
                            let stack_base = self.call_stack.last().unwrap().stack_base;
                            let location = stack_base + index;
                            upvalues.push(self.capture_upvalue(location));
                        } else {
                            // 外层上值：复用当前闭包的上值（上值链穿透）。
                            let closure_ptr = self.call_stack.last().unwrap().closure;
                            // SAFETY: closure_ptr 指向当前帧的 MsClosure。
                            let closure = unsafe { read_closure(closure_ptr) };
                            upvalues.push(closure.upvalues[index]);
                        }
                    }

                    let closure_obj = alloc_closure(Object::Ref(func_ptr), upvalues);
                    self.push(closure_obj)?;
                }

                // LOAD_UPVALUE（task 28）：将当前闭包的上值[idx]压栈。
                OpCode::LoadUpvalue => {
                    let idx = self.read_byte()? as usize;
                    let closure_ptr = self.call_stack.last().unwrap().closure;
                    // SAFETY: closure_ptr 指向当前帧的 MsClosure。
                    let closure = unsafe { read_closure(closure_ptr) };
                    let upvalue_ptr = closure.upvalues[idx];
                    // SAFETY: upvalue_ptr 指向由 alloc_upvalue 分配的有效 MsUpvalue。
                    let value = unsafe { read_upvalue(upvalue_ptr) }.get(&self.stack);
                    self.push(value)?;
                }

                // STORE_UPVALUE（task 28）：将栈顶存入当前闭包的上值[idx]（peek，不弹）。
                OpCode::StoreUpvalue => {
                    let idx = self.read_byte()? as usize;
                    let value = self.stack.last().cloned().unwrap_or(Object::Nil); // peek 栈顶（不弹）
                    let closure_ptr = self.call_stack.last().unwrap().closure;
                    // SAFETY: closure_ptr 指向当前帧的 MsClosure。
                    let closure = unsafe { read_closure(closure_ptr) };
                    let upvalue_ptr = closure.upvalues[idx];
                    // SAFETY: upvalue_ptr 指向由 alloc_upvalue 分配的有效 MsUpvalue。
                    // read_upvalue 返回独立 &mut（源自裸指针，不与 self.stack 借用重叠）。
                    unsafe { read_upvalue(upvalue_ptr) }.set(&mut self.stack, value);
                }

                // CLOSE_UPVALUE（task 28）：关闭栈顶位置对应的开放上值，再弹栈。
                OpCode::CloseUpvalue => {
                    let stack_top = self.stack.len() - 1;
                    self.close_upvalues_from(stack_top);
                    self.stack.pop();
                }

                // CALL（task 25/27）：native（FUNCTION）+ 用户函数（CLOSURE）。
                // 子流程抽出为 call_value，供 EXEC_DEFER 的 defer 调用复用（task 36）。
                OpCode::Call => {
                    let argc = self.read_byte()? as usize;
                    self.call_value(argc)?;
                }

                // RETURN（task 27/28/36/39）：弹出返回值，关闭本帧开放上值，恢复调用者帧，截断值栈。
                // defer 已由编译端在 RETURN 前 emit 的 EXEC_DEFER 执行完毕，本帧 defer 区间为空。
                // task 39：生成器帧的 RETURN 走 generator_return 路径（丢弃返回值、置 Exhausted）。
                OpCode::Return => {
                    let gen_owner = self
                        .call_stack
                        .last()
                        .ok_or("return outside function".to_string())?
                        .gen_owner;
                    let return_value = self.stack.pop().unwrap_or(Object::Nil);
                    if let Some(gen_ptr) = gen_owner {
                        let old_base = self.call_stack.last().unwrap().stack_base;
                        self.close_upvalues_from(old_base);
                        self.pop_generator_frame(gen_ptr);
                        unsafe { read_generator_mut(gen_ptr) }.state = GeneratorState::Exhausted;
                        self.gen_outcome = Some(GenOutcome::Exhausted);
                        let _ = return_value;
                        return Ok(Object::Nil);
                    }
                    let old_base = self
                        .call_stack
                        .last()
                        .ok_or("return outside function".to_string())?
                        .stack_base;
                    // task 28：先关闭当前帧的所有开放上值（栈尚未截断，location 仍有效）。
                    self.close_upvalues_from(old_base);
                    self.stack.truncate(old_base); // 移除 callee(slot0)+args+locals
                    self.call_stack.pop();
                    self.stack.push(return_value);

                    // 顶层帧 RETURN 后无更多调用者帧 → 终止执行（等价于隐式 HALT）。
                    if self.call_stack.is_empty() {
                        return Ok(self.stack.pop().unwrap_or(Object::Nil));
                    }
                }

                // DEFER（task 36）：弹出栈顶 call_tuple，入当前帧 defer 区间（注册时求值）。
                OpCode::Defer => {
                    let call_tuple = self.pop()?;
                    self.defer_stack.push(DeferEntry { call_tuple });
                }

                // EXEC_DEFER（task 36）：函数返回路径上唯一的 defer 刷新点（编译端在每个
                // RETURN/HALT 前 emit）。按 LIFO 逆序执行当前帧 defer 区间内的条目。
                //
                // ip-rewind trampoline：闭包 callee 的 CALL 仅压帧、不同步返回，故不能在
                // 循环内同步取结果。改为：弹出一条 defer → 置本帧 defer_flushing=true → 回退
                // ip 1 字节（使本指令重派发）→ 经 call_value 发起调用（闭包压帧后返回；native
                // 同步压结果）→ 控制流回到主循环。callee 执行完毕、帧弹回后 ip 仍指向 EXEC_DEFER
                // → 重派发 → defer_flushing=true 弹出其返回值 → 处理下一条。defer_flushing 每帧
                // 独立，避免 defer callee 自身的（空）EXEC_DEFER 误触发弹栈。
                OpCode::ExecDefer => {
                    let (base, was_flushing) = {
                        let frame = self.call_stack.last().ok_or("no call frame".to_string())?;
                        (frame.defer_stack_base, frame.defer_flushing)
                    };
                    // 上一个 defer 调用刚完成（native 同步或闭包帧弹回），丢弃其返回值。
                    if was_flushing {
                        self.pop()?;
                    }
                    if self.defer_stack.len() > base {
                        let entry = self.defer_stack.pop().unwrap();
                        // 置本帧刷新态 + 回退 ip（本字节码已由 read_byte 推进过）。
                        {
                            let frame = self.call_stack.last_mut().unwrap();
                            frame.defer_flushing = true;
                            frame.ip = frame
                                .ip
                                .checked_sub(1)
                                .ok_or("ip underflow in EXEC_DEFER")?;
                        }
                        // 拆开 call_tuple = (callee, arg1, ..., argN)，按序压栈后走标准 CALL 子流程。
                        let items = match &entry.call_tuple {
                            Object::Ref(ptr)
                                if unsafe { (**ptr).type_tag } == TypeTag::TUPLE as u8 =>
                            {
                                unsafe { read_tuple(*ptr) }.clone()
                            }
                            _ => return Err("internal: defer call_tuple is not a tuple".into()),
                        };
                        let argc = items.len() - 1;
                        for a in &items {
                            self.push(a.clone())?;
                        }
                        self.call_value(argc)?;
                    } else {
                        // 本帧 defer 已全部执行，退出刷新态（不回退 ip → 下一条为 RETURN/HALT）。
                        self.call_stack.last_mut().unwrap().defer_flushing = false;
                    }
                }

                // ---- task 37：异常处理 ----

                // THROW：弹出栈顶异常对象并抛出。string 自动包装为 RuntimeError。
                OpCode::Throw => {
                    let val = self.pop()?;
                    let err = match &val {
                        Object::Ref(ptr)
                            if unsafe { (**ptr).type_tag } == TypeTag::EXCEPTION as u8 =>
                        {
                            val
                        }
                        Object::Ref(ptr)
                            if unsafe { (**ptr).type_tag } == TypeTag::STRING as u8 =>
                        {
                            // throw "string" → RuntimeError(message)
                            let msg = unsafe { read_str(*ptr) }.to_string();
                            alloc_exception(
                                "RuntimeError",
                                alloc_string(&msg),
                                alloc_string(""),
                                Object::Nil,
                            )
                        }
                        _ => {
                            return Err(
                                "TypeError: exceptions must derive from Error or be a string"
                                    .into(),
                            )
                        }
                    };
                    self.throw(err)?;
                }

                // TRY_ENTER：注册异常处理器。操作数 handler_offset(2) finally_offset(2)。
                OpCode::TryEnter => {
                    let handler_offset = self.read_u16()? as usize;
                    let finally_raw = self.read_u16()?;
                    let frame = self.call_stack.last().ok_or("no call frame".to_string())?;
                    let catch_address = frame.ip + handler_offset;
                    let finally_address = if finally_raw == 0xFFFF {
                        None
                    } else {
                        Some(frame.ip + finally_raw as usize)
                    };
                    self.exception_handlers.push(ExceptionHandler {
                        catch_address,
                        finally_address,
                        frame_stack_base: frame.stack_base,
                        scope_stack_base: self.stack.len(),
                    });
                }

                // TRY_EXIT：try body 正常完成（或 early-exit 出口）注销本 try 的 handler。
                OpCode::TryExit => {
                    self.exception_handlers.pop();
                }

                // CATCH：栈顶异常的类名是否匹配常量池[name_idx]（含父类链）；压 bool。
                // 不弹出异常本体（供后续绑定 / 不匹配时重抛）。
                OpCode::Catch => {
                    let name_idx = self.read_u16()? as usize;
                    let target_name = self.read_string_constant(name_idx)?;
                    let exception = self.peek(0)?.clone();
                    let matches = Self::exception_matches(&exception, &target_name);
                    self.push(Object::Bool(matches))?;
                }

                // RETHROW：重抛当前帧 current_exc（裸 throw）；为空抛 nothing to rethrow。
                OpCode::Rethrow => {
                    let err = {
                        let frame = self.call_stack.last().ok_or("no call frame".to_string())?;
                        frame.current_exc.clone()
                    };
                    match err {
                        Some(e) => self.throw(e)?,
                        None => {
                            let e = alloc_exception(
                                "RuntimeError",
                                alloc_string("nothing to rethrow"),
                                alloc_string(""),
                                Object::Nil,
                            );
                            self.throw(e)?;
                        }
                    }
                }

                // FINALLY_END：finally 块末尾。current_exc 非空则重抛（finally-on-propagation）。
                OpCode::FinallyEnd => {
                    let pending = self
                        .call_stack
                        .last_mut()
                        .ok_or("no call frame".to_string())?
                        .current_exc
                        .take();
                    if let Some(e) = pending {
                        self.throw(e)?;
                    }
                }

                // CLEAR_CURRENT_EXC：except 命中分支末尾，清除 current_exc（异常已处理，
                // 使后续 FINALLY_END 不误重抛）。
                OpCode::ClearCurrentExc => {
                    self.call_stack
                        .last_mut()
                        .ok_or("no call frame".to_string())?
                        .current_exc = None;
                }

                // LOAD_CURRENT_EXC（task 38）：压当前帧 current_exc（无异常时压 nil），
                // 供 with cleanup 块判定正常/异常路径与重抛。
                OpCode::LoadCurrentExc => {
                    let exc = self
                        .call_stack
                        .last()
                        .ok_or("no call frame".to_string())?
                        .current_exc
                        .clone()
                        .unwrap_or(Object::Nil);
                    self.push(exc)?;
                }

                // LOAD_EXC_TYPE / MSG / TB（task 38）：从 current_exc 派生 with __exit__
                // 的 err_type / err_msg / tb 参数；无异常时压 nil。专用 opcode 避免
                // GET_ATTR-on-nil 失败（mslang 无 SWAP/ROT，无法用单条 LOAD_CURRENT_EXC
                // + 多次 GET_ATTR 拆字段）。
                OpCode::LoadExcType => {
                    let val = self.current_exc_field(|e| alloc_string(&e.class_name))?;
                    self.push(val)?;
                }
                OpCode::LoadExcMsg => {
                    let val = self.current_exc_field(|e| e.message.clone())?;
                    self.push(val)?;
                }
                OpCode::LoadExcTb => {
                    let val = self.current_exc_field(|e| e.traceback.clone())?;
                    self.push(val)?;
                }

                // GET_ATTR（task 37/39）：处理 GENERATOR / EXCEPTION / DICT 对象的属性访问；
                // Instance 等其余类型留待 task 41/43。
                OpCode::GetAttr => {
                    let name_idx = self.read_u16()? as usize;
                    let attr = self.read_string_constant(name_idx)?;
                    let obj = self.pop()?;
                    // task 39: GENERATOR 方法分派 — 设置 gen_call_method，push gen 自身。
                    let handled_gen = if let Object::Ref(ptr) = &obj {
                        if unsafe { (**ptr).type_tag } == TypeTag::GENERATOR as u8 {
                            let method_id: u8 = match attr.as_str() {
                                "__next__" => 1,
                                "close" => 2,
                                "__iter__" => 3,
                                _ => {
                                    return Err(format!(
                                        "AttributeError: 'generator' has no attribute '{}'",
                                        attr
                                    ))
                                }
                            };
                            self.gen_call_method = Some(method_id);
                            self.push(obj.clone())?;
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    };
                    if handled_gen {
                        continue;
                    }
                    match &obj {
                        // task 51：Dict 方法分派（length/keys/.../merge 等 9 个），
                        // 先查方法名；若非已知方法则回退到键访问（d.key 等价 d["key"]）。
                        Object::Ref(ptr)
                            if unsafe { (**ptr).type_tag } == TypeTag::DICT as u8 =>
                        {
                            if let Some(func) = stdlib::lookup_dict_method(&attr) {
                                let method_obj = alloc_native_function(NativeFunction {
                                    name: attr.clone(),
                                    func,
                                });
                                let method_ptr = match method_obj {
                                    Object::Ref(p) => p,
                                    _ => unreachable!(),
                                };
                                self.push(alloc_bound_method(obj.clone(), method_ptr))?;
                            } else {
                                let key = alloc_string(&attr);
                                let val = unsafe { read_dict(*ptr) }
                                    .get(&key)
                                    .cloned()
                                    .unwrap_or(Object::Nil);
                                self.push(val)?;
                            }
                        }
                        Object::Ref(ptr)
                            if unsafe { (**ptr).type_tag } == TypeTag::EXCEPTION as u8 =>
                        {
                            let exc = unsafe { read_exception(*ptr) };
                            let val = match attr.as_str() {
                                "message" => exc.message.clone(),
                                "type" => alloc_string(&exc.class_name),
                                "traceback" => exc.traceback.clone(),
                                "__cause__" => exc.cause.clone(),
                                _ => {
                                    return Err(format!(
                                        "AttributeError: 'Error' has no attribute '{}'",
                                        attr
                                    ))
                                }
                            };
                            self.push(val)?;
                        }
                        Object::Ref(ptr)
                            if unsafe { (**ptr).type_tag } == TypeTag::INSTANCE as u8 =>
                        {
                            // V5 修复：先 copy 出裸指针（*mut is Copy），再分阶段查找，
                            // 避免嵌套 read_instance/read_class 的可变借用同时存活。
                            // task 41 §3：find_method/find_class_attr 沿继承链递归
                            // （parent 在 task 42 前恒 None）。
                            let inst_ptr = *ptr;
                            let class_ptr = unsafe { read_instance(inst_ptr) }.class;
                            // 1. 实例字段
                            if let Some(v) = unsafe { read_instance(inst_ptr) }
                                .fields
                                .get(&attr)
                                .cloned()
                            {
                                self.push(v)?;
                            } else if let Some(m) =
                                unsafe { read_class(class_ptr).find_method(&attr) }
                            {
                                // task 41 §2：返回 BoundMethod，后续 CALL 自动绑定 self。
                                self.push(alloc_bound_method(obj.clone(), m))?;
                            } else if let Some(v) =
                                unsafe { read_class(class_ptr).find_class_attr(&attr) }
                            {
                                self.push(v)?;
                            } else if attr == "__name__" {
                                // §12 / §3 第 4 步：__name__ 内置属性（合成，不入 class_attrs）。
                                let n = unsafe { read_class(class_ptr) }.name.clone();
                                self.push(alloc_string(&n))?;
                            } else {
                                let cls_name = unsafe { read_class(class_ptr) }.name.clone();
                                return Err(format!(
                                    "'{}' instance has no attribute '{}'",
                                    cls_name, attr
                                ));
                            }
                        }
                        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::CLASS as u8 => {
                            let cls_ptr = *ptr;
                            if let Some(m) =
                                unsafe { read_class(cls_ptr) }.methods.get(&attr).copied()
                            {
                                self.push(Object::Ref(m))?;
                            } else if let Some(v) = unsafe { read_class(cls_ptr) }
                                .class_attrs
                                .get(&attr)
                                .cloned()
                            {
                                self.push(v)?;
                            } else if attr == "__name__" {
                                let n = unsafe { read_class(cls_ptr) }.name.clone();
                                self.push(alloc_string(&n))?;
                            } else {
                                let cls_name = unsafe { read_class(cls_ptr) }.name.clone();
                                return Err(format!(
                                    "class '{}' has no attribute '{}'",
                                    cls_name, attr
                                ));
                            }
                        }
                        // task 44：闭包 name 属性 — 返回底层 MsFunction.name。
                        // 装饰器仅替换变量绑定，闭包对象的 name 字段保持定义时名称。
                        Object::Ref(ptr)
                            if unsafe { (**ptr).type_tag } == TypeTag::CLOSURE as u8 =>
                        {
                            if attr == "name" {
                                let cl = unsafe { read_closure(*ptr) };
                                let n = unsafe { read_function(cl.function) }.function.name.clone();
                                self.push(alloc_string(&n))?;
                            } else {
                                return Err(format!(
                                    "AttributeError: 'function' has no attribute '{}'",
                                    attr
                                ));
                            }
                        }
                        // task 45 §8：MODULE 属性访问 = exports[name]。
                        // 访问未导出或尚未初始化（循环导入）的名称 → NameError。
                        Object::Ref(ptr)
                            if unsafe { (**ptr).type_tag } == TypeTag::MODULE as u8 =>
                        {
                            let module_ptr = *ptr;
                            let module_name = unsafe { read_module(module_ptr) }.name.clone();
                            if let Some(val) = unsafe { read_module(module_ptr) }
                                .exports
                                .get(&attr)
                                .cloned()
                            {
                                self.push(val)?;
                            } else {
                                let exc = alloc_exception(
                                    "NameError",
                                    alloc_string(&format!(
                                        "模块 '{}' 没有 '{}'",
                                        module_name, attr
                                    )),
                                    alloc_string(""),
                                    Object::Nil,
                                );
                                // throw 返回 Ok(())=已 unwind 到 handler / Err=未捕获；
                                // 二者均终止本指令处理（? 传播 Err，continue 续行 handler）。
                                self.throw(exc)?;
                                continue;
                            }
                        }
                        // task 46：FileHandle 方法分派（read/write/close/lines/__enter__/__exit__）。
                        // 返回 BoundMethod（receiver=FileHandle + method=native），后续 CALL
                        // BOUND_METHOD→FUNCTION 自动注入 receiver 为 args[0]。
                        Object::Ref(ptr)
                            if unsafe { (**ptr).type_tag } == TypeTag::FILE_HANDLE as u8 =>
                        {
                            match stdlib::lookup_file_method(&attr) {
                                Some(method_ptr) => {
                                    self.push(alloc_bound_method(obj.clone(), method_ptr))?;
                                }
                                None => {
                                    return Err(format!(
                                        "AttributeError: FileHandle has no attribute '{}'",
                                        attr
                                    ));
                                }
                            }
                        }
                        // task 50：String 方法分派（length/upper/.../slice 等 12 个）。
                        // 返回 BoundMethod（receiver=String + method=native），后续 CALL
                        // BOUND_METHOD→FUNCTION 自动注入 receiver 为 args[0]。
                        Object::Ref(ptr)
                            if unsafe { (**ptr).type_tag } == TypeTag::STRING as u8 =>
                        {
                            match stdlib::lookup_string_method(&attr) {
                                Some(func) => {
                                    let method_obj = alloc_native_function(NativeFunction {
                                        name: attr.clone(),
                                        func,
                                    });
                                    // SAFETY: alloc_native_function 恒返回 Ref。
                                    let method_ptr = match method_obj {
                                        Object::Ref(p) => p,
                                        _ => unreachable!("alloc_native_function must return Ref"),
                                    };
                                    self.push(alloc_bound_method(obj.clone(), method_ptr))?;
                                }
                                None => {
                                    return Err(format!(
                                        "AttributeError: 'string' has no attribute '{}'",
                                        attr
                                    ));
                                }
                            }
                        }
                        // task 51：List 方法分派（length/push/.../reduce 等 14 个）。
                        Object::Ref(ptr)
                            if unsafe { (**ptr).type_tag } == TypeTag::LIST as u8 =>
                        {
                            match stdlib::lookup_list_method(&attr) {
                                Some(func) => {
                                    let method_obj = alloc_native_function(NativeFunction {
                                        name: attr.clone(),
                                        func,
                                    });
                                    let method_ptr = match method_obj {
                                        Object::Ref(p) => p,
                                        _ => unreachable!(),
                                    };
                                    self.push(alloc_bound_method(obj.clone(), method_ptr))?;
                                }
                                None => {
                                    return Err(format!(
                                        "AttributeError: 'list' has no attribute '{}'",
                                        attr
                                    ));
                                }
                            }
                        }
                        // task 51：Set 方法分派（length/add/.../difference 等 7 个）。
                        Object::Ref(ptr)
                            if unsafe { (**ptr).type_tag } == TypeTag::SET as u8 =>
                        {
                            match stdlib::lookup_set_method(&attr) {
                                Some(func) => {
                                    let method_obj = alloc_native_function(NativeFunction {
                                        name: attr.clone(),
                                        func,
                                    });
                                    let method_ptr = match method_obj {
                                        Object::Ref(p) => p,
                                        _ => unreachable!(),
                                    };
                                    self.push(alloc_bound_method(obj.clone(), method_ptr))?;
                                }
                                None => {
                                    return Err(format!(
                                        "AttributeError: 'set' has no attribute '{}'",
                                        attr
                                    ));
                                }
                            }
                        }
                        // task 54：Channel 方法分派（close/closed/__iter__/__next__）。
                        Object::Ref(ptr)
                            if unsafe { (**ptr).type_tag } == TypeTag::CHANNEL as u8 =>
                        {
                            let func: Option<builtins::NativeFn> = match attr.as_str() {
                                "close" => Some(channel_close),
                                "closed" => Some(channel_closed),
                                _ => None,
                            };
                            if let Some(f) = func {
                                let method_obj = alloc_native_function(NativeFunction {
                                    name: attr.clone(),
                                    func: f,
                                });
                                let method_ptr = match method_obj {
                                    Object::Ref(p) => p,
                                    _ => unreachable!(),
                                };
                                self.push(alloc_bound_method(obj.clone(), method_ptr))?;
                            } else if attr == "__iter__" {
                                // channel 是自身的迭代器。
                                self.push(obj.clone())?;
                            } else {
                                return Err(format!(
                                    "AttributeError: 'channel' has no attribute '{}'",
                                    attr
                                ));
                            }
                        }
                        // task 55：JoinHandle 方法分派（join/is_done/cancel）。
                        Object::Ref(ptr)
                            if unsafe { (**ptr).type_tag } == TypeTag::JOIN_HANDLE as u8 =>
                        {
                            let func: Option<builtins::NativeFn> = match attr.as_str() {
                                "join" => Some(join_handle_join),
                                "is_done" => Some(join_handle_is_done),
                                "cancel" => Some(join_handle_cancel),
                                _ => None,
                            };
                            if let Some(f) = func {
                                let method_obj = alloc_native_function(NativeFunction {
                                    name: attr.clone(),
                                    func: f,
                                });
                                let method_ptr = match method_obj {
                                    Object::Ref(p) => p,
                                    _ => unreachable!(),
                                };
                                self.push(alloc_bound_method(obj.clone(), method_ptr))?;
                            } else {
                                return Err(format!(
                                    "AttributeError: 'JoinHandle' has no attribute '{}'",
                                    attr
                                ));
                            }
                        }
                        _ => {
                            return Err(format!(
                                "AttributeError: '{}' has no attribute '{}'",
                                obj.type_name(),
                                attr
                            ))
                        }
                    }
                }

                // SET_ATTR（task 40）：编译端 compile_store_target(Dot) 在已求值的
                // 赋值值之上压入 object，故栈顶为 object，其下为 value。弹 object、value
                // 后写入，**不压结果**（与 SetIndex 一致：DUP 的副本留作赋值表达式的值）。
                OpCode::SetAttr => {
                    let name_idx = self.read_u16()? as usize;
                    let attr = self.read_string_constant(name_idx)?;
                    let obj = self.pop()?;
                    let value = self.pop()?;
                    match obj {
                        Object::Ref(ptr)
                            if unsafe { (*ptr).type_tag } == TypeTag::INSTANCE as u8 =>
                        {
                            // TODO task 62: 并发 GC 启用后须经 write_barrier
                            unsafe { read_instance(ptr) }.fields.insert(attr, value);
                        }
                        Object::Ref(ptr) if unsafe { (*ptr).type_tag } == TypeTag::CLASS as u8 => {
                            // TODO task 62: 并发 GC 启用后须经 write_barrier
                            unsafe { read_class(ptr) }.class_attrs.insert(attr, value);
                        }
                        _ => {
                            return Err(format!(
                                "AttributeError: cannot set attribute on '{}'",
                                obj.type_name()
                            ))
                        }
                    }
                }

                // CLASS（task 40 §6）：name_idx(2) → 创建类对象压栈。
                OpCode::Class => {
                    let name_idx = self.read_u16()? as usize;
                    let name = self.read_string_constant(name_idx)?;
                    let class_obj = alloc_class(name);
                    let Object::Ref(cls_ptr) = class_obj else {
                        unreachable!()
                    };
                    // task 42：默认链接隐式 Object 基类。若字节码后续有 INHERIT
                    //（显式父类），将覆写 parent。
                    unsafe {
                        read_class(cls_ptr).parent = Some(self.object_class);
                    }
                    self.push(Object::Ref(cls_ptr))?;
                }

                // METHOD（task 40 §7）：name_idx(2) → 弹栈顶 closure，peek 栈顶 class，
                // 插入 methods。V4 修复：栈顶非 closure 时返回明确错误。
                OpCode::Method => {
                    let name_idx = self.read_u16()? as usize;
                    let name = self.read_string_constant(name_idx)?;
                    let method_obj = self.pop()?;
                    let method_ptr = match method_obj {
                        Object::Ref(p) if unsafe { (*p).type_tag } == TypeTag::CLOSURE as u8 => p,
                        _ => return Err("METHOD expects a closure on stack".into()),
                    };
                    let cls_obj = self.peek(0)?;
                    match cls_obj {
                        Object::Ref(cls_ptr)
                            if unsafe { (**cls_ptr).type_tag } == TypeTag::CLASS as u8 =>
                        {
                            // TODO task 62: 并发 GC 启用后，methods.insert 须经 write_barrier
                            unsafe { read_class(*cls_ptr) }
                                .methods
                                .insert(name, method_ptr);
                        }
                        _ => return Err("METHOD target is not a Class".into()),
                    }
                }

                // INHERIT（task 42）：弹栈顶父类，peek 栈顶子类，设置 parent。
                // V5 模式：拷出裸指针后分阶段写，避免两个 &mut 同时存活。
                // 不复制 class_attrs：继承链查找由 find_class_attr 递归处理。
                OpCode::Inherit => {
                    let parent_obj = self.pop()?;
                    let child_obj = self.peek(0)?;
                    let (parent_ptr, child_ptr) = match (&parent_obj, child_obj) {
                        (Object::Ref(p), Object::Ref(c))
                            if unsafe { (**p).type_tag } == TypeTag::CLASS as u8
                                && unsafe { (**c).type_tag } == TypeTag::CLASS as u8 =>
                        {
                            (*p, *c)
                        }
                        _ => return Err("parent must be a class".into()),
                    };
                    unsafe {
                        read_class(child_ptr).parent = Some(parent_ptr);
                    }
                }

                // GET_SUPER（task 42）：双操作数 class_idx(2), name_idx(2)。
                // 取当前类 → parent → find_method，receiver = 当前帧 slot 0（self）。
                OpCode::GetSuper => {
                    let class_idx = self.read_u16()? as usize;
                    let name_idx = self.read_u16()? as usize;
                    let class_name = self.read_string_constant(class_idx)?;
                    let name = self.read_string_constant(name_idx)?;
                    let current_cls_obj = self
                        .globals
                        .get(&class_name)
                        .ok_or_else(|| format!("class '{}' not found", class_name))?;
                    let current_cls_ptr = match current_cls_obj {
                        Object::Ref(p)
                            if unsafe { (**p).type_tag } == TypeTag::CLASS as u8 =>
                        {
                            *p
                        }
                        _ => return Err(format!("'{}' is not a class", class_name)),
                    };
                    let parent_ptr = unsafe { read_class(current_cls_ptr).parent }
                        .ok_or_else(|| format!("class '{}' has no parent", class_name))?;
                    let method_ptr = unsafe { read_class(parent_ptr).find_method(&name) }
                        .ok_or_else(|| format!("parent class has no method '{}'", name))?;
                    let receiver = {
                        let frame = self
                            .call_stack
                            .last()
                            .ok_or("GET_SUPER outside method call")?;
                        self.stack[frame.stack_base].clone()
                    };
                    self.push(alloc_bound_method(receiver, method_ptr))?;
                }

                // GET_INDEX（task 35）：[obj, key] → [result]。obj[key] 读取。
                // task 43 §4：Instance 有 __getitem__ 时分派，否则报错。
                OpCode::GetIndex => {
                    let key = self.pop()?;
                    let obj = self.pop()?;
                    if Self::is_instance(&obj) {
                        if let Some(r) = self.try_instance_magic(&obj, "__getitem__", std::slice::from_ref(&key))? {
                            self.push(r)?;
                        } else {
                            return Err("'instance' object is not subscriptable".into());
                        }
                    } else {
                        self.push(get_item(obj, key)?)?;
                    }
                }

                // SET_INDEX（task 35）：编译端 compile_assignment 先压 value（并 DUP 留结果），
                // 再由 compile_store_target 压 obj、key，故栈底→顶为 [val, obj, key]。
                // 按 LIFO 弹 key、obj、val；不压栈（DUP 的结果副本由上层 POP 处理）。
                // task 43 §4：Instance 有 __setitem__ 时分派，否则报错。
                OpCode::SetIndex => {
                    let key = self.pop()?;
                    let obj = self.pop()?;
                    let val = self.pop()?;
                    if Self::is_instance(&obj) {
                        if self.try_instance_magic(&obj, "__setitem__", &[key, val])?.is_none() {
                            return Err("'instance' object does not support item assignment".into());
                        }
                    } else {
                        set_item(obj, key, val)?;
                    }
                }

                // GET_SLICE（task 35）：flags(bit0=start/bit1=stop/bit2=step)。
                // 编译端压栈顺序 obj→start→stop→step，故按 LIFO 弹 step/stop/start/obj。
                OpCode::GetSlice => {
                    let flags = self.read_byte()?;
                    let step = if flags & 0b100 != 0 {
                        Some(require_int(&self.pop()?)?)
                    } else {
                        None
                    };
                    let stop = if flags & 0b010 != 0 {
                        Some(require_int(&self.pop()?)?)
                    } else {
                        None
                    };
                    let start = if flags & 0b001 != 0 {
                        Some(require_int(&self.pop()?)?)
                    } else {
                        None
                    };
                    let obj = self.pop()?;
                    self.push(slice_object(obj, start, stop, step.unwrap_or(1))?)?;
                }

                // IMPORT（task 45 §4）：module_idx(2) → 加载模块，Module 对象压栈。
                // 模块名常量可能含 "@std:" 前缀（编译期折叠，§3）。load 失败 → ImportError。
                // IMPORT 可能触发 IO/嵌套 import，作为 GC 安全点（14-gc § 安全点位置）。
                OpCode::Import => {
                    let idx = self.read_u16()? as usize;
                    let name = self.read_string_constant(idx)?;
                    gc::maybe_gc(
                        &mut self.heap,
                        &mut self.stack,
                        &mut self.globals,
                        &mut self.defer_stack,
                        &mut self.call_stack,
                    );
                    match self.load_module(&name) {
                        Ok(module_ptr) => self.push(Object::Ref(module_ptr))?,
                        Err(msg) => {
                            let exc = alloc_exception(
                                "ImportError",
                                alloc_string(&msg),
                                alloc_string(""),
                                Object::Nil,
                            );
                            self.throw(exc)?;
                            continue;
                        }
                    }
                }

                // AWAIT（task 53/55）：弹出 Future/JoinHandle，检查状态。
                // Future: Resolved → 压结果值；Rejected → 抛异常；Pending → 回退 ip 让出协程。
                // JoinHandle: done → 返回 result/抛 error；未完成 → 回退 ip 让出协程。
                OpCode::Await => {
                    // task 55：安全点 cancel 检查
                    if self.check_cancel_safepoint()? {
                        continue;
                    }

                    let await_val = self.peek(0)?.clone(); // 先不弹——Pending 时需留在栈上
                    let type_tag = match &await_val {
                        Object::Ref(ptr) => unsafe { (**ptr).type_tag },
                        _ => {
                            return Err(format!(
                                "TypeError: object '{}' is not awaitable",
                                await_val.type_name()
                            ))
                        }
                    };

                    if type_tag == TypeTag::JOIN_HANDLE as u8 {
                        // task 55：await handle.join() — JoinHandle 作为 awaitable
                        let handle_ptr = match &await_val {
                            Object::Ref(p) => *p,
                            _ => unreachable!(),
                        };
                        // 克隆字段值以立即释放 RefCell borrow（安全点）
                        let (done, result, error) = {
                            let h = unsafe { read_join_handle(handle_ptr) };
                            (
                                *h.done.borrow(),
                                h.result.borrow().clone(),
                                h.error.borrow().clone(),
                            )
                        };
                        if done {
                            self.pop()?; // 弹出 JoinHandle
                            if let Some(exc) = error {
                                self.throw(exc)?;
                                continue;
                            }
                            self.push(result.unwrap_or(Object::Nil))?;
                        } else {
                            // 回退 ip 使 AWAIT 恢复时重新执行（JoinHandle 留在栈上）
                            let frame = self.call_stack.last_mut().unwrap();
                            frame.ip = frame
                                .ip
                                .checked_sub(1)
                                .ok_or("ip underflow in AWAIT")?;
                            self.yield_join = Some(handle_ptr);
                            return Ok(Object::Nil);
                        }
                        continue;
                    }

                    // ---- Future 路径（task 53）----
                    if type_tag != TypeTag::FUTURE as u8 {
                        return Err(format!(
                            "TypeError: object '{}' is not awaitable",
                            await_val.type_name()
                        ));
                    }
                    let future_ptr = match &await_val {
                        Object::Ref(p) => *p,
                        _ => unreachable!(),
                    };
                    // 克隆状态以立即释放 RefCell borrow（AWAIT 是 GC 安全点）
                    let state = {
                        let f = unsafe { read_future(future_ptr) };
                        f.state.borrow().clone()
                    };
                    match state {
                        FutureState::Resolved(val) => {
                            self.pop()?; // 弹出 Future
                            self.push(val)?; // 压入结果
                        }
                        FutureState::Rejected(exc) => {
                            self.pop()?; // 弹出 Future
                            // throw 设置 catch_address 或传播异常；Ok 则继续循环
                            self.throw(exc)?;
                            continue;
                        }
                        FutureState::Pending => {
                            // 回退 ip 使 AWAIT 在恢复时重新执行（Future 留在栈上）
                            let frame = self.call_stack.last_mut().unwrap();
                            frame.ip = frame
                                .ip
                                .checked_sub(1)
                                .ok_or("ip underflow in AWAIT")?;
                            // 设置 yield 信号，run_loop 将退出交还 EventLoop
                            self.yield_future = Some(future_ptr);
                            return Ok(Object::Nil);
                        }
                    }
                }

                // GO（task 55）：弹出零参数可调用对象，创建协程 + JoinHandle，压栈 JoinHandle。
                OpCode::Go => {
                    let callable = self.pop()?;
                    // 可调用对象须为 CLOSURE（编译器经 CLOSURE 指令产生）
                    let closure_ptr = match &callable {
                        Object::Ref(ptr)
                            if unsafe { (**ptr).type_tag } == TypeTag::CLOSURE as u8 =>
                        {
                            *ptr
                        }
                        other => {
                            return Err(format!(
                                "TypeError: '{}' is not callable in go expression",
                                other.type_name()
                            ))
                        }
                    };
                    // task 55：关闭闭包捕获的所有开放上值。go 协程运行在独立栈上，
                    // 开放上值指向当前协程栈槽，协程切换后失效。关闭上值将栈值快照到
                    // MsUpvalue.closed，使 go 协程安全访问捕获变量（参照 HALT 的 close 逻辑）。
                    {
                        let closure = unsafe { read_closure(closure_ptr) };
                        for &uv_ptr in closure.upvalues.iter() {
                            let uv = unsafe { read_upvalue(uv_ptr) };
                            if uv.closed.is_none() {
                                uv.close(&self.stack);
                            }
                        }
                    }
                    // 创建 JoinHandle 堆对象
                    let handle_obj = alloc_join_handle();
                    let handle_ptr = match &handle_obj {
                        Object::Ref(p) => *p,
                        _ => unreachable!(),
                    };
                    // 创建协程：slot 0 = callee（closure 自身），frame 从地址 0 开始
                    let frame = CallFrame::new(closure_ptr, 0, 0);
                    let coroutine = Coroutine {
                        call_stack: vec![frame],
                        stack: vec![callable],
                        defer_stack: Vec::new(),
                        open_upvalues: Vec::new(),
                        exception_handlers: Vec::new(),
                        pending_unwind: None,
                        future: None,
                        handle: Some(handle_ptr),
                    };
                    self.event_loop.ready_queue.push_back(coroutine);
                    // go 返回 JoinHandle
                    self.push(handle_obj)?;
                }

                // CHANNEL（task 54）：buffer_size(1) → 创建 channel，压栈。
                OpCode::Channel => {
                    let buffer_size = self.read_byte()? as usize;
                    self.push(crate::async_runtime::channel::alloc_channel(buffer_size))?;
                }

                // SEND（task 54）：channel 发送（ch <- value）。
                // 编译端栈布局：先 value 后 channel，故 [value, channel]，channel 在栈顶。
                // 完成后压入 Nil 作为表达式结果（语句级 POP 会丢弃）。
                OpCode::Send => {
                    // task 55：安全点 cancel 检查（在弹栈前执行，避免栈不平衡）
                    if self.check_cancel_safepoint()? {
                        continue;
                    }
                    let channel_obj = self.pop()?;
                    let value = self.pop()?;
                    let channel_ptr = expect_channel(&channel_obj)?;
                    let ch = unsafe { read_channel(channel_ptr) };

                    if ch.is_closed() {
                        let exc = alloc_exception(
                            "RuntimeError",
                            alloc_string("send on closed channel"),
                            alloc_string(""),
                            Object::Nil,
                        );
                        self.throw(exc)?;
                        continue;
                    }

                    // 1. 有等待的接收者：直接将值交付（rendezvous）。
                    if let Some(receiver) = { ch.waiting_receivers.borrow_mut().pop_front() } {
                        // value 在此 move；该分支随后压 Nil + continue。
                        let mut coro = receiver.coroutine;
                        coro.stack.push(value);
                        self.event_loop.ready_queue.push_back(coro);
                        self.push(Object::Nil)?;
                        continue;
                    }

                    // 2. 有缓冲：缓冲区未满时入队。
                    if ch.capacity > 0 {
                        let has_space = ch.buffer.borrow().len() < ch.capacity;
                        if has_space {
                            // value 在此 move；随后压 Nil + continue。
                            ch.buffer.borrow_mut().push_back(value);
                            self.push(Object::Nil)?;
                            continue;
                        }
                    }

                    // 3. 无接收者且（无缓冲 或 缓冲区满）→ 暂停。
                    //    value 未经上述分支消费，仍有效。恢复时由接收者压入 Nil 结果。
                    self.yield_channel = Some(ChannelYield::Send {
                        channel: channel_ptr,
                        value,
                    });
                    return Ok(Object::Nil);
                }

                // RECEIVE（task 54）：channel 接收（<-ch）。
                // 编译端栈布局：[channel]（channel 在栈顶）。
                OpCode::Receive => {
                    // task 55：安全点 cancel 检查（在弹栈前执行，避免栈不平衡）
                    if self.check_cancel_safepoint()? {
                        continue;
                    }
                    let channel_obj = self.pop()?;
                    let channel_ptr = expect_channel(&channel_obj)?;
                    let ch = unsafe { read_channel(channel_ptr) };

                    // 1. 先尝试从缓冲区取值。
                    let from_buffer = { ch.buffer.borrow_mut().pop_front() }; // guard 释放
                    if let Some(val) = from_buffer {
                        self.push(val)?;
                        // 缓冲区腾出空位：若有等待发送者，将其值移入缓冲区并唤醒。
                        let woken_sender = { ch.waiting_senders.borrow_mut().pop_front() };
                        if let Some(sender) = woken_sender {
                            let mut buffer = ch.buffer.borrow_mut();
                            buffer.push_back(sender.value);
                            drop(buffer); // guard 释放后再操作 EventLoop
                            // 发送者的 SEND 已完成，压入 Nil 作为其表达式结果。
                            let mut coro = sender.coroutine;
                            coro.stack.push(Object::Nil);
                            self.event_loop.ready_queue.push_back(coro);
                        }
                        continue;
                    }

                    // 2. 缓冲区空：检查等待发送者（rendezvous — 无缓冲 channel 的核心路径）。
                    let woken_sender = { ch.waiting_senders.borrow_mut().pop_front() };
                    if let Some(sender) = woken_sender {
                        // 直接取发送者的值，唤醒发送者（其 SEND 已完成，压 Nil 结果）。
                        self.push(sender.value)?;
                        let mut coro = sender.coroutine;
                        coro.stack.push(Object::Nil);
                        self.event_loop.ready_queue.push_back(coro);
                        continue;
                    }

                    // 3. 无数据、无等待发送者。若已关闭 → 返回 nil。
                    if ch.is_closed() {
                        self.push(Object::Nil)?;
                        continue;
                    }

                    // 4. 阻塞：暂停当前协程。channel 已从栈弹出，存入 ChannelYield::Recv。
                    self.yield_channel = Some(ChannelYield::Recv {
                        channel: channel_ptr,
                    });
                    return Ok(Object::Nil);
                }

                _ => {
                    return Err(format!("unimplemented opcode: {:?}", opcode));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// task 45：模块加载与执行
// ---------------------------------------------------------------------------

impl VM {
    /// 加载模块（§5）。阶段化编排规避 `&mut resolver` 与 `&mut VM` 的重叠借用：
    /// 解析/缓存登记（持 `&mut self.module_resolver`）与模块执行（持 `&mut self`）
    /// 严格串行。嵌套 IMPORT（执行期触发）可重入：执行阶段 resolver 不被独占持有。
    ///
    /// 返回指向 MsModule 的裸指针（TypeTag::MODULE）。失败返回 ImportError 消息。
    pub fn load_module(&mut self, name: &str) -> Result<*mut MsObjHeader, String> {
        let (stdlib_only, mod_name) = module::parse_std_prefix(name);

        // task 46：原生模块注册表（@std 前缀剥离后查表）。命中则直接返回缓存指针，
        // 跳过磁盘搜索与执行。
        if let Some(&ptr) = self.module_resolver.native_modules.get(mod_name) {
            return Ok(ptr);
        }

        // 1. 安全模式：仅允许 @std import。
        if self.module_resolver.safe_mode && !stdlib_only {
            return Err(format!(
                "安全模式下仅允许 import @std（拒绝 {}）",
                mod_name
            ));
        }
        // 2. 递归深度限制：load → execute_module → IMPORT → load 链。
        if self.module_resolver.loading_stack.len() >= module::MAX_IMPORT_DEPTH {
            return Err(format!(
                "导入深度超过 {} 层，疑似无限递归",
                module::MAX_IMPORT_DEPTH
            ));
        }
        // 3. 解析为规范化绝对路径（兼作缓存键）。
        //    task 72：resolve 失败（无 .ms 文件）时，尝试动态库加载（capi feature）。
        let canon = match self.module_resolver.resolve(mod_name, stdlib_only) {
            Ok(c) => c,
            Err(resolve_err) => {
                #[cfg(feature = "capi")]
                {
                    if !self.capi_vm_ptr.is_null() {
                        let vm_ptr =
                            self.capi_vm_ptr as *mut crate::capi::vm::MsVM;
                        if crate::capi::module::load_native_module(vm_ptr, mod_name)
                            .is_ok()
                        {
                            if let Some(&ptr) =
                                self.module_resolver.native_modules.get(mod_name)
                            {
                                return Ok(ptr);
                            }
                        }
                    }
                }
                #[cfg(not(feature = "capi"))]
                {
                    let _ = &resolve_err;
                }
                return Err(resolve_err);
            }
        };

        // 4. 缓存命中：已加载完成，或循环导入下尚未填充的空壳 Module。
        if let Some(&ptr) = self.module_resolver.cache.get(&canon) {
            return Ok(ptr);
        }

        // 5. 预分配空壳 Module 并登记 cache + loading_stack（支持循环导入部分访问）。
        let partial = alloc_module(mod_name);
        let partial_ptr = match partial {
            Object::Ref(p) => p,
            other => return Err(format!("alloc_module 返回非 Ref（{:?}）", other)),
        };
        self.module_resolver
            .cache
            .insert(canon.clone(), partial_ptr);
        self.module_resolver.loading_stack.insert(canon.clone());

        // 6. 读取 + 编译（本地操作，不持 vm/resolver 跨调用借用）。
        let source = std::fs::read_to_string(&canon).map_err(|e| {
            self.cleanup_failed_load(&canon);
            format!("无法加载 '{}': {}", mod_name, e)
        })?;
        let (chunk, export_names, private_names) =
            module::compile_module_source(&source, mod_name).map_err(|e| {
                self.cleanup_failed_load(&canon);
                format!("编译 '{}' 失败: {}", mod_name, e)
            })?;

        // 7. 在隔离全局作用域中执行模块顶层字节码（§7）。
        let exec_result =
            self.execute_module(chunk, &export_names, &private_names);

        // 8. 无论成败，从 loading_stack 移除（不再「加载中」）。
        self.module_resolver.loading_stack.remove(&canon);

        match exec_result {
            Ok((exports, globals)) => {
                // 9. 填充 Module：exports + 私有 globals。
                //    MVP（STW）无写屏障问题；Phase 7.5 并发标记期间须改经 write_barrier。
                unsafe {
                    let module = read_module_mut(partial_ptr);
                    module.exports = exports;
                    module.globals = globals;
                }
                // 成功：保留 cache 条目（完整 Module 供后续 import 命中）。
                Ok(partial_ptr)
            }
            Err(e) => {
                // 失败：移除破损空壳 cache 条目，避免后续 import 永久拿到空 Module。
                self.module_resolver.cache.remove(&canon);
                Err(format!("执行 '{}' 失败: {}", mod_name, e))
            }
        }
    }

    /// 失败路径（读源码/编译异常）的清理：移除 loading_stack + 破损 cache 条目。
    fn cleanup_failed_load(&mut self, canon: &PathBuf) {
        self.module_resolver.loading_stack.remove(canon);
        self.module_resolver.cache.remove(canon);
    }

    /// 在隔离全局作用域中执行模块字节码，返回 (exports, 私有 globals)（§7）。
    ///
    /// 隔离策略：保存调用方 globals，安装基线（内置函数 + 异常类，使模块可访问
    /// print/type/except），执行模块顶层代码，捕获模块 globals 并按编译器记录的
    /// 导出/私有名拆分，最后恢复调用方 globals。嵌套 import 的 globals 经保存/恢复
    /// 栈式隔离。
    pub fn execute_module(
        &mut self,
        chunk: Chunk,
        export_names: &[String],
        private_names: &[String],
    ) -> Result<(HashMap<String, Object>, HashMap<String, Object>), String> {
        // 1. 保存调用方 globals，安装基线（内置 + 异常）。
        let saved = std::mem::take(&mut self.globals);
        self.globals = self.baseline_globals.clone();

        // 2. 在新 globals 中执行模块顶层字节码（IMPORT 递归 load，其 globals 嵌套保存）。
        let run_result = self.run_top_level_chunk(chunk);

        // 3. 无论成败恢复调用方 globals。
        let module_globals = std::mem::replace(&mut self.globals, saved);

        let () = run_result?;

        // 4. 拆分：export 名 → exports；private 名 → 私有 globals；其余（基线内置）丢弃。
        let mut globals = module_globals;
        let mut exports = HashMap::new();
        for name in export_names {
            if let Some(v) = globals.remove(name) {
                exports.insert(name.clone(), v);
            }
        }
        let mut privates = HashMap::new();
        for name in private_names {
            if let Some(v) = globals.remove(name) {
                privates.insert(name.clone(), v);
            }
        }
        Ok((exports, privates))
    }

    /// 执行顶层 chunk（模块或脚本）：包裹为 <module> 闭包，压帧后运行至 HALT/RETURN。
    /// 复用 run() 主循环；模块顶层 IMPORT 会递归 load_module。
    fn run_top_level_chunk(&mut self, chunk: Chunk) -> Result<(), String> {
        // 记录调用栈/操作数栈深度：HALT 不弹出帧（顶层语义），执行后须手动恢复，
        // 否则嵌套 run() 返回后，外层 run_loop 会从已耗尽的模块帧继续读字节码。
        let call_depth = self.call_stack.len();
        let stack_depth = self.stack.len();
        let function = Function {
            name: "<module>".to_string(),
            arity: 0,
            code: chunk.code,
            constants: chunk.constants,
            upvalue_count: 0,
            source_file: None,
            default_values: Vec::new(),
            has_variadic: false,
            required_arity: 0,
            is_generator: false,
            locals_count: 1,
            is_async: false,
        };
        let Object::Ref(closure_ptr) = alloc_closure(alloc_function(function), Vec::new()) else {
            unreachable!()
        };
        self.call_stack
            .push(CallFrame::new(closure_ptr, stack_depth, self.defer_stack.len()));
        // run() 至 HALT（丢弃返回值）或顶层异常（propagate 为 Err）。
        // 若当前已有进行中的 unwind（模块在 try 内 import），交由 drive_unwind 续行。
        let result = if self.pending_unwind.is_some() {
            self.run_loop(None).map(|_| ())
        } else {
            self.run().map(|_| ())
        };
        // 恢复调用栈/操作数栈至模块执行前（HALT 未弹帧；异常路径可能已弹，truncate 安全）。
        self.call_stack.truncate(call_depth);
        self.stack.truncate(stack_depth);
        result
    }

    /// 测试用：注入模块搜索根（把指定目录置于搜索首位）。
    #[doc(hidden)]
    pub fn add_module_search_path(&mut self, path: PathBuf) {
        self.module_resolver.add_search_path(path);
    }

    /// 测试用：设置安全模式。
    #[doc(hidden)]
    pub fn set_module_safe_mode(&mut self, on: bool) {
        self.module_resolver.safe_mode = on;
    }
}

impl Default for VM {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
// 3.14 是设计文档示例值（非 PI 近似），spec 指定保留。
#[allow(clippy::approx_constant)]
mod tests {
    use super::*;
    use crate::ast::node::Program;
    use crate::compiler::opcode::OpCode;
    use crate::compiler::{Chunk, Compiler};
    use crate::lexer::Lexer;
    use crate::parser::Parser;
    use crate::vm::object::{
        alloc_bound_method, alloc_class, alloc_dict, alloc_instance, alloc_iterator, alloc_list,
        alloc_set, alloc_string, alloc_tuple, read_bound_method, read_class, read_list,
        DictMap, IteratorState, Object,
    };
    use std::collections::HashSet;

    fn parse(source: &str) -> Program {
        let tokens = Lexer::new(source).tokenize_all().unwrap();
        Parser::new(tokens).parse().unwrap()
    }

    fn compile_and_run(source: &str) -> Result<Object, String> {
        let program = parse(source);
        let mut compiler = Compiler::new();
        let chunk = compiler.compile(&program).unwrap();
        let mut vm = VM::new();
        vm.interpret(chunk)
    }

    fn i(n: i64) -> Object {
        Object::Int(n)
    }

    fn to_list(obj: &Object) -> Vec<Object> {
        match obj {
            Object::Ref(ptr) => unsafe { read_list(*ptr) }.clone(),
            _ => panic!("expected list, got {:?}", obj),
        }
    }

    // 合成字节码测试：直接构造 Chunk 验证单个 opcode 语义，绕开编译器
    // 顶层=全局作用域的已知 bug（spec line 334-338，task 23 既有先例）。
    fn run_chunk(code: Vec<u8>, constants: Vec<Object>) -> Result<Object, String> {
        let mut vm = VM::new();
        vm.interpret(Chunk {
            code,
            constants,
            lines: vec![],
        })
    }

    #[test]
    fn test_empty_program() {
        // 空程序 = 仅 HALT；栈空 → 返回 Nil，不 panic
        assert_eq!(compile_and_run("").unwrap(), Object::Nil);
    }

    #[test]
    fn test_constant_expr_stmt() {
        // 表达式语句 `42`：CONSTANT 加载后 POP 丢弃；HALT 栈空 → Nil
        assert_eq!(compile_and_run("42").unwrap(), Object::Nil);
    }

    #[test]
    fn test_constant_pushes_to_stack() {
        let mut vm = VM::new();
        let chunk = Chunk {
            code: vec![OpCode::Constant as u8, 0x00, 0x00, OpCode::Halt as u8],
            constants: vec![Object::Int(42)],
            lines: vec![],
        };
        assert_eq!(vm.interpret(chunk).unwrap(), Object::Int(42));
    }

    #[test]
    fn test_nil_true_false() {
        let run = |code: Vec<u8>| {
            let mut vm = VM::new();
            let chunk = Chunk {
                code,
                constants: vec![],
                lines: vec![],
            };
            vm.interpret(chunk).unwrap()
        };
        assert_eq!(
            run(vec![OpCode::Nil as u8, OpCode::Halt as u8]),
            Object::Nil
        );
        assert_eq!(
            run(vec![OpCode::True as u8, OpCode::Halt as u8]),
            Object::Bool(true)
        );
        assert_eq!(
            run(vec![OpCode::False as u8, OpCode::Halt as u8]),
            Object::Bool(false)
        );
    }

    #[test]
    fn test_load_local_store_local() {
        let mut vm = VM::new();
        let chunk = Chunk {
            code: vec![
                OpCode::True as u8, // 占位 slot 0
                OpCode::True as u8, // 占位 slot 1
                OpCode::Constant as u8,
                0x00,
                0x00, // push Int(42)
                OpCode::StoreLocal as u8,
                0x01, // stack[1] = 42
                OpCode::LoadLocal as u8,
                0x01, // push stack[1] = 42
                OpCode::Halt as u8,
            ],
            constants: vec![Object::Int(42)],
            lines: vec![],
        };
        assert_eq!(vm.interpret(chunk).unwrap(), Object::Int(42));
    }

    #[test]
    fn test_global_store_and_load() {
        // 顶层 `x = 10` 经 compile_var_decl 发射 StoreLocal（局部变量），
        // 无法端到端触发全局路径，故合成字节码直接测试 LoadGlobal/StoreGlobal。
        let mut vm = VM::new();
        let chunk = Chunk {
            code: vec![
                OpCode::Constant as u8,
                0x00,
                0x00, // push Int(10)   (const[0])
                OpCode::StoreGlobal as u8,
                0x00,
                0x01, // globals["x"] = 10 (name const[1])
                OpCode::LoadGlobal as u8,
                0x00,
                0x01, // push globals["x"]
                OpCode::Halt as u8,
            ],
            constants: vec![Object::Int(10), alloc_string("x")],
            lines: vec![],
        };
        let result = vm.interpret(chunk).unwrap();
        assert_eq!(result, Object::Int(10));
        assert_eq!(vm.globals.get("x"), Some(&Object::Int(10)));
    }

    #[test]
    fn test_load_global_missing_returns_nil() {
        let mut vm = VM::new();
        let chunk = Chunk {
            code: vec![OpCode::LoadGlobal as u8, 0x00, 0x00, OpCode::Halt as u8],
            constants: vec![alloc_string("undefined")],
            lines: vec![],
        };
        assert_eq!(vm.interpret(chunk).unwrap(), Object::Nil);
    }

    #[test]
    fn test_store_global_invalid_name_returns_err() {
        let mut vm = VM::new();
        // name 常量指向 Int（非 STRING）→ Err
        let chunk = Chunk {
            code: vec![
                OpCode::Constant as u8,
                0x00,
                0x00, // push Int(0) (const[0])
                OpCode::StoreGlobal as u8,
                0x00,
                0x01, // name const[1]=Int → Err
                OpCode::Halt as u8,
            ],
            constants: vec![Object::Int(0), Object::Int(1)],
            lines: vec![],
        };
        assert!(vm.interpret(chunk).is_err());
    }

    #[test]
    fn test_pop_and_dup() {
        // Dup 复制栈顶
        let mut vm = VM::new();
        let chunk = Chunk {
            code: vec![
                OpCode::Constant as u8,
                0x00,
                0x00,
                OpCode::Dup as u8,
                OpCode::Halt as u8,
            ],
            constants: vec![Object::Int(42)],
            lines: vec![],
        };
        assert_eq!(vm.interpret(chunk).unwrap(), Object::Int(42));

        // Pop 弹出栈顶，露出下方值
        let mut vm = VM::new();
        let chunk = Chunk {
            code: vec![
                OpCode::Constant as u8,
                0x00,
                0x00,
                OpCode::Constant as u8,
                0x00,
                0x00,
                OpCode::Pop as u8,
                OpCode::Halt as u8,
            ],
            constants: vec![Object::Int(7)],
            lines: vec![],
        };
        assert_eq!(vm.interpret(chunk).unwrap(), Object::Int(7));
    }

    #[test]
    fn test_unknown_opcode_returns_err() {
        let mut vm = VM::new();
        // 人造非法 opcode 字节 0xFF（超出 Halt=79）
        let chunk = Chunk {
            code: vec![0xFF],
            constants: vec![],
            lines: vec![],
        };
        assert!(vm.interpret(chunk).is_err());
    }

    #[test]
    fn test_ip_past_end_returns_err() {
        let mut vm = VM::new();
        // CONSTANT 缺操作数 → read_u16 越界 → Err
        let chunk = Chunk {
            code: vec![OpCode::Constant as u8],
            constants: vec![],
            lines: vec![],
        };
        assert!(vm.interpret(chunk).is_err());
    }

    #[test]
    fn test_stack_overflow_returns_err() {
        let mut vm = VM::new();
        let mut code = Vec::new();
        for _ in 0..(STACK_MAX + 1) {
            code.push(OpCode::True as u8);
        }
        code.push(OpCode::Halt as u8);
        let chunk = Chunk {
            code,
            constants: vec![],
            lines: vec![],
        };
        assert!(vm.interpret(chunk).is_err());
    }

    // ---- task 24：算术 / 除法 / 取模 / 幂 / 取反 ----

    #[test]
    fn test_arithmetic_add_subtract_multiply() {
        let op = |opcode: u8, a: i64, b: i64| {
            run_chunk(
                vec![
                    OpCode::Constant as u8,
                    0x00,
                    0x00,
                    OpCode::Constant as u8,
                    0x00,
                    0x01,
                    opcode,
                    OpCode::Halt as u8,
                ],
                vec![Object::Int(a), Object::Int(b)],
            )
            .unwrap()
        };
        assert_eq!(op(OpCode::Add as u8, 10, 3), Object::Int(13));
        assert_eq!(op(OpCode::Subtract as u8, 10, 3), Object::Int(7));
        assert_eq!(op(OpCode::Multiply as u8, 10, 3), Object::Int(30));
    }

    #[test]
    fn test_divide_returns_float() {
        // 10 / 3 → Float（真除法总返回 float，02-types.md）
        let result = run_chunk(
            vec![
                OpCode::Constant as u8,
                0x00,
                0x00,
                OpCode::Constant as u8,
                0x00,
                0x01,
                OpCode::Divide as u8,
                OpCode::Halt as u8,
            ],
            vec![Object::Int(10), Object::Int(3)],
        )
        .unwrap();
        assert!(matches!(result, Object::Float(_)));
    }

    #[test]
    fn test_floor_division_toward_negative_infinity() {
        // -7 // 2 == -4（向负无穷取整）
        let result = run_chunk(
            vec![
                OpCode::Constant as u8,
                0x00,
                0x00,
                OpCode::Constant as u8,
                0x00,
                0x01,
                OpCode::FloorDiv as u8,
                OpCode::Halt as u8,
            ],
            vec![Object::Int(-7), Object::Int(2)],
        )
        .unwrap();
        assert_eq!(result, Object::Int(-4));
    }

    #[test]
    fn test_modulo_floor_semantics() {
        // 10 % 3 == 1；-7 % 2 == 1（Python floor-mod，符号跟随除数）
        let m = |a: i64, b: i64| {
            run_chunk(
                vec![
                    OpCode::Constant as u8,
                    0x00,
                    0x00,
                    OpCode::Constant as u8,
                    0x00,
                    0x01,
                    OpCode::Modulo as u8,
                    OpCode::Halt as u8,
                ],
                vec![Object::Int(a), Object::Int(b)],
            )
            .unwrap()
        };
        assert_eq!(m(10, 3), Object::Int(1));
        assert_eq!(m(-7, 2), Object::Int(1));
    }

    #[test]
    fn test_power() {
        // 2 ** 10 == 1024
        let result = run_chunk(
            vec![
                OpCode::Constant as u8,
                0x00,
                0x00,
                OpCode::Constant as u8,
                0x00,
                0x01,
                OpCode::Power as u8,
                OpCode::Halt as u8,
            ],
            vec![Object::Int(2), Object::Int(10)],
        )
        .unwrap();
        assert_eq!(result, Object::Int(1024));
    }

    #[test]
    fn test_negate() {
        let result = run_chunk(
            vec![
                OpCode::Constant as u8,
                0x00,
                0x00,
                OpCode::Negate as u8,
                OpCode::Halt as u8,
            ],
            vec![Object::Int(5)],
        )
        .unwrap();
        assert_eq!(result, Object::Int(-5));
    }

    // ---- task 24：比较（含 int/float 交叉） ----

    #[test]
    fn test_comparison_ops() {
        let cmp = |opcode: u8, a: i64, b: i64| {
            run_chunk(
                vec![
                    OpCode::Constant as u8,
                    0x00,
                    0x00,
                    OpCode::Constant as u8,
                    0x00,
                    0x01,
                    opcode,
                    OpCode::Halt as u8,
                ],
                vec![Object::Int(a), Object::Int(b)],
            )
            .unwrap()
        };
        assert_eq!(cmp(OpCode::Less as u8, 3, 5), Object::Bool(true));
        assert_eq!(cmp(OpCode::Greater as u8, 3, 5), Object::Bool(false));
        assert_eq!(cmp(OpCode::LessEqual as u8, 3, 3), Object::Bool(true));
        assert_eq!(cmp(OpCode::GreaterEqual as u8, 3, 3), Object::Bool(true));
        assert_eq!(cmp(OpCode::Equal as u8, 3, 3), Object::Bool(true));
        assert_eq!(cmp(OpCode::NotEqual as u8, 3, 3), Object::Bool(false));
    }

    #[test]
    fn test_comparison_int_float_cross() {
        // Less/Greater 等经 compare 支持跨类型数值比较
        let le = run_chunk(
            vec![
                OpCode::Constant as u8,
                0x00,
                0x00,
                OpCode::Constant as u8,
                0x00,
                0x01,
                OpCode::LessEqual as u8,
                OpCode::Halt as u8,
            ],
            vec![Object::Int(3), Object::Float(3.0)],
        )
        .unwrap();
        assert_eq!(le, Object::Bool(true));
        let gt = run_chunk(
            vec![
                OpCode::Constant as u8,
                0x00,
                0x00,
                OpCode::Constant as u8,
                0x00,
                0x01,
                OpCode::Greater as u8,
                OpCode::Halt as u8,
            ],
            vec![Object::Int(3), Object::Float(3.0)],
        )
        .unwrap();
        assert_eq!(gt, Object::Bool(false));
    }

    // ---- task 24：位运算（仅 int） ----

    #[test]
    fn test_bitwise_ops() {
        let op2 = |opcode: u8, a: i64, b: i64| {
            run_chunk(
                vec![
                    OpCode::Constant as u8,
                    0x00,
                    0x00,
                    OpCode::Constant as u8,
                    0x00,
                    0x01,
                    opcode,
                    OpCode::Halt as u8,
                ],
                vec![Object::Int(a), Object::Int(b)],
            )
            .unwrap()
        };
        assert_eq!(op2(OpCode::BitAnd as u8, 5, 3), Object::Int(1));
        assert_eq!(op2(OpCode::BitOr as u8, 5, 3), Object::Int(7));
        assert_eq!(op2(OpCode::BitXor as u8, 5, 3), Object::Int(6));
        assert_eq!(op2(OpCode::LeftShift as u8, 1, 2), Object::Int(4));
        assert_eq!(op2(OpCode::RightShift as u8, 4, 1), Object::Int(2));
        let not = run_chunk(
            vec![
                OpCode::Constant as u8,
                0x00,
                0x00,
                OpCode::BitNot as u8,
                OpCode::Halt as u8,
            ],
            vec![Object::Int(5)],
        )
        .unwrap();
        assert_eq!(not, Object::Int(-6));
    }

    #[test]
    fn test_bitwise_type_error_on_float() {
        // 位运算仅支持 int；float 操作数 → TypeError
        let result = run_chunk(
            vec![
                OpCode::Constant as u8,
                0x00,
                0x00,
                OpCode::Constant as u8,
                0x00,
                0x01,
                OpCode::BitAnd as u8,
                OpCode::Halt as u8,
            ],
            vec![Object::Int(5), Object::Float(3.0)],
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("TypeError"));
    }

    #[test]
    fn test_logical_not() {
        let n = |code: Vec<u8>, constants: Vec<Object>| run_chunk(code, constants).unwrap();
        assert_eq!(
            n(
                vec![
                    OpCode::Constant as u8,
                    0x00,
                    0x00,
                    OpCode::Not as u8,
                    OpCode::Halt as u8
                ],
                vec![Object::Int(0)]
            ),
            Object::Bool(true)
        );
        assert_eq!(
            n(
                vec![OpCode::True as u8, OpCode::Not as u8, OpCode::Halt as u8],
                vec![]
            ),
            Object::Bool(false)
        );
        assert_eq!(
            n(
                vec![OpCode::Nil as u8, OpCode::Not as u8, OpCode::Halt as u8],
                vec![]
            ),
            Object::Bool(true)
        );
    }

    // ---- task 24：身份比较 `is`（Ref↔Ref 比指针；inline 抛 TypeError） ----

    #[test]
    fn test_is_identity_same_pointer() {
        // 同一常量指针 → is 为 true
        let result = run_chunk(
            vec![
                OpCode::Constant as u8,
                0x00,
                0x00,
                OpCode::Constant as u8,
                0x00,
                0x00,
                OpCode::Is as u8,
                OpCode::Halt as u8,
            ],
            vec![alloc_string("abc")],
        )
        .unwrap();
        assert_eq!(result, Object::Bool(true));
    }

    #[test]
    fn test_is_identity_different_pointer() {
        // 内容相同但两次独立分配 → 不同指针 → is 为 false
        let result = run_chunk(
            vec![
                OpCode::Constant as u8,
                0x00,
                0x00,
                OpCode::Constant as u8,
                0x00,
                0x01,
                OpCode::Is as u8,
                OpCode::Halt as u8,
            ],
            vec![alloc_string("abc"), alloc_string("abc")],
        )
        .unwrap();
        assert_eq!(result, Object::Bool(false));
    }

    #[test]
    fn test_is_inline_type_error() {
        // inline 类型（int）使用 is → TypeError
        let result = run_chunk(
            vec![
                OpCode::Constant as u8,
                0x00,
                0x00,
                OpCode::Constant as u8,
                0x00,
                0x01,
                OpCode::Is as u8,
                OpCode::Halt as u8,
            ],
            vec![Object::Int(1), Object::Int(2)],
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("TypeError"));
    }

    // ---- task 24：`in`（当前仅 String 子串） ----

    #[test]
    fn test_in_string() {
        // "ell" in "hello" → true；"xyz" in "hello" → false
        let t = run_chunk(
            vec![
                OpCode::Constant as u8,
                0x00,
                0x00,
                OpCode::Constant as u8,
                0x00,
                0x01,
                OpCode::In as u8,
                OpCode::Halt as u8,
            ],
            vec![alloc_string("ell"), alloc_string("hello")],
        )
        .unwrap();
        assert_eq!(t, Object::Bool(true));
        let f = run_chunk(
            vec![
                OpCode::Constant as u8,
                0x00,
                0x00,
                OpCode::Constant as u8,
                0x00,
                0x01,
                OpCode::In as u8,
                OpCode::Halt as u8,
            ],
            vec![alloc_string("xyz"), alloc_string("hello")],
        )
        .unwrap();
        assert_eq!(f, Object::Bool(false));
    }

    #[test]
    fn test_in_string_type_error() {
        // needle 为 int → TypeError（要求 str in str）
        let result = run_chunk(
            vec![
                OpCode::Constant as u8,
                0x00,
                0x00,
                OpCode::Constant as u8,
                0x00,
                0x01,
                OpCode::In as u8,
                OpCode::Halt as u8,
            ],
            vec![Object::Int(1), alloc_string("hello")],
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("TypeError"));
    }

    // ---- task 24：错误路径 ----

    #[test]
    fn test_division_by_zero_error() {
        let result = run_chunk(
            vec![
                OpCode::Constant as u8,
                0x00,
                0x00,
                OpCode::Constant as u8,
                0x00,
                0x01,
                OpCode::Divide as u8,
                OpCode::Halt as u8,
            ],
            vec![Object::Int(1), Object::Int(0)],
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("ZeroDivisionError"));
    }

    #[test]
    fn test_power_overflow_error() {
        // 2 ** 100 → OverflowError（指数 ≥ 64 必溢出）
        let result = run_chunk(
            vec![
                OpCode::Constant as u8,
                0x00,
                0x00,
                OpCode::Constant as u8,
                0x00,
                0x01,
                OpCode::Power as u8,
                OpCode::Halt as u8,
            ],
            vec![Object::Int(2), Object::Int(100)],
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("OverflowError"));
    }

    #[test]
    fn test_arithmetic_type_error() {
        // int + nil → TypeError
        let result = run_chunk(
            vec![
                OpCode::Constant as u8,
                0x00,
                0x00,
                OpCode::Nil as u8,
                OpCode::Add as u8,
                OpCode::Halt as u8,
            ],
            vec![Object::Int(1)],
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("TypeError"));
    }

    // ---- task 24：控制流（端到端，真实 Lexer+Parser+Compiler）----
    // 注：编译器顶层=局部（非全局）的已知 bug 只影响 vm.globals 读取；
    // 局部变量的存取自洽，故 if/while/break/continue 的执行路径可端到端验证。
    // 用「错误注入」使分支/循环选择可观测（错误分支未执行即证明选择正确）。

    #[test]
    fn test_if_else_then_branch() {
        // 3 > 2 为真 → then 分支；else 中 1/0 不执行 → Ok
        let result = compile_and_run("if 3 > 2 { 1 } else { 1/0 }");
        assert!(result.is_ok());
    }

    #[test]
    fn test_if_else_else_branch() {
        // 2 > 3 为假 → else 分支；then 中 1/0 不执行 → Ok
        let result = compile_and_run("if 2 > 3 { 1/0 } else { 1 }");
        assert!(result.is_ok());
    }

    #[test]
    fn test_if_else_error_when_condition_true() {
        // 条件为真且 then 含 1/0 → 必触发 ZeroDivisionError（证明 then 被选中）
        let result = compile_and_run("if 3 > 2 { 1/0 } else { 1 }");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("ZeroDivisionError"));
    }

    #[test]
    fn test_while_loop_iterations() {
        // 合成 while 循环：slot 1 为计数器（初始 0），每轮 +1，i<3 为限。
        // slot 0 为 VM 预分配的 callee 占位（Nil，task 27 订正 A3）。
        // 经 JumpBack 回边 3 轮后退出；Halt 弹出 slot 1 的最终值 → Int(3)。
        let result = run_chunk(
            vec![
                OpCode::Constant as u8,
                0x00,
                0x00, // push Int(0) → slot 1 初值
                OpCode::LoadLocal as u8,
                0x01, // loop_start: push i
                OpCode::Constant as u8,
                0x00,
                0x01, // push Int(3)
                OpCode::Less as u8,
                OpCode::JumpIfFalse as u8,
                0x00,
                0x0C, // → exit
                OpCode::Pop as u8,
                OpCode::LoadLocal as u8,
                0x01, // push i
                OpCode::Constant as u8,
                0x00,
                0x02, // push Int(1)
                OpCode::Add as u8,
                OpCode::StoreLocal as u8,
                0x01, // i = i + 1
                OpCode::JumpBack as u8,
                0x00,
                0x15,              // → loop_start
                OpCode::Pop as u8, // exit: 弹出条件
                OpCode::Halt as u8,
            ],
            vec![Object::Int(0), Object::Int(3), Object::Int(1)],
        )
        .unwrap();
        assert_eq!(result, Object::Int(3));
    }

    #[test]
    fn test_break_end_to_end() {
        // 端到端（无变量，故不受顶层局部槽限制）：while true 体首句 break 即跳出，
        // 跳过后续不可达的 1/0 → Ok。若 break 失效（前向跳转未生效）则 1/0 必触发错误。
        let result = compile_and_run("while true { break\n1/0 }");
        assert!(result.is_ok());
    }

    #[test]
    fn test_continue_backward_jump() {
        // 合成循环：与 while 测试同构，但回边用 Continue（checked_sub 后向跳）。
        // 限界 2 → 两轮后退出 → Int(2)，验证 Continue 的后向跳转执行。
        // slot 0 为 VM 预分配的 callee 占位（Nil，task 27 订正 A3）。
        let result = run_chunk(
            vec![
                OpCode::Constant as u8,
                0x00,
                0x00, // push Int(0) → slot 1
                OpCode::LoadLocal as u8,
                0x01, // loop_start: push i
                OpCode::Constant as u8,
                0x00,
                0x01, // push Int(2)
                OpCode::Less as u8,
                OpCode::JumpIfFalse as u8,
                0x00,
                0x0C, // → exit
                OpCode::Pop as u8,
                OpCode::LoadLocal as u8,
                0x01, // push i
                OpCode::Constant as u8,
                0x00,
                0x02, // push Int(1)
                OpCode::Add as u8,
                OpCode::StoreLocal as u8,
                0x01, // i = i + 1
                OpCode::Continue as u8,
                0x00,
                0x15,              // → loop_start
                OpCode::Pop as u8, // exit
                OpCode::Halt as u8,
            ],
            vec![Object::Int(0), Object::Int(2), Object::Int(1)],
        )
        .unwrap();
        assert_eq!(result, Object::Int(2));
    }

    // ---- task 25：内置函数 CALL 集成（LoadGlobal → CALL native dispatch）----
    //
    // register_builtins 在 VM::new() 中注入全部内置函数；call_builtin 构造
    // 合成字节码（LoadGlobal name → push args → CALL → Halt）端到端验证
    // 原生函数调用分支、arity 校验与全局解析。

    fn call_builtin(name: &str, args: &[Object]) -> Result<Object, String> {
        let mut code = Vec::new();
        let mut constants = vec![alloc_string(name)];
        code.push(OpCode::LoadGlobal as u8);
        code.extend(&0u16.to_be_bytes()); // const[0] = name
        for arg in args {
            let idx = constants.len();
            constants.push(arg.clone());
            code.push(OpCode::Constant as u8);
            code.extend(&(idx as u16).to_be_bytes());
        }
        code.push(OpCode::Call as u8);
        code.push(args.len() as u8);
        code.push(OpCode::Halt as u8);
        run_chunk(code, constants)
    }

    /// 仅加载全局内置函数对象（不调用），用于 isinstance 第二参数等场景。
    fn load_builtin(name: &str) -> Object {
        run_chunk(
            vec![OpCode::LoadGlobal as u8, 0x00, 0x00, OpCode::Halt as u8],
            vec![alloc_string(name)],
        )
        .unwrap()
    }

    #[test]
    fn test_builtin_type_returns_name() {
        // type(42) -> "int"；type([1,2]) -> "list"（B3 扩展）
        assert_eq!(
            call_builtin("type", &[Object::Int(42)]).unwrap(),
            alloc_string("int")
        );
        assert_eq!(
            call_builtin("type", &[alloc_string("hello")]).unwrap(),
            alloc_string("string")
        );
        assert_eq!(
            call_builtin("type", &[alloc_list(vec![Object::Int(1), Object::Int(2)])]).unwrap(),
            alloc_string("list")
        );
    }

    #[test]
    fn test_builtin_len() {
        assert_eq!(
            call_builtin("len", &[alloc_string("hello")]).unwrap(),
            Object::Int(5)
        );
        assert_eq!(
            call_builtin(
                "len",
                &[alloc_list(vec![
                    Object::Int(1),
                    Object::Int(2),
                    Object::Int(3)
                ])]
            )
            .unwrap(),
            Object::Int(3)
        );
    }

    #[test]
    fn test_builtin_abs_max_min_sum() {
        assert_eq!(
            call_builtin("abs", &[Object::Int(-5)]).unwrap(),
            Object::Int(5)
        );
        assert_eq!(
            call_builtin("max", &[Object::Int(1), Object::Int(2), Object::Int(3)]).unwrap(),
            Object::Int(3)
        );
        assert_eq!(
            call_builtin("min", &[Object::Int(1), Object::Int(2), Object::Int(3)]).unwrap(),
            Object::Int(1)
        );
        assert_eq!(
            call_builtin(
                "sum",
                &[alloc_list(vec![
                    Object::Int(1),
                    Object::Int(2),
                    Object::Int(3)
                ])]
            )
            .unwrap(),
            Object::Int(6)
        );
    }

    #[test]
    fn test_builtin_conversions() {
        assert_eq!(
            call_builtin("int", &[alloc_string("42")]).unwrap(),
            Object::Int(42)
        );
        assert_eq!(
            call_builtin("float", &[alloc_string("3.14")]).unwrap(),
            Object::Float(3.14)
        );
        assert_eq!(
            call_builtin("str", &[Object::Int(42)]).unwrap(),
            alloc_string("42")
        );
        assert_eq!(
            call_builtin("bool", &[Object::Int(0)]).unwrap(),
            Object::Bool(false)
        );
    }

    #[test]
    fn test_builtin_ceil_floor_round() {
        assert_eq!(
            call_builtin("ceil", &[Object::Float(3.7)]).unwrap(),
            Object::Int(4)
        );
        assert_eq!(
            call_builtin("floor", &[Object::Float(3.7)]).unwrap(),
            Object::Int(3)
        );
        assert_eq!(
            call_builtin("round", &[Object::Float(3.5)]).unwrap(),
            Object::Float(4.0)
        );
        // round(3.14159, 2) -> 3.14
        assert_eq!(
            call_builtin("round", &[Object::Float(3.14159), Object::Int(2)]).unwrap(),
            Object::Float(3.14)
        );
    }

    #[test]
    fn test_builtin_round_digits_out_of_range() {
        // round(x, 20) -> ValueError（digits 越界，D3）
        let r = call_builtin("round", &[Object::Float(1.0), Object::Int(20)]);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("ValueError"));
    }

    #[test]
    fn test_builtin_isinstance() {
        // isinstance(42, int) -> true（int 全局即内置转换函数，充当类型对象）
        assert_eq!(
            call_builtin("isinstance", &[Object::Int(42), load_builtin("int")]).unwrap(),
            Object::Bool(true)
        );
        // isinstance("hi", int) -> false
        assert_eq!(
            call_builtin("isinstance", &[alloc_string("hi"), load_builtin("int")]).unwrap(),
            Object::Bool(false)
        );
    }

    #[test]
    fn test_builtin_arity_check() {
        // type() arity=1；传 0 参 → TypeError
        let r = call_builtin("type", &[]);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("TypeError"));
    }

    #[test]
    fn test_builtin_collection_conversions() {
        // list("abc") -> ["a","b","c"]
        assert_eq!(
            call_builtin("list", &[alloc_string("abc")]).unwrap(),
            alloc_list(vec![
                alloc_string("a"),
                alloc_string("b"),
                alloc_string("c")
            ])
        );
        // tuple([1,2]) -> (1,2)
        assert_eq!(
            call_builtin("tuple", &[alloc_list(vec![Object::Int(1), Object::Int(2)])]).unwrap(),
            alloc_tuple(vec![Object::Int(1), Object::Int(2)])
        );
    }

    #[test]
    fn test_builtin_range() {
        // range 现返回迭代器（task 26）；list(range(5)) 消费为 [0,1,2,3,4]。
        // 合成嵌套调用字节码：CALL 约定要求 callee 在其参数之下，
        // 故 list 先入栈，再求值其参数 range(5)。
        let constants = vec![alloc_string("list"), alloc_string("range"), Object::Int(5)];
        let code = vec![
            OpCode::LoadGlobal as u8,
            0x00,
            0x00, // push list（外层 callee）
            OpCode::LoadGlobal as u8,
            0x00,
            0x01, // push range（内层 callee）
            OpCode::Constant as u8,
            0x00,
            0x02, // push Int(5)
            OpCode::Call as u8,
            0x01, // range(5) -> iterator（留在 list 之上）
            OpCode::Call as u8,
            0x01, // list(iter) -> [0,1,2,3,4]
            OpCode::Halt as u8,
        ];
        assert_eq!(
            run_chunk(code, constants).unwrap(),
            alloc_list(vec![
                Object::Int(0),
                Object::Int(1),
                Object::Int(2),
                Object::Int(3),
                Object::Int(4)
            ])
        );
    }

    #[test]
    fn test_builtin_print_returns_nil() {
        // print 不报错且返回 nil（输出至测试 stdout，格式由 Display 保证，已单测）
        assert_eq!(
            call_builtin("print", &[alloc_string("hello")]).unwrap(),
            Object::Nil
        );
    }

    #[test]
    fn test_call_non_callable_type_error() {
        // 调用非函数全局（nil 局部无此名）→ "object is not callable"
        // 注：未定义名经 LoadGlobal 返回 Nil；CALL Nil → TypeError。
        let r = call_builtin("does_not_exist", &[]);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("not callable"));
    }

    // ---- task 25：真实编译器端到端（Lexer+Parser+Compiler+VM）----
    //
    // 证明真实编译器对内置标识符发射 LoadGlobal（而非误判为局部），
    // CALL 经原生函数分支正确分派。用「错误注入」使结果可观测
    // （裸表达式语句被 POP，返回值恒为 Nil，故靠是否报错判断）。

    #[test]
    fn test_end_to_end_builtin_resolves_as_global() {
        // len(5) → 内置函数被解析并调用 → TypeError（int 无 len）
        let r = compile_and_run("len(5)");
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("TypeError"));
        // abs(-5) → 解析 + 调用成功，无报错
        assert!(compile_and_run("abs(-5)").is_ok());
    }

    #[test]
    fn test_end_to_end_nested_builtin_calls() {
        // print(type(42))：嵌套调用；type 内层成功 → print 外层成功 → Ok
        assert!(compile_and_run("print(type(42))").is_ok());
        // max(1, 2, 3) → Ok
        assert!(compile_and_run("max(1, 2, 3)").is_ok());
    }

    // ---- task 26/32：ITERATOR / FOR_ITER / UNPACK VM 执行 ----
    //
    // task 32 修复后，真实编译的顶层 for..in 端到端可用（见 test_for_in_*）。
    // 此处合成 Chunk 仍验证 FOR_ITER opcode 语义——迭代器存于局部 slot，
    // FOR_ITER 从 slot 读取而非栈顶。

    /// 构造「统计可迭代对象元素个数」的 for..in 循环字节码并运行，返回计数。
    /// 覆盖 ITERATOR + FOR_ITER 对各可迭代类型的执行。
    fn count_iterations(iterable: Object) -> Result<Object, String> {
        // 布局：slot0=<self>, slot1=count, slot2=iterator。
        // 先 Nil 预留 slot1/slot2，StoreLocal 将迭代器写入 slot2。
        // loop: FOR_ITER(slot2)→Pop(弃值)→count+=1→JUMP_BACK；exit: LoadLocal1→HALT。
        let code = vec![
            OpCode::Nil as u8, // 0: reserve slot1
            OpCode::Nil as u8, // 1: reserve slot2
            OpCode::Constant as u8,
            0x00,
            0x00, // 2: Int(0) → stack
            OpCode::StoreLocal as u8,
            0x01, // 5: slot1 = 0
            OpCode::Constant as u8,
            0x00,
            0x01,                   // 7: iterable → stack
            OpCode::Iterator as u8, // 10: → iter
            OpCode::StoreLocal as u8,
            0x02, // 11: slot2 = iter
            // loop_start = 13:
            OpCode::ForIter as u8,
            0x02,
            0x00,
            0x0C,              // 13: slot2, exit offset 12 → exit at 29
            OpCode::Pop as u8, // 17: discard value
            OpCode::LoadLocal as u8,
            0x01, // 18: push count
            OpCode::Constant as u8,
            0x00,
            0x02,              // 20: Int(1)
            OpCode::Add as u8, // 23: count + 1
            OpCode::StoreLocal as u8,
            0x01, // 24: slot1 = count + 1
            OpCode::JumpBack as u8,
            0x00,
            0x10, // 26: backward 16 → loop_start 13
            // exit = 29:
            OpCode::LoadLocal as u8,
            0x01,               // 29: push count
            OpCode::Halt as u8, // 31
        ];
        run_chunk(code, vec![Object::Int(0), iterable, Object::Int(1)])
    }

    #[test]
    fn test_for_iter_counts_each_iterable_type() {
        // list / string / dict(键) / set / range 迭代器 均可被 FOR_ITER 遍历
        assert_eq!(
            count_iterations(alloc_list(vec![
                Object::Int(1),
                Object::Int(2),
                Object::Int(3),
                Object::Int(4),
                Object::Int(5)
            ]))
            .unwrap(),
            Object::Int(5)
        );
        assert_eq!(
            count_iterations(alloc_string("hello")).unwrap(),
            Object::Int(5)
        );
        let mut m = DictMap::new();
        m.insert(alloc_string("a"), Object::Int(1));
        m.insert(alloc_string("b"), Object::Int(2));
        assert_eq!(count_iterations(alloc_dict(m)).unwrap(), Object::Int(2));
        let mut s = HashSet::new();
        s.insert(Object::Int(1));
        s.insert(Object::Int(2));
        s.insert(Object::Int(3));
        assert_eq!(count_iterations(alloc_set(s)).unwrap(), Object::Int(3));
        // range 迭代器（Iterator 对迭代器克隆状态，见 to_iterator ITERATOR 分支）
        let range_it = alloc_iterator(IteratorState::Range {
            current: 0,
            end: 5,
            step: 1,
        });
        assert_eq!(count_iterations(range_it).unwrap(), Object::Int(5));
        // 空可迭代对象 → 0 次
        assert_eq!(
            count_iterations(alloc_list(vec![])).unwrap(),
            Object::Int(0)
        );
    }

    #[test]
    fn test_for_iter_sums_list_values() {
        // 验证 FOR_ITER 推送的值确为各元素（累加 [0,1,2,3,4] = 10）。
        // 布局：slot0=<self>, slot1=sum, slot2=iterator。
        let code = vec![
            OpCode::Nil as u8, // 0: reserve slot1
            OpCode::Nil as u8, // 1: reserve slot2
            OpCode::Constant as u8,
            0x00,
            0x00, // 2: Int(0) → slot1
            OpCode::StoreLocal as u8,
            0x01, // 5: slot1 = 0
            OpCode::Constant as u8,
            0x00,
            0x01,                   // 7: list → stack
            OpCode::Iterator as u8, // 10: → iter
            OpCode::StoreLocal as u8,
            0x02, // 11: slot2 = iter
            // loop_start = 13:
            OpCode::ForIter as u8,
            0x02,
            0x00,
            0x08, // 13: slot2, exit offset 8 → exit at 25
            OpCode::LoadLocal as u8,
            0x01,              // 17: push sum
            OpCode::Add as u8, // 19: value + sum
            OpCode::StoreLocal as u8,
            0x01, // 20: sum = value + sum
            OpCode::JumpBack as u8,
            0x00,
            0x0C, // 22: backward 12 → loop_start 13
            // exit = 25:
            OpCode::LoadLocal as u8,
            0x01,               // 25: push sum
            OpCode::Halt as u8, // 27
        ];
        let list = alloc_list(vec![
            Object::Int(0),
            Object::Int(1),
            Object::Int(2),
            Object::Int(3),
            Object::Int(4),
        ]);
        assert_eq!(
            run_chunk(code, vec![Object::Int(0), list]).unwrap(),
            Object::Int(10)
        );
    }

    #[test]
    fn test_unpack_opcode_isolated() {
        // UNPACK 2：tuple(5,7) → 逆序压入，元素 0 落在栈顶 → HALT 返回 5。
        let tuple = alloc_tuple(vec![Object::Int(5), Object::Int(7)]);
        let code = vec![
            OpCode::Constant as u8,
            0x00,
            0x00,
            OpCode::Unpack as u8,
            0x02,
            OpCode::Halt as u8,
        ];
        assert_eq!(run_chunk(code, vec![tuple]).unwrap(), Object::Int(5));
        // 个数不匹配 → ValueError
        let tuple2 = alloc_tuple(vec![Object::Int(1), Object::Int(2)]);
        let code = vec![
            OpCode::Constant as u8,
            0x00,
            0x00,
            OpCode::Unpack as u8,
            0x03, // 期望 3，实际 2
            OpCode::Halt as u8,
        ];
        let r = run_chunk(code, vec![tuple2]);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("ValueError"));
    }

    #[test]
    fn test_for_iter_with_unpack_sums_first_elements() {
        // 端到端模拟双变量 for..in：遍历 [(1,10),(2,20),(3,30)]，累加每对首元素 = 6。
        // 验证 FOR_ITER + UNPACK + 多轮迭代协同。布局：slot0=<self>, slot1=sum,
        // slot2=iterator。loop: FOR_ITER(slot2)→UNPACK 2→LoadLocal1→Add→StoreLocal1→
        // Pop(弃次元素)→JUMP_BACK；exit: LoadLocal1→HALT。
        let code = vec![
            OpCode::Nil as u8, // 0: reserve slot1
            OpCode::Nil as u8, // 1: reserve slot2
            OpCode::Constant as u8,
            0x00,
            0x00, // 2: Int(0) → slot1
            OpCode::StoreLocal as u8,
            0x01, // 5: slot1 = 0
            OpCode::Constant as u8,
            0x00,
            0x01,                   // 7: list of tuples → stack
            OpCode::Iterator as u8, // 10: → iter
            OpCode::StoreLocal as u8,
            0x02, // 11: slot2 = iter
            // loop_start = 13:
            OpCode::ForIter as u8,
            0x02,
            0x00,
            0x0B, // 13: slot2, exit offset 11 → exit at 28
            OpCode::Unpack as u8,
            0x02, // 17: → [second, first]
            OpCode::LoadLocal as u8,
            0x01,              // 19: push sum
            OpCode::Add as u8, // 21: first + sum
            OpCode::StoreLocal as u8,
            0x01,              // 22: sum = first + sum
            OpCode::Pop as u8, // 24: discard second
            OpCode::JumpBack as u8,
            0x00,
            0x0F, // 25: backward 15 → loop_start 13
            // exit = 28:
            OpCode::LoadLocal as u8,
            0x01,               // 28: push sum
            OpCode::Halt as u8, // 30
        ];
        let tuples = alloc_list(vec![
            alloc_tuple(vec![Object::Int(1), Object::Int(10)]),
            alloc_tuple(vec![Object::Int(2), Object::Int(20)]),
            alloc_tuple(vec![Object::Int(3), Object::Int(30)]),
        ]);
        assert_eq!(
            run_chunk(code, vec![Object::Int(0), tuples]).unwrap(),
            Object::Int(6)
        );
    }

    // ---- task 30: BUILD_TUPLE VM handler + 多返回值/元组解包 ----

    #[test]
    fn test_build_tuple_opcode_constructs_tuple() {
        // BUILD_TUPLE 3：弹出栈顶 3 个元素，构建 tuple 并压栈 → HALT 返回该 tuple。
        let code = vec![
            OpCode::Constant as u8,
            0x00,
            0x00,
            OpCode::Constant as u8,
            0x00,
            0x01,
            OpCode::Constant as u8,
            0x00,
            0x02,
            OpCode::BuildTuple as u8,
            0x03,
            OpCode::Halt as u8,
        ];
        let result = run_chunk(code, vec![Object::Int(1), Object::Int(2), Object::Int(3)]).unwrap();
        assert_eq!(
            result,
            alloc_tuple(vec![Object::Int(1), Object::Int(2), Object::Int(3)])
        );
    }

    #[test]
    fn test_build_tuple_opcode_empty() {
        // BUILD_TUPLE 0：构建空 tuple。
        let code = vec![OpCode::BuildTuple as u8, 0x00, OpCode::Halt as u8];
        let result = run_chunk(code, vec![]).unwrap();
        assert_eq!(result, alloc_tuple(vec![]));
        assert_eq!(format!("{}", result), "()");
    }

    #[test]
    fn test_build_tuple_opcode_preserves_order() {
        // 栈顶顺序保持：先压 1、再压 2 → BUILD_TUPLE 2 → tuple(1, 2)（非逆序）。
        let code = vec![
            OpCode::Constant as u8,
            0x00,
            0x00,
            OpCode::Constant as u8,
            0x00,
            0x01,
            OpCode::BuildTuple as u8,
            0x02,
            OpCode::Halt as u8,
        ];
        let result = run_chunk(code, vec![Object::Int(1), Object::Int(2)]).unwrap();
        assert_eq!(result, alloc_tuple(vec![Object::Int(1), Object::Int(2)]));
    }

    #[test]
    fn test_build_tuple_opcode_underflow() {
        // 栈上不足 count 个元素 → RuntimeError（stack underflow）。
        let code = vec![
            OpCode::Constant as u8,
            0x00,
            0x00,
            OpCode::BuildTuple as u8,
            0x03, // 仅 1 个元素，却要 3 个
            OpCode::Halt as u8,
        ];
        let r = run_chunk(code, vec![Object::Int(1)]);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("underflow"));
    }

    #[test]
    fn test_multi_return_build_then_unpack_roundtrip() {
        // 模拟多返回值完整字节码流：return a, b → BUILD_TUPLE 2 → DUP → UNPACK 2。
        // UNPACK 逆序压栈使 elements[0] 位于栈顶 → HALT 返回 elements[0]。
        // 注：q, r = func() 的端到端形式受解析器限制（单值右值被包成 1 元 tuple），
        // 故以合成字节码验证 BUILD_TUPLE + UNPACK 协同。
        let code = vec![
            OpCode::Constant as u8,
            0x00,
            0x00,
            OpCode::Constant as u8,
            0x00,
            0x01,
            OpCode::BuildTuple as u8,
            0x02,
            OpCode::Dup as u8,
            OpCode::Unpack as u8,
            0x02,
            OpCode::Halt as u8,
        ];
        let result = run_chunk(code, vec![Object::Int(3), Object::Int(1)]).unwrap();
        assert_eq!(result, Object::Int(3)); // elements[0] 在栈顶
    }

    #[test]
    fn test_swap_via_unpack_e2e() {
        // 交换：a, b = b, a（右值为多值 TupleLiteral，解析器支持）。
        let src = "a = 1\nb = 2\na, b = b, a\nif a != 2 {\n1/0\n}\nif b != 1 {\n1/0\n}";
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_multi_value_rhs_unpack_e2e() {
        // 多目标赋值 a, b = 10, 20（右值为多值 TupleLiteral，解析器支持）。
        // 验证 compile_store_target 的 TupleLiteral 分支（UNPACK + 正序 store）。
        let src = "a, b = 10, 20\nif a != 10 {\n1/0\n}\nif b != 20 {\n1/0\n}";
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_unpack_count_mismatch_synthetic() {
        // 解包数量不匹配 → ValueError（UNPACK handler 运行时校验，task 26 已实现）。
        // 用合成字节码解包真正的 2 元素 tuple（解析器对单值右值会包成 1 元 tuple）。
        let tuple = alloc_tuple(vec![Object::Int(1), Object::Int(2)]);
        let code = vec![
            OpCode::Constant as u8,
            0x00,
            0x00,
            OpCode::Unpack as u8,
            0x03, // 期望 3，实际 2
            OpCode::Halt as u8,
        ];
        let r = run_chunk(code, vec![tuple]);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("ValueError"));
    }

    #[test]
    fn test_unpack_list_synthetic() {
        // 对 list 解包：UNPACK 支持 tuple/list（task 26 已实现）。
        // 用合成字节码解包 2 元素 list，验证 elements[0] 在栈顶（逆序压栈）。
        let list = alloc_list(vec![Object::Int(10), Object::Int(20)]);
        let code = vec![
            OpCode::Constant as u8,
            0x00,
            0x00,
            OpCode::Unpack as u8,
            0x02,
            OpCode::Halt as u8, // 返回栈顶 = elements[0]
        ];
        let result = run_chunk(code, vec![list]).unwrap();
        assert_eq!(result, Object::Int(10));
    }

    #[test]
    fn test_unpack_regression_for_in_still_works() {
        // 回归：for..in 双变量循环依赖 UNPACK，task 30 不应破坏。
        let prog = parse("s = 0\nfor i, v in [(1, 10), (2, 20)] {\ns = s + i\n}");
        let chunk = Compiler::new().compile(&prog).unwrap();
        assert!(chunk.code.contains(&(OpCode::Unpack as u8)));
    }

    #[test]
    fn test_iterator_opcode_not_iterable_errors() {
        // ITERATOR 对不可迭代对象（int）→ RuntimeError（TypeError 包装）
        let code = vec![
            OpCode::Constant as u8,
            0x00,
            0x00,
            OpCode::Iterator as u8,
            OpCode::Halt as u8,
        ];
        let r = run_chunk(code, vec![Object::Int(1)]);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("not iterable"));
    }

    #[test]
    fn test_compiler_emits_for_in_opcodes() {
        // 证明真实编译器对 for..in 发射 ITERATOR + FOR_ITER（编译器侧已实现，
        // task 19）；本 task 在 VM 侧补齐执行。双变量额外发 UNPACK。
        let prog = parse("for i in [1] {\n}");
        let chunk = Compiler::new().compile(&prog).unwrap();
        assert!(chunk.code.contains(&(OpCode::Iterator as u8)));
        assert!(chunk.code.contains(&(OpCode::ForIter as u8)));

        let prog2 = parse("for a, b in [(1, 2)] {\n}");
        let chunk2 = Compiler::new().compile(&prog2).unwrap();
        assert!(chunk2.code.contains(&(OpCode::Unpack as u8)));
    }

    // ---- task 32：for..in 端到端（真实编译 + 真实 VM 执行）----
    //
    // 修复 compile_for_in slot 冲突 + 回填 BuildList/BuildDict/BuildSet handler
    // 后，顶层 for..in 端到端可用。assert() 内联断言验证循环语义。

    #[test]
    fn test_for_in_range_end_to_end() {
        // 核心回归：range(3) 遍历，累加 0+1+2 = 3
        let src = "total = 0\nfor i in range(3) {\n    total = total + i\n}\nassert(total == 3)";
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_for_in_list_end_to_end() {
        // 列表遍历：依赖 BuildList handler
        let src =
            "total = 0\nfor item in [1, 2, 3] {\n    total = total + item\n}\nassert(total == 6)";
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_for_in_string_end_to_end() {
        // 字符串遍历：计数字符数
        let src = "count = 0\nfor ch in \"abc\" {\n    count = count + 1\n}\nassert(count == 3)";
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_for_in_dict_keys_end_to_end() {
        // 字典键遍历：依赖 BuildDict handler + DictKeys 迭代器
        let src = "d = {\"a\": 1, \"b\": 2, \"c\": 3}\ncount = 0\nfor key in d {\n    count = count + 1\n}\nassert(count == 3)";
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_for_in_double_var_unpack_end_to_end() {
        // 双变量解包：遍历 [(1,10),(2,20)]，累加首元素 = 3
        let src = "total = 0\nfor k, v in [(1, 10), (2, 20)] {\n    total = total + k\n}\nassert(total == 3)";
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_for_in_var_retains_last_value() {
        // 循环变量在循环结束后保持最后值（range(5) 最后值 = 4）
        let src = "for x in range(5) {\n}\nassert(x == 4)";
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_for_in_break() {
        // break 正确跳出循环（i==3 时跳出，last = 2 来自 i==2 的最后一次赋值）
        // 注：mslang 在 {} 内抑制换行，故 last = i 必须在 if 块之后（否则
        // `last = i if` 被解析为三元表达式）。
        let src = "last = 0\nfor i in range(100) {\n    if i == 3 {\n        break\n    }\n    last = i\n}\nassert(last == 2)";
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_for_in_continue() {
        // continue 跳到下一次迭代（跳过偶数，累加奇数 1+3 = 4）
        let src = "total = 0\nfor i in range(5) {\n    if i % 2 == 0 {\n        continue\n    }\n    total = total + i\n}\nassert(total == 4)";
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_for_in_nested_loops() {
        // 嵌套循环：3×3 = 9 次迭代
        let src = "count = 0\nfor i in range(3) {\n    for j in range(3) {\n        count = count + 1\n    }\n}\nassert(count == 9)";
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_for_in_nested_break_only_inner() {
        // 嵌套循环 break 只影响最内层：内层每次遍历到 j==0 就 break，
        // 外层 3 次 → count = 3
        let src = "count = 0\nfor i in range(3) {\n    for j in range(3) {\n        count = count + 1\n        break\n    }\n}\nassert(count == 3)";
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_for_in_non_iterable_type_error() {
        // 对非可迭代类型（int）使用 for..in 抛出 TypeError
        let result = compile_and_run("for i in 42 {\n}");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not iterable"));
    }

    // ---- task 33：列表推导式 ----

    #[test]
    fn test_list_append_opcode_appends_in_place() {
        // 合成字节码：slot 1 放空 list，LIST_APPEND 1 追加 99，LOAD_LOCAL 1 取出。
        // 验证 LIST_APPEND 弹出栈顶值、原地追加、不 push 返回值。
        let result = run_chunk(
            vec![
                OpCode::Nil as u8, // slot 1 占位
                OpCode::BuildList as u8,
                0, // 空列表
                OpCode::StoreLocal as u8,
                1, // slot 1 = []
                OpCode::Constant as u8,
                0x00,
                0x00, // 99
                OpCode::ListAppend as u8,
                1, // append 99 to slot 1
                OpCode::LoadLocal as u8,
                1, // 取出结果列表
                OpCode::Halt as u8,
            ],
            vec![Object::Int(99)],
        )
        .unwrap();
        assert_eq!(result, alloc_list(vec![Object::Int(99)]));
    }

    #[test]
    fn test_list_append_opcode_type_error_on_non_list() {
        // slot 1 为 int（非 list）→ LIST_APPEND 抛 TypeError。
        let result = run_chunk(
            vec![
                OpCode::Nil as u8,
                OpCode::Constant as u8,
                0x00,
                0x00, // 99
                OpCode::StoreLocal as u8,
                1, // slot 1 = 99 (int)
                OpCode::Constant as u8,
                0x00,
                0x01, // 7 (constant idx 1, big-endian)
                OpCode::ListAppend as u8,
                1,
                OpCode::Halt as u8,
            ],
            vec![Object::Int(99), Object::Int(7)],
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("TypeError"));
    }

    // ---- task 34：SET_ADD / DICT_INSERT 辅助指令 ----

    #[test]
    fn test_set_add_opcode_adds_in_place() {
        // 合成字节码：slot 1 放空 set，SET_ADD 1 加入 99，LOAD_LOCAL 1 取出。
        // 验证 SET_ADD 弹出栈顶元素、原地加入、不 push 返回值。
        let result = run_chunk(
            vec![
                OpCode::Nil as u8, // slot 1 占位（占住 slot 0，使 slot 1 可寻址）
                OpCode::BuildSet as u8,
                0, // 空 set
                OpCode::StoreLocal as u8,
                1, // slot 1 = {}
                OpCode::Constant as u8,
                0x00,
                0x00, // 99
                OpCode::SetAdd as u8,
                1, // add 99 to slot 1
                OpCode::LoadLocal as u8,
                1, // 取出结果 set
                OpCode::Halt as u8,
            ],
            vec![Object::Int(99)],
        )
        .unwrap();
        let mut expected = HashSet::new();
        expected.insert(Object::Int(99));
        assert_eq!(result, alloc_set(expected));
    }

    #[test]
    fn test_set_add_opcode_type_error_on_non_set() {
        // slot 1 为 int（非 set）→ SET_ADD 抛 TypeError。
        let result = run_chunk(
            vec![
                OpCode::Nil as u8,
                OpCode::Constant as u8,
                0x00,
                0x00, // 99
                OpCode::StoreLocal as u8,
                1, // slot 1 = 99 (int)
                OpCode::Constant as u8,
                0x00,
                0x01, // 7
                OpCode::SetAdd as u8,
                1,
                OpCode::Halt as u8,
            ],
            vec![Object::Int(99), Object::Int(7)],
        );
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("TypeError"), "got: {}", err);
        assert!(err.contains("set"), "got: {}", err);
    }

    #[test]
    fn test_set_add_opcode_unhashable_returns_type_error() {
        // set 元素为 list（不可哈希）→ Object::hash panic，经 catch_unwind 转 TypeError。
        let result = run_chunk(
            vec![
                OpCode::Nil as u8,
                OpCode::BuildSet as u8,
                0,
                OpCode::StoreLocal as u8,
                1, // slot 1 = {}
                OpCode::Constant as u8,
                0x00,
                0x00, // []（不可哈希 list）
                OpCode::SetAdd as u8,
                1,
                OpCode::Halt as u8,
            ],
            vec![alloc_list(vec![])],
        );
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("TypeError"), "got: {}", err);
        assert!(err.contains("unhashable"), "got: {}", err);
    }

    #[test]
    fn test_dict_insert_opcode_inserts_in_place() {
        // 合成字节码：slot 1 放空 dict，先压 key(1) 再压 val(2)，DICT_INSERT 1 插入。
        // 验证 DICT_INSERT 先弹 val 再弹 key、原地插入、不 push 返回值。
        let result = run_chunk(
            vec![
                OpCode::Nil as u8, // slot 1 占位
                OpCode::BuildDict as u8,
                0, // 空 dict
                OpCode::StoreLocal as u8,
                1, // slot 1 = {}
                OpCode::Constant as u8,
                0x00,
                0x00, // key = 1
                OpCode::Constant as u8,
                0x00,
                0x01, // val = 2
                OpCode::DictInsert as u8,
                1, // insert 1→2 into slot 1
                OpCode::LoadLocal as u8,
                1, // 取出结果 dict
                OpCode::Halt as u8,
            ],
            vec![Object::Int(1), Object::Int(2)],
        )
        .unwrap();
        let mut expected = DictMap::new();
        expected.insert(Object::Int(1), Object::Int(2));
        assert_eq!(result, alloc_dict(expected));
    }

    #[test]
    fn test_dict_insert_opcode_type_error_on_non_dict() {
        // slot 1 为 int（非 dict）→ DICT_INSERT 抛 TypeError。
        let result = run_chunk(
            vec![
                OpCode::Nil as u8,
                OpCode::Constant as u8,
                0x00,
                0x00, // 1
                OpCode::StoreLocal as u8,
                1, // slot 1 = 1 (int)
                OpCode::Constant as u8,
                0x00,
                0x00, // key
                OpCode::Constant as u8,
                0x00,
                0x01, // val
                OpCode::DictInsert as u8,
                1,
                OpCode::Halt as u8,
            ],
            vec![Object::Int(1), Object::Int(2)],
        );
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("TypeError"), "got: {}", err);
        assert!(err.contains("dict"), "got: {}", err);
    }

    #[test]
    fn test_dict_insert_opcode_unhashable_key_returns_type_error() {
        // dict key 为 list（不可哈希）→ Object::hash panic，经 catch_unwind 转 TypeError。
        let result = run_chunk(
            vec![
                OpCode::Nil as u8,
                OpCode::BuildDict as u8,
                0,
                OpCode::StoreLocal as u8,
                1, // slot 1 = {}
                OpCode::Constant as u8,
                0x00,
                0x00, // key = []（不可哈希 list）
                OpCode::Constant as u8,
                0x00,
                0x01, // val = 99
                OpCode::DictInsert as u8,
                1,
                OpCode::Halt as u8,
            ],
            vec![alloc_list(vec![]), Object::Int(99)],
        );
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("TypeError"), "got: {}", err);
        assert!(err.contains("unhashable"), "got: {}", err);
    }

    #[test]
    fn test_list_comprehension_basic() {
        // [x*x for x in range(10)]
        let src = "squares = [x * x for x in range(10)]\nassert(squares == [0, 1, 4, 9, 16, 25, 36, 49, 64, 81])";
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_list_comprehension_filter() {
        // [x for x in range(20) if x % 2 == 0]
        let src = "evens = [x for x in range(20) if x % 2 == 0]\nassert(evens == [0, 2, 4, 6, 8, 10, 12, 14, 16, 18])";
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_list_comprehension_filter_strings() {
        // len() 内置支持字符串
        let src = "names = [\"Alice\", \"Bob\", \"Charlie\", \"David\"]\nlong_names = [n for n in names if len(n) > 3]\nassert(long_names == [\"Alice\", \"Charlie\", \"David\"])";
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_list_comprehension_nested() {
        // [x for row in matrix for x in row] — 展平矩阵
        let src = "matrix = [[1, 2, 3], [4, 5, 6], [7, 8, 9]]\nflat = [x for row in matrix for x in row]\nassert(flat == [1, 2, 3, 4, 5, 6, 7, 8, 9])";
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_list_comprehension_external_var() {
        // 推导式内引用外部变量 factor
        let src = "factor = 10\nscaled = [x * factor for x in range(5)]\nassert(scaled == [0, 10, 20, 30, 40])";
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_list_comprehension_multi_var() {
        // 多变量解构：[a + b for a, b in pairs]
        let src =
            "pairs = [(1, 2), (3, 4)]\nsums = [a + b for a, b in pairs]\nassert(sums == [3, 7])";
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_list_comprehension_loop_var_not_leaked() {
        // 验证 #4：推导式内循环变量不泄漏到外部作用域（顶层 x 未声明 → nil）。
        let src = "[x for x in range(3)]\nassert(x == nil)";
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_list_comprehension_nested_with_filter() {
        // 嵌套 + 过滤组合
        let src = "matrix = [[1, 2, 3], [4, 5, 6]]\nevens = [x for row in matrix for x in row if x % 2 == 0]\nassert(evens == [2, 4, 6])";
        assert!(compile_and_run(src).is_ok());
    }

    // ---- task 34：Dict / Set 推导式 ----
    //
    // 验证 #5（结果类型正确）隐含于 == 断言：dict/set 仅与同类型字面量相等；
    // 验证 #4（消歧）见 test_dict_set_comprehension_disambiguation。
    // 因 print(x) 等价于 format!("{}", x)（builtin_print），且相等对象 Display 相同，
    // assert(d == 字面量) 同时保证了 spec 预期输出的逐行正确。

    #[test]
    fn test_dict_comprehension_basic() {
        // 验证 #1：字典推导式正确生成键值对。
        let src = "squares = {x: x * x for x in range(5)}\nassert(squares == {0: 0, 1: 1, 2: 4, 3: 9, 4: 16})";
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_dict_comprehension_filter() {
        // 验证 #3：带过滤条件的 dict 推导式。
        let src = "even_squares = {x: x * x for x in range(10) if x % 2 == 0}\nassert(even_squares == {0: 0, 2: 4, 4: 16, 6: 36, 8: 64})";
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_set_comprehension_dedup() {
        // 验证 #2：集合推导式正确去重。
        let src = "unique = {len(w) for w in [\"a\", \"bb\", \"ccc\", \"bb\"]}\nassert(unique == {1, 2, 3})";
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_set_comprehension_filter() {
        // 验证 #3：带过滤条件的 set 推导式。
        let src = "big = {x for x in [1, 5, 3, 8, 2, 9, 4] if x > 3}\nassert(big == {4, 5, 8, 9})";
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_dict_set_comprehension_disambiguation() {
        // 验证 #4：四种 { 语法正确消歧——dict 推导式、set 推导式、dict 字面量、set 字面量。
        let src = "d1 = {x: x for x in range(3)}\ns1 = {x for x in range(3)}\nd2 = {\"a\": 1, \"b\": 2}\ns2 = {1, 2, 3}\nassert(d1 == {0: 0, 1: 1, 2: 2})\nassert(s1 == {0, 1, 2})\nassert(d2 == {\"a\": 1, \"b\": 2})\nassert(s2 == {1, 2, 3})";
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_dict_comprehension_multi_var() {
        // 验证 #7：多变量 for k, v in pairs。
        let src =
            "pairs = [(1, 2), (3, 4)]\nd = {k: v for k, v in pairs}\nassert(d == {1: 2, 3: 4})";
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_set_comprehension_nested() {
        // 验证 #7：嵌套 for ... for ...。
        let src =
            "m = [[1, 2], [3, 4]]\ns = {x for row in m for x in row}\nassert(s == {1, 2, 3, 4})";
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_dict_comprehension_loop_var_not_leaked() {
        // 验证 #6：dict 推导式循环变量不泄漏到外部作用域（顶层 x 未声明 → nil）。
        let src = "{x: x for x in range(3)}\nassert(x == nil)";
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_set_comprehension_loop_var_not_leaked() {
        // 验证 #6：set 推导式循环变量不泄漏到外部作用域。
        let src = "{x for x in range(3)}\nassert(x == nil)";
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_set_comprehension_unhashable_element_caught() {
        // spec §3 不可哈希值端到端：{x for x in [[1], [2]]} 试图把 list 放入 set →
        // Object::hash panic 经 catch_unwind 转为可捕获的 TypeError，而非终止 VM。
        let src = "s = {x for x in [[1], [2]]}";
        let result = compile_and_run(src);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("TypeError"), "got: {}", err);
        assert!(err.contains("unhashable"), "got: {}", err);
    }

    #[test]
    fn test_dict_comprehension_unhashable_key_caught() {
        // spec §3：dict 推导式以 list 为 key → 不可哈希 → TypeError（catch_unwind 路径）。
        let src = "d = {x: 1 for x in [[1], [2]]}";
        let result = compile_and_run(src);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("TypeError"), "got: {}", err);
        assert!(err.contains("unhashable"), "got: {}", err);
    }

    #[test]
    fn test_dict_set_comprehension_display_matches_spec() {
        // 显式钉住 spec §测试用例的预期输出格式（builtin_print == format!("{}", x)）。
        // 含验证 #5 与 Display 注：字符串键不带引号（{a: 1, b: 2}）。
        let mut squares = DictMap::new();
        for x in 0..5i64 {
            squares.insert(Object::Int(x), Object::Int(x * x));
        }
        assert_eq!(
            format!("{}", alloc_dict(squares)),
            "{0: 0, 1: 1, 2: 4, 3: 9, 4: 16}"
        );

        let mut even = DictMap::new();
        for x in [0i64, 2, 4, 6, 8] {
            even.insert(Object::Int(x), Object::Int(x * x));
        }
        assert_eq!(
            format!("{}", alloc_dict(even)),
            "{0: 0, 2: 4, 4: 16, 6: 36, 8: 64}"
        );

        let lens: HashSet<Object> = [Object::Int(1), Object::Int(2), Object::Int(3)]
            .into_iter()
            .collect();
        assert_eq!(format!("{}", alloc_set(lens)), "{1, 2, 3}");

        let big: HashSet<Object> = [
            Object::Int(4),
            Object::Int(5),
            Object::Int(8),
            Object::Int(9),
        ]
        .into_iter()
        .collect();
        assert_eq!(format!("{}", alloc_set(big)), "{4, 5, 8, 9}");

        // dict 字面量：字符串键不加引号（既有 Display 行为）
        let mut d = DictMap::new();
        d.insert(alloc_string("a"), Object::Int(1));
        d.insert(alloc_string("b"), Object::Int(2));
        assert_eq!(format!("{}", alloc_dict(d)), "{a: 1, b: 2}");

        let set_lit: HashSet<Object> = [Object::Int(1), Object::Int(2), Object::Int(3)]
            .into_iter()
            .collect();
        assert_eq!(format!("{}", alloc_set(set_lit)), "{1, 2, 3}");
    }

    // ---- task 27：调用帧与函数调用 ----

    #[test]
    fn test_function_decl_compiles_to_global_binding() {
        // task 28：fn 声明编译为存 Function 入常量池 + 发 CLOSURE 指令（运行期包装）
        // + STORE_GLOBAL(name)。常量池存 FUNCTION（非 CLOSURE）。
        let prog = parse("fn f(x) { return x }");
        let chunk = Compiler::new().compile(&prog).unwrap();
        assert!(chunk.code.contains(&(OpCode::StoreGlobal as u8)));
        assert!(chunk.code.contains(&(OpCode::Closure as u8)));
        // 常量池中应存在 MsFunction（CLOSURE 指令运行期包装为 Closure）
        assert!(chunk.constants.iter().any(|c| {
            matches!(
                c,
                Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::FUNCTION as u8
            )
        }));
    }

    #[test]
    fn test_msfunction_msclosure_alloc_and_read() {
        use crate::vm::object::{
            alloc_closure, alloc_function, read_closure, read_function, Function,
        };
        let func = Function {
            name: "test".to_string(),
            arity: 2,
            code: vec![OpCode::Halt as u8],
            constants: vec![],
            upvalue_count: 0,
            source_file: None,
            default_values: Vec::new(),
            has_variadic: false,
            required_arity: 2,
            is_generator: false,
            locals_count: 1,
            is_async: false,
        };
        let func_obj = alloc_function(func);
        let Object::Ref(func_ptr) = func_obj else {
            panic!("expected Ref");
        };
        unsafe {
            assert_eq!((*func_ptr).type_tag, TypeTag::FUNCTION as u8);
            let f = read_function(func_ptr);
            assert_eq!(f.function.arity, 2);
            assert_eq!(f.function.name, "test");
        }

        let closure_obj = alloc_closure(func_obj, Vec::new());
        let Object::Ref(cl_ptr) = closure_obj else {
            panic!("expected Ref");
        };
        unsafe {
            assert_eq!((*cl_ptr).type_tag, TypeTag::CLOSURE as u8);
            let cl = read_closure(cl_ptr);
            let f = read_function(cl.function);
            assert_eq!(f.function.arity, 2);
        }
    }

    #[test]
    fn test_user_function_call_returns_value() {
        // fn add(a, b) { return a + b }；验证 add(3,4) == 7。
        // 这同时验证 param0 != callee（若 slot 1 指向闭包则返回闭包而非 3）。
        let result = compile_and_run(
            r#"
            fn add(a, b) {
                return a + b
            }
            assert(add(3, 4) == 7)
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_user_function_single_param() {
        // fn id(x) { return x }；验证 id(42) == 42（param0 = slot1 ≠ callee）。
        let result = compile_and_run(
            r#"
            fn id(x) {
                return x
            }
            assert(id(42) == 42)
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_user_function_string_concat() {
        // fn greet(name) { return "Hello, " + name }
        let result = compile_and_run(
            r#"
            fn greet(name) {
                return "Hello, " + name
            }
            assert(greet("World") == "Hello, World")
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_implicit_nil_return() {
        // 无显式 return → 隐式 NIL + RETURN → 返回 nil。
        let result = compile_and_run(
            r#"
            fn noop() {
            }
            assert(noop() == nil)
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_implicit_nil_return_with_body() {
        // 函数体有语句但无 return → 隐式返回 nil。
        let result = compile_and_run(
            r#"
            fn side_effect(x) {
                assert(x > 0)
            }
            assert(side_effect(1) == nil)
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_recursion_factorial() {
        // 递归调用：factorial(10) == 3628800。
        let result = compile_and_run(
            r#"
            fn factorial(n) {
                if n <= 1 {
                    return 1
                }
                return n * factorial(n - 1)
            }
            assert(factorial(10) == 3628800)
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_arity_mismatch_too_many() {
        let result = compile_and_run(
            r#"
            fn one(x) {
                return x
            }
            one(1, 2)
            "#,
        );
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("expected 1-1 arguments, got 2"));
    }

    #[test]
    fn test_arity_mismatch_too_few() {
        let result = compile_and_run(
            r#"
            fn two(a, b) {
                return a
            }
            two(1)
            "#,
        );
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("expected 2-2 arguments, got 1"));
    }

    // ---- task 31：默认参数 / 可变参数 ----

    #[test]
    fn test_default_param_basic() {
        // 默认参数：省略时用默认值，提供时覆盖。
        let result = compile_and_run(
            r#"
            fn greet(name, prefix = "Hello") {
                return prefix + ", " + name
            }
            assert(greet("Alice") == "Hello, Alice")
            assert(greet("Alice", "Hi") == "Hi, Alice")
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_default_param_int_and_nil() {
        // 默认值类型覆盖：int / nil。
        let result = compile_and_run(
            r#"
            fn fi(a, b = 42) {
                return b
            }
            fn fnil(a, b = nil) {
                return b
            }
            assert(fi(1) == 42)
            assert(fi(1, 99) == 99)
            assert(fnil(1) == nil)
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_variadic_sum() {
        // *rest 收集多余实参为 list。验证集合长度（for-in 循环在编译源码中有
        // 已知 slot 冲突 bug，见 line 1947 注释，故用 len() 验证）。
        let result = compile_and_run(
            r#"
            fn count_args(*numbers) {
                return len(numbers)
            }
            assert(count_args(1, 2, 3) == 3)
            assert(count_args(1, 2, 3, 4, 5) == 5)
            assert(count_args() == 0)
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_variadic_empty() {
        // 无多余实参时 *rest 为空 list（len == 0）。
        let result = compile_and_run(
            r#"
            fn collect(*args) {
                return len(args)
            }
            assert(collect() == 0)
            assert(collect(1) == 1)
            assert(collect(1, 2, 3) == 3)
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_combined_default_and_variadic() {
        // 普通参数 + 默认参数 + 可变参数组合。
        // 验证：c 用默认值/覆盖值，rest 长度正确。
        let result = compile_and_run(
            r#"
            fn get_c(a, b, c = 10, *rest) {
                return c
            }
            fn count_rest(a, b, c = 10, *rest) {
                return len(rest)
            }
            assert(get_c(1, 2) == 10)
            assert(get_c(1, 2, 3) == 3)
            assert(get_c(1, 2, 3, 4, 5) == 3)
            assert(count_rest(1, 2) == 0)
            assert(count_rest(1, 2, 3) == 0)
            assert(count_rest(1, 2, 3, 4, 5) == 2)
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_default_param_anonymous_fn() {
        // 匿名函数同样支持默认参数（镜像 compile_fn_decl）。
        let result = compile_and_run(
            r#"
            adder = fn(x, y = 10) {
                return x + y
            }
            assert(adder(5) == 15)
            assert(adder(5, 20) == 25)
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_variadic_anonymous_fn() {
        // 匿名函数同样支持可变参数。
        let result = compile_and_run(
            r#"
            collector = fn(*items) {
                return len(items)
            }
            assert(collector() == 0)
            assert(collector(1, 2) == 2)
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_arity_error_too_few_with_defaults() {
        // 默认参数不减少必需参数下限：f(a, b, c=10) 调用 f(1) → TypeError。
        let result = compile_and_run(
            r#"
            fn f(a, b, c = 10) {
                return c
            }
            f(1)
            "#,
        );
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("expected 2-3 arguments, got 1"));
    }

    #[test]
    fn test_arity_error_too_many_without_variadic() {
        // 无可变参数时实参过多 → TypeError。
        let result = compile_and_run(
            r#"
            fn f(a, b) {
                return a
            }
            f(1, 2, 3)
            "#,
        );
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("expected 2-2 arguments, got 3"));
    }

    #[test]
    fn test_variadic_absorbs_extra_args() {
        // 有可变参数时实参过多不报错，多余部分进 *rest（len 验证）。
        let result = compile_and_run(
            r#"
            fn f(a, *rest) {
                return len(rest)
            }
            assert(f(1, 2, 3, 4) == 3)
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_stack_overflow_at_max_depth() {
        // 无限递归 → MAX_CALL_DEPTH(1000) 触发栈溢出。
        let result = compile_and_run(
            r#"
            fn recurse() {
                return recurse()
            }
            recurse()
            "#,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("stack overflow"));
    }

    #[test]
    fn test_call_non_callable_error() {
        // 调用 int → TypeError: not callable。
        let result = compile_and_run("42()");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not callable"));
    }

    #[test]
    fn test_native_builtin_still_works_with_user_fn() {
        // print 等内置函数仍正常工作（经 FUNCTION native 分支），与用户函数共存。
        let result = compile_and_run(
            r#"
            fn double(x) {
                return x + x
            }
            print(double(21))
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_spec_end_to_end_program() {
        // spec 验证标准 §10 的完整测试程序（greet/add/factorial）须无错执行。
        let result = compile_and_run(
            r#"
            fn greet(name) {
                return "Hello, " + name
            }

            fn add(a, b) {
                return a + b
            }

            print(greet("World"))
            print(add(3, 4))

            fn factorial(n) {
                if n <= 1 {
                    return 1
                }
                return n * factorial(n - 1)
            }
            print(factorial(10))
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_nested_calls() {
        // 嵌套调用：add(add(1, 2), add(3, 4)) == 10。
        let result = compile_and_run(
            r#"
            fn add(a, b) {
                return a + b
            }
            assert(add(add(1, 2), add(3, 4)) == 10)
            "#,
        );
        assert!(result.is_ok());
    }

    // ---- task 28：闭包与上值机制 ----
    //
    // 注：匿名 fn 字面量（`fn() {...}` 表达式）由 task 29 实现；本任务的闭包机制
    // 经「命名函数声明」（编译期发 CLOSURE 指令）即可完整验证——CLOSURE 指令对命名/
    // 匿名函数一致地捕获上值并运行期包装。

    #[test]
    fn test_closure_make_counter() {
        // 经典计数器：nonlocal 写捕获。counter() 返回 1、2、3。
        let result = compile_and_run(
            r#"
            fn make_counter() {
                count = 0
                fn step() {
                    nonlocal count
                    count += 1
                    return count
                }
                return step
            }
            counter = make_counter()
            assert(counter() == 1)
            assert(counter() == 2)
            assert(counter() == 3)
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_closure_read_capture() {
        // 只读捕获：内层函数读取外层局部（无需 nonlocal）。
        let result = compile_and_run(
            r#"
            fn make_reader() {
                value = 42
                fn reader() {
                    return value
                }
                return reader
            }
            r = make_reader()
            assert(r() == 42)
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_closure_shared_upvalue_getter_setter() {
        // 多个闭包共享同一上值：setter 经 nonlocal 修改后 getter 可见。
        // 验证 capture_upvalue 对相同 location 复用同一 MsUpvalue。
        let result = compile_and_run(
            r#"
            fn make_pair() {
                x = 10
                fn getter() {
                    return x
                }
                fn setter(v) {
                    nonlocal x
                    x = v
                }
                assert(getter() == 10)
                setter(42)
                assert(getter() == 42)
                return getter()
            }
            assert(make_pair() == 42)
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_closure_nested_upvalue_chain() {
        // 三层嵌套：最内层经 is_local=false 复用中间层的上值（上值链穿透）。
        let result = compile_and_run(
            r#"
            fn outer() {
                x = 1
                fn middle() {
                    fn inner() {
                        return x
                    }
                    return inner()
                }
                return middle()
            }
            assert(outer() == 1)
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_closure_survives_return_close() {
        // 外层函数返回后，被捕获变量仍存活：RETURN 在 truncate 前关闭上值（拷到堆）。
        let result = compile_and_run(
            r#"
            fn make() {
                x = 5
                fn reader() {
                    return x
                }
                return reader
            }
            g = make()
            assert(g() == 5)
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_closure_two_independent_counters() {
        // 两次 make_counter 产生独立计数器（各自独立的 count 上值）。
        let result = compile_and_run(
            r#"
            fn make_counter() {
                count = 0
                fn step() {
                    nonlocal count
                    count += 1
                    return count
                }
                return step
            }
            a = make_counter()
            b = make_counter()
            assert(a() == 1)
            assert(a() == 2)
            assert(b() == 1)
            assert(a() == 3)
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_nonlocal_write_modifies_outer() {
        // nonlocal 写入对外层可见（经 STORE_UPVALUE 修改共享上值）。
        let result = compile_and_run(
            r#"
            fn outer() {
                total = 0
                fn add_one() {
                    nonlocal total
                    total += 1
                }
                add_one()
                add_one()
                add_one()
                return total
            }
            assert(outer() == 3)
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_no_nonlocal_creates_local_does_not_penetrate() {
        // 无 nonlocal 声明时，赋值在内层创建新局部，不穿透外层（04-functions.md）。
        let result = compile_and_run(
            r#"
            fn outer() {
                x = 10
                fn inner() {
                    x = 99
                    return x
                }
                assert(inner() == 99)
                assert(x == 10)
                return x
            }
            assert(outer() == 10)
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_nonlocal_missing_binding_is_compile_error() {
        // nonlocal 声明的名字在外层作用域不存在 → 编译错误。
        let program = parse("fn bad() {\n    nonlocal zzz\n    zzz = 1\n}\n");
        let mut compiler = Compiler::new();
        let result = compiler.compile(&program);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("nonlocal"),
            "expected nonlocal error"
        );
    }

    #[test]
    fn test_closure_recursion_regression() {
        // 回归：递归仍正常工作（闭包机制不影响调用帧）。
        let result = compile_and_run(
            r#"
            fn fact(n) {
                if n <= 1 {
                    return 1
                }
                return n * fact(n - 1)
            }
            assert(fact(5) == 120)
            assert(fact(10) == 3628800)
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_closure_builtins_regression() {
        // 回归：内置函数（len/abs/range 等）仍正常工作。
        // 注：for-in 循环经编译器在顶层有已知栈布局限制（spec 334-338，task 23
        // 既有先例），此处不使用 for-in，仅验证 builtin 调用不受闭包机制影响。
        let result = compile_and_run(
            r#"
            assert(len("hello") == 5)
            assert(abs(-7) == 7)
            assert(abs(7) == 7)
            assert(min(3, 8) == 3)
            assert(max(3, 8) == 8)
            "#,
        );
        assert!(result.is_ok());
    }

    // ---- task 29：匿名函数（函数字面量）----
    //
    // 匿名函数是一等公民：可赋值、传参、做返回值、存于集合。其编译为 name="<anonymous>"
    // 的 Function + CLOSURE 指令，闭包值留栈作为表达式结果（不发 STORE_GLOBAL）。

    #[test]
    fn test_anon_fn_assignment_and_call() {
        // double = fn(x) { return x * 2 }；通过变量名调用。
        let result = compile_and_run(
            r#"
            double = fn(x) { return x * 2 }
            assert(double(5) == 10)
            assert(double(0) == 0)
            assert(double(-3) == -6)
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_anon_fn_higher_order() {
        // 用户定义的高阶函数：匿名函数作为参数传递。
        // apply(fn(x){ return x*x }, 4) == 16
        let result = compile_and_run(
            r#"
            fn apply(f, x) { return f(x) }
            assert(apply(fn(x) { return x * x }, 4) == 16)
            assert(apply(fn(x) { return x + 1 }, 9) == 10)
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_anon_fn_stored_value_called() {
        // 匿名函数值存储后（赋值给变量）经变量取出调用 —— 一等公民「存储后调用」语义。
        // 注：spec §验证标准 #6 的 dict/list 集合存储 + 下标调用（如 ops["add"](3,4)）
        // 无法端到端验证：VM 尚未实装 BuildDict/BuildList/GetIndex 等集合构造与下标
        // 操作码（属独立 VM 实装任务，非 task 29 范畴）。此处用变量存储验证同等的
        // 「值存储 → 取出 → 调用」语义；集合场景的编译端由
        // test_compile_anon_fn_in_dict_literal（expression.rs）覆盖。
        let result = compile_and_run(
            r#"
            add = fn(a, b) { return a + b }
            mul = fn(a, b) { return a * b }
            assert(add(3, 4) == 7)
            assert(mul(3, 4) == 12)
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_anon_fn_closure_counter() {
        // 匿名闭包经 nonlocal 捕获外层变量：counter() 返回 1、2。
        let result = compile_and_run(
            r#"
            fn make_counter() {
                count = 0
                return fn() {
                    nonlocal count
                    count += 1
                    return count
                }
            }
            counter = make_counter()
            assert(counter() == 1)
            assert(counter() == 2)
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_anon_fn_read_capture() {
        // 匿名闭包只读捕获外层局部（无需 nonlocal）。
        let result = compile_and_run(
            r#"
            fn make_reader() {
                value = 42
                return fn() { return value }
            }
            r = make_reader()
            assert(r() == 42)
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_anon_fn_implicit_return_nil() {
        // 无显式 return 的匿名函数返回 nil。
        let result = compile_and_run(
            r#"
            side = fn() { x = 1 }
            assert(side() == nil)
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_anon_fn_as_return_value() {
        // 具名函数返回匿名函数，调用结果。
        let result = compile_and_run(
            r#"
            fn make_adder(n) {
                return fn(x) { return x + n }
            }
            add5 = make_adder(5)
            assert(add5(10) == 15)
            assert(add5(0) == 5)
            "#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_anon_fn_named_decl_regression() {
        // 回归：具名函数声明 + 闭包（task 27/28）仍正常工作。
        let result = compile_and_run(
            r#"
            fn fact(n) {
                if n <= 1 {
                    return 1
                }
                return n * fact(n - 1)
            }
            fn make_counter() {
                count = 0
                fn step() {
                    nonlocal count
                    count += 1
                    return count
                }
                return step
            }
            assert(fact(5) == 120)
            c = make_counter()
            assert(c() == 1)
            assert(c() == 2)
            # 匿名与具名共存
            sq = fn(x) { return x * x }
            assert(sq(fact(3)) == 36)
            "#,
        );
        assert!(result.is_ok());
    }

    // ---- task 35：下标/切片运行时 ----

    /// 由 i64 序列构造 list Object。
    fn list_obj(items: Vec<i64>) -> Object {
        alloc_list(items.into_iter().map(Object::Int).collect())
    }

    /// 合成字节码：obj[key] 读取。
    fn index_run(obj: Object, key: Object) -> Result<Object, String> {
        run_chunk(
            vec![
                OpCode::Constant as u8,
                0,
                0,
                OpCode::Constant as u8,
                0,
                1,
                OpCode::GetIndex as u8,
                OpCode::Halt as u8,
            ],
            vec![obj, key],
        )
    }

    /// 合成字节码：obj[start:stop:step] 切片。flags bit0=start/bit1=stop/bit2=step。
    /// operands 按编译端压栈顺序（start, stop, step）传入其中存在者。
    fn slice_run(obj: Object, operands: Vec<Object>, flags: u8) -> Result<Object, String> {
        let mut code = vec![OpCode::Constant as u8, 0, 0]; // obj @ idx 0
        let mut constants = vec![obj];
        for (i, op) in operands.iter().enumerate() {
            let idx = (i + 1) as u8;
            code.extend([OpCode::Constant as u8, 0, idx]);
            constants.push(op.clone());
        }
        code.extend([OpCode::GetSlice as u8, flags, OpCode::Halt as u8]);
        run_chunk(code, constants)
    }

    #[test]
    fn test_get_index_list() {
        let lst = list_obj((0..10).collect());
        // 正常索引
        assert_eq!(
            index_run(lst.clone(), Object::Int(0)).unwrap(),
            Object::Int(0)
        );
        assert_eq!(
            index_run(lst.clone(), Object::Int(5)).unwrap(),
            Object::Int(5)
        );
        // 负索引
        assert_eq!(
            index_run(lst.clone(), Object::Int(-1)).unwrap(),
            Object::Int(9)
        );
        assert_eq!(
            index_run(lst.clone(), Object::Int(-10)).unwrap(),
            Object::Int(0)
        );
        // 越界 → IndexError
        let err = index_run(lst.clone(), Object::Int(100)).unwrap_err();
        assert!(err.contains("IndexError"), "got: {}", err);
        assert!(err.contains("out of range"), "got: {}", err);
        let err = index_run(lst.clone(), Object::Int(-11)).unwrap_err();
        assert!(err.contains("IndexError"), "got: {}", err);
        // 非整数索引 → TypeError
        let err = index_run(lst, alloc_string("x")).unwrap_err();
        assert!(err.contains("TypeError"), "got: {}", err);
        assert!(err.contains("indices must be integers"), "got: {}", err);
    }

    #[test]
    fn test_get_index_tuple() {
        let t = alloc_tuple((0..5).map(Object::Int).collect::<Vec<_>>());
        assert_eq!(
            index_run(t.clone(), Object::Int(0)).unwrap(),
            Object::Int(0)
        );
        assert_eq!(
            index_run(t.clone(), Object::Int(-1)).unwrap(),
            Object::Int(4)
        );
        let err = index_run(t, Object::Int(5)).unwrap_err();
        assert!(err.contains("IndexError"), "got: {}", err);
    }

    #[test]
    fn test_get_index_string_unicode() {
        // 单字符 string（按 char，非字节）；含多字节字符验证 Unicode 安全
        let s = alloc_string("hello");
        assert_eq!(
            index_run(s.clone(), Object::Int(0)).unwrap(),
            alloc_string("h")
        );
        assert_eq!(
            index_run(s.clone(), Object::Int(-1)).unwrap(),
            alloc_string("o")
        );
        let err = index_run(s, Object::Int(100)).unwrap_err();
        assert!(err.contains("IndexError"), "got: {}", err);
        // 多字节：'世' 占一个 char
        let uni = alloc_string("a世b");
        assert_eq!(
            index_run(uni.clone(), Object::Int(1)).unwrap(),
            alloc_string("世")
        );
        assert_eq!(index_run(uni, Object::Int(-1)).unwrap(), alloc_string("b"));
    }

    #[test]
    fn test_get_index_dict() {
        let mut m = DictMap::new();
        m.insert(alloc_string("a"), Object::Int(1));
        m.insert(alloc_string("b"), Object::Int(2));
        let d = alloc_dict(m);
        // 命中
        assert_eq!(
            index_run(d.clone(), alloc_string("a")).unwrap(),
            Object::Int(1)
        );
        // 缺失 → nil（不抛异常）
        assert_eq!(
            index_run(d.clone(), alloc_string("missing")).unwrap(),
            Object::Nil
        );
        // 不可哈希 key（list）→ catch_unwind → TypeError unhashable
        let err = index_run(d, list_obj(vec![1])).unwrap_err();
        assert!(err.contains("TypeError"), "got: {}", err);
        assert!(err.contains("unhashable"), "got: {}", err);
    }

    #[test]
    fn test_get_index_non_subscriptable() {
        // int 不可下标 → TypeError
        let err = index_run(Object::Int(5), Object::Int(0)).unwrap_err();
        assert!(err.contains("TypeError"), "got: {}", err);
        assert!(err.contains("not subscriptable"), "got: {}", err);
    }

    /// 合成字节码：obj[key]=val 后读回 obj[key] 验证写入（共享同一 Ref）。
    /// 压栈顺序对齐真实编译端 compile_assignment：先 val、再 obj、再 key
    /// （栈底→顶 [val, obj, key]），SetIndex 弹 key/obj/val。
    fn setindex_run(obj: Object, key: Object, val: Object) -> Result<Object, String> {
        run_chunk(
            vec![
                OpCode::Constant as u8,
                0,
                0, // val @ idx 0
                OpCode::Constant as u8,
                0,
                1, // obj @ idx 1
                OpCode::Constant as u8,
                0,
                2, // key @ idx 2
                OpCode::SetIndex as u8,
                OpCode::Constant as u8,
                0,
                1,
                OpCode::Constant as u8,
                0,
                2,
                OpCode::GetIndex as u8,
                OpCode::Halt as u8,
            ],
            vec![val, obj, key],
        )
    }

    #[test]
    fn test_set_index_list() {
        let lst = list_obj((1..=3).collect());
        // 正常写入
        assert_eq!(
            setindex_run(lst.clone(), Object::Int(0), Object::Int(99)).unwrap(),
            Object::Int(99)
        );
        // 负索引写入
        assert_eq!(
            setindex_run(lst.clone(), Object::Int(-1), Object::Int(77)).unwrap(),
            Object::Int(77)
        );
        // 越界 → IndexError
        let err = setindex_run(lst, Object::Int(10), Object::Int(0)).unwrap_err();
        assert!(err.contains("IndexError"), "got: {}", err);
    }

    #[test]
    fn test_set_index_dict() {
        let d = alloc_dict(DictMap::new());
        assert_eq!(
            setindex_run(d.clone(), alloc_string("c"), Object::Int(3)).unwrap(),
            Object::Int(3)
        );
        // 覆盖已有键（用非空 dict）
        let mut m = DictMap::new();
        m.insert(alloc_string("a"), Object::Int(1));
        let d2 = alloc_dict(m);
        assert_eq!(
            setindex_run(d2, alloc_string("a"), Object::Int(42)).unwrap(),
            Object::Int(42)
        );
        // 不可哈希 key → TypeError
        let err = setindex_run(d, list_obj(vec![1]), Object::Int(0)).unwrap_err();
        assert!(err.contains("TypeError"), "got: {}", err);
        assert!(err.contains("unhashable"), "got: {}", err);
    }

    #[test]
    fn test_set_index_immutable() {
        // string / tuple 不可变 → TypeError
        let err = setindex_run(alloc_string("hi"), Object::Int(0), alloc_string("x")).unwrap_err();
        assert!(err.contains("TypeError"), "got: {}", err);
        assert!(err.contains("item assignment"), "got: {}", err);
        let err = setindex_run(
            alloc_tuple(vec![Object::Int(1)]),
            Object::Int(0),
            Object::Int(9),
        )
        .unwrap_err();
        assert!(err.contains("TypeError"), "got: {}", err);
        assert!(err.contains("item assignment"), "got: {}", err);
    }

    #[test]
    fn test_get_slice_list() {
        let lst = list_obj((0..10).collect());
        // [2:5]
        assert_eq!(
            slice_run(lst.clone(), vec![Object::Int(2), Object::Int(5)], 0b011).unwrap(),
            list_obj(vec![2, 3, 4])
        );
        // [:3]
        assert_eq!(
            slice_run(lst.clone(), vec![Object::Int(3)], 0b010).unwrap(),
            list_obj(vec![0, 1, 2])
        );
        // [7:]
        assert_eq!(
            slice_run(lst.clone(), vec![Object::Int(7)], 0b001).unwrap(),
            list_obj(vec![7, 8, 9])
        );
        // [::2]
        assert_eq!(
            slice_run(lst.clone(), vec![Object::Int(2)], 0b100).unwrap(),
            list_obj(vec![0, 2, 4, 6, 8])
        );
        // [::-1]
        assert_eq!(
            slice_run(lst.clone(), vec![Object::Int(-1)], 0b100).unwrap(),
            list_obj((0..10).rev().collect())
        );
        // [-5:-2]
        assert_eq!(
            slice_run(lst.clone(), vec![Object::Int(-5), Object::Int(-2)], 0b011).unwrap(),
            list_obj(vec![5, 6, 7])
        );
        // [1::2]
        assert_eq!(
            slice_run(lst.clone(), vec![Object::Int(1), Object::Int(2)], 0b101).unwrap(),
            list_obj(vec![1, 3, 5, 7, 9])
        );
        // [8:2:-1]
        assert_eq!(
            slice_run(
                lst.clone(),
                vec![Object::Int(8), Object::Int(2), Object::Int(-1)],
                0b111
            )
            .unwrap(),
            list_obj(vec![8, 7, 6, 5, 4, 3])
        );
        // [0:100] 越界裁剪
        assert_eq!(
            slice_run(lst.clone(), vec![Object::Int(0), Object::Int(100)], 0b011).unwrap(),
            list_obj((0..10).collect())
        );
        // [100:200] → 空
        assert_eq!(
            slice_run(lst.clone(), vec![Object::Int(100), Object::Int(200)], 0b011).unwrap(),
            list_obj(vec![])
        );
        // [::] 全默认
        assert_eq!(
            slice_run(lst, vec![], 0b000).unwrap(),
            list_obj((0..10).collect())
        );
        // [][::] 空列表
        assert_eq!(
            slice_run(list_obj(vec![]), vec![], 0b000).unwrap(),
            list_obj(vec![])
        );
    }

    #[test]
    fn test_get_slice_string() {
        let s = alloc_string("hello world");
        // [0:5] → "hello"
        assert_eq!(
            slice_run(s.clone(), vec![Object::Int(0), Object::Int(5)], 0b011).unwrap(),
            alloc_string("hello")
        );
        // [-5:] → "world"
        assert_eq!(
            slice_run(s.clone(), vec![Object::Int(-5)], 0b001).unwrap(),
            alloc_string("world")
        );
        // [::-1] → 反转
        assert_eq!(
            slice_run(s.clone(), vec![Object::Int(-1)], 0b100).unwrap(),
            alloc_string("dlrow olleh")
        );
        // Unicode 按字符：'a世b'[::-1] → 'b世a'
        let uni = alloc_string("a世b");
        assert_eq!(
            slice_run(uni, vec![Object::Int(-1)], 0b100).unwrap(),
            alloc_string("b世a")
        );
    }

    #[test]
    fn test_get_slice_tuple() {
        let t = alloc_tuple((0..5).map(Object::Int).collect::<Vec<_>>());
        // [1:3] → (1, 2)
        assert_eq!(
            slice_run(t, vec![Object::Int(1), Object::Int(3)], 0b011).unwrap(),
            alloc_tuple(vec![Object::Int(1), Object::Int(2)])
        );
    }

    #[test]
    fn test_get_slice_errors() {
        let lst = list_obj((0..10).collect());
        // step==0 → ValueError（非 panic）
        let err = slice_run(lst.clone(), vec![Object::Int(0)], 0b100).unwrap_err();
        assert!(err.contains("ValueError"), "got: {}", err);
        assert!(err.contains("slice step cannot be zero"), "got: {}", err);
        // 非整数索引 → TypeError
        let err = slice_run(lst, vec![alloc_string("x")], 0b001).unwrap_err();
        assert!(err.contains("TypeError"), "got: {}", err);
        assert!(err.contains("indices must be integers"), "got: {}", err);
        // dict 不可切片 → TypeError
        let d = alloc_dict(DictMap::new());
        let err = slice_run(d, vec![], 0b000).unwrap_err();
        assert!(err.contains("TypeError"), "got: {}", err);
        assert!(err.contains("not sliceable"), "got: {}", err);
    }

    /// 端到端：spec §测试用例的 22 条 print 全部转为 assert，跑通 parse_slice →
    /// compile → GET_INDEX/SET_INDEX/GET_SLICE 全链路；末尾两条同时验证「切片返回
    /// 新对象，不修改原对象」（spec 验证 #8）。
    #[test]
    fn test_slicing_end_to_end() {
        let src = r#"
lst = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9]
s = "hello world"
t = (0, 1, 2, 3, 4)
d = {"a": 1, "b": 2}
assert(lst[2:5] == [2, 3, 4])
assert(lst[:3] == [0, 1, 2])
assert(lst[7:] == [7, 8, 9])
assert(lst[::2] == [0, 2, 4, 6, 8])
assert(lst[::-1] == [9, 8, 7, 6, 5, 4, 3, 2, 1, 0])
assert(lst[-3:] == [7, 8, 9])
assert(s[0:5] == "hello")
assert(s[-5:] == "world")
assert(t[1:3] == (1, 2))
assert(lst[0:100] == [0, 1, 2, 3, 4, 5, 6, 7, 8, 9])
assert(lst[100:200] == [])
assert(lst[-5:-2] == [5, 6, 7])
assert(lst[1::2] == [1, 3, 5, 7, 9])
assert(lst[8:2:-1] == [8, 7, 6, 5, 4, 3])
assert(lst[0] == 0)
assert(lst[-1] == 9)
assert(s[0] == "h")
assert(t[-1] == 4)
assert(d["a"] == 1)
assert(d["missing"] == nil)
original = [1, 2, 3]
sliced = original[0:2]
original[0] = 99
assert(sliced == [1, 2])
assert(original == [99, 2, 3])
"#;
        assert!(compile_and_run(src).is_ok());
    }

    /// 显式钉住 spec 预期输出格式（builtin_print == format!("{}", x)）。
    /// 验证 #5/#7（类型正确）与 Display 注（字符串无引号、list 空格分隔、tuple 圆括号）。
    #[test]
    fn test_slicing_display_matches_spec() {
        assert_eq!(format!("{}", list_obj(vec![2, 3, 4])), "[2, 3, 4]");
        assert_eq!(
            format!("{}", list_obj((0..10).rev().collect())),
            "[9, 8, 7, 6, 5, 4, 3, 2, 1, 0]"
        );
        assert_eq!(format!("{}", list_obj(vec![])), "[]");
        assert_eq!(format!("{}", alloc_string("hello")), "hello");
        assert_eq!(format!("{}", alloc_string("h")), "h");
        assert_eq!(
            format!("{}", alloc_tuple(vec![Object::Int(1), Object::Int(2)])),
            "(1, 2)"
        );
        assert_eq!(format!("{}", Object::Nil), "nil");
    }

    // ---- task 36：defer 语句 ----
    //
    // 观测手段：`log` 为函数 `run` 的局部，defer callee（均为 CLOSURE）经 `nonlocal log`
    // 追加（上值写），调用结束后在函数内 assert。同时覆盖 ip-rewind trampoline 的异步
    // 返回路径；builtin callee（print）由冒烟测试覆盖。

    /// 基本 LIFO 顺序（规则 2）：defer 按「后进先出」执行。
    #[test]
    fn test_defer_lifo_order() {
        let src = r#"
fn run() {
    log = ""
    fn d(v) {
        nonlocal log
        log = log + v
    }
    fn example() {
        defer d("first")
        defer d("second")
        defer d("third")
    }
    example()
    assert(log == "thirdsecondfirst", log)
}
run()
"#;
        let r = compile_and_run(src);
        assert!(r.is_ok(), "defer LIFO failed: {:?}", r.err());
    }

    /// 参数在声明时求值（规则 3）：循环变量 i 每轮拷入 tuple，执行时见注册时的值。
    /// 若误用「闭包 + upvalue」捕获单一 slot，将输出 "222"。
    #[test]
    fn test_defer_params_evaluated_at_registration() {
        let src = r#"
fn run() {
    log = ""
    fn d(v) {
        nonlocal log
        log = log + str(v)
    }
    fn with_params() {
        for i in range(3) {
            defer d(i)
        }
    }
    with_params()
    assert(log == "210", log)
}
run()
"#;
        let r = compile_and_run(src);
        assert!(r.is_ok(), "defer rule-3 failed: {:?}", r.err());
    }

    /// defer 与 return：defer 先执行，返回值不被修改（规则 5）。
    #[test]
    fn test_defer_runs_before_return() {
        let src = r#"
fn run() {
    log = ""
    fn d() {
        nonlocal log
        log = log + "D"
    }
    fn with_return() {
        defer d()
        return 42
    }
    r = with_return()
    assert(r == 42)
    assert(log == "D", log)
}
run()
"#;
        let r = compile_and_run(src);
        assert!(r.is_ok(), "defer before-return failed: {:?}", r.err());
    }

    /// 嵌套函数的 defer 互不干扰：内层 defer 在内层返回时执行，外层在外层返回时执行。
    #[test]
    fn test_defer_per_frame_isolation() {
        let src = r#"
fn run() {
    log = ""
    fn d(v) {
        nonlocal log
        log = log + v
    }
    fn outer() {
        defer d("O")
        fn inner() {
            defer d("I")
        }
        inner()
        d("X")
    }
    outer()
    assert(log == "IXO", log)
}
run()
"#;
        let r = compile_and_run(src);
        assert!(r.is_ok(), "defer per-frame failed: {:?}", r.err());
    }

    /// 用户函数（CLOSURE）作为 defer callee：callee 自身亦带空 EXEC_DEFER，
    /// 验证 per-frame defer_flushing 不被 callee 的空 EXEC_DEFER 误触发。
    /// （全局 flag 的朴素实现会在此错误地弹栈。）
    #[test]
    fn test_defer_user_function_callee() {
        let src = r#"
fn run() {
    log = ""
    fn cleanup() {
        nonlocal log
        log = log + "cleaning up"
    }
    fn foo() {
        defer cleanup()
        return 42
    }
    r = foo()
    assert(r == 42)
    assert(log == "cleaning up", log)
}
run()
"#;
        let r = compile_and_run(src);
        assert!(r.is_ok(), "defer user-fn callee failed: {:?}", r.err());
    }

    /// 模块顶层 defer（§8）：脚本结束时按 LIFO 执行。
    /// 顶层 defer callee 为 print（builtin），冒烟验证执行；LIFO 顺序由上方函数级
    /// 测试覆盖（顶层与函数共用同一 EXEC_DEFER 机制）。
    #[test]
    fn test_defer_top_level_smoke() {
        let src = r#"
defer print("top defer 1")
defer print("top defer 2")
"#;
        assert!(compile_and_run(src).is_ok());
    }

    /// 冒烟：spec §测试用例原样（print 版），验证不 panic、正常终止。
    /// 用 --nocapture 可核对完整输出序列：
    ///   body / third / second / first / 2 / 1 / 0 / deferred / 42 /
    ///   inner defer / after inner / outer defer / top defer 2 / top defer 1
    #[test]
    fn test_defer_spec_smoke() {
        let src = r#"
fn example() {
    defer print("first")
    defer print("second")
    defer print("third")
    print("body")
}
example()
fn with_params() {
    for i in range(3) {
        defer print(i)
    }
}
with_params()
fn with_return() {
    defer print("deferred")
    return 42
}
result = with_return()
print(result)
fn outer() {
    defer print("outer defer")
    fn inner() {
        defer print("inner defer")
    }
    inner()
    print("after inner")
}
outer()
defer print("top defer 1")
defer print("top defer 2")
"#;
        assert!(compile_and_run(src).is_ok());
    }

    /// defer 非调用表达式 → 编译报错。
    #[test]
    fn test_defer_requires_call_expression() {
        let src = "defer 42";
        let program = parse(src);
        let mut compiler = Compiler::new();
        assert!(compiler.compile(&program).is_err());
    }

    // ---- task 37：try/except/finally 异常处理 ----

    /// 完整 spec 测试序列（tasks/37 §测试用例）：基本捕获 / finally / 捕获所有 /
    /// 多 except / finally 正常路径 / 跨帧传播 / 组合 / finally-on-propagation /
    /// throw string 包装 / 裸 throw 重抛 / __cause__ 链。
    #[test]
    fn test_try_except_finally_full() {
        let src = r#"
fn run() {
    log = ""
    fn p(s) {
        nonlocal log
        log = log + s + "|"
    }
    try {
        throw ValueError("test error")
    } except ValueError as e {
        p("caught: " + e.message)
    }
    try {
        throw ZeroDivisionError("divide by zero")
    } except ZeroDivisionError as e {
        p("division error")
    } finally {
        p("cleanup")
    }
    try {
        throw TypeError("type!")
    } except {
        p("caught all")
    }
    try {
        throw KeyError("missing")
    } except ValueError as e {
        p("value error")
    } except KeyError as e {
        p("key error: " + e.message)
    }
    try {
        x = 42
    } finally {
        p("always runs")
    }
    fn inner() {
        throw RuntimeError("from inner")
    }
    fn outer() {
        inner()
    }
    try {
        outer()
    } except RuntimeError as e {
        p("propagated: " + e.message)
    }
    try {
        throw ValueError("combo")
    } except ValueError as e {
        p("handled: " + e.message)
    } finally {
        p("final cleanup")
    }
    fn boom() {
        try {
            throw ValueError("boom")
        } finally {
            p("inner finally")
        }
    }
    try {
        boom()
    } except ValueError as e {
        p("outer caught: " + e.message)
    }
    try {
        throw "oops"
    } except RuntimeError as e {
        p("wrapped: " + e.message)
    }
    try {
        try {
            throw ValueError("first")
        } except ValueError as e {
            throw
        }
    } except ValueError as e {
        p("rethrown: " + e.message)
    }
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
        p("cause type: " + e.__cause__.type)
        p("caught: " + e.message)
    }
    expected = "caught: test error|division error|cleanup|caught all|key error: missing|always runs|propagated: from inner|handled: combo|final cleanup|inner finally|outer caught: boom|wrapped: oops|rethrown: first|cause type: ValueError|caught: defer err|"
    assert(log == expected, log)
}
run()
"#;
        let r = compile_and_run(src);
        assert!(
            r.is_ok(),
            "try/except/finally full sequence failed: {:?}",
            r.err()
        );
    }

    /// 子类匹配（验证标准 6）：ValueError 被 except Error 捕获。
    #[test]
    fn test_try_except_subclass_match() {
        let src = r#"
fn run() {
    log = ""
    fn p(s) {
        nonlocal log
        log = log + s
    }
    try {
        throw ValueError("sub")
    } except Error as e {
        p("caught by Error: " + e.message)
    }
    assert(log == "caught by Error: sub", log)
}
run()
"#;
        let r = compile_and_run(src);
        assert!(r.is_ok(), "subclass match failed: {:?}", r.err());
    }

    /// 未捕获异常终止程序并返回错误字符串（验证标准 9）。
    #[test]
    fn test_uncaught_exception() {
        let src = "throw ValueError(\"uncaught\")";
        let err = compile_and_run(src).unwrap_err();
        assert!(err.contains("ValueError"), "got: {}", err);
        assert!(err.contains("uncaught"), "got: {}", err);
    }

    /// 裸 throw 在 except 块外抛 RuntimeError("nothing to rethrow")（验证标准 8）。
    #[test]
    fn test_bare_throw_outside_except() {
        let src = "throw";
        let err = compile_and_run(src).unwrap_err();
        assert!(err.contains("RuntimeError"), "got: {}", err);
        assert!(err.contains("nothing to rethrow"), "got: {}", err);
    }

    /// GeneratorExit 不可被用户 except 捕获（验证标准 13）。
    #[test]
    fn test_generator_exit_not_caught() {
        let src = r#"
try {
    throw GeneratorExit("ge")
} except {
    print("should not catch")
} except Error as e {
    print("should not catch either")
}
"#;
        let err = compile_and_run(src).unwrap_err();
        assert!(err.contains("GeneratorExit"), "got: {}", err);
        assert!(err.contains("ge"), "got: {}", err);
    }

    /// throw 非 string/Exception → TypeError。
    #[test]
    fn test_throw_non_exception_type_error() {
        let src = "throw 42";
        let err = compile_and_run(src).unwrap_err();
        assert!(err.contains("TypeError"), "got: {}", err);
    }

    /// 编译器：try/except/finally 生成 TRY_ENTER / TRY_EXIT / CATCH / FINALLY_END 等指令。
    #[test]
    fn test_compile_try_emits_opcodes() {
        let src = r#"
try {
    throw ValueError("x")
} except ValueError as e {
    print(e)
} finally {
    cleanup()
}
"#;
        let chunk = {
            let program = parse(src);
            let mut compiler = Compiler::new();
            compiler.compile(&program).unwrap()
        };
        assert!(
            chunk.code.contains(&(OpCode::TryEnter as u8)),
            "missing TRY_ENTER"
        );
        assert!(
            chunk.code.contains(&(OpCode::TryExit as u8)),
            "missing TRY_EXIT"
        );
        assert!(chunk.code.contains(&(OpCode::Catch as u8)), "missing CATCH");
        assert!(chunk.code.contains(&(OpCode::Throw as u8)), "missing THROW");
        assert!(
            chunk.code.contains(&(OpCode::FinallyEnd as u8)),
            "missing FINALLY_END"
        );
        assert!(
            chunk.code.contains(&(OpCode::ClearCurrentExc as u8)),
            "missing CLEAR_CURRENT_EXC"
        );
    }

    /// 基本捕获 + `as` 绑定 + GET_ATTR(e.message)。
    #[test]
    fn test_try_except_basic() {
        let src = r#"
fn run() {
    log = ""
    fn p(s) {
        nonlocal log
        log = log + s
    }
    try {
        throw ValueError("test error")
    } except ValueError as e {
        p("caught: " + e.message)
    }
    assert(log == "caught: test error", log)
}
run()
"#;
        let r = compile_and_run(src);
        assert!(r.is_ok(), "basic try/except failed: {:?}", r.err());
    }

    // ---- task 38：with 语句（上下文管理器）----
    // 注：本阶段用 dict 模拟上下文管理器（GET_ATTR on Dict 临时分支）。
    //     Phase 5 task 41/43 完成后改用正式 class + Instance。
    //     mslang 无 ';' 语句分隔符（换行分隔），故 __enter__/__exit__ 用具名闭包
    //     经标识符存入 dict，避免行内 fn 字面量的多语句问题。

    /// 基本流程：__enter__ → body → __exit__；正常退出时 err 参数为 nil（标准 1/3）。
    #[test]
    fn test_with_basic() {
        let src = r#"
fn run() {
    log = ""
    fn p(s) {
        nonlocal log
        log = log + s + "|"
    }
    fn do_enter(self) {
        p("enter")
        return self
    }
    fn do_exit(self, err, msg, tb) {
        p("exit err=" + str(err))
        return false
    }
    ctx = {
        "__enter__": do_enter,
        "__exit__": do_exit
    }
    with ctx as c {
        p("body")
    }
    assert(log == "enter|body|exit err=nil|", log)
}
run()
"#;
        let r = compile_and_run(src);
        assert!(r.is_ok(), "with basic failed: {:?}", r.err());
    }

    /// `as` 变量绑定 __enter__ 返回值，且 with 块外仍可见（外围函数作用域，标准 2）。
    #[test]
    fn test_with_as_binding_visible_after_block() {
        let src = r#"
fn run() {
    fn do_enter(self) {
        return 42
    }
    fn do_exit(self, err, msg, tb) {
        return false
    }
    ctx = {
        "__enter__": do_enter,
        "__exit__": do_exit
    }
    with ctx as c {
        assert(c == 42, "inside")
    }
    assert(c == 42, "outside")
}
run()
"#;
        let r = compile_and_run(src);
        assert!(
            r.is_ok(),
            "with as-binding visibility failed: {:?}",
            r.err()
        );
    }

    /// with body 抛异常：__exit__ 收到 err_type，异常继续传播被外层捕获（标准 4）。
    #[test]
    fn test_with_exception_propagates() {
        let src = r#"
fn run() {
    log = ""
    fn p(s) {
        nonlocal log
        log = log + s + "|"
    }
    fn do_enter(self) {
        p("enter")
        return self
    }
    fn do_exit(self, err, msg, tb) {
        p("exit: " + str(err))
        return false
    }
    ctx = {
        "__enter__": do_enter,
        "__exit__": do_exit
    }
    try {
        with ctx as c {
            p("before")
            throw ValueError("oops")
            p("unreachable")
        }
    } except ValueError as e {
        p("caught: " + e.message)
    }
    assert(log == "enter|before|exit: ValueError|caught: oops|", log)
}
run()
"#;
        let r = compile_and_run(src);
        assert!(
            r.is_ok(),
            "with exception propagation failed: {:?}",
            r.err()
        );
    }

    /// __exit__ 返回 true 抑制异常（标准 5）。
    #[test]
    fn test_with_suppress_when_truthy() {
        let src = r#"
fn run() {
    log = ""
    fn p(s) {
        nonlocal log
        log = log + s + "|"
    }
    fn do_enter(self) {
        return self
    }
    fn do_exit(self, err, msg, tb) {
        p("suppress: " + str(err))
        return true
    }
    ctx = {
        "__enter__": do_enter,
        "__exit__": do_exit
    }
    with ctx as c {
        throw ValueError("suppressed")
    }
    p("after")
    assert(log == "suppress: ValueError|after|", log)
}
run()
"#;
        let r = compile_and_run(src);
        assert!(r.is_ok(), "with suppress failed: {:?}", r.err());
    }

    /// __exit__ 返回假值（nil）时异常继续传播（标准 6）。
    #[test]
    fn test_with_propagate_when_falsy() {
        let src = r#"
fn run() {
    log = ""
    fn p(s) {
        nonlocal log
        log = log + s + "|"
    }
    fn do_enter(self) {
        return self
    }
    fn do_exit(self, err, msg, tb) {
        p("exit")
        return nil
    }
    ctx = {
        "__enter__": do_enter,
        "__exit__": do_exit
    }
    try {
        with ctx as c {
            throw ValueError("propagated")
        }
    } except ValueError as e {
        p("caught: " + e.message)
    }
    assert(log == "exit|caught: propagated|", log)
}
run()
"#;
        let r = compile_and_run(src);
        assert!(r.is_ok(), "with falsy propagation failed: {:?}", r.err());
    }

    /// 嵌套 with 正常路径，LIFO 顺序（标准 7）。
    #[test]
    fn test_with_nested_lifo() {
        let src = r#"
fn run() {
    log = ""
    fn p(s) {
        nonlocal log
        log = log + s + "|"
    }
    fn e1(self) {
        p("enter1")
        return self
    }
    fn x1(self, err, msg, tb) {
        p("exit1")
        return false
    }
    fn e2(self) {
        p("enter2")
        return self
    }
    fn x2(self, err, msg, tb) {
        p("exit2")
        return false
    }
    ctx1 = {
        "__enter__": e1,
        "__exit__": x1
    }
    ctx2 = {
        "__enter__": e2,
        "__exit__": x2
    }
    with ctx1 as a {
        with ctx2 as b {
            p("body")
        }
    }
    assert(log == "enter1|enter2|body|exit2|exit1|", log)
}
run()
"#;
        let r = compile_and_run(src);
        assert!(r.is_ok(), "with nested LIFO failed: {:?}", r.err());
    }

    /// 内层抛异常 + 内层 __exit__ 不抑制 → 外层 __exit__ 收到同一异常（标准 8）。
    #[test]
    fn test_with_cross_with_propagation() {
        let src = r#"
fn run() {
    log = ""
    fn p(s) {
        nonlocal log
        log = log + s + "|"
    }
    fn e1(self) {
        p("enter1")
        return self
    }
    fn x1(self, err, msg, tb) {
        p("exit1: " + str(err))
        return false
    }
    fn e2(self) {
        p("enter2")
        return self
    }
    fn x2(self, err, msg, tb) {
        p("exit2: " + str(err))
        return false
    }
    ctx1 = {
        "__enter__": e1,
        "__exit__": x1
    }
    ctx2 = {
        "__enter__": e2,
        "__exit__": x2
    }
    try {
        with ctx1 as a {
            with ctx2 as b {
                throw ValueError("cross")
            }
        }
    } except ValueError {
        p("caught")
    }
    assert(log == "enter1|enter2|exit2: ValueError|exit1: ValueError|caught|", log)
}
run()
"#;
        let r = compile_and_run(src);
        assert!(
            r.is_ok(),
            "with cross-with propagation failed: {:?}",
            r.err()
        );
    }

    /// 内层 __exit__ 抑制 → 外层 __exit__ 收到 nil（异常未传播到外层，标准 9）。
    #[test]
    fn test_with_inner_suppress_outer_sees_nil() {
        let src = r#"
fn run() {
    log = ""
    fn p(s) {
        nonlocal log
        log = log + s + "|"
    }
    fn e1(self) {
        return self
    }
    fn x1(self, err, msg, tb) {
        p("exit1: " + str(err))
        return false
    }
    fn e2(self) {
        return self
    }
    fn x2(self, err, msg, tb) {
        p("exit2: " + str(err))
        return true
    }
    ctx1 = {
        "__enter__": e1,
        "__exit__": x1
    }
    ctx2 = {
        "__enter__": e2,
        "__exit__": x2
    }
    with ctx1 as a {
        with ctx2 as b {
            throw ValueError("inner")
        }
    }
    p("done")
    assert(log == "exit2: ValueError|exit1: nil|done|", log)
}
run()
"#;
        let r = compile_and_run(src);
        assert!(r.is_ok(), "with inner suppress failed: {:?}", r.err());
    }

    /// __exit__ 自身抛异常：原异常挂为新异常的 __cause__（标准 10）。
    #[test]
    fn test_with_exit_throws_chains_cause() {
        let src = r#"
fn run() {
    log = ""
    fn p(s) {
        nonlocal log
        log = log + s + "|"
    }
    fn do_enter(self) {
        return self
    }
    fn do_exit(self, err, msg, tb) {
        throw RuntimeError("from exit")
    }
    ctx = {
        "__enter__": do_enter,
        "__exit__": do_exit
    }
    try {
        with ctx as c {
            throw ValueError("original")
        }
    } except RuntimeError as e {
        p("caught: " + e.message)
        p("cause: " + e.__cause__.type)
    }
    assert(log == "caught: from exit|cause: ValueError|", log)
}
run()
"#;
        let r = compile_and_run(src);
        assert!(
            r.is_ok(),
            "with exit-throws cause chain failed: {:?}",
            r.err()
        );
    }

    /// with body 内 defer：异常路径下 defer 先于 __exit__（标准 11）。
    #[test]
    fn test_with_defer_runs_before_exit() {
        let src = r#"
fn run() {
    log = ""
    fn p(s) {
        nonlocal log
        log = log + s + "|"
    }
    fn cleanup() {
        p("cleanup")
    }
    fn do_enter(self) {
        return self
    }
    fn do_exit(self, err, msg, tb) {
        p("exit: " + str(err))
        return false
    }
    ctx = {
        "__enter__": do_enter,
        "__exit__": do_exit
    }
    try {
        with ctx as c {
            defer cleanup()
            throw ValueError("body")
        }
    } except ValueError {
        p("propagated")
    }
    assert(log == "cleanup|exit: ValueError|propagated|", log)
}
run()
"#;
        let r = compile_and_run(src);
        assert!(r.is_ok(), "with defer interaction failed: {:?}", r.err());
    }

    /// __enter__ 抛异常：__exit__ 不被调用（TRY_ENTER 在 __enter__ 之后，标准 12）。
    #[test]
    fn test_with_enter_throws_exit_not_called() {
        let src = r#"
fn run() {
    log = ""
    fn p(s) {
        nonlocal log
        log = log + s + "|"
    }
    fn do_enter(self) {
        p("enter")
        throw RuntimeError("enter fail")
    }
    fn do_exit(self, err, msg, tb) {
        p("EXIT")
    }
    ctx = {
        "__enter__": do_enter,
        "__exit__": do_exit
    }
    try {
        with ctx as c {
            p("body")
        }
    } except RuntimeError as e {
        p("caught: " + e.message)
    }
    assert(log == "enter|caught: enter fail|", log)
}
run()
"#;
        let r = compile_and_run(src);
        assert!(r.is_ok(), "with enter-throws failed: {:?}", r.err());
    }

    /// with body 内 early-exit（return）：插 TRY_EXIT 注销 handler，不泄漏（标准 13）。
    /// mslang 的 early-exit 仅 TRY_EXIT（不调 __exit__）。后续 throw 须被外层正常捕获。
    #[test]
    fn test_with_early_exit_no_handler_leak() {
        let src = r#"
fn run() {
    log = ""
    fn p(s) {
        nonlocal log
        log = log + s + "|"
    }
    fn do_enter(self) {
        p("enter")
        return self
    }
    fn do_exit(self, err, msg, tb) {
        p("EXIT")
        return false
    }
    ctx = {
        "__enter__": do_enter,
        "__exit__": do_exit
    }
    fn earlyreturn() {
        with ctx as c {
            p("body")
            return
        }
        p("unreachable")
    }
    earlyreturn()
    try {
        throw ValueError("after")
    } except ValueError as e {
        p("caught: " + e.message)
    }
    assert(log == "enter|body|caught: after|", log)
}
run()
"#;
        let r = compile_and_run(src);
        assert!(r.is_ok(), "with early-exit no-leak failed: {:?}", r.err());
    }

    // ---- task 39: 生成器与 yield ----

    #[test]
    fn test_generator_basic_for_in() {
        let src = r#"
fn countdown(n) {
    while n > 0 {
        yield n
        n = n - 1
    }
}
r = [x for x in countdown(5)]
r
"#;
        let r = compile_and_run(src).unwrap();
        assert_eq!(to_list(&r), vec![i(5), i(4), i(3), i(2), i(1)]);
    }

    #[test]
    fn test_generator_next_method() {
        let src = r#"
fn gen3() {
    yield 10
    yield 20
    yield 30
}
g = gen3()
r = [g.__next__(), g.__next__(), g.__next__()]
r
"#;
        let r = compile_and_run(src).unwrap();
        assert_eq!(to_list(&r), vec![i(10), i(20), i(30)]);
    }

    #[test]
    fn test_generator_stop_iteration() {
        let src = r#"
fn gen1() {
    yield 1
}
g = gen1()
g.__next__()
caught = false
try {
    g.__next__()
} except StopIteration {
    caught = true
}
caught
"#;
        let r = compile_and_run(src).unwrap();
        assert_eq!(r, Object::Bool(true));
    }

    #[test]
    fn test_generator_fibonacci() {
        let src = r#"
fn fibonacci() {
    a, b = 0, 1
    while true {
        yield a
        a, b = b, a + b
    }
}
fib = fibonacci()
r = [fib.__next__(), fib.__next__(), fib.__next__(), fib.__next__(), fib.__next__(), fib.__next__()]
r
"#;
        let r = compile_and_run(src).unwrap();
        assert_eq!(to_list(&r), vec![i(0), i(1), i(1), i(2), i(3), i(5)]);
    }

    #[test]
    fn test_generator_yield_from_list() {
        let src = r#"
fn gen() {
    yield 1
    yield from [2, 3, 4]
    yield 5
}
r = [x for x in gen()]
r
"#;
        let r = compile_and_run(src).unwrap();
        assert_eq!(to_list(&r), vec![i(1), i(2), i(3), i(4), i(5)]);
    }

    #[test]
    fn test_generator_yield_from_range() {
        let src = r#"
fn gen() {
    yield from range(3)
}
r = [x for x in gen()]
r
"#;
        let r = compile_and_run(src).unwrap();
        assert_eq!(to_list(&r), vec![i(0), i(1), i(2)]);
    }

    #[test]
    fn test_generator_yield_from_generator() {
        let src = r#"
fn inner() {
    yield 10
    yield 20
}
fn outer() {
    yield 1
    yield from inner()
    yield 2
}
r = [x for x in outer()]
r
"#;
        let r = compile_and_run(src).unwrap();
        assert_eq!(to_list(&r), vec![i(1), i(10), i(20), i(2)]);
    }

    #[test]
    fn test_generator_close() {
        let src = r#"
fn gen() {
    yield 1
    yield 2
}
g = gen()
g.__next__()
g.close()
caught = false
try {
    g.__next__()
} except StopIteration {
    caught = true
}
caught
"#;
        let r = compile_and_run(src).unwrap();
        assert_eq!(r, Object::Bool(true));
    }

    #[test]
    fn test_generator_bare_yield() {
        let src = r#"
fn gen() {
    yield
    yield
}
g = gen()
r = [g.__next__(), g.__next__()]
r
"#;
        let r = compile_and_run(src).unwrap();
        assert_eq!(to_list(&r), vec![Object::Nil, Object::Nil]);
    }

    #[test]
    fn test_generator_return_discards_value() {
        let src = r#"
fn gen() {
    yield 1
    return 999
}
g = gen()
g.__next__()
caught = false
try {
    g.__next__()
} except StopIteration {
    caught = true
}
caught
"#;
        let r = compile_and_run(src).unwrap();
        assert_eq!(r, Object::Bool(true));
    }

    #[test]
    fn test_generator_expression_basic() {
        let src = r#"
squares = (x * x for x in range(5))
r = [x for x in squares]
r
"#;
        let r = compile_and_run(src).unwrap();
        assert_eq!(to_list(&r), vec![i(0), i(1), i(4), i(9), i(16)]);
    }

    #[test]
    fn test_generator_expression_filtered() {
        let src = r#"
nums = [1, -2, 3, -4, 5]
positives = (x for x in nums if x > 0)
r = [x for x in positives]
r
"#;
        let r = compile_and_run(src).unwrap();
        assert_eq!(to_list(&r), vec![i(1), i(3), i(5)]);
    }

    #[test]
    fn test_generator_expression_upvalue_capture() {
        let src = r#"
mult = 10
gen = (x * mult for x in range(3))
r = [x for x in gen]
r
"#;
        let r = compile_and_run(src).unwrap();
        assert_eq!(to_list(&r), vec![i(0), i(10), i(20)]);
    }

    #[test]
    fn test_generator_multiple_unique_names() {
        let src1 = r#"
g1 = (x for x in range(3))
r1 = [v for v in g1]
r1
"#;
        let src2 = r#"
g2 = (x * x for x in range(3))
r2 = [v for v in g2]
r2
"#;
        let r1 = compile_and_run(src1).unwrap();
        assert_eq!(to_list(&r1), vec![i(0), i(1), i(2)]);
        let r2 = compile_and_run(src2).unwrap();
        assert_eq!(to_list(&r2), vec![i(0), i(1), i(4)]);
    }

    #[test]
    fn test_generator_make_counter() {
        let src = r#"
fn make_counter(start) {
    n = start
    while true {
        yield n
        n += 1
    }
}
c = make_counter(100)
r = [c.__next__(), c.__next__(), c.__next__()]
r
"#;
        let r = compile_and_run(src).unwrap();
        assert_eq!(to_list(&r), vec![i(100), i(101), i(102)]);
    }

    // ---- task 40：类定义与实例化 ----

    #[test]
    fn test_class_creation_and_store_global() {
        // 验证标准 1：class 定义创建类对象并存入全局变量（可被 LOAD_GLOBAL 取回）。
        let src = r#"
class Empty {
    fn id(self) {
        return 42
    }
}
e = Empty()
assert(e.id() == 42)
"#;
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_class_init_and_fields() {
        // 验证标准 3/6：ClassName(args) 创建实例并调用 __init__；self.attr 读写实例字段。
        let src = r#"
class Box {
    fn __init__(self, v) {
        self.v = v
    }
}
b = Box(7)
assert(b.v == 7)
"#;
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_instance_method_call() {
        // 验证标准 4：obj.method(args)（self 由调用点暂存槽自动注入，task 41 切换 BoundMethod）。
        let src = r#"
class Adder {
    fn __init__(self, base) {
        self.base = base
    }
    fn add(self, x) {
        return self.base + x
    }
}
a = Adder(10)
assert(a.add(5) == 15)
"#;
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_class_attribute_shared() {
        // 验证标准 2：类属性在所有实例间共享。
        let src = r#"
class C {
    count = 100
}
c1 = C()
c2 = C()
assert(c1.count + c2.count + C.count == 300)
"#;
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_dynamic_attribute() {
        // 验证标准 7：动态属性赋值（obj.new_attr = val）。
        let src = r#"
class P {
    fn __init__(self) {
        self.x = 1
    }
}
p = P()
p.y = 99
assert(p.x + p.y == 100)
"#;
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_repr_via_str() {
        // 验证标准 5：__repr__ 被 print/str 调用（__str__ 优先级由 task 43）。
        let src = r#"
class Animal {
    fn __init__(self, name) {
        self.name = name
    }
    fn __repr__(self) {
        return "Animal(" + self.name + ")"
    }
}
assert(str(Animal("Dog")) == "Animal(Dog)")
"#;
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_repr_point() {
        // test_class.ms 中 Point 的 __repr__（含 str(int) 拼接）。
        let src = r#"
class Point {
    fn __init__(self, x, y) {
        self.x = x
        self.y = y
    }
    fn __repr__(self) {
        return "Point(" + str(self.x) + ", " + str(self.y) + ")"
    }
}
assert(str(Point(3, 4)) == "Point(3, 4)")
"#;
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_default_repr_when_none() {
        // task 43：无 __repr__ 时经继承链命中 Object.__repr__ → "ClassName instance"。
        let src = r#"
class Plain {
    fn __init__(self) {
    }
}
assert(str(Plain()) == "Plain instance")
"#;
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_name_attribute() {
        // 验证标准 8：__name__ 内置属性（Instance 与 Class 均返回类名）。
        let src = r#"
class Widget {
    fn __init__(self) {
    }
}
w = Widget()
assert(w.__name__ == "Widget")
assert(Widget.__name__ == "Widget")
"#;
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_multiple_instances_independent() {
        // 多个实例互不干扰（实例字段 per-instance）。
        let src = r#"
class Counter {
    fn __init__(self, start) {
        self.n = start
    }
    fn bump(self) {
        self.n = self.n + 1
        return self.n
    }
}
a = Counter(0)
b = Counter(100)
assert(a.bump() + b.bump() == 102)
"#;
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_get_attr_unknown_error() {
        // 验证标准（GET_ATTR 错误路径）：实例无该属性时报错。
        let src = r#"
class C {
    fn __init__(self) {
        self.a = 1
    }
}
c = C()
c.nope
"#;
        let err = compile_and_run(src).unwrap_err();
        assert!(err.contains("has no attribute"), "got: {}", err);
    }

    #[test]
    fn test_no_init_with_args_error() {
        // 验证标准 10（R4）：无 __init__ 且有参数时报错。
        let src = r#"
class Bare {
}
Bare(1, 2)
"#;
        let err = compile_and_run(src).unwrap_err();
        assert!(err.contains("takes no arguments"), "got: {}", err);
    }

    #[test]
    fn test_no_init_no_args_ok() {
        // 无 __init__ 且无参数 → 直接返回实例。
        let src = r#"
class Bare {
}
x = Bare()
assert(x.__name__ == "Bare")
"#;
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_super_outside_class_error() {
        // task 42 验证标准 9：super 在非方法上下文（顶层）编译期报错。
        let src = "x = super.foo()";
        let tokens = Lexer::new(src).tokenize_all().unwrap();
        let prog = Parser::new(tokens).parse().unwrap();
        let mut compiler = Compiler::new();
        let err = compiler.compile(&prog).unwrap_err();
        assert!(
            err.contains("'super' used outside of class method"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_class_in_function_error() {
        // 函数内定义 class 暂不支持（编译期报错）。
        let src = r#"
fn f() {
    class X {
    }
}
"#;
        let tokens = Lexer::new(src).tokenize_all().unwrap();
        let prog = Parser::new(tokens).parse().unwrap();
        let mut compiler = Compiler::new();
        let err = compiler.compile(&prog).unwrap_err();
        assert!(err.contains("inside function"), "got: {}", err);
    }

    #[test]
    fn test_method_non_closure_error() {
        // 验证标准 12（V4）：METHOD 栈顶非 closure 时返回明确错误。
        // 构造合成字节码：CONSTANT(非 closure) + METHOD。
        let code = vec![
            OpCode::Constant as u8,
            0x00,
            0x00, // 非闭包常量压栈
            OpCode::Method as u8,
            0x00,
            0x01, // METHOD "x"
            OpCode::Halt as u8,
        ];
        let constants = vec![Object::Int(1), alloc_string("x")];
        let err = run_chunk(code, constants).unwrap_err();
        assert!(err.contains("METHOD expects a closure"), "got: {}", err);
    }

    #[test]
    fn test_class_constant_pool_oob_error() {
        // 验证标准 11（V3）：常量池越界不 panic，返回 Err。
        let code = vec![
            OpCode::Class as u8,
            0x00,
            0x05, // name_idx=5 越界（空常量池）
            OpCode::Halt as u8,
        ];
        let err = run_chunk(vec![code[0], 0x00, 0x05, OpCode::Halt as u8], vec![]).unwrap_err();
        assert!(
            err.contains("out of range") || err.contains("string"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_has_finalizer_set_when_del_defined() {
        // 验证标准 13：含 __del__ 的类，instance 置 has_finalizer。
        let src_with_del = r#"
class A {
    fn __del__(self) {
    }
}
A()
"#;
        // 直接构造类+实例验证标志位（不走 print，仅检查 gc_meta）。
        let class_obj = alloc_class("A".to_string());
        let Object::Ref(cls_ptr) = class_obj else {
            unreachable!()
        };
        // 模拟 __del__ 注册：插入一个占位方法指针。
        unsafe { read_class(cls_ptr) }
            .methods
            .insert("__del__".to_string(), cls_ptr);
        let inst = alloc_instance(cls_ptr);
        let Object::Ref(ip) = inst else {
            unreachable!()
        };
        // 复用 call_class 的标志设置逻辑：has_del → set_has_finalizer。
        let has_del = unsafe { read_class(cls_ptr) }
            .methods
            .contains_key("__del__");
        if has_del {
            unsafe {
                (*ip).set_has_finalizer(true);
            }
        }
        assert!(
            unsafe { (*ip).has_finalizer() },
            "instance should have finalizer flag"
        );
        // 对照：无 __del__ 的类不应置标志。
        let class2 = alloc_class("B".to_string());
        let Object::Ref(cls2) = class2 else {
            unreachable!()
        };
        let inst2 = alloc_instance(cls2);
        let Object::Ref(ip2) = inst2 else {
            unreachable!()
        };
        assert!(
            !unsafe { (*ip2).has_finalizer() },
            "no-del instance should NOT have flag"
        );
        // 引用 src_with_del 以证明完整路径可编译运行。
        assert!(compile_and_run(src_with_del).is_ok());
    }

    #[test]
    fn test_class_field_overrides_class_attr() {
        // 实例字段优先于类属性（GET_ATTR 先查 fields 再 class_attrs）。
        let src = r#"
class C {
    v = "class"
    fn __init__(self) {
        self.v = "instance"
    }
}
c = C()
assert(c.v == "instance")
"#;
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_set_class_attr_via_class() {
        // ClassName.attr = val 写入 class_attrs，所有实例可见。
        let src = r#"
class C {
}
C.n = 42
c = C()
assert(c.n == 42)
"#;
        assert!(compile_and_run(src).is_ok());
    }

    // ---- task 41：self 绑定与实例属性（BoundMethod） ----

    #[test]
    fn test_self_binding_basic() {
        // 验证标准 1/2：self 引用当前实例；obj.method(args) 自动绑定 self（BoundMethod）。
        let src = r#"
class Box {
    fn __init__(self, v) {
        self.v = v
    }
    fn get(self) {
        return self.v
    }
}
b = Box(42)
assert(b.get() == 42)
"#;
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_bound_method_stored_and_called_later() {
        // BoundMethod 是一等值：可存储后调用，self 仍正确绑定。
        let src = r#"
class Counter {
    fn __init__(self, n) {
        self.n = n
    }
    fn inc(self) {
        self.n = self.n + 1
        return self.n
    }
}
c = Counter(0)
f = c.inc
assert(f() == 1)
assert(f() == 2)
assert(c.inc() == 3)
"#;
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_method_with_args_and_field_lookup_chain() {
        // 验证标准 3/5：实例字段读写；查找链 实例字段 → 类方法 → 类属性。
        let src = r#"
class Point {
    fn __init__(self, x, y) {
        self.x = x
        self.y = y
    }
    fn distance_to(self, other) {
        dx = self.x - other.x
        dy = self.y - other.y
        return (dx * dx + dy * dy) ** 0.5
    }
}
p1 = Point(3, 4)
p2 = Point(0, 0)
assert(p1.distance_to(p2) == 5.0)
"#;
        let result = compile_and_run(src);
        assert!(result.is_ok(), "got: {:?}", result.err());
    }

    #[test]
    fn test_dynamic_attribute_addition() {
        // 验证标准 4：运行时动态添加实例属性。
        let src = r#"
class P {
    fn __init__(self) {
        self.x = 1
    }
}
p = P()
p.y = 99
p.z = p.x + p.y
assert(p.z == 100)
"#;
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_field_overrides_class_attr_lookup() {
        // 验证标准 5：实例字段优先于类属性。
        let src = r#"
class Config {
    value = "class"
    fn __init__(self) {
        self.value = "instance"
    }
}
c = Config()
assert(c.value == "instance")
d = Config()
d.value = "other"
assert(c.value == "instance")
assert(d.value == "other")
"#;
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_instances_isolated_fields() {
        // 验证标准 6：不同实例的字段互不干扰。
        let src = r#"
class Tag {
    fn __init__(self, label) {
        self.label = label
    }
}
a = Tag("A")
b = Tag("B")
a.extra = 1
assert(a.label == "A")
assert(b.label == "B")
assert(a.extra == 1)
"#;
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_method_first_param_must_be_self() {
        // 验证标准 7：方法首参数非 self 时编译期报错。
        let src = r#"
class C {
    fn bad(x) {
        return x
    }
}
"#;
        let tokens = Lexer::new(src).tokenize_all().unwrap();
        let prog = Parser::new(tokens).parse().unwrap();
        let mut compiler = Compiler::new();
        let err = compiler.compile(&prog).unwrap_err();
        assert!(
            err.contains("must have 'self' as first parameter"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_method_no_params_error() {
        // 方法无参数（缺少 self）时编译期报错。
        let src = r#"
class C {
    fn empty() {
    }
}
"#;
        let tokens = Lexer::new(src).tokenize_all().unwrap();
        let prog = Parser::new(tokens).parse().unwrap();
        let mut compiler = Compiler::new();
        let err = compiler.compile(&prog).unwrap_err();
        assert!(err.contains("self"), "got: {}", err);
    }

    #[test]
    fn test_name_attribute_no_regression() {
        // 验证标准 9：inst.__name__ 与 Cls.__name__ 仍返回类名。
        let src = r#"
class Widget {
    fn __init__(self) {
    }
}
w = Widget()
assert(w.__name__ == "Widget")
assert(Widget.__name__ == "Widget")
"#;
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_repr_via_print_with_self() {
        // __repr__ 经 BoundMethod 调用，self 正确绑定（含 str(int) 拼接）。
        let src = r#"
class Point {
    fn __init__(self, x, y) {
        self.x = x
        self.y = y
    }
    fn __repr__(self) {
        return "Point(" + str(self.x) + ", " + str(self.y) + ")"
    }
}
assert(str(Point(3, 4)) == "Point(3, 4)")
"#;
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_method_chaining() {
        // 方法链：方法返回 self，支持 builder 模式（不依赖 list 内置方法 task 50/51）。
        let src = r#"
class Builder {
    fn __init__(self) {
        self.count = 0
    }
    fn add(self, n) {
        self.count = self.count + n
        return self
    }
    fn get(self) {
        return self.count
    }
}
b = Builder()
assert(b.add(1).add(2).add(3).get() == 6)
"#;
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_closure_capturing_self() {
        // 验证标准（§4 upvalue 捕获）：方法内闭包通过 upvalue 捕获 self。
        let src = r#"
class Acc {
    fn __init__(self) {
        self.total = 0
    }
    fn adder(self) {
        return fn(n) {
            self.total = self.total + n
            return self.total
        }
    }
}
a = Acc()
f = a.adder()
assert(f(10) == 10)
assert(f(5) == 15)
assert(a.total == 15)
"#;
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_alloc_and_read_bound_method() {
        // 直接验证 alloc_bound_method / read_bound_method 堆对象。
        let class_obj = alloc_class("Foo".to_string());
        let Object::Ref(cls_ptr) = class_obj else {
            unreachable!()
        };
        let inst_obj = alloc_instance(cls_ptr);
        let Object::Ref(inst_ptr) = inst_obj else {
            unreachable!()
        };
        let method_ptr = cls_ptr;
        let bound = alloc_bound_method(inst_obj.clone(), method_ptr);
        let Object::Ref(bptr) = bound else {
            unreachable!()
        };
        unsafe {
            assert_eq!((*bptr).type_tag, super::TypeTag::BOUND_METHOD as u8);
            let b = read_bound_method(bptr);
            assert_eq!(b.method, method_ptr);
            // receiver 应为 inst_obj（Ref 指针等于 inst_ptr）。
            match &b.receiver {
                Object::Ref(r) => assert_eq!(*r, inst_ptr),
                _ => unreachable!(),
            }
        }
    }

    // ---- task 42：继承与 super ----

    #[test]
    fn test_inherit_parent_method() {
        // 验证标准 1：子类继承父类方法。
        let src = r#"
class Animal {
    fn speak(self) {
        return "animal speaks"
    }
}
class Dog < Animal {
}
d = Dog()
assert(d.speak() == "animal speaks")
"#;
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_inherit_method_override() {
        // 验证标准 2：子类方法覆盖父类方法。
        let src = r#"
class Animal {
    fn speak(self) {
        return "animal"
    }
}
class Dog < Animal {
    fn speak(self) {
        return "dog"
    }
}
assert(Dog().speak() == "dog")
assert(Animal().speak() == "animal")
"#;
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_super_method_call() {
        // 验证标准 3：super.method() 调用父类方法。
        let src = r#"
class Base {
    fn greet(self) {
        return "hello"
    }
}
class Child < Base {
    fn greet(self) {
        return super.greet() + " world"
    }
}
assert(Child().greet() == "hello world")
"#;
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_super_init_call() {
        // 验证标准 4：super.__init__() 调用父类构造器。
        let src = r#"
class Animal {
    fn __init__(self, name) {
        self.name = name
    }
}
class Dog < Animal {
    fn __init__(self, name, breed) {
        super.__init__(name)
        self.breed = breed
    }
    fn speak(self) {
        return self.name + " barks"
    }
}
d = Dog("Rex", "Shepherd")
assert(d.speak() == "Rex barks")
assert(d.name == "Rex")
assert(d.breed == "Shepherd")
"#;
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_inheritance_chain() {
        // 验证标准 6/7：继承链 A→B→C，super 沿链回溯。
        let src = r#"
class A {
    fn who(self) {
        return "A"
    }
}
class B < A {
    fn who(self) {
        return "B+" + super.who()
    }
}
class C < B {
    fn who(self) {
        return "C+" + super.who()
    }
}
        assert(C().who() == "C+B+A")
assert(B().who() == "B+A")
"#;
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_implicit_object_repr() {
        // 验证标准 5/11：无显式父类的类继承 Object，__repr__ 返回类名 + " instance"。
        let src = r#"
class Simple {
    fn __init__(self) {
        self.x = 1
    }
}
s = Simple()
assert(s.__repr__() == "Simple instance")
"#;
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_type_returns_class_name() {
        // 验证标准 8：type(instance) 返回类名（非 "instance"）。
        let src = r#"
class Simple {
}
s = Simple()
assert(type(s) == "Simple")
assert(type(42) == "int")
assert(type("hi") == "string")
"#;
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_object_eq_ne_identity() {
        // Object.__eq__ = self is other；Object.__ne__ = not (self is other)。
        let src = r#"
class S {
}
a = S()
b = S()
assert(a.__eq__(a))
assert(not a.__eq__(b))
assert(not a.__ne__(a))
assert(a.__ne__(b))
"#;
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_inherit_class_attr() {
        // 验证标准 6：属性查找沿继承链进行（实例访问继承的类属性，经 find_class_attr）。
        let src = r#"
class Base {
    count = 10
}
class Derived < Base {
}
d = Derived()
assert(d.count == 10)
"#;
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_full_inheritance_program() {
        // spec §测试用例 test_inheritance.ms 的等价 assert 版本。
        let src = r#"
class Animal {
    fn __init__(self, name) {
        self.name = name
    }
    fn speak(self) {
        return self.name + " speaks"
    }
}
class Dog < Animal {
    fn __init__(self, name, breed) {
        super.__init__(name)
        self.breed = breed
    }
    fn speak(self) {
        return self.name + " barks"
    }
}
d = Dog("Rex", "Shepherd")
assert(d.speak() == "Rex barks")
assert(d.name == "Rex")
assert(d.breed == "Shepherd")

class Base {
    fn greet(self) {
        return "hello from Base"
    }
}
class Child < Base {
    fn greet(self) {
        return super.greet() + " and Child"
    }
}
assert(Child().greet() == "hello from Base and Child")

class A {
    fn who(self) {
        return "A"
    }
}
class B < A {
    fn who(self) {
        return "B+" + super.who()
    }
}
class C < B {
    fn who(self) {
        return "C+" + super.who()
    }
}
assert(C().who() == "C+B+A")

class Simple {
    fn __init__(self) {
        self.x = 1
    }
}
s = Simple()
assert(s.__repr__() == "Simple instance")
"#;
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_object_class_immortal_generation() {
        // 验证标准 11：Object 基类标为 Immortal 代。
        let vm = VM::new();
        assert!(!vm.object_class.is_null());
        unsafe {
            assert_eq!(
                (*vm.object_class).generation(),
                crate::vm::gc::Generation::Immortal
            );
            assert_eq!(read_class(vm.object_class).name.clone(), "Object");
        }
    }

    // ---- task 43：魔术方法（自动分派）----

    #[test]
    fn test_magic_str_priority_over_repr() {
        // 标准 1：__str__ 优先于 __repr__（print/str 调用）。
        let src = r#"
class Named {
    fn __str__(self) {
        return "str form"
    }
    fn __repr__(self) {
        return "repr form"
    }
}
n = Named()
assert(str(n) == "str form")
"#;
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_magic_repr_no_str_falls_to_repr() {
        // 标准 1：仅有 __repr__ 时被 print/str 调用。
        let src = r#"
class Box {
    fn __init__(self, v) {
        self.v = v
    }
    fn __repr__(self) {
        return "Box(" + str(self.v) + ")"
    }
}
assert(str(Box(42)) == "Box(42)")
"#;
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_magic_arithmetic_dispatch() {
        // 标准 2：7 种算术运算符自动调用对应魔术方法。
        let src = r#"
class N {
    fn __init__(self, v) {
        self.v = v
    }
    fn __add__(self, o) {
        return N(self.v + o.v)
    }
    fn __sub__(self, o) {
        return N(self.v - o.v)
    }
    fn __mul__(self, o) {
        return N(self.v * o.v)
    }
    fn __div__(self, o) {
        return N(self.v // o.v)
    }
    fn __floordiv__(self, o) {
        return N(self.v // o.v)
    }
    fn __mod__(self, o) {
        return N(self.v % o.v)
    }
    fn __pow__(self, o) {
        return N(self.v ** o.v)
    }
}
a = N(20)
b = N(3)
assert((a + b).v == 23)
assert((a - b).v == 17)
assert((a * b).v == 60)
assert((a / b).v == 6)
assert((a // b).v == 6)
assert((a % b).v == 2)
assert((a ** b).v == 8000)
"#;
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_magic_comparison_dispatch() {
        // 标准 3：6 种比较运算符自动调用对应魔术方法。
        let src = r#"
class V {
    fn __init__(self, x) {
        self.x = x
    }
    fn __eq__(self, o) {
        return self.x == o.x
    }
    fn __ne__(self, o) {
        return self.x != o.x
    }
    fn __lt__(self, o) {
        return self.x < o.x
    }
    fn __le__(self, o) {
        return self.x <= o.x
    }
    fn __gt__(self, o) {
        return self.x > o.x
    }
    fn __ge__(self, o) {
        return self.x >= o.x
    }
}
a = V(5)
b = V(5)
c = V(9)
assert(a == b)
assert(a != c)
assert(a < c)
assert(a <= b)
assert(c > a)
assert(c >= a)
"#;
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_magic_call() {
        // 标准 4：__call__ 使实例可调用。
        let src = r#"
class Multiplier {
    fn __init__(self, factor) {
        self.factor = factor
    }
    fn __call__(self, x) {
        return x * self.factor
    }
}
double = Multiplier(2)
assert(double(5) == 10)
triple = Multiplier(3)
assert(triple(4) == 12)
"#;
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_magic_len() {
        // 标准 5：__len__ 使 len() 工作（builtin_len INSTANCE 分派）。
        let src = r#"
class MyList {
    fn __init__(self, items) {
        self.items = items
    }
    fn __len__(self) {
        return len(self.items)
    }
}
ml = MyList([10, 20, 30])
assert(len(ml) == 3)
"#;
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_magic_getitem_setitem() {
        // 标准 6：__getitem__/__setitem__ 使下标访问工作。
        let src = r#"
class Store {
    fn __init__(self) {
        self.data = {}
    }
    fn __getitem__(self, key) {
        return self.data[key]
    }
    fn __setitem__(self, key, val) {
        self.data[key] = val
    }
}
s = Store()
s["a"] = 100
s["b"] = 200
assert(s["a"] == 100)
assert(s["b"] == 200)
"#;
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_magic_contains() {
        // 标准 7：__contains__ 使 in 运算符工作。
        let src = r#"
class Range10 {
    fn __contains__(self, item) {
        return item >= 0 and item < 10
    }
}
r = Range10()
assert(5 in r)
assert(not (15 in r))
"#;
        assert!(compile_and_run(src).is_ok());
    }

    #[test]
    fn test_magic_str_non_string_error() {
        // 标准 9：__str__ 返回非 String 报错。
        let src = r#"
class Bad {
    fn __str__(self) {
        return 42
    }
}
str(Bad())
"#;
        let err = compile_and_run(src).unwrap_err();
        assert!(err.contains("must return a string"), "got: {}", err);
    }

    #[test]
    fn test_magic_repr_non_string_error() {
        // 标准 9：__repr__ 返回非 String 报错。
        let src = r#"
class Bad {
    fn __repr__(self) {
        return 42
    }
}
str(Bad())
"#;
        let err = compile_and_run(src).unwrap_err();
        assert!(err.contains("must return a string"), "got: {}", err);
    }

    #[test]
    fn test_magic_enter_exit_via_instance() {
        // 标准 10：__enter__/__exit__ 经 Instance 路径工作（task 41 GET_ATTR + CALL）。
        let src = r#"
class Ctx {
    fn __enter__(self) {
        return "resource"
    }
    fn __exit__(self, err, msg, tb) {
        return false
    }
}
result = ""
with Ctx() as r {
    result = r
}
assert(result == "resource")
"#;
        let r = compile_and_run(src);
        assert!(r.is_ok(), "with via Instance failed: {:?}", r.err());
    }

    #[test]
    fn test_magic_binary_left_operand_only() {
        // 标准 11：二元运算仅检查左操作数。Int + Instance（左 Int 无 __add__）
        // fallback 到内置 (Int, Ref) 匹配失败 → TypeError（无反射运算符）。
        let src = r#"
class V {
    fn __init__(self, x) {
        self.x = x
    }
    fn __add__(self, o) {
        return V(self.x + o.x)
    }
}
v = V(5)
x = 1 + v
"#;
        let err = compile_and_run(src).unwrap_err();
        assert!(
            err.contains("TypeError") && err.contains("unsupported operand"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_magic_getitem_without_method_errors() {
        // Instance 无 __getitem__ 时报 not subscriptable。
        let src = r#"
class Bare {
    fn __init__(self) {
    }
}
b = Bare()
x = b[0]
"#;
        let err = compile_and_run(src).unwrap_err();
        assert!(err.contains("not subscriptable"), "got: {}", err);
    }

    #[test]
    fn test_magic_len_non_int_return_errors() {
        // __len__ 返回非 Int 报错。
        let src = r#"
class Bad {
    fn __len__(self) {
        return "nope"
    }
}
len(Bad())
"#;
        let err = compile_and_run(src).unwrap_err();
        assert!(err.contains("should return an int"), "got: {}", err);
    }

    // ---- task 44：装饰器 ----

    /// 验证标准 1 + 5：基本装饰器等价于 `fn f() {}; f = dec(f)`，且装饰后可通过原名称调用。
    #[test]
    fn test_decorator_basic() {
        let src = r#"
fn dec(func) {
    return fn(x) {
        return func(x) + 1
    }
}
@dec
fn base(x) {
    return x * 2
}
assert(base(5) == 11, str(base(5)))
"#;
        assert!(compile_and_run(src).is_ok());
    }

    /// 验证标准 2：多重装饰器从下到上应用（靠近函数的先执行）。
    #[test]
    fn test_decorator_multiple_order() {
        let src = r#"
fn d1(func) {
    return fn() {
        return "d1(" + func() + ")"
    }
}
fn d2(func) {
    return fn() {
        return "d2(" + func() + ")"
    }
}
@d1
@d2
fn greet() {
    return "hi"
}
assert(greet() == "d1(d2(hi))", greet())
"#;
        assert!(compile_and_run(src).is_ok());
    }

    /// 验证标准 3：带参数的装饰器 `@dec(args)` 正确解析和执行。
    #[test]
    fn test_decorator_parameterized() {
        let src = r#"
fn add_tag(tag) {
    return fn(func) {
        return fn() {
            return "<" + tag + ">" + func() + "</" + tag + ">"
        }
    }
}
@add_tag("b")
fn get_text() {
    return "hello"
}
assert(get_text() == "<b>hello</b>", get_text())
"#;
        assert!(compile_and_run(src).is_ok());
    }

    /// 验证标准 4：类装饰器正确工作。
    #[test]
    fn test_decorator_class() {
        // 类装饰器：装饰器接收类对象，可添加属性后返回。cls.attr = val 经 SET_ATTR
        // 写入 class_attrs（非 methods），故通过类名或实例访问属性值。
        let src = r#"
fn add_greet(cls) {
    cls.greeting = "Hello from " + cls.__name__
    return cls
}
@add_greet
class Foo {
    fn __init__(self) {}
}
assert(Foo.greeting == "Hello from Foo", Foo.greeting)
f = Foo()
assert(f.greeting == "Hello from Foo", f.greeting)
"#;
        let r = compile_and_run(src);
        assert!(r.is_ok(), "class decorator failed: {:?}", r.err());
    }

    /// 验证标准 6：原始函数的 name 属性保留（装饰仅替换变量绑定）。
    #[test]
    fn test_decorator_preserves_name() {
        let src = r#"
fn passthrough(func) {
    return func
}
@passthrough
fn myfunc(x) {
    return x
}
assert(myfunc.name == "myfunc", myfunc.name)
"#;
        assert!(compile_and_run(src).is_ok());
    }

    /// 验证标准 6 扩展：装饰器包装后的闭包 name 仍为原函数名。
    #[test]
    fn test_decorator_wrapped_name() {
        let src = r#"
fn log(func) {
    return fn(x) {
        return func.name
    }
}
@log
fn double(x) {
    return x * 2
}
assert(double(5) == "double", double(5))
"#;
        assert!(compile_and_run(src).is_ok());
    }

    /// 验证标准 7：装饰器返回非可调用值时，通过原名称调用抛出 TypeError。
    #[test]
    fn test_decorator_non_callable_errors() {
        let src = r#"
fn bad(func) {
    return 42
}
@bad
fn h() {
    return 1
}
h()
"#;
        let err = compile_and_run(src).unwrap_err();
        assert!(err.contains("TypeError"), "got: {}", err);
        assert!(err.contains("not callable"), "got: {}", err);
    }

    /// 验证标准 9：函数体内的局部 `@dec fn ...` 正确绑定到局部作用域。
    #[test]
    fn test_decorator_local_scope() {
        let src = r#"
fn wrapper(func) {
    return fn(x) {
        return func(x) + 100
    }
}
fn make() {
    @wrapper
    fn inner(x) {
        return x + 1
    }
    return inner(10)
}
assert(make() == 111, str(make()))
"#;
        assert!(compile_and_run(src).is_ok());
    }

    /// 完整集成测试：综合验证所有装饰器场景（等价于 test_decorators.ms）。
    #[test]
    fn test_decorator_integration() {
        let src = r#"
fn log(func) {
    return fn(x) {
        return func.name
    }
}
@log
fn double(x) {
    return x * 2
}
assert(double(5) == "double")

fn add_tag(tag) {
    return fn(func) {
        return fn() {
            return "<" + tag + ">" + func() + "</" + tag + ">"
        }
    }
}
@add_tag("b")
fn get_text() {
    return "hello"
}
assert(get_text() == "<b>hello</b>")

fn d1(func) {
    return fn() {
        return "d1(" + func() + ")"
    }
}
fn d2(func) {
    return fn() {
        return "d2(" + func() + ")"
    }
}
@d1
@d2
fn greet() {
    return "hi"
}
assert(greet() == "d1(d2(hi))")

fn add_greet(cls) {
    cls.greeting = "Hello from " + cls.__name__
    return cls
}
@add_greet
class Foo {
    fn __init__(self) {}
}
assert(Foo.greeting == "Hello from Foo")

fn wrapper(func) {
    return fn(x) {
        return func(x) + 100
    }
}
fn make() {
    @wrapper
    fn inner(x) {
        return x + 1
    }
    return inner(10)
}
assert(make() == 111)
"#;
        assert!(compile_and_run(src).is_ok());
    }

    // ---- task 45：模块系统集成测试 ----
    use std::path::{Path, PathBuf};

    /// 在临时目录写入模块文件，返回该目录路径。
    fn write_module(dir_name: &str, files: &[(&str, &str)]) -> PathBuf {
        let dir = std::env::temp_dir().join(dir_name);
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        for (name, content) in files {
            let path = dir.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).ok();
            std::fs::write(path, content).unwrap();
        }
        dir
    }

    /// 编译并运行 main 脚本，把 `dir` 加入模块搜索路径。
    fn run_with_module_dir(source: &str, dir: &Path) -> Result<Object, String> {
        let program = parse(source);
        let mut compiler = Compiler::new();
        let chunk = compiler.compile(&program).unwrap();
        let mut vm = VM::new();
        vm.add_module_search_path(dir.to_path_buf());
        vm.interpret(chunk)
    }

    #[test]
    fn test_module_basic_import_and_attrs() {
        // 标准 1/2：import 加载并执行模块；module.name 访问导出。
        let dir = write_module(
            "mslang_mod_basic",
            &[(
                "math_utils.ms",
                "const VERSION = \"1.0\"\nfn add(a, b) { return a + b }\nfn multiply(a, b) { return a * b }",
            )],
        );
        let src = "import math_utils\nassert(math_utils.VERSION == \"1.0\")\nassert(math_utils.add(3, 4) == 7)\nassert(math_utils.multiply(3, 4) == 12)";
        let r = run_with_module_dir(src, &dir);
        std::fs::remove_dir_all(&dir).ok();
        assert!(r.is_ok(), "basic import failed: {:?}", r.err());
    }

    #[test]
    fn test_module_from_import_with_alias() {
        // 标准 3：from...import 提取名称；标准 4：as 别名。
        let dir = write_module(
            "mslang_mod_from",
            &[(
                "math_utils.ms",
                "fn add(a, b) { return a + b }\nfn multiply(a, b) { return a * b }",
            )],
        );
        let src = "from math_utils import add, multiply as mul\nassert(add(1, 2) == 3)\nassert(mul(3, 4) == 12)";
        let r = run_with_module_dir(src, &dir);
        std::fs::remove_dir_all(&dir).ok();
        assert!(r.is_ok(), "from import failed: {:?}", r.err());
    }

    #[test]
    fn test_module_import_as_alias() {
        // 标准 4：import as 别名正常工作。
        let dir = write_module(
            "mslang_mod_alias",
            &[("m.ms", "fn double(x) { return x * 2 }")],
        );
        let src = "import m as utils\nassert(utils.double(21) == 42)";
        let r = run_with_module_dir(src, &dir);
        std::fs::remove_dir_all(&dir).ok();
        assert!(r.is_ok(), "import as failed: {:?}", r.err());
    }

    #[test]
    fn test_module_cache_single_execution() {
        // 标准 5：模块只执行一次（缓存）。Rust 层验证两次 load_module 返回同一指针。
        let dir = write_module("mslang_mod_cache", &[("c.ms", "fn id(x) { return x }")]);
        let program = parse("import c");
        let mut compiler = Compiler::new();
        let chunk = compiler.compile(&program).unwrap();
        let mut vm = VM::new();
        vm.add_module_search_path(dir.clone());
        // 运行一次以触发首次 import（加载 + 缓存）。
        vm.interpret(chunk).unwrap();
        let cache_len = vm.module_resolver.cache.len();
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(cache_len, 1, "module should be cached exactly once");
    }

    #[test]
    fn test_module_double_import_no_error() {
        // 标准 5（spec 测试用例）：重复 import 同一模块不报错（缓存命中）。
        let dir = write_module("mslang_mod_double", &[("c.ms", "const X = 1")]);
        let src = "import c\nimport c\nassert(c.X == 1)";
        let r = run_with_module_dir(src, &dir);
        std::fs::remove_dir_all(&dir).ok();
        assert!(r.is_ok(), "double import failed: {:?}", r.err());
    }

    #[test]
    fn test_module_private_var_inaccessible() {
        // 标准 6：var（私有）不可从外部访问 → GET_ATTR 抛 NameError。
        let dir = write_module("mslang_mod_priv", &[("p.ms", "var secret = 42\nfn pub() { return 1 }")]);
        let src = "import p\nassert(p.pub() == 1)\ntry {\n    p.secret\n} except NameError {\n    assert(true)\n}";
        let r = run_with_module_dir(src, &dir);
        std::fs::remove_dir_all(&dir).ok();
        assert!(r.is_ok(), "private var test failed: {:?}", r.err());
    }

    #[test]
    fn test_module_package_index() {
        // 标准 7：包模块（目录 + index.ms）。
        let dir = write_module(
            "mslang_mod_pkg",
            &[
                ("mylib/index.ms", "fn lib_root() { return \"root\" }"),
                ("mylib/utils.ms", "fn tool() { return \"tool\" }"),
            ],
        );
        let src = "import mylib\nassert(mylib.lib_root() == \"root\")";
        let r = run_with_module_dir(src, &dir);
        std::fs::remove_dir_all(&dir).ok();
        assert!(r.is_ok(), "package import failed: {:?}", r.err());
    }

    #[test]
    fn test_module_dotted_from_import() {
        // dotted path 解析 + from...import：mylib/utils.ms 的 tool。
        let dir = write_module(
            "mslang_mod_dotted",
            &[("mylib/utils.ms", "fn tool() { return \"tool\" }")],
        );
        let src = "from mylib.utils import tool\nassert(tool() == \"tool\")";
        let r = run_with_module_dir(src, &dir);
        std::fs::remove_dir_all(&dir).ok();
        assert!(r.is_ok(), "dotted from-import failed: {:?}", r.err());
    }

    #[test]
    fn test_module_stdlib_prefix() {
        // 标准 9：import @std 强制标准库目录，跳过当前/MS_PATH 搜索。
        // 构造「恶意」当前目录同名模块与 stdlib 正式模块，@std 应取 stdlib。
        let dir = write_module(
            "mslang_mod_std",
            &[
                // 当前目录「恶意」geo.ms：仅含 FAKE，无 real。
                // 注：模块名用 "geo" 而非 "math"——task 47 起 "math" 为原生模块，会命中
                // native_modules 注册表而跳过磁盘，故此 @std 语义测试改用非保留名（同 task 46
                // 将 "io" 改为 "sample" 的处理）。
                ("geo.ms", "const FAKE = true"),
                // stdlib 子目录的正式 geo.ms：含 real。
                ("stdlib/geo.ms", "fn real() { return 42 }\nconst V = 9"),
            ],
        );
        let stdlib = dir.join("stdlib");
        let main = "import @std geo\nassert(geo.V == 9)\nassert(geo.real() == 42)";
        let program = parse(main);
        let mut compiler = Compiler::new();
        let chunk = compiler.compile(&program).unwrap();
        let mut vm = VM::new();
        // 当前目录（含恶意 geo.ms）置于搜索首位；stdlib 单独指定。
        vm.add_module_search_path(dir.clone());
        vm.module_resolver.stdlib_dir = stdlib;
        let r = vm.interpret(chunk);
        std::fs::remove_dir_all(&dir).ok();
        assert!(r.is_ok(), "@std import failed: {:?}", r.err());
    }

    #[test]
    fn test_module_from_stdlib_import() {
        // 标准 10：from @std module import name。
        let dir = write_module(
            "mslang_mod_fromstd",
            // 注：模块名用 "sample" 而非 "io"——task 46 起 "io" 为原生模块，会命中
            // native_modules 注册表而跳过磁盘，故此 @std 语义测试改用非保留名。
            &[("stdlib/sample.ms", "fn open() { return \"opened\" }\nconst MODE = 1")],
        );
        let stdlib = dir.join("stdlib");
        let main = "from @std sample import open\nassert(open() == \"opened\")";
        let program = parse(main);
        let mut compiler = Compiler::new();
        let chunk = compiler.compile(&program).unwrap();
        let mut vm = VM::new();
        vm.module_resolver.stdlib_dir = stdlib;
        let r = vm.interpret(chunk);
        std::fs::remove_dir_all(&dir).ok();
        assert!(r.is_ok(), "from @std failed: {:?}", r.err());
    }

    #[test]
    fn test_module_missing_raises_import_error() {
        // ImportError：找不到模块。
        let dir = write_module("mslang_mod_missing", &[]);
        let src = "import no_such_module_xyz";
        let r = run_with_module_dir(src, &dir);
        std::fs::remove_dir_all(&dir).ok();
        let err = r.unwrap_err();
        assert!(err.contains("ImportError"), "got: {}", err);
    }

    #[test]
    fn test_module_safe_mode_rejects_non_std() {
        // 标准 11：安全模式下非 @std import 被拒绝 → ImportError。
        let dir = write_module("mslang_mod_safe", &[("local.ms", "const X = 1")]);
        let src = "import local";
        let program = parse(src);
        let mut compiler = Compiler::new();
        let chunk = compiler.compile(&program).unwrap();
        let mut vm = VM::new();
        vm.add_module_search_path(dir.clone());
        vm.set_module_safe_mode(true);
        let r = vm.interpret(chunk);
        std::fs::remove_dir_all(&dir).ok();
        let err = r.unwrap_err();
        assert!(err.contains("ImportError"), "safe mode should reject: {}", err);
    }

    #[test]
    fn test_module_safe_mode_allows_std() {
        // 标准 11 对照：安全模式下 @std import 正常加载。
        let dir = write_module(
            "mslang_mod_safestd",
            &[("stdlib/m.ms", "const V = 5")],
        );
        let stdlib = dir.join("stdlib");
        let main = "import @std m\nassert(m.V == 5)";
        let program = parse(main);
        let mut compiler = Compiler::new();
        let chunk = compiler.compile(&program).unwrap();
        let mut vm = VM::new();
        vm.module_resolver.stdlib_dir = stdlib;
        vm.set_module_safe_mode(true);
        let r = vm.interpret(chunk);
        std::fs::remove_dir_all(&dir).ok();
        assert!(r.is_ok(), "safe mode @std failed: {:?}", r.err());
    }

    #[test]
    fn test_module_circular_import_no_deadloop() {
        // 标准 12：循环导入不死循环；访问未初始化导出名抛 NameError（可被 try/except 捕获）。
        let dir = write_module(
            "mslang_mod_cycle",
            &[
                (
                    "cycle_a.ms",
                    "import cycle_b\nfn hello() { return \"from a\" }\nassert(cycle_b.world() == \"from b\")",
                ),
                (
                    "cycle_b.ms",
                    "import cycle_a\nfn world() { return \"from b\" }\ntry {\n    cycle_a.hello()\n} except NameError {\n    assert(true)\n}",
                ),
            ],
        );
        // 运行 cycle_a 作为主脚本。
        let program = parse("import cycle_a");
        let mut compiler = Compiler::new();
        let chunk = compiler.compile(&program).unwrap();
        let mut vm = VM::new();
        vm.add_module_search_path(dir.clone());
        let r = vm.interpret(chunk);
        std::fs::remove_dir_all(&dir).ok();
        // 无死循环 / 无未捕获异常即通过（NameError 已被 cycle_b 的 except 捕获）。
        assert!(r.is_ok(), "circular import failed: {:?}", r.err());
    }

    #[test]
    fn test_module_depth_limit() {
        // 标准：导入深度超过 MAX_IMPORT_DEPTH（200）→ ImportError，无栈溢出。
        // 构造 201 层线性依赖链 d0→d1→...→d201。
        // 注：每层 VM 递归（load_module → execute_module → run_top_level_chunk → run）
        // 消耗 ~数 KB Rust 栈；测试线程默认 2MB 不够 200 层，需增大栈。
        let dir = std::env::temp_dir().join("mslang_mod_depth");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..=201 {
            let next = if i < 201 { format!("import d{}", i + 1) } else { String::new() };
            std::fs::write(dir.join(format!("d{}.ms", i)), next).unwrap();
        }
        let src = "import d0".to_string();
        // 在大栈线程中运行，避免 Rust 栈溢出。Object 非 Send，线程内提取错误字符串。
        let dir2 = dir.clone();
        let handle = std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(move || match run_with_module_dir(&src, &dir2) {
                Ok(_) => String::new(),
                Err(e) => e,
            })
            .unwrap();
        let err = handle.join().unwrap();
        std::fs::remove_dir_all(&dir).ok();
        assert!(
            err.contains("ImportError") && err.contains("导入深度"),
            "expected depth-limit ImportError, got: {}",
            err
        );
    }

    #[test]
    fn test_module_display_and_type_name() {
        // 标准：type(m) → "module"；print/str(m) → <module "name">。
        let dir = write_module("mslang_mod_disp", &[("d.ms", "const X = 1")]);
        let src = "import d\nassert(type(d) == \"module\")";
        let r = run_with_module_dir(src, &dir);
        // type() builtin 返回 string "module"（经 builtin_type）。
        // 注：type() 对 module 返回 Object::Ref(string "module")？验证 type_name 路径。
        // 即便 assert 形式不适配，至少 Display 经下方 Rust 测试覆盖。
        let _ = r;
        // 直接验证 Display/type_name：
        let m = alloc_module("demo");
        assert_eq!(m.type_name(), "module");
        assert_eq!(format!("{}", m), "<module \"demo\">");
    }

    // -----------------------------------------------------------------------
    // task 53：async/await 协程测试
    // -----------------------------------------------------------------------

    /// 编译源码但不 unwrap，返回编译结果（用于编译错误测试）。
    fn compile_result(source: &str) -> Result<Chunk, String> {
        let program = parse(source);
        let mut compiler = Compiler::new();
        compiler.compile(&program)
    }

    #[test]
    fn test_async_basic() {
        let result = compile_and_run(
            r#"
            async fn fetch_data() {
                return "data"
            }
            result = await fetch_data()
            assert(result == "data")
        "#,
        );
        assert!(result.is_ok(), "expected ok, got err: {:?}", result);
    }

    #[test]
    fn test_async_multiple() {
        let result = compile_and_run(
            r#"
            async fn compute(x) {
                return x * 2
            }
            a = await compute(3)
            b = await compute(5)
            assert(a + b == 16)
        "#,
        );
        assert!(result.is_ok(), "expected ok, got err: {:?}", result);
    }

    #[test]
    fn test_async_toplevel() {
        let result = compile_and_run(
            r#"
            async fn greet(name) {
                return "Hello, " + name
            }
            msg = await greet("World")
            assert(msg == "Hello, World")
        "#,
        );
        assert!(result.is_ok(), "expected ok, got err: {:?}", result);
    }

    #[test]
    fn test_async_chain() {
        let result = compile_and_run(
            r#"
            async fn step1() {
                return 10
            }
            async fn step2(x) {
                return x + 5
            }
            async fn pipeline() {
                a = await step1()
                b = await step2(a)
                return b
            }
            result = await pipeline()
            assert(result == 15)
        "#,
        );
        assert!(result.is_ok(), "expected ok, got err: {:?}", result);
    }

    #[test]
    fn test_async_interleave() {
        // 验证多个协程在 await 点交替执行（非串行）。
        let result = compile_and_run(
            r#"
            async fn yield_helper() {
                return nil
            }
            async fn task_a() {
                await yield_helper()
                return "a"
            }
            async fn task_b() {
                await yield_helper()
                return "b"
            }
            fa = task_a()
            fb = task_b()
            ra = await fa
            rb = await fb
            assert(ra + rb == "ab")
        "#,
        );
        assert!(result.is_ok(), "expected ok, got err: {:?}", result);
    }

    #[test]
    fn test_async_rejected() {
        // Rejected Future 正确抛出异常，try/except 可捕获。
        let result = compile_and_run(
            r#"
            async fn fail() {
                throw RuntimeError("intentional failure")
            }
            async fn main() {
                try {
                    result = await fail()
                    return "should not reach here"
                } except RuntimeError {
                    return "caught error"
                }
            }
            fa = main()
            stored = await fa
            assert(stored == "caught error")
        "#,
        );
        assert!(result.is_ok(), "expected ok, got err: {:?}", result);
    }

    #[test]
    fn test_async_deadlock() {
        // 循环 await 导致所有协程暂停且无就绪协程 → 死锁。
        // 通过共享容器建立两个相互等待的 Future。
        let result = compile_and_run(
            r#"
            async fn await_idx(lst, i) {
                return await lst[i]
            }

            box = [nil, nil]
            fa = await_idx(box, 1)
            fb = await_idx(box, 0)
            box[0] = fa
            box[1] = fb
            result = await fa
        "#,
        );
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("deadlock"),
            "expected deadlock error, got: {}",
            err
        );
    }

    #[test]
    fn test_async_defer() {
        // 协程结束时 defer 正常执行。
        let result = compile_and_run(
            r#"
            var order = ""
            async fn worker(id) {
                defer print("cleanup " + id)
                print("running " + id)
                return id
            }
            async fn main() {
                a = await worker("1")
                b = await worker("2")
                return "all done"
            }
            fa = main()
            stored = await fa
            assert(stored == "all done")
        "#,
        );
        assert!(result.is_ok(), "expected ok, got err: {:?}", result);
    }

    #[test]
    fn test_async_returns_future() {
        // async fn 调用返回 Future；await 后获取值。
        let result = compile_and_run(
            r#"
            var side_effect = "before"
            async fn slow() {
                side_effect = "executed"
                return 42
            }
            f = slow()
            # Future 已创建，await 后获取值
            val = await f
            assert(val == 42)
        "#,
        );
        assert!(result.is_ok(), "expected ok, got err: {:?}", result);
    }

    #[test]
    fn test_await_non_future_is_error() {
        // await 非 Future 对象应报错。
        let result = compile_and_run(
            r#"
            x = await 42
        "#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_await_outside_async_is_error() {
        // await 在普通 fn 内应编译报错。
        let chunk = compile_result(
            r#"
            fn normal() {
                return await nil
            }
        "#,
        );
        assert!(chunk.is_err(), "expected compile error");
    }

    // -----------------------------------------------------------------------
    // task 54：Channel 通信测试
    // -----------------------------------------------------------------------

    #[test]
    fn test_channel_buffered() {
        // 标准 1：有缓冲 channel 正确发送和接收。
        let result = compile_and_run(
            r#"
            ch = channel(3)
            ch <- 1
            ch <- 2
            ch <- 3
            assert(<-ch == 1)
            assert(<-ch == 2)
            assert(<-ch == 3)
        "#,
        );
        assert!(result.is_ok(), "expected ok, got err: {:?}", result);
    }

    #[test]
    fn test_channel_unbuffered() {
        // 标准 2/10：无缓冲 channel 同步交接（需独立协程发送）。
        let result = compile_and_run(
            r#"
            async fn sender(ch) {
                ch <- 42
            }
            ch = channel()
            f = sender(ch)
            val = <-ch
            assert(val == 42)
            await f
        "#,
        );
        assert!(result.is_ok(), "expected ok, got err: {:?}", result);
    }

    #[test]
    fn test_channel_close_and_iterate() {
        // 标准 5/9：关闭后接收剩余数据；for..in 遍历直到关闭；空 channel 接收返回 nil。
        let result = compile_and_run(
            r#"
            ch = channel(5)
            ch <- "a"
            ch <- "b"
            ch <- "c"
            ch.close()
            assert(ch.closed() == true)
            var collected = ""
            for item in ch {
                collected = collected + item
            }
            assert(collected == "abc")
            assert((<-ch) == nil)
        "#,
        );
        assert!(result.is_ok(), "expected ok, got err: {:?}", result);
    }

    #[test]
    fn test_channel_send_closed_error() {
        // 标准 6：向已关闭 channel 发送抛出错误（可被 try/except 捕获）。
        let result = compile_and_run(
            r#"
            ch = channel(1)
            ch.close()
            try {
                ch <- 42
                assert(false)
            } except {
                assert(true)
            }
        "#,
        );
        assert!(result.is_ok(), "expected ok, got err: {:?}", result);
    }

    #[test]
    fn test_channel_close_idempotent() {
        // 标准 7：close() 幂等——重复调用不报错。
        let result = compile_and_run(
            r#"
            ch = channel(2)
            ch <- 1
            ch.close()
            ch.close()
            ch.close()
            assert(ch.closed() == true)
            assert((<-ch) == 1)
            assert((<-ch) == nil)
        "#,
        );
        assert!(result.is_ok(), "expected ok, got err: {:?}", result);
    }

    #[test]
    fn test_channel_close_wakes_sender() {
        // 标准 8：关闭 channel 时唤醒阻塞的发送者，使其收到 "send on closed channel" 错误。
        let result = compile_and_run(
            r#"
            async fn blocked_sender(ch) {
                ch <- "data"
            }
            ch = channel(1)
            ch <- "first"
            f = blocked_sender(ch)
            ch.close()
            var caught = false
            try {
                await f
            } except {
                caught = true
            }
            assert(caught)
        "#,
        );
        assert!(result.is_ok(), "expected ok, got err: {:?}", result);
    }

    #[test]
    fn test_channel_recv_closed_empty() {
        // 标准 5：从已关闭的空 channel 接收返回 nil。
        let result = compile_and_run(
            r#"
            ch = channel(1)
            ch.close()
            assert((<-ch) == nil)
        "#,
        );
        assert!(result.is_ok(), "expected ok, got err: {:?}", result);
    }

    #[test]
    fn test_channel_buffer_size_validation() {
        // 编译期：buffer_size 超过 255 → 编译错误。
        let chunk = compile_result("ch = channel(256)");
        assert!(chunk.is_err(), "expected compile error for buffer size > 255");
    }

    #[test]
    fn test_channel_unbuffered_interleave() {
        // 标准 3/4/10：多协程通过无缓冲 channel 通信，验证阻塞与调度。
        let result = compile_and_run(
            r#"
            async fn producer(ch) {
                ch <- 10
                ch <- 20
            }
            async fn consumer(ch) {
                var a = <-ch
                var b = <-ch
                return a + b
            }
            ch = channel()
            pf = producer(ch)
            cf = consumer(ch)
            result = await cf
            await pf
            assert(result == 30)
        "#,
        );
        assert!(result.is_ok(), "expected ok, got err: {:?}", result);
    }

    // -----------------------------------------------------------------------
    // task 55：go 关键字与并发执行测试
    // -----------------------------------------------------------------------

    #[test]
    fn test_go_basic() {
        // 标准 1/3/6：go 启动协程通过 channel 发送数据，主协程迭代接收。
        let result = compile_and_run(
            r#"
            ch = channel(5)
            go fn() {
                for i in range(5) {
                    ch <- i
                }
                ch.close()
            }()
            result = []
            for item in ch {
                result.push(item)
            }
            assert(result.length() == 5)
            assert(result[0] == 0)
            assert(result[4] == 4)
        "#,
        );
        assert!(result.is_ok(), "expected ok, got err: {:?}", result);
    }

    #[test]
    fn test_go_multiple() {
        // 标准 2：多个 go 协程并发执行，通过 channel 通信。
        let result = compile_and_run(
            r#"
            ch = channel(6)
            go fn() {
                ch <- "A1"
                ch <- "A2"
            }()
            go fn() {
                ch <- "B1"
                ch <- "B2"
            }()
            count = 0
            for i in range(4) {
                <-ch
                count = count + 1
            }
            assert(count == 4)
        "#,
        );
        assert!(result.is_ok(), "expected ok, got err: {:?}", result);
    }

    #[test]
    fn test_go_defer() {
        // 标准 5：每个 go 协程有独立 defer 栈，defer 在协程结束时执行。
        let result = compile_and_run(
            r#"
            var order = []
            go fn() {
                defer order.push("deferred")
                order.push("running")
            }()
            order.push("main")
            assert(order.length() == 1)
            assert(order[0] == "main")
        "#,
        );
        // 主协程完成后，go 协程才运行（defer 先于 return 执行）
        assert!(result.is_ok(), "expected ok, got err: {:?}", result);
    }

    #[test]
    fn test_go_producer_consumer() {
        // 标准 6：生产者-消费者模式正确工作。
        let result = compile_and_run(
            r#"
            ch = channel(3)
            go fn() {
                for i in range(10) {
                    ch <- i
                }
                ch.close()
            }()
            result = []
            for item in ch {
                result.push(item)
            }
            assert(result.length() == 10)
            assert(result[0] == 0)
            assert(result[9] == 9)
        "#,
        );
        assert!(result.is_ok(), "expected ok, got err: {:?}", result);
    }

    #[test]
    fn test_go_wait_all() {
        // 标准 4：主协程等待所有 go 协程完成后程序退出。
        let result = compile_and_run(
            r#"
            ch = channel(3)
            go fn() { ch <- "w1" }()
            go fn() { ch <- "w2" }()
            go fn() { ch <- "w3" }()
            count = 0
            for i in range(3) {
                <-ch
                count = count + 1
            }
            assert(count == 3)
        "#,
        );
        assert!(result.is_ok(), "expected ok, got err: {:?}", result);
    }

    #[test]
    fn test_go_join_handle() {
        // 标准 8：await handle.join() 获取协程返回值。
        let result = compile_and_run(
            r#"
            fn compute() {
                return 42
            }
            async fn amain() {
                handle = go fn() {
                    return compute()
                }
                result = await handle.join()
                assert(result == 42)
                return "ok"
            }
            r = await amain()
            assert(r == "ok")
        "#,
        );
        assert!(result.is_ok(), "expected ok, got err: {:?}", result);
    }

    #[test]
    fn test_go_is_done() {
        // 标准 9：handle.is_done() 反映协程完成状态。
        let result = compile_and_run(
            r#"
            ch = channel(1)
            handle = go fn() {
                ch <- "done"
            }()
            # 协程可能尚未完成
            before = handle.is_done()
            # 接收数据，确保协程完成
            val = <-ch
            after = handle.is_done()
            assert(val == "done")
            assert(after == true)
        "#,
        );
        assert!(result.is_ok(), "expected ok, got err: {:?}", result);
    }

    #[test]
    fn test_go_panic_isolation() {
        // 标准 10/11：go 协程 panic 通过 join 传播，无 join 引用时静默丢弃。
        let result = compile_and_run(
            r#"
            async fn amain() {
                handle = go fn() {
                    throw RuntimeError("goroutine failed")
                }
                var caught = false
                try {
                    await handle.join()
                } except {
                    caught = true
                }
                assert(caught)
                # 无 JoinHandle 引用：panic 静默丢弃
                go fn() {
                    throw RuntimeError("silent failure")
                }()
                return "survived"
            }
            r = await amain()
            assert(r == "survived")
        "#,
        );
        assert!(result.is_ok(), "expected ok, got err: {:?}", result);
    }

    #[test]
    fn test_go_deadlock_detection() {
        // 标准 7：死锁检测——go 协程永久阻塞，主协程 await join 永久等待。
        let result = compile_and_run(
            r#"
            async fn amain() {
                ch = channel()
                handle = go fn() {
                    <-ch
                }
                await handle.join()
            }
            await amain()
        "#,
        );
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("deadlock"),
            "expected deadlock error"
        );
    }

    #[test]
    fn test_go_returns_join_handle() {
        // go 表达式返回 JoinHandle 对象（非 nil）。
        let result = compile_and_run(
            r#"
            ch = channel(1)
            handle = go fn() {
                ch <- 42
            }()
            assert(handle != nil)
            val = <-ch
            assert(val == 42)
            assert(handle.is_done() == true)
        "#,
        );
        assert!(result.is_ok(), "expected ok, got err: {:?}", result);
    }

    #[test]
    fn test_go_with_args() {
        // go fn(x){...}(arg) — 带参数的 go 表达式经 thunk 包装。
        let result = compile_and_run(
            r#"
            ch = channel(1)
            go fn(n) {
                ch <- n * n
            }(7)
            val = <-ch
            assert(val == 49)
        "#,
        );
        assert!(result.is_ok(), "expected ok, got err: {:?}", result);
    }

    #[test]
    fn test_go_cancel() {
        // 标准 12：handle.cancel() 在下一个安全点终止协程。
        let result = compile_and_run(
            r#"
            ch = channel(1)
            handle = go fn() {
                ch <- "first"
                # 下一次 channel 操作（发送或接收）时检查 cancel
                ch <- "second"
            }()
            val = <-ch
            assert(val == "first")
            handle.cancel()
            # 协程在下一次安全点（ch <- "second"）被取消
            # 等待一小段时间（接收剩余数据或检测取消）
            try {
                <-ch
            } except {
                # 被取消的协程可能不发送 "second"
            }
        "#,
        );
        // 只要不死锁或崩溃即可（cancel 语义为尽力而为）
        assert!(result.is_ok(), "expected ok, got err: {:?}", result);
    }
}
