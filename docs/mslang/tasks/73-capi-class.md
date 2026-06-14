# C API — Class 操作

## 所属阶段

Phase 6 — 模块系统 + 标准库

## 前置任务

- 66-capi-vm
- 68-capi-value-convert

## 目标

实现 `class.h` 中定义的全部 C API 函数，覆盖三个功能域：

1. **获取和实例化**：从 VM 全局作用域按名称获取 Class 对象、通过 Class 创建实例（含 `__init__` 调用）
2. **实例属性**：获取/设置实例字段、类型判断（`msIsInstance`，含继承链检查）
3. **C 侧定义 Class**：纯 C 侧创建类、添加实例方法和静态属性，与 mslang 脚本侧类系统完全互操作

## 设计规格

参照 [13-capi.md](../13-capi.md) § class.h：

### 获取和实例化

```c
MS_API MsValue*  msGetClass(MsVM* vm, const char* name);
MS_API MsValue*  msInstanceNew(MsVM* vm, MsValue* cls, MsValue* const* args, int nargs);
```

- `msGetClass`：在 VM 全局作用域查找名为 `name` 的 Class 对象。未找到或非 Class 类型返回 NULL。
- `msInstanceNew`：以 `cls` 为模板创建新实例。若类定义了 `__init__` 方法，自动调用并传入 `args`。返回新实例的 `MsValue*`，失败返回 NULL。

### 实例属性

```c
MS_API MsValue*  msInstanceGet(MsVM* vm, MsValue* obj, const char* attr);
MS_API MsStatus  msInstanceSet(MsVM* vm, MsValue* obj, const char* attr, MsValue* val);
MS_API int       msIsInstance(MsVM* vm, MsValue* obj, MsValue* cls);
```

- `msInstanceGet`：按属性查找链（实例字段 → 类方法 → 父类 MRO）查找 `attr`。未找到返回 NULL 并设置 AttributeError。
- `msInstanceSet`：在实例字段中设置 `attr = val`。触发写屏障。返回 `MS_OK` 或 `MS_ERROR`。
- `msIsInstance`：检查 `obj` 是否是 `cls`（或 `cls` 任意父类）的实例。返回 `MS_TRUE` / `MS_FALSE`。

### C 侧定义 Class

```c
MS_API MsValue*  msClassDefine(MsVM* vm, const char* name, MsValue* parent);
MS_API MsStatus  msClassAddMethod(MsVM* vm, MsValue* cls, const char* name, MsCFunction method);
MS_API MsStatus  msClassAddStatic(MsVM* vm, MsValue* cls, const char* name, MsValue* val);
```

- `msClassDefine`：在 VM 中创建新 Class 对象。`parent` 为 NULL 时隐式继承 `Object`。注册为全局变量 `name`。返回 `MsValue*`。
- `msClassAddMethod`：向 `cls` 添加实例方法。将 `MsCFunction` 包装为 NativeFunction 并加入类方法表。返回 `MS_OK` 或 `MS_ERROR`。
- `msClassAddStatic`：向 `cls` 添加静态属性（值可以是函数、常量等）。返回 `MS_OK` 或 `MS_ERROR`。

## 实现细节

### 文件位置

- `src/capi/class.rs` — 本任务新增，实现全部 8 个 `ms*` 函数
- `include/mslang/class.h` — cbindgen 从 `src/capi/class.rs` 自动生成

### 依赖关系

本任务复用以下符号：

| 符号 | 来源 | 用途 |
|---|---|---|
| `MsVM` / `VmInner` | `src/capi/vm.rs` | VM 不透明类型及内部状态 |
| `MsValue` | `src/capi/types.rs` | 值的不透明类型 |
| `MsStatus` | `src/capi/types.rs` | 返回状态枚举 |
| `MsCFunction` | `include/mslang/types.h` | C 函数指针类型 |
| `Object::Ref` + `TypeTag::CLASS` | `src/vm/object.rs` | 运行时 Class 对象 |
| `Object::Ref` + `TypeTag::INSTANCE` | `src/vm/object.rs` | 运行时 Instance 对象 |
| `Object::Ref` + `TypeTag::FUNCTION` | `src/vm/object.rs` | 原生函数（通过 `alloc_native_function`） |

### msGetClass

```rust
#[no_mangle]
pub extern "C" fn msGetClass(vm: *mut MsVM, name: *const i8) -> *mut MsValue {
    if vm.is_null() || name.is_null() {
        return std::ptr::null_mut();
    }
    let name_str = unsafe {
        std::ffi::CStr::from_ptr(name).to_string_lossy().into_owned()
    };
    let vm_ref = unsafe { &*vm };
    let inner = vm_ref.inner.lock().unwrap();
    match inner.vm.globals().get(&name_str) {
        Some(obj) => {
            if matches!(obj, Object::Class(_)) {
                let val = Box::new(MsValue { inner: obj.clone() });
                Box::into_raw(val)
            } else {
                std::ptr::null_mut()
            }
        }
        None => std::ptr::null_mut(),
    }
}
```

逻辑：

1. 参数空指针检查
2. C 字符串转 Rust String
3. 加锁访问 VM 全局作用域
4. 查找 `name` 对应的全局变量
5. 类型检查：仅当值为 `Object::Class` 时返回
6. 克隆并包装为 `MsValue*` 返回

### msInstanceNew

```rust
#[no_mangle]
pub extern "C" fn msInstanceNew(
    vm: *mut MsVM,
    cls: *mut MsValue,
    args: *const *mut MsValue,
    nargs: i32,
) -> *mut MsValue {
    if vm.is_null() || cls.is_null() {
        return std::ptr::null_mut();
    }
    let class_obj = unsafe { (*cls).inner.clone() };
    if !matches!(class_obj, Object::Class(_)) {
        return std::ptr::null_mut();
    }

    let vm_ref = unsafe { &*vm };
    let mut inner = vm_ref.inner.lock().unwrap();

    let instance = match inner.vm.create_instance(&class_obj) {
        Ok(inst) => inst,
        Err(_) => return std::ptr::null_mut(),
    };

    let init_name = "__init__";
    if let Some(init_fn) = find_class_method(&class_obj, init_name) {
        let arg_vec = collect_args(args, nargs);
        let mut call_args = vec![instance.clone()];
        call_args.extend_from_slice(&arg_vec);
        match inner.vm.call_function(&init_fn, &call_args) {
            Ok(_) => {}
            Err(_) => return std::ptr::null_mut(),
        }
    }

    let val = Box::new(MsValue { inner: instance });
    Box::into_raw(val)
}

fn collect_args(args: *const *mut MsValue, nargs: i32) -> Vec<Object> {
    if nargs <= 0 || args.is_null() {
        return Vec::new();
    }
    (0..nargs as usize)
        .map(|i| unsafe { (*(*args.add(i))).inner.clone() })
        .collect()
}

fn find_class_method(class_obj: &Object, name: &str) -> Option<Object> {
    if let Object::Class(cls) = class_obj {
        cls.find_method(name)
    } else {
        None
    }
}
```

逻辑：

1. 参数空指针检查
2. 克隆 class 对象，验证类型为 Class
3. 加锁
4. 调用 `vm.create_instance` 创建实例
5. 查找 `__init__` 方法（沿 MRO 链）
6. 若存在 `__init__`：构造参数列表 `[self, args...]`，调用 `__init__`
7. 返回实例作为 `MsValue*`

`collect_args` 辅助函数将 C 侧 `MsValue* const*` 转为 `Vec<Object>`，供多个函数复用。

`find_class_method` 辅助函数沿 MRO 链查找方法，供 `msInstanceNew` 和 `msInstanceGet` 复用。

### msInstanceGet

```rust
#[no_mangle]
pub extern "C" fn msInstanceGet(
    vm: *mut MsVM,
    obj: *mut MsValue,
    attr: *const i8,
) -> *mut MsValue {
    if vm.is_null() || obj.is_null() || attr.is_null() {
        return std::ptr::null_mut();
    }
    let attr_str = unsafe {
        std::ffi::CStr::from_ptr(attr).to_string_lossy().into_owned()
    };
    let obj_inner = unsafe { (*obj).inner.clone() };

    let vm_ref = unsafe { &*vm };
    let inner = vm_ref.inner.lock().unwrap();

    match &obj_inner {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::INSTANCE as u8 => {
            let inst = unsafe { read_instance(*ptr) };
            if let Some(field_val) = inst.fields.get(&attr_str) {
                let val = Box::new(MsValue { inner: field_val.clone() });
                return Box::into_raw(val);
            }
            if let Some(method) = inst.class_obj.find_method(&attr_str) {
                let bound = inner.vm.bind_method(obj_inner.clone(), method);
                let val = Box::new(MsValue { inner: bound });
                return Box::into_raw(val);
            }
            set_attribute_error(&inner, &attr_str, "Instance");
            std::ptr::null_mut()
        }
        _ => std::ptr::null_mut(),
    }
}
```

逻辑：

1. 参数空指针检查
2. 克隆对象，C 字符串转 Rust String
3. 加锁
4. 验证 `obj` 为 Instance 类型
5. **查找顺序**：
   - 先查实例字段 (`instance.fields`)
   - 再查类方法 MRO 链 (`class_obj.find_method`)
   - 方法找到时，创建 BoundMethod 返回
6. 均未找到：设置 AttributeError 并返回 NULL

### msInstanceSet

```rust
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
    let attr_str = unsafe {
        std::ffi::CStr::from_ptr(attr).to_string_lossy().into_owned()
    };
    let val_obj = unsafe { (*val).inner.clone() };
    let obj_inner = unsafe { (*obj).inner.clone() };

    let vm_ref = unsafe { &*vm };
    let mut inner = vm_ref.inner.lock().unwrap();

    match &mut obj_inner {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::INSTANCE as u8 => {
            let inst = unsafe { read_instance_mut(*ptr) };
            let old = inst.fields.insert(attr_str, val_obj);
            if old.is_none() || old != Some(val_obj) {
                inner.vm.write_barrier(&obj_inner, &unsafe { (*val).inner });
            }
            MsStatus::MS_OK
        }
        _ => MsStatus::MS_ERROR,
    }
}
```

逻辑：

1. 参数空指针检查
2. 克隆值和对象
3. 加锁（可变访问）
4. 验证 `obj` 为 Instance 类型
5. 在 `instance.fields` 中插入/更新属性
6. 触发写屏障（`write_barrier`）：通知 GC 新引用关系
7. 返回 `MS_OK`

> 注意：`msInstanceSet` 内部已包含写屏障调用，C 侧不需要额外调用 `msWriteBarrier`。

### msIsInstance

```rust
#[no_mangle]
pub extern "C" fn msIsInstance(
    vm: *mut MsVM,
    obj: *mut MsValue,
    cls: *mut MsValue,
) -> i32 {
    if vm.is_null() || obj.is_null() || cls.is_null() {
        return MS_FALSE;
    }
    let obj_inner = unsafe { (*obj).inner.clone() };
    let cls_inner = unsafe { (*cls).inner.clone() };

    let obj_class = match &obj_inner {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::INSTANCE as u8 => {
            &unsafe { read_instance(*ptr) }.class_obj
        }
        _ => return MS_FALSE,
    };

    let _vm_ref = unsafe { &*vm };

    if let Object::Class(target_cls) = &cls_inner {
        if check_instance_of(obj_class, target_cls) {
            MS_TRUE
        } else {
            MS_FALSE
        }
    } else {
        MS_FALSE
    }
}

fn check_instance_of(
    obj_class: &Object,
    target_cls: &crate::vm::object::MsClass,
) -> bool {
    if let Object::Class(cls) = obj_class {
        let mut current = Some(cls);
        while let Some(c) = current {
            if std::ptr::eq(c as *const _, target_cls as *const _) {
                return true;
            }
            current = c.parent.as_ref().map(|p| p.as_ref());
        }
    }
    false
}
```

逻辑：

1. 参数空指针检查
2. 克隆对象，提取 `obj` 的 class 引用
3. 验证 `obj` 为 Instance，`cls` 为 Class
4. **沿 MRO 链遍历**：从实例的类开始，逐级向上检查是否等于目标类
5. 找到匹配返回 `MS_TRUE`，遍历完未找到返回 `MS_FALSE`

`check_instance_of` 辅助函数遍历 `class.parent` 链实现 MRO 检查。单继承场景下 MRO 为线性链，无需 C3 线性化。

### msClassDefine

```rust
#[no_mangle]
pub extern "C" fn msClassDefine(
    vm: *mut MsVM,
    name: *const i8,
    parent: *mut MsValue,
) -> *mut MsValue {
    if vm.is_null() || name.is_null() {
        return std::ptr::null_mut();
    }
    let name_str = unsafe {
        std::ffi::CStr::from_ptr(name).to_string_lossy().into_owned()
    };
    let parent_obj = if parent.is_null() {
        None
    } else {
        let p = unsafe { (*parent).inner.clone() };
        if matches!(p, Object::Class(_)) {
            Some(p)
        } else {
            None
        }
    };

    let vm_ref = unsafe { &*vm };
    let mut inner = vm_ref.inner.lock().unwrap();

    let class_obj = inner.vm.create_class(&name_str, parent_obj);
    let class_val = Box::new(MsValue { inner: class_obj.clone() });
    let raw = Box::into_raw(class_val);

    inner.vm.globals_mut().insert(name_str, class_obj);

    raw
}
```

逻辑：

1. 参数空指针检查
2. 解析类名字符串
3. 处理 `parent` 参数：NULL 表示隐式继承 `Object`；非 NULL 验证为 Class 类型
4. 加锁
5. 调用 `vm.create_class` 创建 Class 对象
6. 包装为 `MsValue*`
7. 注册到全局作用域（`globals[name] = class`）
8. 返回 Class 的 `MsValue*`

C 侧创建的类与 mslang 脚本中 `class Foo { ... }` 定义的类在运行时完全等价，均可从脚本侧实例化和调用。

### msClassAddMethod

```rust
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
    let name_str = unsafe {
        std::ffi::CStr::from_ptr(name).to_string_lossy().into_owned()
    };
    let cls_inner = unsafe { (*cls).inner.clone() };

    let vm_ref = unsafe { &*vm };
    let mut inner = vm_ref.inner.lock().unwrap();

    match &mut cls_inner {
        Object::Class(cls_obj) => {
            let native_fn = inner.vm.create_native_function(
                &name_str,
                method,
            );
            cls_obj.methods.insert(name_str, native_fn);
            MsStatus::MS_OK
        }
        _ => MsStatus::MS_ERROR,
    }
}
```

逻辑：

1. 参数空指针检查（含 `method` 函数指针 NULL 检查）
2. 解析方法名字符串
3. 加锁
4. 验证 `cls` 为 Class 类型
5. 将 `MsCFunction` 包装为 `NativeFunction` 对象（通过 `vm.create_native_function`）
6. 插入类方法表 (`class.methods[name] = native_fn`)
7. 返回 `MS_OK`

添加的方法作为实例方法：当实例调用该方法时，`self` 自动绑定为实例本身。方法接收参数为 `[self, arg1, arg2, ...]`。

### msClassAddStatic

```rust
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
    let name_str = unsafe {
        std::ffi::CStr::from_ptr(name).to_string_lossy().into_owned()
    };
    let val_obj = unsafe { (*val).inner.clone() };
    let cls_inner = unsafe { (*cls).inner.clone() };

    let vm_ref = unsafe { &*vm };
    let mut inner = vm_ref.inner.lock().unwrap();

    match &mut cls_inner {
        Object::Class(cls_obj) => {
            cls_obj.static_attrs.insert(name_str, val_obj);
            inner.vm.write_barrier(&cls_inner, &unsafe { (*val).inner });
            MsStatus::MS_OK
        }
        _ => MsStatus::MS_ERROR,
    }
}
```

逻辑：

1. 参数空指针检查
2. 解析属性名，克隆值
3. 加锁
4. 验证 `cls` 为 Class 类型
5. 插入类的静态属性表 (`class.static_attrs[name] = val`)
6. 触发写屏障
7. 返回 `MS_OK`

静态属性通过 `ClassName.attr_name` 访问，不依赖实例。典型用途：类常量、工厂方法、工具函数。

### 模块声明更新

`src/capi/mod.rs`：

```rust
#[cfg(feature = "capi")]
pub mod class;
```

由任务 65-capi-infrastructure 已包含占位声明，本任务填充实现。

### cbindgen 头文件生成

`build.rs` 已配置从 `src/capi/class.rs` 生成 `include/mslang/class.h`。本任务完成后：

1. `cargo build --features capi` 自动生成 `class.h`
2. 更新 `include/mslang/mslang.h` 取消 `#include "class.h"` 的注释

### 错误处理策略

| 函数 | 错误条件 | 返回值 |
|---|---|---|
| `msGetClass` | 名字不存在 / 非 Class 类型 | NULL |
| `msInstanceNew` | cls 非 Class / `__init__` 抛异常 | NULL |
| `msInstanceGet` | obj 非 Instance / 属性不存在 | NULL + AttributeError |
| `msInstanceSet` | obj 非 Instance | MS_ERROR |
| `msIsInstance` | obj 非 Instance / cls 非 Class | MS_FALSE |
| `msClassDefine` | name 为 NULL | NULL |
| `msClassAddMethod` | cls 非 Class / method 为 NULL | MS_ERROR |
| `msClassAddStatic` | cls 非 Class / val 为 NULL | MS_ERROR |

### 线程安全

所有函数内部自动加锁（`inner.lock().unwrap()`），与任务 66-capi-vm 的线程安全策略一致。C 侧无需手动加锁，除非需要多步操作的原子性。

## 验证标准

1. `msGetClass` 能获取 mslang 脚本定义的类
2. `msGetClass` 对非 Class 全局变量返回 NULL
3. `msGetClass` 对不存在的名字返回 NULL
4. `msInstanceNew` 正确创建实例
5. `msInstanceNew` 自动调用 `__init__` 并传递参数
6. `msInstanceNew` 对非 Class 参数返回 NULL
7. `msInstanceGet` 能获取实例字段
8. `msInstanceGet` 能获取类方法（返回 BoundMethod）
9. `msInstanceGet` 沿 MRO 链查找父类方法
10. `msInstanceGet` 对不存在的属性设置 AttributeError 并返回 NULL
11. `msInstanceSet` 正确设置实例字段
12. `msInstanceSet` 对非 Instance 参数返回 MS_ERROR
13. `msIsInstance` 正确判断直接实例关系
14. `msIsInstance` 沿继承链判断（子类实例对父类返回 MS_TRUE）
15. `msIsInstance` 对无关节选返回 MS_FALSE
16. `msClassDefine` 创建的类可从 mslang 脚本实例化
17. `msClassDefine` 创建的类正确继承父类方法
18. `msClassAddMethod` 添加的方法可作为实例方法调用
19. `msClassAddMethod` 添加的方法 `self` 正确绑定
20. `msClassAddStatic` 添加的静态属性可通过 `ClassName.attr` 访问
21. 所有函数对 NULL 指针参数安全处理（不崩溃）

## 测试用例

### Rust 单元测试

`src/capi/class.rs` 中 `#[cfg(test)] mod tests`：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::capi::vm::*;
    use crate::capi::types::*;
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
        exec(vm, r#"
            class Point {
                fn __init__(self, x, y) {
                    self.x = x
                    self.y = y
                }
            }
        "#);

        let name = cstr("Point");
        let cls = unsafe { msGetClass(vm, name.as_ptr()) };
        assert!(!cls.is_null());

        let x_arg = crate::capi::value::msInt(10);
        let y_arg = crate::capi::value::msInt(20);
        let args = [x_arg, y_arg];
        let instance = unsafe {
            msInstanceNew(vm, cls, args.as_ptr(), 2)
        };
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
        exec(vm, r#"
            class Box {
                fn __init__(self) {
                    self.value = 0
                }
            }
        "#);

        let name = cstr("Box");
        let cls = unsafe { msGetClass(vm, name.as_ptr()) };
        let instance = unsafe { msInstanceNew(vm, cls, std::ptr::null(), 0) };
        assert!(!instance.is_null());

        let attr = cstr("value");
        let val = unsafe { msInstanceGet(vm, instance, attr.as_ptr()) };
        assert!(!val.is_null());

        let new_val = crate::capi::value::msInt(99);
        let status = unsafe {
            msInstanceSet(vm, instance, attr.as_ptr(), new_val)
        };
        assert_eq!(status, MsStatus::MS_OK);

        let updated = unsafe { msInstanceGet(vm, instance, attr.as_ptr()) };
        assert!(!updated.is_null());

        msVmFree(vm);
    }

    #[test]
    fn test_instance_get_method() {
        let vm = make_vm();
        exec(vm, r#"
            class Counter {
                fn __init__(self) {
                    self.count = 0
                }
                fn increment(self) {
                    self.count = self.count + 1
                }
            }
        "#);

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
        let status = unsafe {
            msInstanceSet(vm, val, attr.as_ptr(), new_val)
        };
        assert_eq!(status, MsStatus::MS_ERROR);
        msVmFree(vm);
    }

    #[test]
    fn test_is_instance() {
        let vm = make_vm();
        exec(vm, r#"
            class Animal {
                fn __init__(self, name) {
                    self.name = name
                }
            }
            class Dog < Animal {
                fn __init__(self, name) {
                    super.__init__(name)
                }
            }
        "#);

        let dog_name = cstr("Dog");
        let dog_cls = unsafe { msGetClass(vm, dog_name.as_ptr()) };
        let arg = crate::capi::value::msString(vm, cstr("Rex").as_ptr());
        let args = [arg];
        let dog = unsafe {
            msInstanceNew(vm, dog_cls, args.as_ptr(), 1)
        };
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
        let cls_name = cstr("something");
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

        exec(vm, r#"
            w = Widget()
        "#);

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

        let status = unsafe {
            msClassAddMethod(vm, cls, method_name.as_ptr(), Some(add_method))
        };
        assert_eq!(status, MsStatus::MS_OK);

        exec(vm, r#"
            c = Calculator()
            result = c.add(3, 4)
        "#);

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
        let status = unsafe {
            msClassAddStatic(vm, cls, version_name.as_ptr(), version_val)
        };
        assert_eq!(status, MsStatus::MS_OK);

        let count_name = cstr("count");
        let count_val = crate::capi::value::msInt(0);
        let status2 = unsafe {
            msClassAddStatic(vm, cls, count_name.as_ptr(), count_val)
        };
        assert_eq!(status2, MsStatus::MS_OK);

        exec(vm, r#"
            v = Config.version
            c = Config.count
        "#);

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
        exec(vm, r#"
            class Base {
                fn greet(self) {
                    return "hello"
                }
            }
        "#);

        let base_name = cstr("Base");
        let base_cls = unsafe { msGetClass(vm, base_name.as_ptr()) };

        let child_name = cstr("Child");
        let child_cls = unsafe {
            msClassDefine(vm, child_name.as_ptr(), base_cls)
        };
        assert!(!child_cls.is_null());

        let child = unsafe {
            msInstanceNew(vm, child_cls, std::ptr::null(), 0)
        };
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

        let status = unsafe {
            msClassAddMethod(vm, val, method_name.as_ptr(), Some(dummy))
        };
        assert_eq!(status, MsStatus::MS_ERROR);
        msVmFree(vm);
    }

    #[test]
    fn test_class_add_static_not_class() {
        let vm = make_vm();
        let val = crate::capi::value::msInt(42);
        let attr_name = cstr("foo");
        let attr_val = crate::capi::value::msInt(1);
        let status = unsafe {
            msClassAddStatic(vm, val, attr_name.as_ptr(), attr_val)
        };
        assert_eq!(status, MsStatus::MS_ERROR);
        msVmFree(vm);
    }

    #[test]
    fn test_null_safety() {
        let vm = make_vm();

        assert!(unsafe { msGetClass(std::ptr::null_mut(), cstr("X").as_ptr()) }.is_null());
        assert!(unsafe { msGetClass(vm, std::ptr::null()) }.is_null());
        assert!(unsafe { msInstanceNew(std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null(), 0) }.is_null());
        assert!(unsafe { msInstanceGet(std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null()) }.is_null());
        assert_eq!(unsafe { msInstanceSet(std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null(), std::ptr::null_mut()) }, MsStatus::MS_ERROR);
        assert_eq!(unsafe { msIsInstance(std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut()) }, MS_FALSE);
        assert!(unsafe { msClassDefine(std::ptr::null_mut(), std::ptr::null(), std::ptr::null_mut()) }.is_null());
        assert_eq!(unsafe { msClassAddMethod(std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null(), None) }, MsStatus::MS_ERROR);
        assert_eq!(unsafe { msClassAddStatic(std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null(), std::ptr::null_mut()) }, MsStatus::MS_ERROR);

        msVmFree(vm);
    }
}
```

### C 集成测试

`tests/c/test_class.c`：

```c
#include <mslang.h>
#include <assert.h>
#include <string.h>

void test_get_class_and_instantiate(void) {
    MsVM* vm = msVmNew();
    msExecString(vm,
        "class Point {\n"
        "  fn __init__(self, x, y) {\n"
        "    self.x = x\n"
        "    self.y = y\n"
        "  }\n"
        "}\n",
        "test.ms");

    MsValue* cls = msGetClass(vm, "Point");
    assert(cls != NULL);

    MsValue* x = msInt(3);
    MsValue* y = msInt(4);
    MsValue* args[] = {x, y};
    MsValue* p = msInstanceNew(vm, cls, args, 2);
    assert(p != NULL);

    MsValue* px = msInstanceGet(vm, p, "x");
    assert(px != NULL);
    assert(msToInt(vm, px) == 3);

    MsValue* py = msInstanceGet(vm, p, "y");
    assert(py != NULL);
    assert(msToInt(vm, py) == 4);

    msVmFree(vm);
}

void test_instance_attributes(void) {
    MsVM* vm = msVmNew();
    msExecString(vm,
        "class Box {\n"
        "  fn __init__(self) {\n"
        "    self.value = 0\n"
        "  }\n"
        "}\n",
        "test.ms");

    MsValue* cls = msGetClass(vm, "Box");
    MsValue* box = msInstanceNew(vm, cls, NULL, 0);
    assert(box != NULL);

    MsValue* orig = msInstanceGet(vm, box, "value");
    assert(orig != NULL);
    assert(msToInt(vm, orig) == 0);

    MsValue* new_val = msInt(99);
    MsStatus s = msInstanceSet(vm, box, "value", new_val);
    assert(s == MS_OK);

    MsValue* updated = msInstanceGet(vm, box, "value");
    assert(updated != NULL);
    assert(msToInt(vm, updated) == 99);

    msVmFree(vm);
}

void test_is_instance(void) {
    MsVM* vm = msVmNew();
    msExecString(vm,
        "class Animal {\n"
        "  fn __init__(self, name) {\n"
        "    self.name = name\n"
        "  }\n"
        "}\n"
        "class Dog < Animal {\n"
        "  fn __init__(self, name) {\n"
        "    super.__init__(name)\n"
        "  }\n"
        "}\n",
        "test.ms");

    MsValue* dog_cls = msGetClass(vm, "Dog");
    MsValue* animal_cls = msGetClass(vm, "Animal");
    assert(dog_cls != NULL);
    assert(animal_cls != NULL);

    MsValue* name_arg = msString(vm, "Rex");
    MsValue* args[] = {name_arg};
    MsValue* dog = msInstanceNew(vm, dog_cls, args, 1);
    assert(dog != NULL);

    assert(msIsInstance(vm, dog, dog_cls) == MS_TRUE);
    assert(msIsInstance(vm, dog, animal_cls) == MS_TRUE);
    assert(msIsInstance(vm, dog_cls, dog) == MS_FALSE);

    msVmFree(vm);
}

static MsValue* add_method(MsVM* vm, MsValue* const* args, int nargs) {
    if (nargs < 3) return NULL;
    int64_t a = msToInt(vm, args[1]);
    int64_t b = msToInt(vm, args[2]);
    return msInt(a + b);
}

void test_c_define_class(void) {
    MsVM* vm = msVmNew();

    MsValue* cls = msClassDefine(vm, "Calculator", NULL);
    assert(cls != NULL);

    MsStatus s = msClassAddMethod(vm, cls, "add", add_method);
    assert(s == MS_OK);

    msExecString(vm,
        "c = Calculator()\n"
        "result = c.add(10, 20)\n",
        "test.ms");

    MsValue* result = msGetGlobal(vm, "result");
    assert(result != NULL);
    assert(msToInt(vm, result) == 30);

    msVmFree(vm);
}

void test_static_attributes(void) {
    MsVM* vm = msVmNew();

    MsValue* cls = msClassDefine(vm, "Config", NULL);
    assert(cls != NULL);

    MsValue* ver = msString(vm, "1.0.0");
    MsStatus s1 = msClassAddStatic(vm, cls, "version", ver);
    assert(s1 == MS_OK);

    MsValue* count = msInt(42);
    MsStatus s2 = msClassAddStatic(vm, cls, "default_count", count);
    assert(s2 == MS_OK);

    msExecString(vm,
        "v = Config.version\n"
        "c = Config.default_count\n",
        "test.ms");

    MsValue* v = msGetGlobal(vm, "v");
    assert(v != NULL);
    assert(strcmp(msToString(vm, v), "1.0.0") == 0);

    MsValue* c = msGetGlobal(vm, "c");
    assert(c != NULL);
    assert(msToInt(vm, c) == 42);

    msVmFree(vm);
}

int main(void) {
    test_get_class_and_instantiate();
    test_instance_attributes();
    test_is_instance();
    test_c_define_class();
    test_static_attributes();
    return 0;
}
```
