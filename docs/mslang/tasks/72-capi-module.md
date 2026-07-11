# C API — C 扩展模块注册与动态加载

## 所属阶段

Phase 6 — 模块系统 + 标准库

## 前置任务

65-capi-infrastructure, 66-capi-vm, 68-capi-value-convert, 70-capi-call

- **65-capi-infrastructure**：提供 `src/capi/` 模块框架、`#[cfg(feature = "capi")]` 编译门控、cbindgen 头文件生成流程
- **66-capi-vm**：提供 `msVmNew` / `msVmFree` / `msExecString` 等 VM 生命周期 API，以及 `lock_vm` / `VmInner` 访问模式
- **68-capi-value-convert**：提供 `MsValue { inner: Object }` 包装、`msValueFree`
- **70-capi-call**：提供 `alloc_c_native_function`（`src/vm/builtins.rs:115`）将 `MsCFunction` 包装为 `MsCNativeFunction` 堆对象（`TypeTag::NATIVE_C_FUNCTION`）

## 目标

实现 `module.h` 的全部 API，覆盖以下三个维度：

1. **模块定义结构体**：`MsFuncDef` / `MsConstDef` / `MsModuleDef` 的 Rust 侧 FFI 读取
2. **静态注册与动态构建**：`msRegisterModule`（从 C 结构体批量注册）、`msModuleNew` + `msModuleAddFunc` + `msModuleAddAsyncFunc` + `msModuleAddConst` + `msRegisterModuleValue`（逐个添加后注册）
3. **动态库加载**：`import foo` 找不到 `foo.ms` 时，搜索 `foo.dll` / `libfoo.so` / `libfoo.dylib`，通过 `libloading` 加载并调用 `msModuleInit` 入口函数，注册返回的模块定义

## 设计规格

参照 [13-capi](../13-capi.md) § module.h。

### 模块定义结构

```c
typedef struct MsFuncDef {
    const char* name;
    MsCFunction func;
} MsFuncDef;

typedef struct MsConstDef {
    const char* name;
    MsValue* val;
} MsConstDef;

typedef struct MsModuleDef {
    const char* name;
    const MsFuncDef* methods;   // NULL 终止
    const MsConstDef* consts;   // NULL 终止
} MsModuleDef;
```

- `methods` 和 `consts` 均为 NULL 终止数组（最后一个元素的 `name` 为 `NULL`）
- `consts` 可为 `NULL`，表示模块无常量

### 静态注册

```c
MS_API MsStatus msRegisterModule(MsVM* vm, const MsModuleDef* def);
```

从 `MsModuleDef` 读取全部方法和常量，创建 Module 对象，注册到 VM 的模块缓存。

### 动态构建模块

```c
MS_API MsValue*  msModuleNew(MsVM* vm, const char* name);
MS_API MsStatus  msModuleAddFunc(MsVM* vm, MsValue* mod, const char* name, MsCFunction fn);
MS_API MsStatus  msModuleAddAsyncFunc(MsVM* vm, MsValue* mod, const char* name, MsAsyncFunction fn);
MS_API MsStatus  msModuleAddConst(MsVM* vm, MsValue* mod, const char* name, MsValue* val);
MS_API MsStatus  msRegisterModuleValue(MsVM* vm, MsValue* mod);
```

逐个构建模块内容后一次性注册。适合需要运行时决定模块内容的场景。

### 动态加载

入口函数签名：

```c
MS_MODULE_INIT const MsModuleDef* msModuleInit(MsVM* vm);
```

加载规则：

1. 脚本 `import foo`，搜索路径中找不到 `foo.ms`
2. 搜索平台对应的动态库文件（`foo.dll` / `libfoo.so` / `libfoo.dylib`）
3. `dlopen` 加载库，`dlsym("msModuleInit")` 查找入口符号
4. 调用 `msModuleInit(vm)`，获取 `MsModuleDef*`
5. 调用 `msRegisterModule` 注册返回的模块定义

安全提示：动态库加载执行任意原生代码，无签名验证。仅在可信环境中使用。

构建命令：

```bash
gcc -shared -fPIC -o mymath.so mymath.c -lmslang    # Linux/macOS
cl /LD mymath.c mslang.lib                            # Windows
```

## 实现细节

### 文件结构

```
src/capi/
├── mod.rs          # 已存在（Task 65）
├── module.rs       # 本任务主体
└── ...
```

### 1. Rust 侧 FFI 结构体定义

在 `src/capi/module.rs` 顶部定义与 C 侧对齐的结构体：

```rust
#[repr(C)]
pub struct MsFuncDef {
    pub name: *const c_char,
    pub func: Option<MsCFunction>,
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
```

`MsCFunction` 在 `src/capi/types.rs`（Task 65）中已定义。此处定义的
`#[repr(C)]` 结构体由 cbindgen 自动生成到 `module.h`（types.h:123-124
注释已预留）。确保 cbindgen.toml 的 `[export] exclude` 不排除这些类型。

### 2. msRegisterModule

```rust
use crate::capi::vm::{lock_vm, MsVM};
use crate::capi::types::{MsCFunction, MsStatus, MsValue};
use crate::vm::object::{alloc_module, alloc_c_native_function, read_module_mut, Object, TypeTag};
use crate::vm::builtins::alloc_c_native_function;

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

    // 重复注册检查
    if inner.vm.module_resolver.native_modules.contains_key(&module_name) {
        return MsStatus::MS_ERROR;
    }

    // 创建 Module 堆对象（TypeTag::MODULE）
    let module_obj = alloc_module(&module_name);
    let module_ptr = match module_obj {
        Object::Ref(p) => p,
        _ => return MsStatus::MS_ERROR,
    };

    // 注册方法：遍历 methods 直到 NULL 终止
    const MAX_EXPORTS: usize = 1024;
    if !def_ref.methods.is_null() {
        let mut ptr = def_ref.methods;
        unsafe {
            for _ in 0..MAX_EXPORTS {
                if (*ptr).name.is_null() { break; }
                let method_name = CStr::from_ptr((*ptr).name)
                    .to_string_lossy()
                    .into_owned();
                if let Some(func) = (*ptr).func {
                    let fn_obj = alloc_c_native_function(&method_name, func, -1);
                    read_module_mut(module_ptr).exports.insert(method_name, fn_obj);
                }
                ptr = ptr.add(1);
            }
        }
    }

    // 注册常量：遍历 consts 直到 NULL 终止
    if !def_ref.consts.is_null() {
        let mut ptr = def_ref.consts;
        unsafe {
            for _ in 0..MAX_EXPORTS {
                if (*ptr).name.is_null() { break; }
                let const_name = CStr::from_ptr((*ptr).name)
                    .to_string_lossy()
                    .into_owned();
                if !(*ptr).val.is_null() {
                    let val_obj = (*(*ptr).val).inner.clone();
                    read_module_mut(module_ptr).exports.insert(const_name, val_obj);
                }
                ptr = ptr.add(1);
            }
        }
    }

    // 注册到 VM 的原生模块表
    inner.vm.module_resolver.native_modules.insert(module_name, module_ptr);

    MsStatus::MS_OK
}
```

关键点：
- `alloc_c_native_function` 复用 Task 70 的逻辑（`src/vm/builtins.rs:115`），arity = -1 表示可变参数
- `alloc_module` 创建 MsModule 堆对象（`src/vm/object.rs:1029`）
- `read_module_mut` 直接操作 MsModule.exports（`src/vm/object.rs:1059`）
- 注册到 `inner.vm.module_resolver.native_modules`（`src/module/resolver.rs:41`），使 `import` 可命中
- `MAX_EXPORTS` 防止缺少 NULL 终止符时的无限遍历

### 3. msModuleNew

```rust
#[no_mangle]
pub extern "C" fn msModuleNew(
    vm: *mut MsVM,
    name: *const c_char,
) -> *mut MsValue {
    if vm.is_null() || name.is_null() {
        return std::ptr::null_mut();
    }

    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };

    let module_name = unsafe { CStr::from_ptr(name) }
        .to_string_lossy()
        .into_owned();

    let module_obj = alloc_module(&module_name);
    Box::into_raw(Box::new(MsValue { inner: module_obj }))
}
```

- 返回 `MsValue*`（Object::Ref → TypeTag::MODULE），调用方后续通过 `msModuleAddFunc` / `msModuleAddConst` 填充 exports
- 返回的 MsValue\* 由 Box 管理，调用方应 `msRoot` 或及时 `msValueFree`

### 4. msModuleAddFunc

```rust
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

    let native_fn = match fn_ptr {
        Some(f) => alloc_c_native_function(&func_name, f, -1),
        None => return MsStatus::MS_ERROR,
    };

    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };

    // 类型校验：mod_val 必须为 MODULE
    match &unsafe { &*mod_val }.inner {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::MODULE as u8 => {
            unsafe { read_module_mut(*ptr) }.exports.insert(func_name, native_fn);
            MsStatus::MS_OK
        }
        _ => MsStatus::MS_ERROR,
    }
}
```

> **msModuleAddAsyncFunc**：异步函数注册依赖 `MsAsyncFunction` 桥接和
> `msCallAsync`（task 76）。本任务 MVP 不实现，返回 `MS_ERROR` 占位
> 或标注为 `#[cfg(feature = "capi")] fn msModuleAddAsyncFunc(...) -> MsStatus { MS_ERROR }`。

### 5. msModuleAddConst

```rust
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

    match &unsafe { &*mod_val }.inner {
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::MODULE as u8 => {
            unsafe { read_module_mut(*ptr) }.exports.insert(const_name, val_obj);
            MsStatus::MS_OK
        }
        _ => MsStatus::MS_ERROR,
    }
}
```

### 6. msRegisterModuleValue

```rust
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
        Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::MODULE as u8 => {
            let module_name = unsafe { read_module(*ptr) }.name.clone();
            if inner.vm.module_resolver.native_modules.contains_key(&module_name) {
                return MsStatus::MS_ERROR;
            }
            inner.vm.module_resolver.native_modules.insert(module_name, *ptr);
            MsStatus::MS_OK
        }
        _ => MsStatus::MS_ERROR,
    }
}
```

重复注册同名模块返回 `MS_ERROR`，防止覆盖已缓存模块。

### 7. 动态库加载

#### 7.1 Cargo.toml 变更

将 `libloading` 作为 optional dependency，通过 capi feature 启用：

```toml
[dependencies]
libloading = { version = "0.8", optional = true }

[features]
capi = ["dep:libloading"]
```

#### 7.2 平台库文件名格式

```rust
fn format_native_lib_name(name: &str) -> String {
    if cfg!(target_os = "windows") {
        format!("{}.dll", name)
    } else if cfg!(target_os = "macos") {
        format!("lib{}.dylib", name)
    } else {
        format!("lib{}.so", name)
    }
}
```

#### 7.3 模块搜索路径

VmInner 的 `module_paths` 为 `Vec<String>`（`src/capi/vm.rs:40`）。
ModuleResolver 的 `search_paths` 为 `Vec<PathBuf>`（`src/module/resolver.rs:30`）。
动态库搜索复用 ModuleResolver.search_paths：

```rust
fn search_native_module(
    search_paths: &[PathBuf],
    lib_filename: &str,
) -> Option<PathBuf> {
    search_paths.iter()
        .map(|p| p.join(lib_filename))
        .find(|p| p.exists())
}
```

#### 7.4 加载核心逻辑

```rust
fn load_native_module(
    vm: *mut MsVM,
    name: &str,
) -> Result<(), String> {
    let lib_filename = format_native_lib_name(name);

    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };

    let path = search_native_module(
        &inner.vm.module_resolver.search_paths,
        &lib_filename,
    ).ok_or_else(|| format!("native module '{}' not found", name))?;

    drop(guard); // 释放锁，dlopen 可能耗时长

    let lib = unsafe {
        libloading::Library::new(&path)
            .map_err(|e| format!("cannot load '{}': {}", path.display(), e))?
    };

    let init_fn: libloading::Symbol<unsafe extern "C" fn(*mut MsVM) -> *const MsModuleDef> =
        unsafe {
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

    // 保持库句柄存活：存入 VM 的 loaded_libs，生命周期与 VM 相同
    let guard = lock_vm(vm);
    let inner = unsafe { &mut *guard.get() };
    inner.vm.loaded_libs.push(lib);

    Ok(())
}
```

关键设计决策：
- `lib` 句柄存入 `VM.loaded_libs: Vec<libloading::Library>`，生命周期与 VM 相同（msVmFree 时 drop）
- `Library` 必须存活以保持 C 函数指针有效（MsCNativeFunction 存储裸指针，不持有 Library 引用）
- 加载前先 drop guard 释放锁（dlopen 可能耗时长），注册时重新获取锁

#### 7.5 与 VM::load_module 集成

在 `VM::load_module`（`src/vm/mod.rs:3181`）中，`native_modules` 查找和
`.ms` 文件查找都失败后，尝试动态库加载：

```rust
// src/vm/mod.rs — VM::load_module 中，resolve() 失败后添加：
#[cfg(feature = "capi")]
{
    if let Ok(()) = crate::capi::module::load_native_module(
        self.capi_vm_ptr as *mut crate::capi::vm::MsVM,
        mod_name,
    ) {
        // load_native_module 内部已注册到 native_modules
        if let Some(&ptr) = self.module_resolver.native_modules.get(mod_name) {
            return Ok(ptr);
        }
    }
}
```

`#[cfg(feature = "capi")]` 门控确保不启用 capi 时不引入 libloading 依赖。

> **注意**：`load_native_module` 内部调用 `lock_vm(vm)` 加锁，但
> `VM::load_module` 由 VM 内部调用（已在 C API 层加锁或纯 Rust 调用）。
> 纯 Rust 调用时 `capi_vm_ptr` 为 null，此时动态库加载跳过（仅 C API 上下文可用）。

### 8. VM 内部结构扩展

在 `VM` 结构体中新增字段（`src/vm/mod.rs`）：

```rust
pub struct VM {
    // ... 已有字段 ...

    /// 已加载的动态库句柄，生命周期与 VM 相同。
    /// Library 必须存活以保持 C 函数指针有效。
    #[cfg(feature = "capi")]
    pub loaded_libs: Vec<libloading::Library>,
}
```

`VM::new()` 中初始化为 `Vec::new()`。模块注册通过已有的
`module_resolver.native_modules: HashMap<String, *mut MsObjHeader>`
（`src/module/resolver.rs:41`），无需新增字段。

### 9. 辅助函数

本任务的辅助函数已在 §2–§6 的代码中内联，无需独立的 helper 模块：

- 模块创建：`alloc_module(name)` → `Object::Ref`（`src/vm/object.rs:1029`）
- C 函数包装：`alloc_c_native_function(name, func, arity)` → `Object::Ref`（`src/vm/builtins.rs:115`）
- 模块读写：`read_module(ptr)` / `read_module_mut(ptr)`（`src/vm/object.rs:1050/1059`）
- 模块注册：`inner.vm.module_resolver.native_modules.insert(name, ptr)`

### 10. module.h 头文件

cbindgen 从 `src/capi/module.rs` 的 `#[repr(C)]` 结构体（MsFuncDef/MsConstDef/MsModuleDef）
和 `#[no_mangle] pub extern "C"` 函数自动生成。Task 65 的 `build.rs` 已配置
`module` → `module.h` 的生成规则。

生成后取消 `include/mslang/mslang.h` 中 `module.h` 的注释：

```c
#include "module.h"
```

## 验证标准

1. `msRegisterModule` 传入包含 2 个函数和 1 个常量的 `MsModuleDef`，注册成功，mslang 脚本可 `import` 并调用
2. `msModuleNew` + `msModuleAddFunc` + `msModuleAddConst` + `msRegisterModuleValue` 动态构建的模块可被 mslang 脚本正常导入使用
3. ~~`msModuleAddAsyncFunc` 注册的异步函数可通过 `msCallAsync` 调用~~（**Deferred to task 76**，本任务 `msModuleAddAsyncFunc` 为占位实现）
4. 模块常量（`MsConstDef`）可从 mslang 脚本通过 `module.CONST_NAME` 访问
5. 动态库加载：编译一个 C 扩展为 `.dll` / `.so`，`import` 时自动搜索、加载、调用 `msModuleInit`，模块功能可用
6. 动态库加载在库文件不存在时返回错误，不崩溃
7. `msModuleInit` 返回 NULL 时，加载失败并报告错误
8. 同名模块重复注册返回 `MS_ERROR`，不覆盖已有模块
9. 多个不同模块可同时注册且互不冲突
10. `msModuleAddFunc`/`msModuleAddConst` 对非 Module 类型的 `MsValue*` 返回 `MS_ERROR`
11. `cargo build --features capi` 编译无错误，`cargo test --features capi` 通过
12. 13-capi.md 中的完整扩展模块示例（fileio 模块）端到端工作正常

## 测试用例

### Rust 单元测试

```rust
#[cfg(test)]
#[cfg(feature = "capi")]
mod tests {
    use super::*;
    use crate::capi::vm::*;
    use crate::capi::value::*;

    extern "C" fn test_add(
        vm: *mut MsVM,
        args: *const *mut MsValue,
        nargs: c_int,
    ) -> *mut MsValue {
        let a = unsafe { msToInt(vm, *args.offset(0)) };
        let b = unsafe { msToInt(vm, *args.offset(1)) };
        msInt(a + b)
    }

    extern "C" fn test_mul(
        vm: *mut MsVM,
        args: *const *mut MsValue,
        nargs: c_int,
    ) -> *mut MsValue {
        let a = unsafe { msToInt(vm, *args.offset(0)) };
        let b = unsafe { msToInt(vm, *args.offset(1)) };
        msInt(a * b)
    }

    #[test]
    fn test_register_module() {
        let vm = msVmNew();

        let c_name = b"testmod\0".as_ptr() as *const c_char;

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
            name: c_name,
            methods: methods.as_ptr(),
            consts: consts.as_ptr(),
        };

        let status = msRegisterModule(vm, &def);
        assert_eq!(status, MsStatus::MS_OK);

        let script = b"import testmod\nprint(testmod.add(3, 4))\0";
        let result = msExecString(vm, script.as_ptr() as *const c_char, null());
        assert_eq!(result, MsStatus::MS_OK);

        msVmFree(vm);
    }

    #[test]
    fn test_dynamic_build() {
        let vm = msVmNew();

        let mod_val = msModuleNew(vm, b"dynmod\0".as_ptr() as *const c_char);
        assert!(!mod_val.is_null());

        let status = msModuleAddFunc(
            vm,
            mod_val,
            b"double\0".as_ptr() as *const c_char,
            Some(test_add), // 复用：此处仅验证注册流程
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

        let script = b"import dynmod\nprint(dynmod.ANSWER)\0";
        let result = msExecString(vm, script.as_ptr() as *const c_char, null());
        assert_eq!(result, MsStatus::MS_OK);

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

        assert_eq!(
            msRegisterModule(vm, null()),
            MsStatus::MS_ERROR,
        );
        assert!(msModuleNew(vm, null()).is_null());
        assert!(msModuleNew(null(), b"x\0".as_ptr() as *const c_char).is_null());

        msVmFree(vm);
    }

    #[test]
    fn test_module_consts_accessible() {
        let vm = msVmNew();

        let version_val = msString(vm, b"2.0\0".as_ptr() as *const c_char);
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

        let methods = vec![
            MsFuncDef { name: null(), func: None },
        ];

        let def = MsModuleDef {
            name: b"constmod\0".as_ptr() as *const c_char,
            methods: methods.as_ptr(),
            consts: consts.as_ptr(),
        };

        msRegisterModule(vm, &def);

        let script = b"import constmod\nassert(constmod.VERSION == \"2.0\")\0";
        let result = msExecString(vm, script.as_ptr() as *const c_char, null());
        assert_eq!(result, MsStatus::MS_OK);

        msVmFree(vm);
    }
}
```

### 集成测试 — 动态库加载（test_load_native）

此测试需要 C 编译器，CI 环境中可跳过：

```rust
#[cfg(test)]
#[cfg(feature = "capi")]
mod integration {
    use super::*;

    #[test]
    #[ignore] // 需要 C 编译器和动态库构建
    fn test_load_native_module() {
        // 前置条件：test_fixtures/mymath.c 已编译为 mymath.dll / libmymath.so

        let vm = msVmNew();

        let fixture_dir = std::env::var("CARGO_MANIFEST_DIR")
            .map(|d| std::path::PathBuf::from(d).join("tests/fixtures/native_modules"))
            .unwrap();
        let dir_str = fixture_dir.to_string_lossy().into_owned();
        msAddModulePath(vm, dir_str.as_ptr() as *const c_char);

        let script = b"import mymath\nprint(mymath.square(5))\0";
        let result = msExecString(vm, script.as_ptr() as *const c_char, null());
        assert_eq!(result, MsStatus::MS_OK);

        msVmFree(vm);
    }

    #[test]
    #[ignore]
    fn test_native_module_not_found() {
        let vm = msVmNew();

        let script = b"import nonexistent_native_module_xyz\0";
        let result = msExecString(vm, script.as_ptr() as *const c_char, null());
        assert_eq!(result, MsStatus::MS_ERROR);

        msVmFree(vm);
    }
}
```

### C 扩展测试夹具 — tests/fixtures/native_modules/mymath.c

```c
#include <mslang.h>

static MsValue* square(MsVM* vm, MsValue* const* args, int nargs) {
    int64_t n = msToInt(vm, args[0]);
    return msInt(n * n);
}

static const MsFuncDef funcs[] = {
    {"square", square},
    {NULL, NULL}
};

static const MsModuleDef def = {
    .name = "mymath",
    .methods = funcs,
    .consts = NULL,
};

MS_MODULE_INIT const MsModuleDef* msModuleInit(MsVM* vm) {
    return &def;
}
```

构建脚本（tests/fixtures/native_modules/build.sh）：

```bash
#!/bin/bash
gcc -shared -fPIC -o libmymath.so mymath.c \
    -I../../../include \
    -L../../../target/debug \
    -lmslang
```

### 端到端验证 — 完整 fileio 扩展

使用 13-capi.md 中的完整 fileio 扩展示例进行端到端验证：

1. 编译 fileio.c 为动态库
2. 将动态库所在路径加入 VM 搜索路径
3. 执行 mslang 脚本 `import fileio`，调用 `fileio.read` / `fileio.write`
4. 验证文件读写结果正确
