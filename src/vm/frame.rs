use crate::vm::object::MsObjHeader;

/// 调用帧。每个帧对应一次函数调用（或顶层脚本）。
/// closure 指向 MsClosure，经由 closure.function 取 MsFunction 的字节码与常量池。
#[derive(Clone)]
pub struct CallFrame {
    pub closure: *mut MsObjHeader,
    pub ip: usize,
    pub stack_base: usize,
    pub defer_stack_base: usize,
}

impl CallFrame {
    pub fn new(closure: *mut MsObjHeader, stack_base: usize) -> Self {
        Self {
            closure,
            ip: 0,
            stack_base,
            defer_stack_base: 0,
        }
    }

    pub fn snapshot(&self) -> CallFrame {
        self.clone()
    }
}
