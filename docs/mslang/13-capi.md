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
| 错误可查 | 函数返回错误指示，通过 msErrFetch 获取详情 |
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
#define MS_VERSION_MAJOR  0
#define MS_VERSION_MINOR  1
#define MS_VERSION_PATCH  0
#define MS_VERSION        "0.1.0"

#define MS_VERSION_AT_LEAST(major, minor, patch) \
    (MS_VERSION_MAJOR > (major) || \
     (MS_VERSION_MAJOR == (major) && MS_VERSION_MINOR > (minor)) || \
     (MS_VERSION_MAJOR == (major) && MS_VERSION_MINOR == (minor) && \
      MS_VERSION_PATCH >= (patch)))
```

## 平台兼容性宏

```c
#ifdef _WIN32
  #ifdef MS_BUILDING
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
MS_API MsVM* msVmNew(void);
MS_API void  msVmFree(MsVM* vm);
```

每个 `MsVM` 拥有独立的全局作用域、模块缓存、事件循环和 GC 堆。不同 VM 实例可在不同线程中并行使用。

### 配置

```c
MS_API void msAddModulePath(MsVM* vm, const char* path);
MS_API void msSetArgs(MsVM* vm, int argc, const char** argv);

typedef int (*MsWriteFn)(const char* data, size_t len, void* userdata);
MS_API void msSetStdout(MsVM* vm, MsWriteFn fn, void* userdata);
MS_API void msSetStderr(MsVM* vm, MsWriteFn fn, void* userdata);
```

### 脚本执行

```c
MS_API MsStatus msExecFile(MsVM* vm, const char* path);
MS_API MsStatus msExecString(MsVM* vm, const char* source, const char* filename);
MS_API MsValue* msEval(MsVM* vm, const char* expr);
```

- `msExecFile`：执行 `.ms` 文件
- `msExecString`：执行源码字符串，`filename` 用于错误信息（可为 NULL）
- `msEval`：求值表达式字符串，返回结果（新引用）

### 全局变量

```c
MS_API MsValue* msGetGlobal(MsVM* vm, const char* name);
MS_API MsStatus msSetGlobal(MsVM* vm, const char* name, MsValue* val);
MS_API void     msDelGlobal(MsVM* vm, const char* name);
```

### 线程安全

```c
MS_API void msVmLock(MsVM* vm);
MS_API void msVmUnlock(MsVM* vm);
```

通常不需要手动加锁，所有 `ms*` API 内部自动管理。仅在需要保证多步操作的原子性时使用。

---

## value.h — 值操作

### 引用管理（GC Root 注册）

mslang 使用**并发三色标记清扫 GC**（详见 [14-gc](14-gc.md)），不使用引用计数。C 侧通过 Root 注册机制告知 GC 某个对象正在被使用：

```c
MS_API MsValue* msRoot(MsVM* vm, MsValue* val);
MS_API void     msUnroot(MsVM* vm, MsValue* val);
```

- `msRoot`：将对象注册为 GC 根。注册后 GC 不会回收此对象。返回 `val` 本身。
- `msUnroot`：注销 GC 根。注销后对象可能被 GC 回收，C 侧不应再访问该指针。

**使用模式**：

```c
MsValue* obj = msGetGlobal(vm, "config");
msRoot(vm, obj);

// ... 跨越多次 API 调用，GC 期间 obj 不会被回收 ...

msUnroot(vm, obj);
```

**不需要 root 的场景**：

- API 返回值在当前调用帧内立即使用（如 `msToString` 提取 C 字符串）
- 值仅作为参数传递给其他 API 调用（调用期间 GC 不会回收参数）

### 特殊值

```c
MS_API MsValue* msNil(void);
MS_API MsValue* msBoolVal(int val);

#define MS_NIL       (msNil())
#define MS_TRUE_VAL  (msBoolVal(1))
#define MS_FALSE_VAL (msBoolVal(0))
```

单例值，不需要 root（但 root/unroot 安全）。

### 值创建

```c
MS_API MsValue* msInt(int64_t val);
MS_API MsValue* msFloat(double val);

MS_API MsValue* msString(MsVM* vm, const char* str);
MS_API MsValue* msStringn(MsVM* vm, const char* str, size_t len);
MS_API MsValue* msStringFmt(MsVM* vm, const char* fmt, ...);
```

### 集合创建

```c
MS_API MsValue* msListNew(MsVM* vm);
MS_API MsValue* msDictNew(MsVM* vm);
MS_API MsValue* msSetNew(MsVM* vm);

MS_API MsValue* msListFrom(MsVM* vm, MsValue* const* items, int count);
MS_API MsValue* msTupleFrom(MsVM* vm, MsValue* const* items, int count);
MS_API MsValue* msDictFrom(MsVM* vm, MsValue* const* pairs, int count);
```

### 类型判断

```c
MS_API MsType msTypeof(MsValue* val);

MS_API int msIsNil(MsValue* val);
MS_API int msIsBool(MsValue* val);
MS_API int msIsInt(MsValue* val);
MS_API int msIsFloat(MsValue* val);
MS_API int msIsNumber(MsValue* val);
MS_API int msIsString(MsValue* val);
MS_API int msIsList(MsValue* val);
MS_API int msIsDict(MsValue* val);
MS_API int msIsTuple(MsValue* val);
MS_API int msIsSet(MsValue* val);
MS_API int msIsFunction(MsValue* val);
MS_API int msIsClass(MsValue* val);
MS_API int msIsInstance(MsValue* val);
MS_API int msIsGenerator(MsValue* val);
MS_API int msIsFuture(MsValue* val);
MS_API int msIsChannel(MsValue* val);
```

所有类型判断函数返回 `MS_TRUE` / `MS_FALSE`。

### 值转换

```c
MS_API int64_t     msToInt(MsVM* vm, MsValue* val);
MS_API double      msToFloat(MsVM* vm, MsValue* val);
MS_API int         msToBool(MsValue* val);
MS_API const char* msToString(MsVM* vm, MsValue* val);
MS_API char*       msToStringCopy(MsVM* vm, MsValue* val);
```

- `msToInt` / `msToFloat`：类型不匹配时设置异常并返回 0
- `msToBool`：按 truthy 规则转换，不设异常
- `msToString`：返回内部指针（借用引用），不需要 free，仅在 val 存活期间有效
- `msToStringCopy`：返回副本，调用方必须 `free()`

### 显式类型转换

```c
MS_API MsValue* msConvertInt(MsVM* vm, MsValue* val);
MS_API MsValue* msConvertFloat(MsVM* vm, MsValue* val);
MS_API MsValue* msConvertStr(MsVM* vm, MsValue* val);
MS_API MsValue* msConvertBool(MsVM* vm, MsValue* val);
MS_API MsValue* msConvertList(MsVM* vm, MsValue* val);
```

对应 mslang 的 `int()`、`str()` 等内置函数。

### 比较

```c
MS_API int     msEq(MsVM* vm, MsValue* a, MsValue* b);
MS_API int     msLt(MsVM* vm, MsValue* a, MsValue* b);
MS_API int     msLe(MsVM* vm, MsValue* a, MsValue* b);
MS_API int     msGt(MsVM* vm, MsValue* a, MsValue* b);
MS_API int     msGe(MsVM* vm, MsValue* a, MsValue* b);
MS_API int     msIs(MsValue* a, MsValue* b);
MS_API int64_t msHash(MsVM* vm, MsValue* val);
```

### 字符串操作

```c
MS_API size_t     msStringLen(MsVM* vm, MsValue* str);
MS_API const char* msStringData(MsVM* vm, MsValue* str);
MS_API MsValue*   msStringConcat(MsVM* vm, MsValue* a, MsValue* b);
MS_API MsValue*   msStringSlice(MsVM* vm, MsValue* str, int start, int end);
```

### List 操作

```c
MS_API int      msListLen(MsVM* vm, MsValue* list);
MS_API MsValue* msListGet(MsVM* vm, MsValue* list, int index);
MS_API MsStatus msListSet(MsVM* vm, MsValue* list, int index, MsValue* val);
MS_API MsStatus msListPush(MsVM* vm, MsValue* list, MsValue* val);
MS_API MsValue* msListPop(MsVM* vm, MsValue* list);
MS_API MsStatus msListInsert(MsVM* vm, MsValue* list, int index, MsValue* val);
MS_API int      msListContains(MsVM* vm, MsValue* list, MsValue* val);
MS_API MsValue* msListSlice(MsVM* vm, MsValue* list, int start, int end, int step);
```

### Dict 操作

```c
MS_API int      msDictLen(MsVM* vm, MsValue* dict);
MS_API MsValue* msDictGet(MsVM* vm, MsValue* dict, MsValue* key);
MS_API MsValue* msDictGetDefault(MsVM* vm, MsValue* dict, MsValue* key, MsValue* defaultVal);
MS_API MsStatus msDictSet(MsVM* vm, MsValue* dict, MsValue* key, MsValue* val);
MS_API MsStatus msDictRemove(MsVM* vm, MsValue* dict, MsValue* key);
MS_API int      msDictContains(MsVM* vm, MsValue* dict, MsValue* key);
MS_API MsValue* msDictKeys(MsVM* vm, MsValue* dict);
MS_API MsValue* msDictValues(MsVM* vm, MsValue* dict);
MS_API MsValue* msDictItems(MsVM* vm, MsValue* dict);
```

### Tuple 操作

```c
MS_API int      msTupleLen(MsVM* vm, MsValue* tup);
MS_API MsValue* msTupleGet(MsVM* vm, MsValue* tup, int index);
MS_API MsStatus msTupleUnpack(MsVM* vm, MsValue* tup, MsValue*** items, int* count);
```

`msTupleUnpack`：调用方负责 `free(items)`，但不需要释放各元素（借用引用）。

### Set 操作

```c
MS_API int      msSetLen(MsVM* vm, MsValue* set);
MS_API MsStatus msSetAdd(MsVM* vm, MsValue* set, MsValue* val);
MS_API MsStatus msSetRemove(MsVM* vm, MsValue* set, MsValue* val);
MS_API int      msSetContains(MsVM* vm, MsValue* set, MsValue* val);
```

### 迭代器

```c
MS_API MsValue* msIter(MsVM* vm, MsValue* iterable);
MS_API MsStatus msNext(MsVM* vm, MsValue* iterator, MsValue** out);
```

`msNext`：返回 `MS_OK` 成功（`*out` 设为值），`MS_ERROR` 表示迭代结束（StopIteration）或异常。

### 通用属性/下标访问

```c
MS_API MsValue* msGetAttr(MsVM* vm, MsValue* obj, const char* attr);
MS_API MsStatus msSetAttr(MsVM* vm, MsValue* obj, const char* attr, MsValue* val);
MS_API MsValue* msGetItem(MsVM* vm, MsValue* obj, MsValue* key);
MS_API MsStatus msSetItem(MsVM* vm, MsValue* obj, MsValue* key, MsValue* val);
MS_API int64_t  msLen(MsVM* vm, MsValue* val);
MS_API MsValue* msRepr(MsVM* vm, MsValue* val);
```

---

## call.h — 函数调用

### 同步调用

```c
MS_API MsValue* msCall(MsVM* vm, MsValue* func, MsValue* const* args, int nargs);
```

`func` 必须是可调用对象（函数、闭包、带 `__call__` 的实例）。返回函数返回值（新引用），异常时返回 NULL。

```c
#define msCall0(vm, f)                        msCall(vm, f, NULL, 0)
#define msCall1(vm, f, a)                     ...
#define msCall2(vm, f, a, b)                  ...
#define msCall3(vm, f, a, b, c)               ...
```

### 异步调用

```c
MS_API MsValue* msCallAsync(MsVM* vm, MsValue* func, MsValue* const* args, int nargs);
MS_API MsValue* msAwait(MsVM* vm, MsValue* future);

typedef enum MsFutureState {
  MS_FUTURE_PENDING,
  MS_FUTURE_RESOLVED,
  MS_FUTURE_REJECTED,
} MsFutureState;

MS_API MsFutureState msFutureState(MsVM* vm, MsValue* future);
MS_API void msFutureResolve(MsVM* vm, MsValue* future, MsValue* result);
MS_API void msFutureReject(MsVM* vm, MsValue* future, MsValue* error);
```

- `msCallAsync`：异步调用，立即返回 Future
- `msAwait`：阻塞等待 Future 完成
- `msFutureResolve/reject`：手动完成 Future（C async 函数中使用）

### C 侧 async 函数

```c
typedef void (*MsAsyncFunction)(MsVM* vm, MsValue* const* args, int nargs,
    MsValue* future);
```

C async 函数接收参数和一个 Future。必须在异步操作完成后调用 `msFutureResolve` 或 `msFutureReject`。

### Channel 操作

```c
MS_API MsValue*  msChannel(MsVM* vm, int bufferSize);
MS_API MsStatus  msChannelSend(MsVM* vm, MsValue* ch, MsValue* val);
MS_API MsValue*  msChannelRecv(MsVM* vm, MsValue* ch);
MS_API MsStatus  msChannelClose(MsVM* vm, MsValue* ch);
MS_API int       msChannelIsClosed(MsVM* vm, MsValue* ch);
```

### 生成器操作

```c
MS_API MsValue*  msGeneratorIter(MsVM* vm, MsValue* generator);
MS_API MsStatus  msGeneratorNext(MsVM* vm, MsValue* generator, MsValue** out);
```

---

## error.h — 异常处理

### 异常查询

```c
MS_API int      msErrOccurred(MsVM* vm);
MS_API MsValue* msErrFetch(MsVM* vm);
MS_API void     msErrClear(MsVM* vm);
```

- `msErrOccurred`：是否有异常待处理
- `msErrFetch`：取出异常对象（清除当前异常），返回新引用
- `msErrClear`：清除异常（不获取对象）

### 异常对象属性

```c
MS_API const char* msErrTypeName(MsVM* vm, MsValue* err);
MS_API const char* msErrMessage(MsVM* vm, MsValue* err);
MS_API const char* msErrTraceback(MsVM* vm, MsValue* err);
MS_API MsValue*    msErrCause(MsVM* vm, MsValue* err);
```

返回借用引用，仅在 err 存活期间有效。

### 从 C 抛出异常

```c
MS_API MsStatus msThrow(MsVM* vm, const char* type, const char* fmt, ...);
MS_API MsStatus msThrowValue(MsVM* vm, MsValue* err);
MS_API MsStatus msThrowRethrow(MsVM* vm);

MS_API MsStatus msThrowTypeError(MsVM* vm, const char* expected, const char* actual);
MS_API MsStatus msThrowValueError(MsVM* vm, const char* fmt, ...);
MS_API MsStatus msThrowIndexError(MsVM* vm, const char* fmt, ...);
MS_API MsStatus msThrowKeyError(MsVM* vm, MsValue* key);
MS_API MsStatus msThrowRuntimeError(MsVM* vm, const char* fmt, ...);
MS_API MsStatus msThrowIoError(MsVM* vm, const char* fmt, ...);
```

所有 `msThrow*` 函数始终返回 `MS_ERROR`，可直接 `return`。

### try/catch 模式

```c
MS_API MsStatus msTry(MsVM* vm, MsValue* func, MsValue* const* args, int nargs,
    MsValue** result);
```

示例：

```c
MsValue* result = NULL;
if (msTry(vm, riskyFunc, args, 2, &result) != MS_OK) {
  MsValue* err = msErrFetch(vm);
  fprintf(stderr, "caught: %s\n", msErrMessage(vm, err));
  msUnroot(vm, err);
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
MS_API MsStatus msRegisterModule(MsVM* vm, const MsModuleDef* def);
```

### 动态构建模块

```c
MS_API MsValue*  msModuleNew(MsVM* vm, const char* name);
MS_API MsStatus  msModuleAddFunc(MsVM* vm, MsValue* mod, const char* name, MsCFunction fn);
MS_API MsStatus  msModuleAddAsyncFunc(MsVM* vm, MsValue* mod, const char* name, MsAsyncFunction fn);
MS_API MsStatus  msModuleAddConst(MsVM* vm, MsValue* mod, const char* name, MsValue* val);
MS_API MsStatus  msRegisterModuleValue(MsVM* vm, MsValue* mod);
```

### 动态加载（.dll / .so）

C 扩展编译为动态库时，约定导出 `msModuleInit` 入口函数：

```c
MS_MODULE_INIT const MsModuleDef* msModuleInit(MsVM* vm);
```

加载规则：

1. 脚本 `import foo` 时，搜索路径中找不到 `foo.ms`
2. 搜索 `foo.dll`（Windows）或 `foo.so`（Linux/macOS）
3. 动态加载库，调用 `msModuleInit(vm)`
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
MS_API MsValue*  msGetClass(MsVM* vm, const char* name);
MS_API MsValue*  msInstanceNew(MsVM* vm, MsValue* cls, MsValue* const* args, int nargs);
```

### 实例属性

```c
MS_API MsValue*  msInstanceGet(MsVM* vm, MsValue* obj, const char* attr);
MS_API MsStatus  msInstanceSet(MsVM* vm, MsValue* obj, const char* attr, MsValue* val);
MS_API int       msIsInstance(MsVM* vm, MsValue* obj, MsValue* cls);
```

### C 侧定义 Class

```c
MS_API MsValue*  msClassDefine(MsVM* vm, const char* name, MsValue* parent);
MS_API MsStatus  msClassAddMethod(MsVM* vm, MsValue* cls, const char* name, MsCFunction method);
MS_API MsStatus  msClassAddStatic(MsVM* vm, MsValue* cls, const char* name, MsValue* val);
```

---

## GC 交互

### 写屏障

C 扩展直接修改 mslang 堆对象的引用字段时，必须调用写屏障：

```c
MS_API void msWriteBarrier(MsVM* vm, MsValue* parent, MsValue* new_val);
```

> 注意：`msListPush`、`msDictSet`、`msInstanceSet` 等内置操作已内部包含写屏障。仅当 C 侧直接操作对象内部结构时需要手动调用。

### Finalizer 注册

```c
MS_API MsStatus msOnFinalize(MsVM* vm, MsValue* obj, MsFinalizerFn fn, void* userdata);
```

注册 C finalizer 回调。对象被 GC 回收前，在 mutator 线程中调用回调。

### GC 控制

```c
MS_API void msGcCollect(MsVM* vm, MsGcType type);
MS_API void msGcEnable(MsVM* vm, int enable);
MS_API int  msGcIsEnabled(MsVM* vm);

MS_API void msGcSetThreshold(MsVM* vm, MsGcType type, double threshold);
MS_API void msGcSetPromotionAge(MsVM* vm, uint32_t age);
MS_API void msGcSetGcThreads(MsVM* vm, uint32_t threads);
```

### GC 调试模式

```c
MS_API void msGcSetDebug(MsVM* vm, int enable);
```

启用 debug 模式后（仅 `debug_assertions` 构建可用），GC 增加以下运行时检查：

- **root/unroot 配对检查**：检测重复 `msUnroot`、未 root 先 unroot
- **解引用类型标签校验**：每次通过 `MsValue*` 访问堆对象时验证 `type_tag` 合法
- **GC 后堆一致性验证**：每次 GC 完成后遍历堆，检查所有可达对象类型标签一致

> debug 模式有显著性能开销，仅用于开发调试。

### GC 统计

```c
typedef struct MsGcStats {
  uint64_t minorGcCount;
  uint64_t majorGcCount;
  uint64_t totalPauseNs;
  uint64_t lastPauseNs;
  uint64_t youngSize;
  uint64_t oldSize;
  uint64_t losSize;
  uint64_t bytesFreed;
} MsGcStats;

MS_API MsGcStats msGcStats(MsVM* vm);
```

---

## 完整嵌入示例

```c
#include <mslang.h>
#include <stdio.h>

int main(void) {
  MsVM* vm = msVmNew();

  const char* script =
    "fn fibonacci(n) {\n"
    "  if n <= 1 { return n }\n"
    "  return fibonacci(n - 1) + fibonacci(n - 2)\n"
    "}\n";

  if (msExecString(vm, script, "fib.ms") != MS_OK) {
    MsValue* err = msErrFetch(vm);
    fprintf(stderr, "error: %s\n", msErrMessage(vm, err));
    msUnroot(vm, err);
    msVmFree(vm);
    return 1;
  }

  MsValue* fib = msGetGlobal(vm, "fibonacci");
  msRoot(vm, fib);

  MsValue* arg = msInt(10);
  MsValue* result = msCall1(vm, fib, arg);

  if (!msErrOccurred(vm)) {
    printf("fibonacci(10) = %ld\n", msToInt(vm, result));
    msUnroot(vm, result);
  } else {
    MsValue* err = msErrFetch(vm);
    fprintf(stderr, "call error: %s\n", msErrMessage(vm, err));
    msUnroot(vm, err);
  }

  msUnroot(vm, arg);
  msUnroot(vm, fib);
  msVmFree(vm);
  return 0;
}
```

## 完整扩展模块示例

```c
#include <mslang.h>
#include <stdio.h>
#include <stdlib.h>

static MsValue* fileRead(MsVM* vm, MsValue* const* args, int nargs) {
  if (nargs < 1 || !msIsString(args[0])) {
    return msThrowTypeError(vm, "string", "other");
  }
  const char* path = msToString(vm, args[0]);

  FILE* f = fopen(path, "rb");
  if (!f) {
    return msThrowIoError(vm, "cannot open: %s", path);
  }

  fseek(f, 0, SEEK_END);
  long size = ftell(f);
  fseek(f, 0, SEEK_SET);

  char* buf = malloc(size + 1);
  fread(buf, 1, size, f);
  buf[size] = '\0';
  fclose(f);

  MsValue* result = msStringn(vm, buf, size);
  free(buf);
  return result;
}

static MsValue* fileWrite(MsVM* vm, MsValue* const* args, int nargs) {
  if (nargs < 2 || !msIsString(args[0]) || !msIsString(args[1])) {
    return msThrowTypeError(vm, "string, string", "other");
  }
  const char* path = msToString(vm, args[0]);
  const char* data = msToString(vm, args[1]);

  FILE* f = fopen(path, "wb");
  if (!f) {
    return msThrowIoError(vm, "cannot open: %s", path);
  }
  fputs(data, f);
  fclose(f);
  return msNil();
}

static const MsFuncDef fileioFuncs[] = {
  {"read",  fileRead},
  {"write", fileWrite},
  {NULL, NULL}
};

MS_MODULE_INIT const MsModuleDef* msModuleInit(MsVM* vm) {
  static const MsModuleDef def = {
    .name = "fileio",
    .methods = fileioFuncs,
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
| Linux | `lib{name}.so` | `msModuleInit` |
| macOS | `lib{name}.dylib` | `msModuleInit` |
| Windows | `{name}.dll` | `__declspec(dllexport) msModuleInit` |
