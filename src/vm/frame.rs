use crate::vm::object::{MsObjHeader, Object};

/// 调用帧。每个帧对应一次函数调用（或顶层脚本）。
/// closure 指向 MsClosure，经由 closure.function 取 MsFunction 的字节码与常量池。
#[derive(Clone)]
pub struct CallFrame {
    pub closure: *mut MsObjHeader,
    pub ip: usize,
    pub stack_base: usize,
    pub defer_stack_base: usize,
    /// task 36：EXEC_DEFER trampoline 状态。true 表示本帧正处于 defer 刷新中——
    /// 下次进入 EXEC_DEFER 时须先弹出刚完成的 defer 调用返回值。每帧独立，
    /// 避免 defer callee 自身的（空）EXEC_DEFER 误触发弹栈。
    pub defer_flushing: bool,
    /// task 37：当前帧正在处理的异常（裸 throw 重抛 + finally-on-propagation 共用）。
    /// throw() 跳入 except 分派器前设为 Some(err)；except 命中分支经
    /// CLEAR_CURRENT_EXC 置 None（异常已处理），FINALLY_END 据此决定是否重抛。
    pub current_exc: Option<Object>,
    /// task 39：生成器帧标记。普通帧为 None；生成器帧为 Some(gen_ptr)，指向
    /// 所属 MsGenerator。YIELD / RETURN 据此识别生成器帧并走快照保存路径。
    pub gen_owner: Option<*mut MsObjHeader>,
}

impl CallFrame {
    pub fn new(closure: *mut MsObjHeader, stack_base: usize, defer_stack_base: usize) -> Self {
        Self {
            closure,
            ip: 0,
            stack_base,
            defer_stack_base,
            defer_flushing: false,
            current_exc: None,
            gen_owner: None,
        }
    }

    pub fn snapshot(&self) -> CallFrame {
        self.clone()
    }
}
