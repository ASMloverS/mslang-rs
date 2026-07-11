//! task 53：async/await 协程运行时。
//!
//! 核心类型（Coroutine / EventLoop / YieldReason / PausedCoroutine）定义于
//! `vm/mod.rs`，因它们与 VM 内部类型（CallFrame / DeferEntry / ExceptionHandler）
//! 紧耦合。Future 对象定义于 `vm/object.rs`。
//!
//! 本模块为文档锚点与未来扩展点（task 54 Channel / task 55 go 关键字）。

pub use crate::vm::{Coroutine, EventLoop, PausedCoroutine};
