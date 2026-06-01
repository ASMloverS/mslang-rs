# C API 设计

## 概述

mslang C API 提供 C/C++ 程序与 mslang 脚本语言的完整交互能力，支持两个方向：

- **嵌入 (Embedding)**：C 程序创建 mslang VM，执行脚本，调用脚本函数
- **扩展 (Extension)**：用 C 编写原生模块，供 mslang 脚本 import 调用

### 设计原则

| 原则 | 说明 |
|---|---|
| 稳定 ABI | MsValue 等核心结构体完全隐藏，仅通过函数操作 |
| 线程安全 | per-VM 互斥锁，不同 VM 实例可并行 |
| 所有权明确 | GC root 注册机制管理 C 侧引用生命周期 |
| 错误可查 | 函数返回错误指示，通过 ms_err_fetch 获取详情 |
| 完整覆盖 | 覆盖 mslang 全部语言特性 |

## 头文件结构

```
include/mslang/
├── mslang.h              // umbrella，include 全部子头文件
├── types.h               // 核心类型定义、枚举、宏、版本号
├── vm.h                  // VM 生命周期、脚本执行、全局变量
├── value.h               // 值创建、类型判断、转换、比较、集合操作
├── call.h                // 函数调用（同步/异步）、参数传递
├── error.h               // 异常抛出、捕获、查询
├── module.h              // C 扩展模块定义、注册、动态加载
└── class.h               // Class 创建、实例属性、继承
```

使用方式：

```c
#include <mslang.h>       // 包含所有 API
// 或按需引用：
#include <mslang/vm.h>
#include <mslang/value.h>
```

## 版本宏

```c
#define MSLANG_VERSION_MAJOR  0
#define MSLANG_VERSION_MINOR  1
#define MSLANG_VERSION_PATCH  0
#define MSLANG_VERSION        "0.1.0"

#define MSLANG_VERSION_AT_LEAST(major, minor, patch) \
    (MSLANG_VERSION_MAJOR > (major) || \
     (MSLANG_VERSION_MAJOR == (major) && MSLANG_VERSION_MINOR > (minor)) || \
     (MSLANG_VERSION_MAJOR == (major) && MSLANG_VERSION_MINOR == (minor) && \
      MSLANG_VERSION_PATCH >= (patch)))
```

## 平台兼容性宏

```c
#ifdef _WIN32
    #ifdef MSLANG_BUILDING
        #define MS_API __declspec(dllexport)
    #else
        #define MS_API __declspec(dllimport)
    #endif
#else
    #define MS_API __attribute__((visibility("default")))
#endif

#ifdef _WIN32
    #define MS_MODULE_INIT __declspec(dllexport)
#else
    #define MS_MODULE_INIT __attribute__((visibility("default")))
#endif
```

---

## types.h — 核心类型

### 不透明类型

```c
typedef struct MsVM MsVM;
typedef struct MsValue MsValue;
```

- `MsVM`：虚拟机实例。线程安全（per-VM 锁）。
- `MsValue`：mslang 值的不透明引用。GC 管理生命周期。

### 函数类型

```c
typedef MsValue* (*MsCFunction)(MsVM* vm, MsValue* const* args, int nargs);

typedef void (*MsAsyncFunction)(MsVM* vm, MsValue* const* args, int nargs,
                                MsValue* future);

typedef void (*MsFinalizerFn)(MsVM* vm, MsValue* obj, void* userdata);
```

### 枚举

```c
typedef enum MsType {
    MS_TYPE_NIL = 0,
    MS_TYPE_BOOL,
    MS_TYPE_INT,
    MS_TYPE_FLOAT,
    MS_TYPE_STRING,
    MS_TYPE_LIST,
    MS_TYPE_DICT,
    MS_TYPE_TUPLE,
    MS_TYPE_SET,
    MS_TYPE_FUNCTION,
    MS_TYPE_CLASS,
    MS_TYPE_INSTANCE,
    MS_TYPE_MODULE,
    MS_TYPE_GENERATOR,
    MS_TYPE_FUTURE,
    MS_TYPE_CHANNEL,
    MS_TYPE_ITERATOR,
    MS_TYPE_BOUND_METHOD,
    MS_TYPE_JOIN_HANDLE,
} MsType;

typedef enum MsStatus {
    MS_OK      =  0,
    MS_ERROR   = -1,
    MS_YIELD   =  1,
} MsStatus;

typedef enum MsGcType {
    MS_GC_MINOR = 0,
    MS_GC_MAJOR = 1,
    MS_GC_FULL  = 2,
} MsGcType;
```

### 布尔常量

```c
#define MS_TRUE  1
#define MS_FALSE 0
```

---

## vm.h — VM 生命周期

### 创建与销毁

```c
MS_API MsVM* ms_vm_new(void);
MS_API void  ms_vm_free(MsVM* vm);
```

每个 `MsVM` 拥有独立的全局作用域、模块缓存、事件循环和 GC 堆。不同 VM 实例可在不同线程中并行使用。

### 配置

```c
MS_API void ms_add_module_path(MsVM* vm, const char* path);
MS_API void ms_set_args(MsVM* vm, int argc, const char** argv);

typedef int (*MsWriteFn)(const char* data, size_t len, void* userdata);
MS_API void ms_set_stdout(MsVM* vm, MsWriteFn fn, void* userdata);
MS_API void ms_set_stderr(MsVM* vm, MsWriteFn fn, void* userdata);
```

### 脚本执行

```c
MS_API MsStatus ms_exec_file(MsVM* vm, const char* path);
MS_API MsStatus ms_exec_string(MsVM* vm, const char* source, const char* filename);
MS_API MsValue* ms_eval(MsVM* vm, const char* expr);
```

- `ms_exec_file`：执行 `.ms` 文件
- `ms_exec_string`：执行源码字符串，`filename` 用于错误信息（可为 NULL）
- `ms_eval`：求值表达式字符串，返回结果（新引用）

### 全局变量

```c
MS_API MsValue* ms_get_global(MsVM* vm, const char* name);
MS_API MsStatus ms_set_global(MsVM* vm, const char* name, MsValue* val);
MS_API void     ms_del_global(MsVM* vm, const char* name);
```

### 线程安全

```c
MS_API void ms_vm_lock(MsVM* vm);
MS_API void ms_vm_unlock(MsVM* vm);
```

通常不需要手动加锁，所有 `ms_*` API 内部自动管理。仅在需要保证多步操作的原子性时使用。

---

## value.h — 值操作

### 引用管理（GC Root 注册）

mslang 使用**并发三色标记清扫 GC**（详见 [14-gc](14-gc.md)），不使用引用计数。C 侧通过 Root 注册机制告知 GC 某个对象正在被使用：

```c
MS_API MsValue* ms_root(MsVM* vm, MsValue* val);
MS_API void     ms_unroot(MsVM* vm, MsValue* val);
```

- `ms_root`：将对象注册为 GC 根。注册后 GC 不会回收此对象。返回 `val` 本身。
- `ms_unroot`：注销 GC 根。注销后对象可能被 GC 回收，C 侧不应再访问该指针。

**使用模式**：

```c
MsValue* obj = ms_get_global(vm, "config");
ms_root(vm, obj);

// ... 跨越多次 API 调用，GC 期间 obj 不会被回收 ...

ms_unroot(vm, obj);
```

**不需要 root 的场景**：

- API 返回值在当前调用帧内立即使用（如 `ms_to_string` 提取 C 字符串）
- 值仅作为参数传递给其他 API 调用（调用期间 GC 不会回收参数）

### 特殊值

```c
MS_API MsValue* ms_nil(void);
MS_API MsValue* ms_bool_val(int val);

#define MS_NIL       (ms_nil())
#define MS_TRUE_VAL  (ms_bool_val(1))
#define MS_FALSE_VAL (ms_bool_val(0))
```

单例值，不需要 root（但 root/unroot 安全）。

### 值创建

```c
MS_API MsValue* ms_int(int64_t val);
MS_API MsValue* ms_float(double val);

MS_API MsValue* ms_string(MsVM* vm, const char* str);
MS_API MsValue* ms_stringn(MsVM* vm, const char* str, size_t len);
MS_API MsValue* ms_string_fmt(MsVM* vm, const char* fmt, ...);
```

### 集合创建

```c
MS_API MsValue* ms_list_new(MsVM* vm);
MS_API MsValue* ms_dict_new(MsVM* vm);
MS_API MsValue* ms_set_new(MsVM* vm);

MS_API MsValue* ms_list_from(MsVM* vm, MsValue* const* items, int count);
MS_API MsValue* ms_tuple_from(MsVM* vm, MsValue* const* items, int count);
MS_API MsValue* ms_dict_from(MsVM* vm, MsValue* const* pairs, int count);
```

### 类型判断

```c
MS_API MsType ms_typeof(MsValue* val);

MS_API int ms_is_nil(MsValue* val);
MS_API int ms_is_bool(MsValue* val);
MS_API int ms_is_int(MsValue* val);
MS_API int ms_is_float(MsValue* val);
MS_API int ms_is_number(MsValue* val);
MS_API int ms_is_string(MsValue* val);
MS_API int ms_is_list(MsValue* val);
MS_API int ms_is_dict(MsValue* val);
MS_API int ms_is_tuple(MsValue* val);
MS_API int ms_is_set(MsValue* val);
MS_API int ms_is_function(MsValue* val);
MS_API int ms_is_class(MsValue* val);
MS_API int ms_is_instance(MsValue* val);
MS_API int ms_is_generator(MsValue* val);
MS_API int ms_is_future(MsValue* val);
MS_API int ms_is_channel(MsValue* val);
```

所有类型判断函数返回 `MS_TRUE` / `MS_FALSE`。

### 值转换

```c
MS_API int64_t     ms_to_int(MsVM* vm, MsValue* val);
MS_API double      ms_to_float(MsVM* vm, MsValue* val);
MS_API int         ms_to_bool(MsValue* val);
MS_API const char* ms_to_string(MsVM* vm, MsValue* val);
MS_API char*       ms_to_string_copy(MsVM* vm, MsValue* val);
```

- `ms_to_int` / `ms_to_float`：类型不匹配时设置异常并返回 0
- `ms_to_bool`：按 truthy 规则转换，不设异常
- `ms_to_string`：返回内部指针（借用引用），不需要 free，仅在 val 存活期间有效
- `ms_to_string_copy`：返回副本，调用方必须 `free()`

### 显式类型转换

```c
MS_API MsValue* ms_convert_int(MsVM* vm, MsValue* val);
MS_API MsValue* ms_convert_float(MsVM* vm, MsValue* val);
MS_API MsValue* ms_convert_str(MsVM* vm, MsValue* val);
MS_API MsValue* ms_convert_bool(MsVM* vm, MsValue* val);
MS_API MsValue* ms_convert_list(MsVM* vm, MsValue* val);
```

对应 mslang 的 `int()`、`str()` 等内置函数。

### 比较

```c
MS_API int     ms_eq(MsVM* vm, MsValue* a, MsValue* b);
MS_API int     ms_lt(MsVM* vm, MsValue* a, MsValue* b);
MS_API int     ms_le(MsVM* vm, MsValue* a, MsValue* b);
MS_API int     ms_gt(MsVM* vm, MsValue* a, MsValue* b);
MS_API int     ms_ge(MsVM* vm, MsValue* a, MsValue* b);
MS_API int     ms_is(MsValue* a, MsValue* b);
MS_API int64_t ms_hash(MsVM* vm, MsValue* val);
```

### 字符串操作

```c
MS_API size_t     ms_string_len(MsVM* vm, MsValue* str);
MS_API const char* ms_string_data(MsVM* vm, MsValue* str);
MS_API MsValue*   ms_string_concat(MsVM* vm, MsValue* a, MsValue* b);
MS_API MsValue*   ms_string_slice(MsVM* vm, MsValue* str, int start, int end);
```

### List 操作

```c
MS_API int      ms_list_len(MsVM* vm, MsValue* list);
MS_API MsValue* ms_list_get(MsVM* vm, MsValue* list, int index);
MS_API MsStatus ms_list_set(MsVM* vm, MsValue* list, int index, MsValue* val);
MS_API MsStatus ms_list_push(MsVM* vm, MsValue* list, MsValue* val);
MS_API MsValue* ms_list_pop(MsVM* vm, MsValue* list);
MS_API MsStatus ms_list_insert(MsVM* vm, MsValue* list, int index, MsValue* val);
MS_API int      ms_list_contains(MsVM* vm, MsValue* list, MsValue* val);
MS_API MsValue* ms_list_slice(MsVM* vm, MsValue* list, int start, int end, int step);
```

### Dict 操作

```c
MS_API int      ms_dict_len(MsVM* vm, MsValue* dict);
MS_API MsValue* ms_dict_get(MsVM* vm, MsValue* dict, MsValue* key);
MS_API MsValue* ms_dict_get_default(MsVM* vm, MsValue* dict, MsValue* key, MsValue* default_val);
MS_API MsStatus ms_dict_set(MsVM* vm, MsValue* dict, MsValue* key, MsValue* val);
MS_API MsStatus ms_dict_remove(MsVM* vm, MsValue* dict, MsValue* key);
MS_API int      ms_dict_contains(MsVM* vm, MsValue* dict, MsValue* key);
MS_API MsValue* ms_dict_keys(MsVM* vm, MsValue* dict);
MS_API MsValue* ms_dict_values(MsVM* vm, MsValue* dict);
MS_API MsValue* ms_dict_items(MsVM* vm, MsValue* dict);
```

### Tuple 操作

```c
MS_API int      ms_tuple_len(MsVM* vm, MsValue* tup);
MS_API MsValue* ms_tuple_get(MsVM* vm, MsValue* tup, int index);
MS_API MsStatus ms_tuple_unpack(MsVM* vm, MsValue* tup, MsValue*** items, int* count);
```

`ms_tuple_unpack`：调用方负责 `free(items)`，但不需要释放各元素（借用引用）。

### Set 操作

```c
MS_API int      ms_set_len(MsVM* vm, MsValue* set);
MS_API MsStatus ms_set_add(MsVM* vm, MsValue* set, MsValue* val);
MS_API MsStatus ms_set_remove(MsVM* vm, MsValue* set, MsValue* val);
MS_API int      ms_set_contains(MsVM* vm, MsValue* set, MsValue* val);
```

### 迭代器

```c
MS_API MsValue* ms_iter(MsVM* vm, MsValue* iterable);
MS_API MsStatus ms_next(MsVM* vm, MsValue* iterator, MsValue** out);
```

`ms_next`：返回 `MS_OK` 成功（`*out` 设为值），`MS_ERROR` 表示迭代结束（StopIteration）或异常。

### 通用属性/下标访问

```c
MS_API MsValue* ms_get_attr(MsVM* vm, MsValue* obj, const char* attr);
MS_API MsStatus ms_set_attr(MsVM* vm, MsValue* obj, const char* attr, MsValue* val);
MS_API MsValue* ms_get_item(MsVM* vm, MsValue* obj, MsValue* key);
MS_API MsStatus ms_set_item(MsVM* vm, MsValue* obj, MsValue* key, MsValue* val);
MS_API int64_t  ms_len(MsVM* vm, MsValue* val);
MS_API MsValue* ms_repr(MsVM* vm, MsValue* val);
```

---

## call.h — 函数调用

### 同步调用

```c
MS_API MsValue* ms_call(MsVM* vm, MsValue* func, MsValue* const* args, int nargs);
```

`func` 必须是可调用对象（函数、闭包、带 `__call__` 的实例）。返回函数返回值（新引用），异常时返回 NULL。

```c
#define ms_call0(vm, f)                        ms_call(vm, f, NULL, 0)
#define ms_call1(vm, f, a)                     ...
#define ms_call2(vm, f, a, b)                  ...
#define ms_call3(vm, f, a, b, c)               ...
```

### 异步调用

```c
MS_API MsValue* ms_call_async(MsVM* vm, MsValue* func, MsValue* const* args, int nargs);
MS_API MsValue* ms_await(MsVM* vm, MsValue* future);

typedef enum MsFutureState {
    MS_FUTURE_PENDING,
    MS_FUTURE_RESOLVED,
    MS_FUTURE_REJECTED,
} MsFutureState;

MS_API MsFutureState ms_future_state(MsVM* vm, MsValue* future);
MS_API void ms_future_resolve(MsVM* vm, MsValue* future, MsValue* result);
MS_API void ms_future_reject(MsVM* vm, MsValue* future, MsValue* error);
```

- `ms_call_async`：异步调用，立即返回 Future
- `ms_await`：阻塞等待 Future 完成
- `ms_future_resolve/reject`：手动完成 Future（C async 函数中使用）

### C 侧 async 函数

```c
typedef void (*MsAsyncFunction)(MsVM* vm, MsValue* const* args, int nargs,
                                MsValue* future);
```

C async 函数接收参数和一个 Future。必须在异步操作完成后调用 `ms_future_resolve` 或 `ms_future_reject`。

### Channel 操作

```c
MS_API MsValue*  ms_channel(MsVM* vm, int buffer_size);
MS_API MsStatus  ms_channel_send(MsVM* vm, MsValue* ch, MsValue* val);
MS_API MsValue*  ms_channel_recv(MsVM* vm, MsValue* ch);
MS_API MsStatus  ms_channel_close(MsVM* vm, MsValue* ch);
MS_API int       ms_channel_is_closed(MsVM* vm, MsValue* ch);
```

### 生成器操作

```c
MS_API MsValue*  ms_generator_iter(MsVM* vm, MsValue* generator);
MS_API MsStatus  ms_generator_next(MsVM* vm, MsValue* generator, MsValue** out);
```

---

## error.h — 异常处理

### 异常查询

```c
MS_API int      ms_err_occurred(MsVM* vm);
MS_API MsValue* ms_err_fetch(MsVM* vm);
MS_API void     ms_err_clear(MsVM* vm);
```

- `ms_err_occurred`：是否有异常待处理
- `ms_err_fetch`：取出异常对象（清除当前异常），返回新引用
- `ms_err_clear`：清除异常（不获取对象）

### 异常对象属性

```c
MS_API const char* ms_err_type_name(MsVM* vm, MsValue* err);
MS_API const char* ms_err_message(MsVM* vm, MsValue* err);
MS_API const char* ms_err_traceback(MsVM* vm, MsValue* err);
MS_API MsValue*    ms_err_cause(MsVM* vm, MsValue* err);
```

返回借用引用，仅在 err 存活期间有效。

### 从 C 抛出异常

```c
MS_API MsStatus ms_throw(MsVM* vm, const char* type, const char* fmt, ...);
MS_API MsStatus ms_throw_value(MsVM* vm, MsValue* err);
MS_API MsStatus ms_throw_rethrow(MsVM* vm);

MS_API MsStatus ms_throw_type_error(MsVM* vm, const char* expected, const char* actual);
MS_API MsStatus ms_throw_value_error(MsVM* vm, const char* fmt, ...);
MS_API MsStatus ms_throw_index_error(MsVM* vm, const char* fmt, ...);
MS_API MsStatus ms_throw_key_error(MsVM* vm, MsValue* key);
MS_API MsStatus ms_throw_runtime_error(MsVM* vm, const char* fmt, ...);
MS_API MsStatus ms_throw_io_error(MsVM* vm, const char* fmt, ...);
```

所有 `ms_throw*` 函数始终返回 `MS_ERROR`，可直接 `return`。

### try/catch 模式

```c
MS_API MsStatus ms_try(MsVM* vm, MsValue* func, MsValue* const* args, int nargs,
                       MsValue** result);
```

示例：

```c
MsValue* result = NULL;
if (ms_try(vm, risky_func, args, 2, &result) != MS_OK) {
    MsValue* err = ms_err_fetch(vm);
    fprintf(stderr, "caught: %s\n", ms_err_message(vm, err));
    ms_unroot(vm, err);
}
```

---

## module.h — C 扩展模块

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

### 静态注册

```c
MS_API MsStatus ms_register_module(MsVM* vm, const MsModuleDef* def);
```

### 动态构建模块

```c
MS_API MsValue*  ms_module_new(MsVM* vm, const char* name);
MS_API MsStatus  ms_module_add_func(MsVM* vm, MsValue* mod, const char* name, MsCFunction fn);
MS_API MsStatus  ms_module_add_async_func(MsVM* vm, MsValue* mod, const char* name, MsAsyncFunction fn);
MS_API MsStatus  ms_module_add_const(MsVM* vm, MsValue* mod, const char* name, MsValue* val);
MS_API MsStatus  ms_register_module_value(MsVM* vm, MsValue* mod);
```

### 动态加载（.dll / .so）

C 扩展编译为动态库时，约定导出 `ms_module_init` 入口函数：

```c
MS_MODULE_INIT const MsModuleDef* ms_module_init(MsVM* vm);
```

加载规则：

1. 脚本 `import foo` 时，搜索路径中找不到 `foo.ms`
2. 搜索 `foo.dll`（Windows）或 `foo.so`（Linux/macOS）
3. 动态加载库，调用 `ms_module_init(vm)`
4. 将返回的模块定义注册到 VM

> **安全提示**：动态库加载执行任意原生代码，无签名验证。仅在可信环境中使用，避免从不可信路径加载 `.dll`/`.so` 文件。

构建命令：

```bash
gcc -shared -fPIC -o mymath.so mymath.c -lmslang    # Linux/macOS
cl /LD mymath.c mslang.lib                            # Windows
```

---

## class.h — Class 操作

### 获取和实例化

```c
MS_API MsValue*  ms_get_class(MsVM* vm, const char* name);
MS_API MsValue*  ms_instance_new(MsVM* vm, MsValue* cls, MsValue* const* args, int nargs);
```

### 实例属性

```c
MS_API MsValue*  ms_instance_get(MsVM* vm, MsValue* obj, const char* attr);
MS_API MsStatus  ms_instance_set(MsVM* vm, MsValue* obj, const char* attr, MsValue* val);
MS_API int       ms_isinstance(MsVM* vm, MsValue* obj, MsValue* cls);
```

### C 侧定义 Class

```c
MS_API MsValue*  ms_class_define(MsVM* vm, const char* name, MsValue* parent);
MS_API MsStatus  ms_class_add_method(MsVM* vm, MsValue* cls, const char* name, MsCFunction method);
MS_API MsStatus  ms_class_add_static(MsVM* vm, MsValue* cls, const char* name, MsValue* val);
```

---

## GC 交互

### 写屏障

C 扩展直接修改 mslang 堆对象的引用字段时，必须调用写屏障：

```c
MS_API void ms_write_barrier(MsVM* vm, MsValue* parent, MsValue* new_val);
```

> 注意：`ms_list_push`、`ms_dict_set`、`ms_instance_set` 等内置操作已内部包含写屏障。仅当 C 侧直接操作对象内部结构时需要手动调用。

### Finalizer 注册

```c
MS_API MsStatus ms_on_finalize(MsVM* vm, MsValue* obj, MsFinalizerFn fn, void* userdata);
```

注册 C finalizer 回调。对象被 GC 回收前，在 mutator 线程中调用回调。

### GC 控制

```c
MS_API void ms_gc_collect(MsVM* vm, MsGcType type);
MS_API void ms_gc_enable(MsVM* vm, int enable);
MS_API int  ms_gc_is_enabled(MsVM* vm);

MS_API void ms_gc_set_threshold(MsVM* vm, MsGcType type, double threshold);
MS_API void ms_gc_set_promotion_age(MsVM* vm, uint32_t age);
MS_API void ms_gc_set_gc_threads(MsVM* vm, uint32_t threads);
```

### GC 调试模式

```c
MS_API void ms_gc_set_debug(MsVM* vm, int enable);
```

启用 debug 模式后（仅 `debug_assertions` 构建可用），GC 增加以下运行时检查：

- **root/unroot 配对检查**：检测重复 `ms_unroot`、未 root 先 unroot
- **解引用类型标签校验**：每次通过 `MsValue*` 访问堆对象时验证 `type_tag` 合法
- **GC 后堆一致性验证**：每次 GC 完成后遍历堆，检查所有可达对象类型标签一致

> debug 模式有显著性能开销，仅用于开发调试。

### GC 统计

```c
typedef struct MsGcStats {
    uint64_t minor_gc_count;
    uint64_t major_gc_count;
    uint64_t total_pause_ns;
    uint64_t last_pause_ns;
    uint64_t young_size;
    uint64_t old_size;
    uint64_t los_size;
    uint64_t bytes_freed;
} MsGcStats;

MS_API MsGcStats ms_gc_stats(MsVM* vm);
```

---

## 完整嵌入示例

```c
#include <mslang.h>
#include <stdio.h>

int main(void) {
    MsVM* vm = ms_vm_new();

    const char* script =
        "fn fibonacci(n) {\n"
        "    if n <= 1 { return n }\n"
        "    return fibonacci(n - 1) + fibonacci(n - 2)\n"
        "}\n";

    if (ms_exec_string(vm, script, "fib.ms") != MS_OK) {
        MsValue* err = ms_err_fetch(vm);
        fprintf(stderr, "error: %s\n", ms_err_message(vm, err));
        ms_unroot(vm, err);
        ms_vm_free(vm);
        return 1;
    }

    MsValue* fib = ms_get_global(vm, "fibonacci");
    ms_root(vm, fib);

    MsValue* arg = ms_int(10);
    MsValue* result = ms_call1(vm, fib, arg);

    if (!ms_err_occurred(vm)) {
        printf("fibonacci(10) = %ld\n", ms_to_int(vm, result));
        ms_unroot(vm, result);
    } else {
        MsValue* err = ms_err_fetch(vm);
        fprintf(stderr, "call error: %s\n", ms_err_message(vm, err));
        ms_unroot(vm, err);
    }

    ms_unroot(vm, arg);
    ms_unroot(vm, fib);
    ms_vm_free(vm);
    return 0;
}
```

## 完整扩展模块示例

```c
#include <mslang.h>
#include <stdio.h>
#include <stdlib.h>

static MsValue* file_read(MsVM* vm, MsValue* const* args, int nargs) {
    if (nargs < 1 || !ms_is_string(args[0])) {
        return ms_throw_type_error(vm, "string", "other");
    }
    const char* path = ms_to_string(vm, args[0]);

    FILE* f = fopen(path, "rb");
    if (!f) {
        return ms_throw_io_error(vm, "cannot open: %s", path);
    }

    fseek(f, 0, SEEK_END);
    long size = ftell(f);
    fseek(f, 0, SEEK_SET);

    char* buf = malloc(size + 1);
    fread(buf, 1, size, f);
    buf[size] = '\0';
    fclose(f);

    MsValue* result = ms_stringn(vm, buf, size);
    free(buf);
    return result;
}

static MsValue* file_write(MsVM* vm, MsValue* const* args, int nargs) {
    if (nargs < 2 || !ms_is_string(args[0]) || !ms_is_string(args[1])) {
        return ms_throw_type_error(vm, "string, string", "other");
    }
    const char* path = ms_to_string(vm, args[0]);
    const char* data = ms_to_string(vm, args[1]);

    FILE* f = fopen(path, "wb");
    if (!f) {
        return ms_throw_io_error(vm, "cannot open: %s", path);
    }
    fputs(data, f);
    fclose(f);
    return ms_nil();
}

static const MsFuncDef fileio_funcs[] = {
    {"read",  file_read},
    {"write", file_write},
    {NULL, NULL}
};

MS_MODULE_INIT const MsModuleDef* ms_module_init(MsVM* vm) {
    static const MsModuleDef def = {
        .name = "fileio",
        .methods = fileio_funcs,
        .consts = NULL,
    };
    return &def;
}
```

## 构建与集成

### CMake 集成

```cmake
find_package(mslang REQUIRED)

add_executable(myapp main.c)
target_link_libraries(myapp mslang)

add_library(myfileio SHARED fileio.c)
target_link_libraries(myfileio mslang)
```

### 动态库约定

| 平台 | 文件名 | 导出符号 |
|---|---|---|
| Linux | `lib{name}.so` | `ms_module_init` |
| macOS | `lib{name}.dylib` | `ms_module_init` |
| Windows | `{name}.dll` | `__declspec(dllexport) ms_module_init` |
