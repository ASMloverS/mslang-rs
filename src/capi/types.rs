//! C API 类型定义（Rust 侧），与 `include/mslang/types.h` 对应。
//!
//! task 65 在 C 头文件 `types.h` 中手写了 MsType/MsStatus/MsGcType/MsFutureState
//! 枚举和 MsGcStats 结构体。本模块在 Rust 侧创建对应的类型，使 C API 函数
//! （vm.rs/value.rs 等）能以 C 兼容的方式返回/接受这些类型。
//!
//! 注意：这些 Rust 类型仅供 Rust 内部使用（C API 函数的参数/返回值类型），
//! 不由 cbindgen 导出（已在 cbindgen.toml `[export] exclude` 中排除）——
//! C 侧定义来自手写的 types.h。

use crate::vm::object::Object;

/// C API 值的不透明包装。C 侧操作 `MsValue*`，Rust 侧经 Box 管理。
///
/// `inner` 为 `pub(crate)` 而非 `pub`：Object 非 `#[repr(C)]`，对外暴露布局无意义；
/// capi 模块内构造/解构仍可访问。
#[repr(C)]
pub struct MsValue {
    pub(crate) inner: Object,
}

/// C API 返回状态（与 types.h 中 MsStatus 对应）。
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum MsStatus {
    MS_OK = 0,
    MS_ERROR = -1,
    MS_YIELD = 1,
}

/// C API 类型标签（与 types.h 中 MsType 对应）。
///
/// 注意：此枚举与内部 TypeTag（14-gc.md:89-112）不同——MsType 包含
/// Nil/Bool/Int/Float 内联类型，TypeTag 仅含堆类型。转换映射由 task 67 实现。
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum MsType {
    Nil = 0,
    Bool = 1,
    Int = 2,
    Float = 3,
    String = 4,
    List = 5,
    Dict = 6,
    Tuple = 7,
    Set = 8,
    Function = 9,
    Class = 10,
    Instance = 11,
    Module = 12,
    Generator = 13,
    Future = 14,
    Channel = 15,
    Iterator = 16,
    BoundMethod = 17,
    JoinHandle = 18,
}

/// C 原生函数签名（与 types.h 中 `MsCFunction` 对应）。
/// `Option` 表示可为 NULL（C 侧）。MsVM 为不透明指针，使用裸指针类型。
pub type MsCFunction = Option<
    extern "C" fn(*mut crate::capi::vm::MsVM, *const *mut MsValue, i32) -> *mut MsValue,
>;
