//! C API — Class 操作（task 73）。
//!
//! 参照 [73-capi-class](../../docs/mslang/tasks/73-capi-class.md)。
//!
//! 实现 `class.h` 中定义的全部 8 个 C API 函数，覆盖三个功能域：
//! 1. 获取和实例化（msGetClass / msInstanceNew）
//! 2. 实例属性（msInstanceGet / msInstanceSet / msIsInstance）
//! 3. C 侧定义 Class（msClassDefine / msClassAddMethod / msClassAddStatic）

use std::os::raw::c_int;

use crate::capi::types::{MsCFunction, MsStatus, MsValue};
use crate::capi::vm::{lock_vm, MsVM};
use crate::vm::builtins::alloc_c_native_function;
use crate::vm::object::{
    alloc_bound_method, alloc_class, alloc_instance, read_class, read_instance, MsObjHeader,
    Object, TypeTag,
};
use crate::capi::value::{MS_FALSE, MS_TRUE};

fn is_class(obj: &Object) -> bool {
    matches!(obj, Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::CLASS as u8)
}

fn is_instance(obj: &Object) -> bool {
    matches!(obj, Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::INSTANCE as u8)
}

fn extract_ref_ptr(obj: &Object) -> *mut MsObjHeader {
    match obj {
        Object::Ref(ptr) => *ptr,
        _ => std::ptr::null_mut(),
    }
}

fn collect_args(args: *const *mut MsValue, nargs: c_int) -> Vec<Object> {
    if nargs <= 0 || args.is_null() {
        return Vec::new();
    }
    (0..nargs as usize)
        .filter_map(|i| {
            let ptr = unsafe { *args.add(i) };
            if ptr.is_null() {
                None
            } else {
                Some(unsafe { (*ptr).inner.clone() })
            }
        })
        .collect()
}

fn check_instance_of(class_ptr: *mut MsObjHeader, target_ptr: *mut MsObjHeader) -> bool {
    let mut current = class_ptr;
    loop {
        if current == target_ptr {
            return true;
        }
        current = match unsafe { read_class(current) }.parent {
            Some(p) => p,
            None => return false,
        };
    }
}

#[no_mangle]
pub extern "C" fn msGetClass(vm: *mut MsVM, name: *const i8) -> *mut MsValue {
    if vm.is_null() || name.is_null() {
        return std::ptr::null_mut();
    }
    let name_str = unsafe { std::ffi::CStr::from_ptr(name) }
        .to_string_lossy()
        .into_owned();
    let guard = lock_vm(vm);
    let inner = unsafe { &*guard.get() };
    match inner.vm.globals().get(&name_str) {
        Some(obj) => {
            if is_class(obj) {
                Box::into_raw(Box::new(MsValue {
                    inner: obj.clone(),
                }))
            } else {
                std::ptr::null_mut()
            }
        }
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "C" fn msInstanceNew(
    vm: *mut MsVM,
    cls: *mut MsValue,
    args: *const *mut MsValue,
    nargs: c_int,
) -> *mut MsValue {
    if vm.is_null() || cls.is_null() || nargs < 0 {
        return std::ptr::null_mut();
    }
    let class_obj = unsafe { (*cls).inner.clone() };
    let class_ptr = if is_class(&class_obj) {
        extract_ref_ptr(&class_obj)
    } else {
        return std::ptr::null_mut();
    };

    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };

    let instance = alloc_instance(class_ptr);

    let init_ptr = unsafe { read_class(class_ptr).find_method("__init__") };

    if let Some(method_ptr) = init_ptr {
        let arg_vec = collect_args(args, nargs);
        let bound = alloc_bound_method(instance.clone(), method_ptr);
        if inner.vm.call_function(&bound, &arg_vec).is_err() {
            return std::ptr::null_mut();
        }
    }

    Box::into_raw(Box::new(MsValue { inner: instance }))
}

#[no_mangle]
pub extern "C" fn msInstanceGet(
    vm: *mut MsVM,
    obj: *mut MsValue,
    attr: *const i8,
) -> *mut MsValue {
    if vm.is_null() || obj.is_null() || attr.is_null() {
        return std::ptr::null_mut();
    }
    let attr_str = unsafe { std::ffi::CStr::from_ptr(attr) }
        .to_string_lossy()
        .into_owned();
    let obj_inner = unsafe { (*obj).inner.clone() };

    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };

    if let Object::Ref(ptr) = &obj_inner {
        let tag = unsafe { (**ptr).type_tag };
        if tag == TypeTag::INSTANCE as u8 {
            let inst = unsafe { read_instance(*ptr) };
            if let Some(field_val) = inst.fields.get(&attr_str) {
                return Box::into_raw(Box::new(MsValue {
                    inner: field_val.clone(),
                }));
            }
            let method_ptr = unsafe { read_class(inst.class).find_method(&attr_str) };
            if let Some(mp) = method_ptr {
                let bound = alloc_bound_method(obj_inner.clone(), mp);
                return Box::into_raw(Box::new(MsValue { inner: bound }));
            }
            inner.vm.has_error = true;
            inner.vm.error_message =
                format!("AttributeError: 'instance' has no attribute '{}'", attr_str);
            return std::ptr::null_mut();
        }
    }
    std::ptr::null_mut()
}

#[no_mangle]
pub extern "C" fn msInstanceSet(
    vm: *mut MsVM,
    obj: *mut MsValue,
    attr: *const i8,
    val: *mut MsValue,
) -> MsStatus {
    if vm.is_null() || obj.is_null() || attr.is_null() || val.is_null() {
        return MsStatus::MS_ERROR;
    }
    let attr_str = unsafe { std::ffi::CStr::from_ptr(attr) }
        .to_string_lossy()
        .into_owned();
    let val_obj = unsafe { (*val).inner.clone() };

    let guard = lock_vm(vm);
    let _inner = unsafe { &mut *guard.get() };

    let obj_inner = unsafe { &(*obj).inner };
    if is_instance(obj_inner) {
        let ptr = extract_ref_ptr(obj_inner);
        let inst = unsafe { read_instance(ptr) };
        inst.fields.insert(attr_str, val_obj);
        MsStatus::MS_OK
    } else {
        MsStatus::MS_ERROR
    }
}

#[no_mangle]
pub extern "C" fn msIsInstance(vm: *mut MsVM, obj: *mut MsValue, cls: *mut MsValue) -> c_int {
    if vm.is_null() || obj.is_null() || cls.is_null() {
        return MS_FALSE;
    }

    let obj_inner = unsafe { &(*obj).inner };
    let obj_class_ptr = if is_instance(obj_inner) {
        unsafe { read_instance(extract_ref_ptr(obj_inner)) }.class
    } else {
        return MS_FALSE;
    };

    let cls_inner = unsafe { &(*cls).inner };
    let target_ptr = if is_class(cls_inner) {
        extract_ref_ptr(cls_inner)
    } else {
        return MS_FALSE;
    };

    if check_instance_of(obj_class_ptr, target_ptr) {
        MS_TRUE
    } else {
        MS_FALSE
    }
}

#[no_mangle]
pub extern "C" fn msClassDefine(
    vm: *mut MsVM,
    name: *const i8,
    parent: *mut MsValue,
) -> *mut MsValue {
    if vm.is_null() || name.is_null() {
        return std::ptr::null_mut();
    }
    let name_str = unsafe { std::ffi::CStr::from_ptr(name) }
        .to_string_lossy()
        .into_owned();

    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };

    let class_obj = alloc_class(name_str.clone());
    let class_ptr = extract_ref_ptr(&class_obj);
    if class_ptr.is_null() {
        return std::ptr::null_mut();
    }

    let parent_ptr = if parent.is_null() {
        inner.vm.object_class
    } else {
        let parent_inner = unsafe { &(*parent).inner };
        if is_class(parent_inner) {
            extract_ref_ptr(parent_inner)
        } else {
            inner.vm.object_class
        }
    };
    unsafe {
        read_class(class_ptr).parent = Some(parent_ptr);
    }

    inner.vm.globals_mut().insert(name_str, class_obj.clone());

    Box::into_raw(Box::new(MsValue { inner: class_obj }))
}

#[no_mangle]
pub extern "C" fn msClassAddMethod(
    vm: *mut MsVM,
    cls: *mut MsValue,
    name: *const i8,
    method: MsCFunction,
) -> MsStatus {
    if vm.is_null() || cls.is_null() || name.is_null() || method.is_none() {
        return MsStatus::MS_ERROR;
    }
    let name_str = unsafe { std::ffi::CStr::from_ptr(name) }
        .to_string_lossy()
        .into_owned();

    let guard = lock_vm(vm);
    let _inner = unsafe { &mut *guard.get() };

    let cls_inner = unsafe { &(*cls).inner };
    if is_class(cls_inner) {
        let native_fn = alloc_c_native_function(&name_str, method, -1);
        let fn_ptr = extract_ref_ptr(&native_fn);
        if fn_ptr.is_null() {
            return MsStatus::MS_ERROR;
        }
        unsafe { read_class(extract_ref_ptr(cls_inner)) }.methods.insert(name_str, fn_ptr);
        MsStatus::MS_OK
    } else {
        MsStatus::MS_ERROR
    }
}

#[no_mangle]
pub extern "C" fn msClassAddStatic(
    vm: *mut MsVM,
    cls: *mut MsValue,
    name: *const i8,
    val: *mut MsValue,
) -> MsStatus {
    if vm.is_null() || cls.is_null() || name.is_null() || val.is_null() {
        return MsStatus::MS_ERROR;
    }
    let name_str = unsafe { std::ffi::CStr::from_ptr(name) }
        .to_string_lossy()
        .into_owned();
    let val_obj = unsafe { (*val).inner.clone() };

    let guard = lock_vm(vm);
    let _inner = unsafe { &mut *guard.get() };

    let cls_inner = unsafe { &(*cls).inner };
    if is_class(cls_inner) {
        unsafe { read_class(extract_ref_ptr(cls_inner)) }
            .class_attrs
            .insert(name_str, val_obj);
        MsStatus::MS_OK
    } else {
        MsStatus::MS_ERROR
    }
}

// ---------------------------------------------------------------------------
// Rust 单元测试
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(unused_unsafe)]
mod tests {
    use super::*;
    use crate::capi::types::MsStatus;
    use crate::capi::vm::*;
    use std::ffi::CString;

    fn make_vm() -> *mut MsVM {
        msVmNew()
    }

    fn exec(vm: *mut MsVM, source: &str) -> MsStatus {
        let s = CString::new(source).unwrap();
        let f = CString::new("test.ms").unwrap();
        unsafe { msExecString(vm, s.as_ptr(), f.as_ptr()) }
    }

    fn cstr(s: &str) -> CString {
        CString::new(s).unwrap()
    }

    #[test]
    fn test_get_class_and_instantiate() {
        let vm = make_vm();
        exec(
            vm,
            "class Point {\n  fn __init__(self, x, y) {\n    self.x = x\n    self.y = y\n  }\n}\n",
        );

        let name = cstr("Point");
        let cls = unsafe { msGetClass(vm, name.as_ptr()) };
        assert!(!cls.is_null());

        let x_arg = crate::capi::value::msInt(10);
        let y_arg = crate::capi::value::msInt(20);
        let args = [x_arg, y_arg];
        let instance = unsafe { msInstanceNew(vm, cls, args.as_ptr(), 2) };
        assert!(!instance.is_null());

        let attr_x = cstr("x");
        let val = unsafe { msInstanceGet(vm, instance, attr_x.as_ptr()) };
        assert!(!val.is_null());

        let attr_y = cstr("y");
        let val2 = unsafe { msInstanceGet(vm, instance, attr_y.as_ptr()) };
        assert!(!val2.is_null());

        msVmFree(vm);
    }

    #[test]
    fn test_get_class_not_found() {
        let vm = make_vm();
        let name = cstr("NonExistent");
        let cls = unsafe { msGetClass(vm, name.as_ptr()) };
        assert!(cls.is_null());
        msVmFree(vm);
    }

    #[test]
    fn test_get_class_not_a_class() {
        let vm = make_vm();
        exec(vm, "x = 42");
        let name = cstr("x");
        let cls = unsafe { msGetClass(vm, name.as_ptr()) };
        assert!(cls.is_null());
        msVmFree(vm);
    }

    #[test]
    fn test_instance_attributes() {
        let vm = make_vm();
        exec(
            vm,
            "class Box {\n  fn __init__(self) {\n    self.value = 0\n  }\n}\n",
        );

        let name = cstr("Box");
        let cls = unsafe { msGetClass(vm, name.as_ptr()) };
        let instance = unsafe { msInstanceNew(vm, cls, std::ptr::null(), 0) };
        assert!(!instance.is_null());

        let attr = cstr("value");
        let val = unsafe { msInstanceGet(vm, instance, attr.as_ptr()) };
        assert!(!val.is_null());

        let new_val = crate::capi::value::msInt(99);
        let status = unsafe { msInstanceSet(vm, instance, attr.as_ptr(), new_val) };
        assert_eq!(status, MsStatus::MS_OK);

        let updated = unsafe { msInstanceGet(vm, instance, attr.as_ptr()) };
        assert!(!updated.is_null());

        msVmFree(vm);
    }

    #[test]
    fn test_instance_get_method() {
        let vm = make_vm();
        exec(
            vm,
            "class Counter {\n  fn __init__(self) {\n    self.count = 0\n  }\n  fn increment(self) {\n    self.count = self.count + 1\n  }\n}\n",
        );

        let name = cstr("Counter");
        let cls = unsafe { msGetClass(vm, name.as_ptr()) };
        let instance = unsafe { msInstanceNew(vm, cls, std::ptr::null(), 0) };

        let method_name = cstr("increment");
        let method = unsafe { msInstanceGet(vm, instance, method_name.as_ptr()) };
        assert!(!method.is_null());

        msVmFree(vm);
    }

    #[test]
    fn test_instance_set_not_instance() {
        let vm = make_vm();
        let val = crate::capi::value::msInt(42);
        let attr = cstr("x");
        let new_val = crate::capi::value::msInt(1);
        let status = unsafe { msInstanceSet(vm, val, attr.as_ptr(), new_val) };
        assert_eq!(status, MsStatus::MS_ERROR);
        msVmFree(vm);
    }

    #[test]
    fn test_is_instance() {
        let vm = make_vm();
        exec(
            vm,
            "class Animal {\n  fn __init__(self, name) {\n    self.name = name\n  }\n}\nclass Dog < Animal {\n  fn __init__(self, name) {\n    super.__init__(name)\n  }\n}\n",
        );

        let dog_name = cstr("Dog");
        let dog_cls = unsafe { msGetClass(vm, dog_name.as_ptr()) };
        let arg = crate::capi::value::msString(vm, cstr("Rex").as_ptr());
        let args = [arg];
        let dog = unsafe { msInstanceNew(vm, dog_cls, args.as_ptr(), 1) };
        assert!(!dog.is_null());

        let animal_name = cstr("Animal");
        let animal_cls = unsafe { msGetClass(vm, animal_name.as_ptr()) };

        let result_dog = unsafe { msIsInstance(vm, dog, dog_cls) };
        assert_eq!(result_dog, MS_TRUE);

        let result_animal = unsafe { msIsInstance(vm, dog, animal_cls) };
        assert_eq!(result_animal, MS_TRUE);

        let not_instance = unsafe { msIsInstance(vm, dog_cls, dog) };
        assert_eq!(not_instance, MS_FALSE);

        msVmFree(vm);
    }

    #[test]
    fn test_is_instance_not_instance_obj() {
        let vm = make_vm();
        let val = crate::capi::value::msInt(42);
        let cls_ptr = crate::capi::value::msInt(0);
        let result = unsafe { msIsInstance(vm, val, cls_ptr) };
        assert_eq!(result, MS_FALSE);
        msVmFree(vm);
    }

    #[test]
    fn test_c_define_class() {
        let vm = make_vm();

        let class_name = cstr("Widget");
        let cls = unsafe { msClassDefine(vm, class_name.as_ptr(), std::ptr::null_mut()) };
        assert!(!cls.is_null());

        let retrieved = unsafe { msGetClass(vm, class_name.as_ptr()) };
        assert!(!retrieved.is_null());

        exec(vm, "w = Widget()\n");

        let w_name = cstr("w");
        let w_val = unsafe { crate::capi::vm::msGetGlobal(vm, w_name.as_ptr()) };
        assert!(!w_val.is_null());

        msVmFree(vm);
    }

    #[test]
    fn test_c_define_class_add_method() {
        let vm = make_vm();

        let class_name = cstr("Calculator");
        let cls = unsafe { msClassDefine(vm, class_name.as_ptr(), std::ptr::null_mut()) };
        assert!(!cls.is_null());

        let method_name = cstr("add");

        extern "C" fn add_method(
            vm: *mut MsVM,
            args: *const *mut MsValue,
            nargs: i32,
        ) -> *mut MsValue {
            if nargs < 3 {
                return std::ptr::null_mut();
            }
            let a = unsafe { crate::capi::value::msToInt(vm, *args.add(1)) };
            let b = unsafe { crate::capi::value::msToInt(vm, *args.add(2)) };
            crate::capi::value::msInt(a + b)
        }

        let status = unsafe { msClassAddMethod(vm, cls, method_name.as_ptr(), Some(add_method)) };
        assert_eq!(status, MsStatus::MS_OK);

        exec(vm, "c = Calculator()\nresult = c.add(3, 4)\n");

        let result_name = cstr("result");
        let result = unsafe { crate::capi::vm::msGetGlobal(vm, result_name.as_ptr()) };
        assert!(!result.is_null());

        msVmFree(vm);
    }

    #[test]
    fn test_static_attributes() {
        let vm = make_vm();

        let class_name = cstr("Config");
        let cls = unsafe { msClassDefine(vm, class_name.as_ptr(), std::ptr::null_mut()) };

        let version_name = cstr("version");
        let version_val = crate::capi::value::msString(vm, cstr("1.0.0").as_ptr());
        let status =
            unsafe { msClassAddStatic(vm, cls, version_name.as_ptr(), version_val) };
        assert_eq!(status, MsStatus::MS_OK);

        let count_name = cstr("count");
        let count_val = crate::capi::value::msInt(0);
        let status2 =
            unsafe { msClassAddStatic(vm, cls, count_name.as_ptr(), count_val) };
        assert_eq!(status2, MsStatus::MS_OK);

        exec(vm, "v = Config.version\nc = Config.count\n");

        let v_name = cstr("v");
        let v_val = unsafe { crate::capi::vm::msGetGlobal(vm, v_name.as_ptr()) };
        assert!(!v_val.is_null());

        let c_name = cstr("c");
        let c_val = unsafe { crate::capi::vm::msGetGlobal(vm, c_name.as_ptr()) };
        assert!(!c_val.is_null());

        msVmFree(vm);
    }

    #[test]
    fn test_c_define_class_with_parent() {
        let vm = make_vm();
        exec(
            vm,
            "class Base {\n  fn greet(self) {\n    return \"hello\"\n  }\n}\n",
        );

        let base_name = cstr("Base");
        let base_cls = unsafe { msGetClass(vm, base_name.as_ptr()) };

        let child_name = cstr("Child");
        let child_cls =
            unsafe { msClassDefine(vm, child_name.as_ptr(), base_cls) };
        assert!(!child_cls.is_null());

        let child = unsafe { msInstanceNew(vm, child_cls, std::ptr::null(), 0) };
        assert!(!child.is_null());

        let greet_name = cstr("greet");
        let greet_method = unsafe { msInstanceGet(vm, child, greet_name.as_ptr()) };
        assert!(!greet_method.is_null());

        let is_base = unsafe { msIsInstance(vm, child, base_cls) };
        assert_eq!(is_base, MS_TRUE);

        msVmFree(vm);
    }

    #[test]
    fn test_class_add_method_not_class() {
        let vm = make_vm();
        let val = crate::capi::value::msInt(42);
        let method_name = cstr("foo");

        extern "C" fn dummy(
            _vm: *mut MsVM,
            _args: *const *mut MsValue,
            _nargs: i32,
        ) -> *mut MsValue {
            std::ptr::null_mut()
        }

        let status =
            unsafe { msClassAddMethod(vm, val, method_name.as_ptr(), Some(dummy)) };
        assert_eq!(status, MsStatus::MS_ERROR);
        msVmFree(vm);
    }

    #[test]
    fn test_class_add_static_not_class() {
        let vm = make_vm();
        let val = crate::capi::value::msInt(42);
        let attr_name = cstr("foo");
        let attr_val = crate::capi::value::msInt(1);
        let status =
            unsafe { msClassAddStatic(vm, val, attr_name.as_ptr(), attr_val) };
        assert_eq!(status, MsStatus::MS_ERROR);
        msVmFree(vm);
    }

    #[test]
    fn test_null_safety() {
        let vm = make_vm();

        assert!(unsafe { msGetClass(std::ptr::null_mut(), cstr("X").as_ptr()) }.is_null());
        assert!(unsafe { msGetClass(vm, std::ptr::null()) }.is_null());
        assert!(unsafe {
            msInstanceNew(
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
                0,
            )
        }
        .is_null());
        assert!(unsafe {
            msInstanceGet(
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        }
        .is_null());
        assert_eq!(
            unsafe {
                msInstanceSet(
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    std::ptr::null_mut(),
                )
            },
            MsStatus::MS_ERROR
        );
        assert_eq!(
            unsafe { msIsInstance(std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut()) },
            MS_FALSE
        );
        assert!(unsafe {
            msClassDefine(std::ptr::null_mut(), std::ptr::null(), std::ptr::null_mut())
        }
        .is_null());
        assert_eq!(
            unsafe {
                msClassAddMethod(
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    None,
                )
            },
            MsStatus::MS_ERROR
        );
        assert_eq!(
            unsafe {
                msClassAddStatic(
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    std::ptr::null_mut(),
                )
            },
            MsStatus::MS_ERROR
        );

        msVmFree(vm);
    }
}
