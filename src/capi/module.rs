//! C API — C 扩展模块注册与动态加载（task 72）。
//!
//! 参照 [72-capi-module](../../docs/mslang/tasks/72-capi-module.md)。
//!
//! 实现模块定义结构体（MsFuncDef/MsConstDef/MsModuleDef）、静态注册
//! （msRegisterModule）、动态构建（msModuleNew + msModuleAddFunc +
//! msModuleAddConst + msRegisterModuleValue）、以及动态库加载（import 时
//! 自动搜索 .dll/.so/.dylib，调用 msModuleInit 入口函数）。

use std::ffi::CStr;
use std::os::raw::c_char;
use std::path::PathBuf;
use std::ptr;

use crate::capi::types::{MsAsyncFunction, MsCFunction, MsStatus, MsValue};
use crate::capi::vm::{lock_vm, MsVM};
use crate::vm::builtins::alloc_c_native_function;
use crate::vm::object::{
    alloc_module, alloc_native_async_function, read_module, read_module_mut, Object, TypeTag,
};

// ---------------------------------------------------------------------------
// FFI 结构体定义（cbindgen 自动生成到 module.h）
// ---------------------------------------------------------------------------

#[repr(C)]
pub struct MsFuncDef {
    pub name: *const c_char,
    pub func: MsCFunction,
}

#[repr(C)]
pub struct MsConstDef {
    pub name: *const c_char,
    pub val: *mut MsValue,
}

#[repr(C)]
pub struct MsModuleDef {
    pub name: *const c_char,
    pub methods: *const MsFuncDef,
    pub consts: *const MsConstDef,
}

const MAX_EXPORTS: usize = 1024;

// ---------------------------------------------------------------------------
// msRegisterModule — 从 MsModuleDef 批量注册
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn msRegisterModule(
    vm: *mut MsVM,
    def: *const MsModuleDef,
) -> MsStatus {
    if vm.is_null() || def.is_null() {
        return MsStatus::MS_ERROR;
    }

    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };

    let def_ref = unsafe { &*def };

    let module_name = unsafe { CStr::from_ptr(def_ref.name) }
        .to_string_lossy()
        .into_owned();

    if inner
        .vm
        .module_resolver
        .native_modules
        .contains_key(&module_name)
    {
        return MsStatus::MS_ERROR;
    }

    let module_obj = alloc_module(&module_name);
    let module_ptr = match module_obj {
        Object::Ref(p) => p,
        _ => return MsStatus::MS_ERROR,
    };

    if !def_ref.methods.is_null() {
        let mut mptr = def_ref.methods;
        unsafe {
            for _ in 0..MAX_EXPORTS {
                if (*mptr).name.is_null() {
                    break;
                }
                let method_name = CStr::from_ptr((*mptr).name)
                    .to_string_lossy()
                    .into_owned();
                if let Some(func) = (*mptr).func {
                    let fn_obj =
                        alloc_c_native_function(&method_name, Some(func), -1);
                    read_module_mut(module_ptr)
                        .exports
                        .insert(method_name, fn_obj);
                }
                mptr = mptr.add(1);
            }
        }
    }

    if !def_ref.consts.is_null() {
        let mut cptr = def_ref.consts;
        unsafe {
            for _ in 0..MAX_EXPORTS {
                if (*cptr).name.is_null() {
                    break;
                }
                let const_name = CStr::from_ptr((*cptr).name)
                    .to_string_lossy()
                    .into_owned();
                if !(*cptr).val.is_null() {
                    let val_obj = (*(*cptr).val).inner.clone();
                    read_module_mut(module_ptr)
                        .exports
                        .insert(const_name, val_obj);
                }
                cptr = cptr.add(1);
            }
        }
    }

    inner
        .vm
        .module_resolver
        .native_modules
        .insert(module_name, module_ptr);

    MsStatus::MS_OK
}

// ---------------------------------------------------------------------------
// msModuleNew — 创建空模块（动态构建入口）
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn msModuleNew(vm: *mut MsVM, name: *const c_char) -> *mut MsValue {
    if vm.is_null() || name.is_null() {
        return ptr::null_mut();
    }

    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };
    let _ = inner;

    let module_name = unsafe { CStr::from_ptr(name) }
        .to_string_lossy()
        .into_owned();

    let module_obj = alloc_module(&module_name);
    Box::into_raw(Box::new(MsValue {
        inner: module_obj,
    }))
}

// ---------------------------------------------------------------------------
// msModuleAddFunc — 向模块添加函数
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn msModuleAddFunc(
    vm: *mut MsVM,
    mod_val: *mut MsValue,
    name: *const c_char,
    fn_ptr: MsCFunction,
) -> MsStatus {
    if vm.is_null() || mod_val.is_null() || name.is_null() {
        return MsStatus::MS_ERROR;
    }

    let func_name = unsafe { CStr::from_ptr(name) }
        .to_string_lossy()
        .into_owned();

    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };
    let _ = inner;

    match &unsafe { &*mod_val }.inner {
        Object::Ref(ptr_)
            if unsafe { (**ptr_).type_tag } == TypeTag::MODULE as u8 =>
        {
            let func = match fn_ptr {
                Some(f) => f,
                None => return MsStatus::MS_ERROR,
            };
            let native_fn =
                alloc_c_native_function(&func_name, Some(func), -1);
            unsafe { read_module_mut(*ptr_) }
                .exports
                .insert(func_name, native_fn);
            MsStatus::MS_OK
        }
        _ => MsStatus::MS_ERROR,
    }
}

// ---------------------------------------------------------------------------
// msModuleAddAsyncFunc — 向模块添加 C 异步函数（task 76 实现）
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn msModuleAddAsyncFunc(
    vm: *mut MsVM,
    mod_val: *mut MsValue,
    name: *const c_char,
    fn_ptr: MsAsyncFunction,
) -> MsStatus {
    if vm.is_null() || mod_val.is_null() || name.is_null() {
        return MsStatus::MS_ERROR;
    }

    let func_name = unsafe { CStr::from_ptr(name) }
        .to_string_lossy()
        .into_owned();

    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };
    let _ = inner;

    match &unsafe { &*mod_val }.inner {
        Object::Ref(ptr_)
            if unsafe { (**ptr_).type_tag } == TypeTag::MODULE as u8 =>
        {
            let func = match fn_ptr {
                Some(f) => f,
                None => return MsStatus::MS_ERROR,
            };
            let async_fn = alloc_native_async_function(&func_name, Some(func), -1);
            unsafe { read_module_mut(*ptr_) }
                .exports
                .insert(func_name, async_fn);
            MsStatus::MS_OK
        }
        _ => MsStatus::MS_ERROR,
    }
}

// ---------------------------------------------------------------------------
// msModuleAddConst — 向模块添加常量
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn msModuleAddConst(
    vm: *mut MsVM,
    mod_val: *mut MsValue,
    name: *const c_char,
    val: *mut MsValue,
) -> MsStatus {
    if vm.is_null() || mod_val.is_null() || name.is_null() || val.is_null() {
        return MsStatus::MS_ERROR;
    }

    let const_name = unsafe { CStr::from_ptr(name) }
        .to_string_lossy()
        .into_owned();
    let val_obj = unsafe { (*val).inner.clone() };

    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };
    let _ = inner;

    match &unsafe { &*mod_val }.inner {
        Object::Ref(ptr_)
            if unsafe { (**ptr_).type_tag } == TypeTag::MODULE as u8 =>
        {
            unsafe { read_module_mut(*ptr_) }
                .exports
                .insert(const_name, val_obj);
            MsStatus::MS_OK
        }
        _ => MsStatus::MS_ERROR,
    }
}

// ---------------------------------------------------------------------------
// msRegisterModuleValue — 注册已构建的模块
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn msRegisterModuleValue(
    vm: *mut MsVM,
    mod_val: *mut MsValue,
) -> MsStatus {
    if vm.is_null() || mod_val.is_null() {
        return MsStatus::MS_ERROR;
    }

    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };

    match &unsafe { &*mod_val }.inner {
        Object::Ref(ptr_)
            if unsafe { (**ptr_).type_tag } == TypeTag::MODULE as u8 =>
        {
            let module_name = unsafe { read_module(*ptr_) }.name.clone();
            if inner
                .vm
                .module_resolver
                .native_modules
                .contains_key(&module_name)
            {
                return MsStatus::MS_ERROR;
            }
            inner
                .vm
                .module_resolver
                .native_modules
                .insert(module_name, *ptr_);
            MsStatus::MS_OK
        }
        _ => MsStatus::MS_ERROR,
    }
}

// ---------------------------------------------------------------------------
// 动态库加载
// ---------------------------------------------------------------------------

fn format_native_lib_name(name: &str) -> String {
    if cfg!(target_os = "windows") {
        format!("{}.dll", name)
    } else if cfg!(target_os = "macos") {
        format!("lib{}.dylib", name)
    } else {
        format!("lib{}.so", name)
    }
}

fn search_native_module(
    search_paths: &[PathBuf],
    lib_filename: &str,
) -> Option<PathBuf> {
    search_paths
        .iter()
        .map(|p| p.join(lib_filename))
        .find(|p| p.exists())
}

pub(crate) fn load_native_module(
    vm: *mut MsVM,
    name: &str,
) -> Result<(), String> {
    let lib_filename = format_native_lib_name(name);

    let search_paths = {
        let guard = lock_vm(vm);
        let inner = unsafe { &*guard.get() };
        inner.vm.module_resolver.search_paths.clone()
    };

    let path = search_native_module(&search_paths, &lib_filename)
        .ok_or_else(|| format!("native module '{}' not found", name))?;

    let lib = unsafe {
        libloading::Library::new(&path)
            .map_err(|e| format!("cannot load '{}': {}", path.display(), e))?
    };

    let init_fn: libloading::Symbol<
        unsafe extern "C" fn(*mut MsVM) -> *const MsModuleDef,
    > = unsafe {
        lib.get(b"msModuleInit\0")
            .map_err(|e| format!("symbol 'msModuleInit' not found: {}", e))?
    };

    let def_ptr = unsafe { init_fn(vm) };
    if def_ptr.is_null() {
        return Err("msModuleInit returned NULL".into());
    }

    let status = msRegisterModule(vm, def_ptr);
    if status != MsStatus::MS_OK {
        return Err("msRegisterModule failed".into());
    }

    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };
    inner.vm.loaded_libs.push(lib);

    Ok(())
}

// ---------------------------------------------------------------------------
// Rust 单元测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capi::types::MsStatus;
    use crate::capi::value::*;
    use crate::capi::vm::*;
    use std::os::raw::{c_char, c_int};
    use std::ptr::{null, null_mut};

    extern "C" fn test_add(
        vm: *mut MsVM,
        args: *const *mut MsValue,
        _nargs: c_int,
    ) -> *mut MsValue {
        let a = unsafe { msToInt(vm, *args.offset(0)) };
        let b = unsafe { msToInt(vm, *args.offset(1)) };
        msInt(a + b)
    }

    extern "C" fn test_mul(
        vm: *mut MsVM,
        args: *const *mut MsValue,
        _nargs: c_int,
    ) -> *mut MsValue {
        let a = unsafe { msToInt(vm, *args.offset(0)) };
        let b = unsafe { msToInt(vm, *args.offset(1)) };
        msInt(a * b)
    }

    fn exec(vm: *mut MsVM, src: &str) -> MsStatus {
        let cs = std::ffi::CString::new(src).unwrap();
        let fname = std::ffi::CString::new("test.ms").unwrap();
        msExecString(vm, cs.as_ptr(), fname.as_ptr())
    }

    #[test]
    fn test_register_module() {
        let vm = msVmNew();

        let methods = vec![
            MsFuncDef {
                name: b"add\0".as_ptr() as *const c_char,
                func: Some(test_add),
            },
            MsFuncDef {
                name: b"mul\0".as_ptr() as *const c_char,
                func: Some(test_mul),
            },
            MsFuncDef {
                name: null(),
                func: None,
            },
        ];

        let pi_val = msFloat(std::f64::consts::PI);
        let consts = vec![
            MsConstDef {
                name: b"PI\0".as_ptr() as *const c_char,
                val: pi_val,
            },
            MsConstDef {
                name: null(),
                val: null_mut(),
            },
        ];

        let def = MsModuleDef {
            name: b"testmod\0".as_ptr() as *const c_char,
            methods: methods.as_ptr(),
            consts: consts.as_ptr(),
        };

        let status = msRegisterModule(vm, &def);
        assert_eq!(status, MsStatus::MS_OK);

        let result = exec(vm, "import testmod\nprint(testmod.add(3, 4))");
        assert_eq!(result, MsStatus::MS_OK);

        msValueFree(pi_val);
        msVmFree(vm);
    }

    #[test]
    fn test_dynamic_build() {
        let vm = msVmNew();

        let mod_val =
            msModuleNew(vm, b"dynmod\0".as_ptr() as *const c_char);
        assert!(!mod_val.is_null());

        let status = msModuleAddFunc(
            vm,
            mod_val,
            b"double\0".as_ptr() as *const c_char,
            Some(test_add),
        );
        assert_eq!(status, MsStatus::MS_OK);

        let const_val = msInt(42);
        let status = msModuleAddConst(
            vm,
            mod_val,
            b"ANSWER\0".as_ptr() as *const c_char,
            const_val,
        );
        assert_eq!(status, MsStatus::MS_OK);

        let status = msRegisterModuleValue(vm, mod_val);
        assert_eq!(status, MsStatus::MS_OK);

        let result = exec(vm, "import dynmod\nprint(dynmod.ANSWER)");
        assert_eq!(result, MsStatus::MS_OK);

        msValueFree(const_val);
        msValueFree(mod_val);
        msVmFree(vm);
    }

    #[test]
    fn test_duplicate_registration_fails() {
        let vm = msVmNew();

        let methods = vec![
            MsFuncDef {
                name: b"noop\0".as_ptr() as *const c_char,
                func: Some(test_add),
            },
            MsFuncDef {
                name: null(),
                func: None,
            },
        ];

        let def = MsModuleDef {
            name: b"dupmod\0".as_ptr() as *const c_char,
            methods: methods.as_ptr(),
            consts: null(),
        };

        assert_eq!(msRegisterModule(vm, &def), MsStatus::MS_OK);
        assert_eq!(msRegisterModule(vm, &def), MsStatus::MS_ERROR);

        msVmFree(vm);
    }

    #[test]
    fn test_null_def_returns_error() {
        let vm = msVmNew();

        assert_eq!(msRegisterModule(vm, null()), MsStatus::MS_ERROR);
        assert!(msModuleNew(vm, null()).is_null());
        assert!(msModuleNew(
            null_mut(),
            b"x\0".as_ptr() as *const c_char
        )
        .is_null());

        msVmFree(vm);
    }

    #[test]
    fn test_module_consts_accessible() {
        let vm = msVmNew();

        let version_val =
            msString(vm, b"2.0\0".as_ptr() as *const c_char);
        let consts = vec![
            MsConstDef {
                name: b"VERSION\0".as_ptr() as *const c_char,
                val: version_val,
            },
            MsConstDef {
                name: null(),
                val: null_mut(),
            },
        ];

        let methods = vec![MsFuncDef {
            name: null(),
            func: None,
        }];

        let def = MsModuleDef {
            name: b"constmod\0".as_ptr() as *const c_char,
            methods: methods.as_ptr(),
            consts: consts.as_ptr(),
        };

        msRegisterModule(vm, &def);

        let result =
            exec(vm, "import constmod\nassert(constmod.VERSION == \"2.0\")");
        assert_eq!(result, MsStatus::MS_OK);

        msValueFree(version_val);
        msVmFree(vm);
    }

    #[test]
    fn test_add_func_non_module_returns_error() {
        let vm = msVmNew();

        let not_module = msInt(42);
        let status = msModuleAddFunc(
            vm,
            not_module,
            b"foo\0".as_ptr() as *const c_char,
            Some(test_add),
        );
        assert_eq!(status, MsStatus::MS_ERROR);

        msValueFree(not_module);
        msVmFree(vm);
    }

    #[test]
    fn test_add_const_non_module_returns_error() {
        let vm = msVmNew();

        let not_module = msInt(42);
        let val = msInt(99);
        let status = msModuleAddConst(
            vm,
            not_module,
            b"foo\0".as_ptr() as *const c_char,
            val,
        );
        assert_eq!(status, MsStatus::MS_ERROR);

        msValueFree(not_module);
        msValueFree(val);
        msVmFree(vm);
    }

    #[test]
    fn test_register_module_value_non_module_returns_error() {
        let vm = msVmNew();

        let not_module = msInt(42);
        let status = msRegisterModuleValue(vm, not_module);
        assert_eq!(status, MsStatus::MS_ERROR);

        msValueFree(not_module);
        msVmFree(vm);
    }

    #[test]
    fn test_add_func_null_fn_returns_error() {
        let vm = msVmNew();

        let mod_val =
            msModuleNew(vm, b"nullfnmod\0".as_ptr() as *const c_char);
        let status = msModuleAddFunc(
            vm,
            mod_val,
            b"foo\0".as_ptr() as *const c_char,
            None,
        );
        assert_eq!(status, MsStatus::MS_ERROR);

        msValueFree(mod_val);
        msVmFree(vm);
    }

    #[test]
    fn test_multiple_modules_no_conflict() {
        let vm = msVmNew();

        let methods_a = vec![
            MsFuncDef {
                name: b"fn\0".as_ptr() as *const c_char,
                func: Some(test_add),
            },
            MsFuncDef {
                name: null(),
                func: None,
            },
        ];
        let def_a = MsModuleDef {
            name: b"modA\0".as_ptr() as *const c_char,
            methods: methods_a.as_ptr(),
            consts: null(),
        };

        let methods_b = vec![
            MsFuncDef {
                name: b"fn\0".as_ptr() as *const c_char,
                func: Some(test_mul),
            },
            MsFuncDef {
                name: null(),
                func: None,
            },
        ];
        let def_b = MsModuleDef {
            name: b"modB\0".as_ptr() as *const c_char,
            methods: methods_b.as_ptr(),
            consts: null(),
        };

        assert_eq!(msRegisterModule(vm, &def_a), MsStatus::MS_OK);
        assert_eq!(msRegisterModule(vm, &def_b), MsStatus::MS_OK);

        msVmFree(vm);
    }

    #[test]
    fn test_async_func_placeholder() {
        let vm = msVmNew();
        let mod_val =
            msModuleNew(vm, b"asyncmod\0".as_ptr() as *const c_char);

        let status = msModuleAddAsyncFunc(
            vm,
            mod_val,
            b"afn\0".as_ptr() as *const c_char,
            None,
        );
        assert_eq!(status, MsStatus::MS_ERROR);

        msValueFree(mod_val);
        msVmFree(vm);
    }

    #[test]
    fn test_dynamic_build_duplicate_fails() {
        let vm = msVmNew();

        let mod_val =
            msModuleNew(vm, b"ddup\0".as_ptr() as *const c_char);
        msModuleAddFunc(
            vm,
            mod_val,
            b"f\0".as_ptr() as *const c_char,
            Some(test_add),
        );
        assert_eq!(msRegisterModuleValue(vm, mod_val), MsStatus::MS_OK);

        let mod_val2 =
            msModuleNew(vm, b"ddup\0".as_ptr() as *const c_char);
        msModuleAddFunc(
            vm,
            mod_val2,
            b"f\0".as_ptr() as *const c_char,
            Some(test_add),
        );
        assert_eq!(
            msRegisterModuleValue(vm, mod_val2),
            MsStatus::MS_ERROR
        );

        msValueFree(mod_val);
        msValueFree(mod_val2);
        msVmFree(vm);
    }

    #[test]
    #[ignore]
    fn test_load_native_module() {
        let vm = msVmNew();

        let fixture_dir = std::env::var("CARGO_MANIFEST_DIR")
            .map(|d| {
                std::path::PathBuf::from(d)
                    .join("tests/fixtures/native_modules")
            })
            .unwrap();
        let dir_str = fixture_dir.to_string_lossy().into_owned();
        msAddModulePath(vm, dir_str.as_ptr() as *const c_char);

        let result =
            exec(vm, "import mymath\nprint(mymath.square(5))");
        assert_eq!(result, MsStatus::MS_OK);

        msVmFree(vm);
    }

    #[test]
    #[ignore]
    fn test_native_module_not_found() {
        let vm = msVmNew();

        let result = exec(vm, "import nonexistent_native_module_xyz");
        assert_eq!(result, MsStatus::MS_ERROR);

        msVmFree(vm);
    }
}
