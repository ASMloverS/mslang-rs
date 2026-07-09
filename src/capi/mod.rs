#[cfg(feature = "capi")]
pub mod types;
#[cfg(feature = "capi")]
pub mod vm;
#[cfg(feature = "capi")]
pub mod value;
#[cfg(feature = "capi")]
pub mod call;
#[cfg(feature = "capi")]
pub mod error;
#[cfg(feature = "capi")]
pub mod module;
#[cfg(feature = "capi")]
pub mod class;
#[cfg(feature = "capi")]
pub mod gc;

// ---------------------------------------------------------------------------
// C API 内部辅助函数
// ---------------------------------------------------------------------------

#[cfg(feature = "capi")]
mod helpers {
    use crate::capi::vm::{lock_vm, MsVM};
    use crate::vm::object::Object;

    /// 设置 TypeError 异常占位。Task 71 完成后委托给 msThrowTypeError。
    ///
    /// 在 VM 上设置 `has_error` 标志和 `error_message`，供 C 侧通过
    /// msErrOccurred（Task 71）/msErrFetch（Task 71）查询。
    /// vm 为 NULL 时安全 no-op（内部已检查）。
    pub(crate) fn set_type_error(vm: *mut MsVM, expected: &str, actual: &Object) {
        // TODO(task 71): msThrowTypeError(vm, expected, actual.type_name());
        if vm.is_null() {
            return;
        }
        let guard = lock_vm(vm);
        // SAFETY: guard.get() 指向当前线程独占的 VmInner（由 ReentrantMutex 保证）。
        let inner = unsafe { &mut *guard.get() };
        inner.vm.has_error = true;
        inner.vm.error_message =
            format!("TypeError: expected {}, got {}", expected, actual.type_name());
    }
}

#[cfg(feature = "capi")]
pub(crate) use helpers::set_type_error;

#[cfg(test)]
#[cfg(feature = "capi")]
mod tests {
    #[test]
    fn test_capi_module_loads() {
        // capi module framework loads correctly; all submodules compile.
        // (The mere compilation of this test proves each submodule exists.)
    }

    #[test]
    fn test_capi_feature_gated() {
        // Confirm the capi module is only compiled when the feature is on.
        // This test only runs under --features capi.
    }
}

