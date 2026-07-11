//! task 53：async/await 协程运行时。
//!
//! 核心类型（Coroutine / EventLoop / PausedCoroutine）定义于
//! `vm/mod.rs`，因它们与 VM 内部类型（CallFrame / DeferEntry / ExceptionHandler）
//! 紧耦合。Future 对象定义于 `vm/object.rs`。
//!
//! task 54：Channel 通信对象定义于 `channel` 子模块。

pub mod channel;

pub use crate::vm::{Coroutine, EventLoop, PausedCoroutine};
pub use channel::{alloc_channel, read_channel, ChannelState, MsChannel, WaitingReceiver, WaitingSender};
