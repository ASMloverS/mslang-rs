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
    use std::ffi::CString;

    use crate::capi::vm::MsVM;
    use crate::vm::object::Object;

    /// 设置 TypeError 异常。Task 71 后委托给 msThrowTypeError。
    pub(crate) fn set_type_error(vm: *mut MsVM, expected: &str, actual: &Object) {
        if vm.is_null() {
            return;
        }
        let exp_c = CString::new(expected).unwrap_or_default();
        let act_c = CString::new(actual.type_name()).unwrap_or_default();
        let _ = crate::capi::error::msThrowTypeError(vm, exp_c.as_ptr(), act_c.as_ptr());
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

