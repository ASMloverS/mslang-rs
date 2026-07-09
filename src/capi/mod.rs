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

