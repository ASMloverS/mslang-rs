# C API — C 扩展模块注册与动态加载

## 所属阶段

Phase 6 — 模块系统 + 标准库

## 前置任务

65-capi-infrastructure, 66-capi-vm, 70-capi-call

- **65-capi-infrastructure**：提供 `src/capi/` 模块框架、`#[cfg(feature = "capi")]` 编译门控、cbindgen 头文件生成流程
- **66-capi-vm**：提供 `msVmNew` / `msVmFree` / `msExecString` 等 VM 生命周期 API，以及 VM 内部结构的 C API 访问能力
- **70-capi-call**：提供 `msCall` / `msCallAsync` 及 `MsCFunction` / `MsAsyncFunction` 类型包装，本任务复用其 `NativeFunction` / `NativeAsyncFunction` 构建逻辑

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

`MsCFunction` 和 `MsAsyncFunction` 在 Task 70（call）中已定义，此处 `use` 引入。

### 2. msRegisterModule

```rust
#[no_mangle]
pub extern "C" fn msRegisterModule(
    vm: *mut MsVM,
    def: *const MsModuleDef,
) -> MsStatus {
    if vm.is_null() || def.is_null() {
        return MsStatus::MS_ERROR;
    }

    let vm_inner = unsafe { &mut *get_vm_inner(vm) };
    let _lock = vm_inner.lock();

    let def_ref = unsafe { &*def };

    let module_name = unsafe { CStr::from_ptr(def_ref.name) }
        .to_string_lossy()
        .into_owned();

    // 创建 Module 对象
    let module = create_module_object(&module_name);

    // 注册方法：遍历 methods 直到 NULL 终止
    if !def_ref.methods.is_null() {
        let mut ptr = def_ref.methods;
        unsafe {
            while !(*ptr).name.is_null() {
                let method_name = CStr::from_ptr((*ptr).name)
                    .to_string_lossy()
                    .into_owned();
                if let Some(func) = (*ptr).func {
                    let native_fn = wrap_native_function(func);
                    module.add_method(method_name, native_fn);
                }
                ptr = ptr.add(1);
            }
        }
    }

    // 注册常量：遍历 consts 直到 NULL 终止
    if !def_ref.consts.is_null() {
        let mut ptr = def_ref.consts;
        unsafe {
            while !(*ptr).name.is_null() {
                let const_name = CStr::from_ptr((*ptr).name)
                    .to_string_lossy()
                    .into_owned();
                let val = (*ptr).val;
                if !val.is_null() {
                    module.add_const(const_name, val);
                }
                ptr = ptr.add(1);
            }
        }
    }

    // 注册到 VM 模块缓存
    vm_inner.register_module(module_name, module);

    MsStatus::MS_OK
}
```

关键点：
- `wrap_native_function` 复用 Task 70 的逻辑，将 `MsCFunction` 函数指针包装为 VM 内部的 `NativeFunction` 对象
- `create_module_object` 创建一个新的 Module 内部对象（设置 `TypeTag::MODULE`）
- `vm_inner.register_module` 将模块存入 VM 的模块缓存（`HashMap<String, ModuleRef>`）
- 方法遍历以 `name == NULL` 为终止条件，与 C 侧 NULL 终止数组约定一致

### 3. msModuleNew

```rust
#[no_mangle]
pub extern "C" fn msModuleNew(
    vm: *mut MsVM,
    name: *const c_char,
) -> *mut MsValue {
    if vm.is_null() || name.is_null() {
        return null_mut();
    }

    let vm_inner = unsafe { &mut *get_vm_inner(vm) };
    let _lock = vm_inner.lock();

    let module_name = unsafe { CStr::from_ptr(name) }
        .to_string_lossy()
        .into_owned();

    let module = create_module_object(&module_name);
    allocate_value(vm_inner, module)
}
```

- 创建空 Module 对象（仅含 name，无方法和常量）
- 返回 `MsValue*` 指针，调用方后续通过 `msModuleAddFunc` / `msModuleAddConst` 填充内容
- 返回的值已被 GC 追踪，调用方如需跨调用帧持有应 `msRoot`

### 4. msModuleAddFunc / msModuleAddAsyncFunc

```rust
#[no_mangle]
pub extern "C" fn msModuleAddFunc(
    vm: *mut MsVM,
    mod_val: *mut MsValue,
    name: *const c_char,
    fn_ptr: Option<MsCFunction>,
) -> MsStatus {
    if vm.is_null() || mod_val.is_null() || name.is_null() {
        return MsStatus::MS_ERROR;
    }

    let vm_inner = unsafe { &mut *get_vm_inner(vm) };
    let _lock = vm_inner.lock();

    let func_name = unsafe { CStr::from_ptr(name) }
        .to_string_lossy()
        .into_owned();

    let native_fn = match fn_ptr {
        Some(f) => wrap_native_function(f),
        None => return MsStatus::MS_ERROR,
    };

    let module = unsafe { &mut *get_module_inner(mod_val) };
    module.add_method(func_name, native_fn);

    MsStatus::MS_OK
}
```

`msModuleAddAsyncFunc` 结构相同，区别在于：
- 参数类型为 `Option<MsAsyncFunction>`
- 调用 `wrap_native_async_function`（Task 70）包装为 `NativeAsyncFunction`

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

    let vm_inner = unsafe { &mut *get_vm_inner(vm) };
    let _lock = vm_inner.lock();

    let const_name = unsafe { CStr::from_ptr(name) }
        .to_string_lossy()
        .into_owned();

    let module = unsafe { &mut *get_module_inner(mod_val) };
    module.add_const(const_name, val);

    MsStatus::MS_OK
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

    let vm_inner = unsafe { &mut *get_vm_inner(vm) };
    let _lock = vm_inner.lock();

    let module = unsafe { &mut *get_module_inner(mod_val) };
    let module_name = module.name().to_owned();

    // 检查是否已注册同名模块
    if vm_inner.is_module_registered(&module_name) {
        return MsStatus::MS_ERROR;
    }

    vm_inner.register_module(module_name, module);
    MsStatus::MS_OK
}
```

重复注册同名模块返回 `MS_ERROR`，防止覆盖已缓存模块。

### 7. 动态库加载

#### 7.1 Cargo.toml 变更

在 `[dependencies]` 下添加：

```toml
[target.'cfg(feature = "capi")'.dependencies]
libloading = "0.8"
```

`libloading` 仅在 `capi` feature 启用时引入。

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

复用 Task 45 的 `ModuleResolver::search_paths`，在此基础上追加动态库搜索：

```rust
fn search_native_module_paths(
    search_paths: &[PathBuf],
    lib_filename: &str,
) -> Vec<PathBuf> {
    search_paths.iter()
        .map(|p| p.join(lib_filename))
        .filter(|p| p.exists())
        .collect()
}
```

#### 7.4 加载核心逻辑

```rust
fn load_native_module(
    vm: *mut MsVM,
    name: &str,
) -> Result<*mut MsValue, String> {
    let vm_inner = unsafe { &mut *get_vm_inner(vm) };

    let lib_filename = format_native_lib_name(name);
    let candidates = search_native_module_paths(
        &vm_inner.module_paths(),
        &lib_filename,
    );

    if candidates.is_empty() {
        return Err(format!("native module '{}' not found", name));
    }

    let path = &candidates[0];

    let lib = unsafe {
        libloading::Library::new(path)
            .map_err(|e| format!("cannot load '{}': {}", path.display(), e))?
    };

    let init_fn: libloading::Symbol<unsafe extern "C" fn(*mut MsVM) -> *const MsModuleDef> =
        unsafe {
            lib.get(b"msModuleInit\0")
                .map_err(|e| format!("symbol 'msModuleInit' not found in '{}': {}",
                    path.display(), e))?
        };

    let def_ptr = unsafe { init_fn(vm) };
    if def_ptr.is_null() {
        return Err("msModuleInit returned NULL".into());
    }

    let status = msRegisterModule(vm, def_ptr);
    if status != MsStatus::MS_OK {
        return Err("msRegisterModule failed".into());
    }

    // 保持库句柄存活，防止卸载
    vm_inner.add_loaded_library(lib);

    let module_name = name.to_owned();
    Ok(vm_inner.get_cached_module(&module_name)
        .ok_or_else(|| "module not found after registration".to_string())?)
}
```

关键设计决策：
- `lib` 句柄存入 `vm_inner.loaded_libs: Vec<libloading::Library>`，生命周期与 VM 相同
- 找到第一个匹配的动态库即加载，不继续搜索
- 入口函数返回 NULL 视为错误

#### 7.5 与模块系统（Task 45）集成

在 `ModuleResolver::load` 中，`.ms` 文件查找失败后，尝试动态库加载：

```rust
impl ModuleResolver {
    pub fn load(&mut self, name: &str, vm: *mut MsVM) -> Result<ModuleRef, ModuleError> {
        // 1. 检查缓存
        if let Some(m) = self.cache.get(name) {
            return Ok(m.clone());
        }

        // 2. 搜索 .ms 文件
        if let Some(path) = self.resolve(name) {
            return self.load_ms_file(&path, vm);
        }

        // 3. 搜索原生动态库（fallback）
        #[cfg(feature = "capi")]
        {
            if let Ok(module_val) = load_native_module(vm, name) {
                // load_native_module 内部已注册到缓存
                return self.cache.get(name)
                    .cloned()
                    .ok_or(ModuleError::NotFound(name.to_owned()));
            }
        }

        Err(ModuleError::NotFound(name.to_owned()))
    }
}
```

`#[cfg(feature = "capi")]` 门控确保不启用 capi 时不引入 libloading 依赖。

### 8. VM 内部结构扩展

VM 内部需新增以下字段以支持本任务：

```rust
struct VmInner {
    // ... 已有字段 ...

    /// 已加载的模块缓存（Task 45 已定义）
    modules: HashMap<String, *mut MsValueInner>,

    /// 已加载的动态库句柄，生命周期与 VM 相同
    #[cfg(feature = "capi")]
    loaded_libs: Vec<libloading::Library>,

    /// 模块搜索路径（Task 45 已定义，本任务复用）
    module_paths: Vec<PathBuf>,
}
```

新增方法：

```rust
impl VmInner {
    fn register_module(&mut self, name: String, module: *mut MsValueInner) { ... }
    fn is_module_registered(&self, name: &str) -> bool { ... }
    fn get_cached_module(&self, name: &str) -> Option<*mut MsValue> { ... }

    #[cfg(feature = "capi")]
    fn add_loaded_library(&mut self, lib: libloading::Library) {
        self.loaded_libs.push(lib);
    }
}
```

### 9. 辅助函数

```rust
/// 从 MsValue* 提取内部 Module 对象指针
fn get_module_inner(val: *mut MsValue) -> *mut MsValueInner {
    unsafe { (*val).inner }
}

/// 创建 Module 对象并包装为 MsValueInner
fn create_module_object(name: &str) -> *mut MsValueInner {
    // 分配 MsValueInner，设置 TypeTag::MODULE
    // 内部含 name: String, methods: HashMap<String, ...>, consts: HashMap<String, ...>
    ...
}

/// 将 MsCFunction 包装为 VM 内部的 NativeFunction 对象
fn wrap_native_function(func: MsCFunction) -> NativeFunction {
    // 复用 Task 70 的包装逻辑
    ...
}
```

### 10. module.h 头文件

cbindgen 从 `src/capi/module.rs` 的 `#[no_mangle] pub extern "C"` 函数自动生成。Task 65 的 `build.rs` 已配置 `module` → `module.h` 的生成规则。

生成后取消 `include/mslang/mslang.h` 中 `module.h` 的注释：

```c
#include "module.h"
```

## 验证标准

1. `msRegisterModule` 传入包含 2 个函数和 1 个常量的 `MsModuleDef`，注册成功，mslang 脚本可 `import` 并调用
2. `msModuleNew` + `msModuleAddFunc` + `msModuleAddConst` + `msRegisterModuleValue` 动态构建的模块可被 mslang 脚本正常导入使用
3. `msModuleAddAsyncFunc` 注册的异步函数可通过 `msCallAsync` 调用，返回 Future
4. 模块常量（`MsConstDef`）可从 mslang 脚本通过 `module.CONST_NAME` 访问
5. 动态库加载：编译一个 C 扩展为 `.dll` / `.so`，`import` 时自动搜索、加载、调用 `msModuleInit`，模块功能可用
6. 动态库加载在库文件不存在时返回错误，不崩溃
7. `msModuleInit` 返回 NULL 时，加载失败并报告错误
8. 同名模块重复注册返回 `MS_ERROR`，不覆盖已有模块
9. 多个不同模块可同时注册且互不冲突
10. `cargo build --features capi` 编译无错误，`cargo test --features capi` 通过
11. 13-capi.md 中的完整扩展模块示例（fileio 模块）端到端工作正常

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
