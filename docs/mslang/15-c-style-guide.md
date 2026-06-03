# mslang C 语言编码规范

## 1 概述

### 1.1 目的

本规范定义了 mslang C API 及 C 扩展模块的编码标准。所有参与 mslang C 代码编写的开发者都应遵循本规范，以保持代码库的一致性、可读性和可维护性。

### 1.2 适用范围

本规范适用于：

- mslang C API 头文件（`include/mslang/*.h`）
- mslang C API 实现（`.c` 文件）
- 第三方 C 扩展模块

本规范**不适用于** mslang 自身的 Rust 实现代码。

### 1.3 设计哲学

本规范的命名风格参考 Google Java Style Guide（lowerCamelCase、UpperCamelCase），整体结构与代码组织参考 Google C++ Style Guide。在 C 语言的传统实践与 Java 风格之间，本规范寻求平衡：

- **优化读者体验** — 代码被阅读的次数远多于编写次数
- **一致性优先** — 统一的风格减少认知负担
- **安全默认** — 规则应帮助开发者避免常见错误

### 1.4 术语

| 术语 | 含义 |
|---|---|
| PascalCase | 每个单词首字母大写，直接拼接。如 `MsValue`、`MsGcStats` |
| lowerCamelCase | 首单词全小写，后续单词首字母大写。如 `vmNew`、`execFile` |
| UPPER_SNAKE_CASE | 全大写，单词间用下划线连接。如 `MS_OK`、`MS_TYPE_NIL` |
| snake_case | 全小写，单词间用下划线连接。仅用于文件名 |

---

## 2 C 标准版本

### 2.1 目标标准

代码应遵循 **C17**（ISO/IEC 9899:2018）标准。

### 2.2 允许的 C17 特性

- `_Static_assert`（静态断言）
- 匿名结构体/联合体成员
- `_Alignas` / `_Alignof`
- `bool`、`true`、`false`（`<stdbool.h>`）
- `nullptr`（如果编译器支持 C23 扩展，否则使用 `NULL`）
- 指定初始化器（designated initializers）
- 变长数组（VLA）— **禁止使用**（见下文）

### 2.3 禁止的语言特性

| 特性 | 原因 |
|---|---|
| 变长数组（VLA） | 栈溢出风险，性能不可预测 |
| `goto` | 破坏结构化控制流 |
| `setjmp` / `longjmp` | 跳过清理代码，资源泄漏风险 |
| `strncpy` | 不保证 null 终止，容易误用 |
| `gets` | 已在 C11 中移除，缓冲区溢出 |
| `sprintf` / `vsprintf` | 缓冲区溢出风险，使用 `snprintf` 代替 |
| 隐式函数声明 | C99 起已禁止，必须提供原型 |
| 隐式 int | 禁止省略类型说明符 |
| 三字母词（trigraphs） | 已在 C23 移除，可读性差 |
| 位域（除 `bool` 外） | 布尔位域允许，其他类型的位域存储布局由实现定义 |
| `register` 关键字 | 已废弃，编译器自行优化寄存器分配 |

---

## 3 文件组织

### 3.1 文件编码与格式

**所有 C 语言源文件（`.h`、`.c`）必须满足以下要求：**

- **编码**：UTF-8（无 BOM）
- **换行符**：LF（`\n`），禁止 CRLF
- **行尾**：无尾随空白符
- **文件末尾**：保留一个空行

推荐的 `.editorconfig` 配置：

```ini
# .editorconfig
root = true

[*.{c,h}]
charset = utf-8
end_of_line = lf
insert_final_newline = true
trim_trailing_whitespace = true
indent_style = space
indent_size = 2
```

### 3.2 文件命名

| 类型 | 命名风格 | 示例 |
|---|---|---|
| 头文件 | `snake_case.h` | `vm.h`、`value.h`、`gc_stats.h` |
| 源文件 | `snake_case.c` | `vm.c`、`value.c`、`gc_stats.c` |
| 头文件与源文件 | 必须成对 | `vm.h` ↔ `vm.c` |

规则：

- 文件名全部使用小写字母、数字和下划线
- 文件名应简短且能反映内容
- 头文件使用 `.h` 扩展名
- 不得使用 `.inc` 文件

### 3.3 头文件结构

头文件应按以下顺序组织：

```c
#pragma once

// 1. C 标准库头文件
#include <stdbool.h>
#include <stdint.h>
#include <stddef.h>

// 2. 项目内头文件
#include "types.h"

// 3. 宏定义和常量

// 4. 前向声明（不透明类型）
typedef struct MsVM MsVM;

// 5. 枚举定义

// 6. 结构体定义

// 7. 函数声明
```

### 3.4 源文件结构

源文件应按以下顺序组织：

```c
// 1. 对应的头文件（必须第一个）
#include "vm.h"

// 2. C 标准库头文件
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

// 3. 项目内头文件
#include "value.h"
#include "error.h"

// 4. 宏定义

// 5. 内部类型定义

// 6. 静态函数声明

// 7. 函数实现
```

---

## 4 命名约定

### 4.1 通用规则

- 标识符仅使用 ASCII 字母、数字和下划线
- 不使用特殊前缀或后缀标识作用域（如 `m_`、`s_`、`_` 前缀）
- 下划线 `_` 开头的标识符保留给系统使用，**禁止使用**
- 双下划线 `__` 在 C 标准中保留给实现，**禁止使用**
- 名称应有意义，避免无意义的缩写（如 `a`、`b`、`tmp`），但局部循环变量除外（如 `i`、`j`）

### 4.2 命名风格总览

| 类别 | 风格 | 前缀 | 示例 |
|---|---|---|---|
| 类型（struct/enum/typedef） | PascalCase | `Ms` | `MsVM`、`MsValue`、`MsGcStats` |
| 公开函数 | lowerCamelCase | `ms` | `msVmNew`、`msExecFile`、`msListPush` |
| 宏 / 常量 | UPPER_SNAKE_CASE | `MS_` | `MS_OK`、`MS_API`、`MS_VERSION` |
| 枚举常量 | UPPER_SNAKE_CASE | `MS_` | `MS_TYPE_NIL`、`MS_GC_MINOR` |
| 局部变量 | lowerCamelCase | 无 | `argCount`、`result`、`bufSize` |
| 函数参数 | lowerCamelCase | 无 | `vm`、`index`、`defaultVal` |
| 结构体字段 | lowerCamelCase | 无 | `length`、`capacity`、`data` |
| 静态函数 | lowerCamelCase | 无 | `hashString`、`growBuffer` |
| 文件名 | snake_case | 无 | `vm.h`、`gc_stats.c` |

### 4.3 类型命名

所有公开类型使用 PascalCase 加 `Ms` 前缀：

```c
// 不透明类型
typedef struct MsVM MsVM;
typedef struct MsValue MsValue;

// 结构体
typedef struct MsGcStats {
  uint64_t minorGcCount;
  uint64_t majorGcCount;
  uint64_t totalPauseNs;
} MsGcStats;

// 枚举
typedef enum MsType {
  MS_TYPE_NIL = 0,
  MS_TYPE_BOOL,
  MS_TYPE_INT,
} MsType;

// 函数指针类型
typedef MsValue* (*MsCFunction)(MsVM* vm, MsValue* const* args, int nargs);
typedef void (*MsFinalizerFn)(MsVM* vm, MsValue* obj, void* userdata);
```

规则：

- `typedef struct X X` 形式：tag 名与 typedef 名一致
- 不透明类型只声明 `typedef struct X X`，定义在 `.c` 文件中
- 函数指针类型以 `Fn` 结尾（如 `MsFinalizerFn`、`MsWriteFn`）
- 结构体名使用完整单词，不使用缩写（`Statistics` 而非 `Stats`，除非是广泛认可的缩写如 `GcStats`）

### 4.4 函数命名

公开 API 函数使用 lowerCamelCase 加 `ms` 前缀。前缀 `ms` 全小写，紧跟 PascalCase 部分：

```c
// VM 生命周期
MsVM* msVmNew(void);
void  msVmFree(MsVM* vm);

// 脚本执行
MsStatus msExecFile(MsVM* vm, const char* path);
MsStatus msExecString(MsVM* vm, const char* source, const char* filename);

// 值操作
MsValue* msListNew(MsVM* vm);
MsValue* msListGet(MsVM* vm, MsValue* list, int index);
MsStatus msListSet(MsVM* vm, MsValue* list, int index, MsValue* val);
```

命名模式：

| 模式 | 含义 | 示例 |
|---|---|---|
| `msXxxNew` | 构造/创建 | `msVmNew`、`msListNew`、`msDictNew` |
| `msXxxFree` | 销毁/释放 | `msVmFree` |
| `msGetXxx` | 获取值 | `msGetGlobal`、`msGetAttr` |
| `msSetXxx` | 设置值 | `msSetGlobal`、`msSetAttr` |
| `msIsXxx` | 布尔判断 | `msIsNil`、`msIsString`、`msIsInstance` |
| `msToXxx` | 类型转换 | `msToInt`、`msToString` |
| `msConvertXxx` | 显式类型转换 | `msConvertInt`、`msConvertStr` |
| `msThrowXxx` | 抛出异常 | `msThrow`、`msThrowTypeError` |
| `msXxxLen` | 获取长度 | `msListLen`、`msDictLen` |

静态（内部）函数不加 `ms` 前缀，直接使用 lowerCamelCase：

```c
static uint32_t hashString(const char* str, size_t len) {
  // ...
}

static MsValue* growBuffer(MsVM* vm, MsValue* list) {
  // ...
}
```

### 4.5 变量命名

局部变量和函数参数使用 lowerCamelCase：

```c
MsValue* msListGet(MsVM* vm, MsValue* list, int index) {
  int listLen = msListLen(vm, list);
  if (index < 0) {
    index += listLen;
  }
  MsValue* result = internalGet(list, index);
  return result;
}
```

循环变量可以使用简短名称：

```c
for (int i = 0; i < count; i++) {
  // i 是可接受的循环变量名
}
```

### 4.6 宏与常量命名

宏和常量使用 UPPER_SNAKE_CASE 加 `MS_` 前缀：

```c
#define MS_TRUE  1
#define MS_FALSE 0
#define MS_NIL   ((MsValue*)0)

#define MS_VERSION_MAJOR  0
#define MS_VERSION_MINOR  1
#define MS_VERSION_PATCH  0

#define MS_VERSION_AT_LEAST(major, minor, patch) \
    (MS_VERSION_MAJOR > (major) ||                \
     (MS_VERSION_MAJOR == (major) &&              \
      MS_VERSION_MINOR > (minor)) ||              \
     (MS_VERSION_MAJOR == (major) &&              \
      MS_VERSION_MINOR == (minor) &&              \
      MS_VERSION_PATCH >= (patch)))
```

规则：

- 宏名必须全部大写
- 多行宏续行使用 `\`，续行缩进至少 4 空格
- 宏参数必须用括号包裹：`(major)`、`(minor)`
- 函数式宏优先考虑 `static inline` 函数替代
- `MS_API`、`MS_MODULE_INIT` 等平台宏属于此类别

### 4.7 枚举常量命名

枚举常量使用 UPPER_SNAKE_CASE 加 `MS_` 前缀：

```c
typedef enum MsType {
  MS_TYPE_NIL = 0,
  MS_TYPE_BOOL,
  MS_TYPE_INT,
  MS_TYPE_FLOAT,
  MS_TYPE_STRING,
} MsType;

typedef enum MsStatus {
  MS_OK    =  0,
  MS_ERROR = -1,
  MS_YIELD =  1,
} MsStatus;
```

规则：

- 首个枚举值应显式赋值（通常为 `0`）
- 同一组的枚举值共享前缀（如 `MS_TYPE_`、`MS_GC_`）
- 枚举值之间对齐 `=` 号是可选的

### 4.8 CamelCase 转换规则

参考 Google Java Style Guide 的 CamelCase 规则：

1. 将短语转换为纯 ASCII，移除撇号
2. 按空格和标点拆分为单词
3. 全部小写后，按规则大写首字母
4. 拼接为单个标识符

| 原始短语 | lowerCamelCase | PascalCase |
|---|---|---|
| XML HTTP request | `xmlHttpRequest` | `XmlHttpRequest` |
| new customer ID | `newCustomerId` | `NewCustomerId` |
| supports IPv6 on iOS | `supportsIpv6OnIos` | `SupportsIpv6OnIos` |
| GC stats | `gcStats` | `GcStats` |
| VM lifecycle | `vmLifecycle` | `VmLifecycle` |

---

## 5 格式化

### 5.1 缩进

- 每级缩进 **2 个空格**
- 禁止使用 Tab 字符
- 预处理指令不缩进（始终从行首开始）
- case 标签相对于 switch 缩进 2 空格

```c
switch (msGetType(val)) {
  case MS_TYPE_INT:
    return msToInt(vm, val);
  case MS_TYPE_FLOAT:
    return msToFloat(vm, val);
  default:
    return msThrowTypeError(vm, "number", "other");
}
```

### 5.2 花括号

使用 **K&R 风格**：

- 开括号 `{` 不换行，前接一个空格
- 闭括号 `}` 独占一行
- `else`、`else if`、`while`（do-while）紧跟在 `}` 后

```c
if (msIsNil(val)) {
  return msThrowValueError(vm, "value must not be nil");
} else if (msIsInt(val)) {
  return handleInt(vm, val);
} else {
  return handleOther(vm, val);
}
```

函数定义的花括号同样不换行：

```c
MsStatus msExecFile(MsVM* vm, const char* path) {
  if (vm == NULL || path == NULL) {
    return MS_ERROR;
  }
  // ...
  return MS_OK;
}
```

**空块**可以简写为 `{}`：

```c
typedef struct MsFuncDef {
  const char* name;
  MsCFunction func;
} MsFuncDef;
```

但在 `if/else`、`do/while` 等多块语句中，空块也必须换行：

```c
if (flag) {
  // do nothing
} else {
  doSomething();
}
```

### 5.3 行宽

每行不超过 **120 个字符**。

例外：

- `#include` 指令行不受限制
- `#pragma` 指令行不受限制
- 不可分割的长字符串字面量（如 URL）

### 5.4 行拆分

当行超过 120 字符时，按以下规则拆分：

1. **优先在更高级别的语法边界处拆分**
2. **函数调用**：在参数之间拆分，续行缩进 4 空格

```c
MsValue* result = msCall(vm, func,
    (MsValue* const[]){arg1, arg2, arg3},
    3);
```

3. **赋值语句**：在 `=` 之后拆分

```c
MsGcStats stats = msGcCollectWithThreshold(vm,
    MS_GC_MAJOR, newThreshold);
```

4. **长条件表达式**：在逻辑运算符之前拆分

```c
if (msIsString(val)
    && msStringLen(vm, val) > 0
    && !msIsKeyword(msToString(vm, val))) {
  // ...
}
```

5. **宏定义**：续行缩进至少 4 空格，右对齐 `\`

```c
#define MS_VERSION_AT_LEAST(major, minor, patch) \
    (MS_VERSION_MAJOR > (major) ||               \
     (MS_VERSION_MAJOR == (major) &&             \
      MS_VERSION_MINOR > (minor)))
```

### 5.5 空格

#### 5.5.1 必须有空格的位置

```c
// 关键字后接括号
if (condition) { ... }
while (condition) { ... }
for (int i = 0; i < n; i++) { ... }
switch (value) { ... }

// 二元运算符两侧
x = a + b;
y = (a * b) + c;
if (a == b && c != d) { ... }

// 逗号之后
void foo(int a, int b, int c) { ... }
func(arg1, arg2, arg3);

// 开括号前（非函数调用）
if (cond) { ... }
struct MsGcStats { ... };

// 赋值运算符两侧
int count = 0;
count += 1;

// 类型与指针星号之间
MsVM* vm;
MsValue* val;
const char* str;

// 行尾注释 // 之前和之后
int result = compute();  // 说明注释
```

#### 5.5.2 禁止空格的位置

```c
// 函数名与开括号之间
msVmNew()
msListGet(vm, list, 0)

// 一元运算符与操作数之间
*x
&a
!flag
-i

// 指针星号与变量名之间（声明中）
MsVM* vm;     // 正确：* 贴类型
MsVM *vm;     // 错误
MsVM*vm;      // 错误

// 逗号之前
func(a , b);  // 错误
func(a, b);   // 正确

// 分号之前
for (int i = 0 ; i < n ; i++)  // 错误
for (int i = 0; i < n; i++)    // 正确

// 圆括号内侧
func( a, b );  // 错误
func(a, b);    // 正确
```

### 5.6 空行

```c
// 文件顶部和底部各保留一个空行
// 函数之间：一个空行
MsStatus msExecFile(MsVM* vm, const char* path) {
  // ...
}

MsStatus msExecString(MsVM* vm, const char* source, const char* filename) {
  // ...
}

// 函数内部：逻辑段落之间使用一个空行分隔
MsValue* result = msCall(vm, func, args, nargs);

if (msErrOccurred(vm)) {
  return NULL;
}

return result;

// 禁止连续两个以上空行
```

### 5.7 每行一条语句

```c
// 正确
int a = 1;
int b = 2;

// 错误
int a = 1; int b = 2;
```

### 5.8 每行一个声明

```c
// 正确
int a;
int b;

// 错误
int a, b;

// for 循环中允许
for (int i = 0, j = n - 1; i < j; i++, j--) { ... }
```

---

## 6 注释

### 6.1 统一使用 `//` 风格

所有注释统一使用 `//` 单行注释风格。**禁止使用 `/* */` 块注释**。

```c
// 这是一行注释
int result = compute();
```

多行注释使用连续的 `//`：

```c
// 这个函数执行以下操作：
// 1. 验证输入参数
// 2. 调用 VM 执行脚本
// 3. 返回执行结果
MsStatus msExecFile(MsVM* vm, const char* path) {
  // ...
}
```

### 6.2 文档注释

公开 API 的文档注释以 `//` 开头，紧跟在声明之前。使用以下格式：

```c
// 创建新的 VM 实例。
//
// 返回新创建的 MsVM 指针。如果内存分配失败，返回 NULL。
// 每个 MsVM 拥有独立的全局作用域、模块缓存和 GC 堆。
// 不同 VM 实例可在不同线程中并行使用。
//
// 参数：
//   无
//
// 返回值：
//   MsVM* - 新的 VM 实例，需要调用 msVmFree 释放
MsVM* msVmNew(void);
```

```c
// 执行 .ms 脚本文件。
//
// 参数：
//   vm      - VM 实例，不得为 NULL
//   path    - 脚本文件路径，不得为 NULL
//
// 返回值：
//   MS_OK    - 执行成功
//   MS_ERROR - 执行失败，可通过 msErrFetch 获取异常详情
MsStatus msExecFile(MsVM* vm, const char* path);
```

规则：

- 第一行是简短摘要（一句话描述函数用途）
- 空一行 `//` 后跟详细描述
- 参数和返回值使用 `参数：` / `返回值：` 标签
- 每行注释对齐，`//` 后接一个空格

### 6.3 行尾注释

用于简短说明：

```c
int length = msListLen(vm, list);  // 缓存长度避免重复调用
```

规则：

- `//` 之前至少两个空格
- `//` 之后一个空格
- 行尾注释应简短，不超过 60 个字符
- 不要为显而易见的代码添加注释

### 6.4 TODO 注释

```c
// TODO(issues/123): 替换为更高效的哈希算法
// TODO(issues/456): 支持嵌套异常
// TODO(user@example.com): 移除兼容代码，v2.0 后不再需要
```

规则：

- `TODO` 全大写
- 紧跟圆括号包裹的上下文引用（issue 链接、邮箱等）
- 冒号后接具体描述
- 描述应包含可操作的后续计划

### 6.5 注释禁止事项

```c
// 禁止：不解释原因的注释
i++;  // i加1

// 禁止：注释掉的代码（使用版本控制管理历史代码）
// oldFunctionCall(arg1, arg2);

// 禁止：分隔线/装饰性注释
// ================================
// 模块初始化
// ================================

// 禁止：块注释
/* 这种注释风格不允许 */

// 禁止：嵌套注释
// 外层 // 内层（可读性差）
```

---

## 7 头文件规则

### 7.1 #pragma once

所有头文件使用 `#pragma once` 作为 include guard。不使用传统的 `#ifndef` / `#define` / `#endif` 方式。

```c
#pragma once

#include <stdbool.h>
#include <stdint.h>

#include "types.h"

// ... 声明 ...
```

### 7.2 Self-contained 头文件

头文件必须是自包含的（self-contained）：任意 `.c` 文件仅 `#include` 该头文件即可编译通过。

规则：

- 头文件必须包含它所依赖的所有其他头文件
- 不要依赖传递性 include
- 编译测试：创建一个仅 `#include "foo.h"` 的 `.c` 文件，必须能编译通过

### 7.3 Include 顺序

头文件包含按以下分组排列，每组之间用一个空行分隔：

1. **对应头文件**（仅 `.c` 文件中，必须是第一个 `#include`）
2. **C 标准库头文件**（按字母序排列）
3. **项目内头文件**（按字母序排列）

```c
// value.c
#include "value.h"

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "error.h"
#include "gc.h"
#include "vm.h"
```

头文件中的 include 顺序（`.h` 文件）：

```c
// value.h
#pragma once

#include <stdbool.h>
#include <stdint.h>

#include "types.h"
```

### 7.4 Include What You Use

每个文件必须直接 `#include` 它所使用符号的头文件。不依赖传递性 include。

```c
// 错误：依赖 value.h 间接包含 stdint.h
// value.c
#include "value.h"

int64_t count = 0;  // int64_t 来自 stdint.h

// 正确：显式包含
#include "value.h"

#include <stdint.h>

int64_t count = 0;
```

### 7.5 前向声明

最小化头文件依赖。如果一个类型在头文件中只通过指针使用（不需要完整定义），使用前向声明代替 `#include`：

```c
// vm.h
#pragma once

typedef struct MsValue MsValue;  // 前向声明，不 include value.h

typedef struct MsVM MsVM;

// 只使用 MsValue*，不需要 MsValue 的完整定义
MsValue* msGetGlobal(MsVM* vm, const char* name);
```

---

## 8 类型与结构体

### 8.1 不透明类型

不透明类型（opaque type）仅在头文件中前向声明，完整定义放在 `.c` 文件中：

```c
// vm.h
typedef struct MsVM MsVM;

// vm.c
#include "vm.h"

typedef struct MsVM {
  MsValue** globals;
  int globalCapacity;
  int globalCount;
} MsVM;
```

### 8.2 typedef

规则：

- 结构体和枚举必须使用 `typedef`
- `typedef` 声明中结构体 tag 名与 typedef 名一致
- 函数指针使用 `typedef` 定义类型，不直接在参数中使用原始函数指针语法

```c
// 正确
typedef struct MsFuncDef {
  const char* name;
  MsCFunction func;
} MsFuncDef;

typedef MsValue* (*MsCFunction)(MsVM* vm, MsValue* const* args, int nargs);

// 错误：未使用 typedef
struct MsFuncDef {
  const char* name;
  MsValue* (*func)(MsVM* vm, MsValue* const* args, int nargs);
};
```

### 8.3 结构体字段顺序

逻辑相关的字段应放在一起。按照以下顺序组织：

```c
typedef struct MsList {
  MsValue** items;      // 数据指针
  int length;           // 当前长度
  int capacity;         // 容量
} MsList;
```

### 8.4 枚举

- 首个值显式赋值
- 使用 `typedef`
- 不使用匿名枚举

```c
// 正确
typedef enum MsStatus {
  MS_OK    =  0,
  MS_ERROR = -1,
  MS_YIELD =  1,
} MsStatus;

// 错误：匿名枚举
enum {
  MS_OK = 0,
  MS_ERROR = -1,
};
```

### 8.5 固定宽度整数类型

使用 `<stdint.h>` 中的固定宽度类型：

| 使用 | 不使用 |
|---|---|
| `int32_t` | `int`、`long` |
| `int64_t` | `long long` |
| `uint32_t` | `unsigned int` |
| `size_t` | `unsigned long`（表示大小时） |
| `bool` | `int`（表示布尔值时） |

例外：函数参数和局部变量中，当值的范围明确且无跨平台问题时可使用 `int`。

---

## 9 函数

### 9.1 函数声明

```c
// 公开 API 函数
MsVM* msVmNew(void);
void  msVmFree(MsVM* vm);

// 返回值类型和函数名之间使用一个空格
// 函数名与参数列表之间无空格
// 参数列表中每个参数独立声明类型
MsValue* msListGet(MsVM* vm, MsValue* list, int index);
```

### 9.2 参数规则

- 参数使用 lowerCamelCase
- 当函数不接受参数时使用 `void`
- 输出参数放在输入参数之后
- 数组参数需要配套长度参数

```c
// 输出参数在后
MsStatus msTupleUnpack(MsVM* vm, MsValue* tuple,
    MsValue*** items, int* count);

// 数组 + 长度
MsValue* msListFrom(MsVM* vm, MsValue* const* items, int count);
```

### 9.3 返回值约定

| 返回类型 | 成功 | 失败 | 示例 |
|---|---|---|---|
| `MsStatus` | `MS_OK` | `MS_ERROR` | `msExecFile` |
| `MsValue*` | 有效指针 | `NULL` | `msListGet` |
| `int`（布尔） | `MS_TRUE` | `MS_FALSE` | `msIsString` |
| `int64_t` | 有效值 | `0` + 设置异常 | `msToInt` |
| `const char*` | 有效指针 | `NULL` | `msToString` |
| `void` | 不适用 | 设置异常 | `msVmFree` |

### 9.4 静态函数

文件作用域的内部函数使用 `static` 修饰，不加 `ms` 前缀：

```c
static uint32_t hashString(const char* str, size_t len) {
  uint32_t hash = 2166136261u;
  for (size_t i = 0; i < len; i++) {
    hash ^= (uint8_t)str[i];
    hash *= 16777619u;
  }
  return hash;
}
```

如果内部函数需要在同文件内提前使用，在文件顶部声明：

```c
// 文件顶部
static uint32_t hashString(const char* str, size_t len);

// ... 其他代码 ...

static uint32_t hashString(const char* str, size_t len) {
  // 实现
}
```

### 9.5 回调函数

回调函数类型使用 `typedef` 定义，以 `Fn` 或 `Function` 结尾：

```c
typedef MsValue* (*MsCFunction)(MsVM* vm, MsValue* const* args, int nargs);
typedef void (*MsFinalizerFn)(MsVM* vm, MsValue* obj, void* userdata);
typedef int (*MsWriteFn)(const char* data, size_t len, void* userdata);
```

### 9.6 内联函数

使用 `static inline` 代替函数式宏：

```c
// 正确
static inline bool msErrOccurred(MsVM* vm) {
  return vm->hasError;
}

// 错误：使用宏
#define ms_err_occurred(vm) ((vm)->hasError)
```

### 9.7 函数长度

单个函数建议不超过 **80 行**。超过时应考虑拆分为更小的辅助函数。

例外：简单的 `switch` 分发函数可能超出此限制。

### 9.8 const 正确性

尽可能使用 `const`：

```c
// 不修改指针指向的内容
const char* msToString(MsVM* vm, const MsValue* val);

// 不修改数组元素
MsValue* msListFrom(MsVM* vm, MsValue* const* items, int count);

// 不修改结构体
int msListLen(MsVM* vm, const MsValue* list);
```

---

## 10 内存管理约定

### 10.1 所有权模型

mslang C API 使用以下所有权模型：

| 所有权类型 | 说明 | 释放责任 |
|---|---|---|
| 调用方拥有（owned） | 函数返回的新分配资源 | 调用方负责释放 |
| 借用引用（borrowed） | 函数返回内部指针，不转移所有权 | 不可释放，仅在源对象存活期间有效 |
| GC 管理 | `MsValue*` 的生命周期由 GC 管理 | 通过 `msRoot`/`msUnroot` 管理 |

### 10.2 GC Root 注册

`MsValue*` 的生命周期由垃圾回收器管理。C 侧通过 root 注册机制保护对象不被回收：

```c
MsValue* obj = msGetGlobal(vm, "config");
msRoot(vm, obj);

// ... 跨越多次 API 调用，GC 期间 obj 不会被回收 ...

msUnroot(vm, obj);
```

**必须 root 的场景：**

- API 返回值需要在多次 API 调用之间保持存活
- 存储到 C 侧全局变量或长生命周期的数据结构中

**不需要 root 的场景：**

- API 返回值在当前调用帧内立即使用
- 值仅作为参数传递给其他 API 调用

**规则：**

- `msRoot` 和 `msUnroot` 必须**严格配对**
- 在错误处理路径（`goto cleanup`、`if (error)`）中不要遗漏 `msUnroot`
- 推荐 RAII 模式：在函数入口 root，统一在尾部 unroot

### 10.3 malloc / free 规则

- 分配者负责释放
- 每个 `malloc` 必须有对应的 `free`
- 释放后立即将指针置为 `NULL`（防止 use-after-free）
- 使用 `goto cleanup` 模式处理多步分配的错误路径

```c
MsStatus loadData(MsVM* vm, const char* path, MsValue** out) {
  FILE* f = fopen(path, "rb");
  if (!f) {
    return msThrowIoError(vm, "cannot open: %s", path);
  }

  fseek(f, 0, SEEK_END);
  long fileSize = ftell(f);
  fseek(f, 0, SEEK_SET);

  char* buf = malloc(fileSize + 1);
  if (!buf) {
    fclose(f);
    return msThrowRuntimeError(vm, "out of memory");
  }

  size_t bytesRead = fread(buf, 1, fileSize, f);
  fclose(f);
  buf[bytesRead] = '\0';

  MsValue* result = msStringn(vm, buf, bytesRead);
  free(buf);

  *out = result;
  return MS_OK;
}
```

### 10.4 禁止事项

- 禁止使用 `realloc` 直接修改原指针（先赋值给临时变量）
- 禁止 `free(NULL)` 之外的解引用已释放指针
- 禁止在 `MsValue*` 上直接调用 `free`（由 GC 管理）
- 禁止内存泄漏：所有分配路径必须有对应释放路径

```c
// 正确：安全使用 realloc
void* newBuf = realloc(oldBuf, newSize);
if (!newBuf) {
  free(oldBuf);
  return MS_ERROR;
}
oldBuf = newBuf;

// 错误：realloc 失败时原指针丢失
oldBuf = realloc(oldBuf, newSize);  // 泄漏风险
```

---

## 11 错误处理约定

### 11.1 返回值错误码

使用 `MsStatus` 作为函数返回值表示操作结果：

```c
MsStatus result = msExecFile(vm, "script.ms");
if (result != MS_OK) {
  MsValue* err = msErrFetch(vm);
  const char* msg = msErrMessage(vm, err);
  fprintf(stderr, "error: %s\n", msg);
  msUnroot(vm, err);
}
```

### 11.2 错误传播

C 扩展函数中发生错误时的处理模式：

```c
static MsValue* myFunction(MsVM* vm, MsValue* const* args, int nargs) {
  // 1. 验证参数
  if (nargs < 1 || !msIsString(args[0])) {
    return msThrowTypeError(vm, "string", "other");
  }

  // 2. 执行操作
  const char* path = msToString(vm, args[0]);
  FILE* f = fopen(path, "rb");
  if (!f) {
    return msThrowIoError(vm, "cannot open: %s", path);
  }

  // 3. 成功返回
  // ... 处理 ...
  return result;
}
```

所有 `msThrowXxx` 系列函数始终返回 `MS_ERROR`，可直接 `return`。

### 11.3 NULL 检查

公开 API 函数必须检查必要参数是否为 NULL：

```c
MsStatus msExecFile(MsVM* vm, const char* path) {
  if (vm == NULL || path == NULL) {
    return MS_ERROR;
  }
  // ...
}
```

### 11.4 try/catch 模式

使用 `msTry` 捕获异常：

```c
MsValue* result = NULL;
if (msTry(vm, riskyFunc, args, 2, &result) != MS_OK) {
  MsValue* err = msErrFetch(vm);
  fprintf(stderr, "caught: %s\n", msErrMessage(vm, err));
  msUnroot(vm, err);
  // 处理错误
}
```

### 11.5 错误处理禁止事项

- 禁止忽略返回值（特别是 `MsStatus`）
- 禁止捕获异常后不处理（至少记录日志）
- 禁止在错误路径上继续执行可能失败的操作

```c
// 错误：忽略返回值
msExecString(vm, script, NULL);  // 未检查返回值

// 正确
if (msExecString(vm, script, NULL) != MS_OK) {
  // 处理错误
}
```

---

## 12 并发与线程安全

### 12.1 线程模型

mslang 使用 **per-VM** 线程模型：

- 每个 `MsVM` 实例绑定一个互斥锁
- 不同 VM 实例可在不同线程中并行使用
- 同一 VM 实例在多线程中使用时，API 内部自动加锁

### 12.2 扩展模块中的线程安全

C 扩展函数默认在 VM 锁保护下执行。如果扩展需要额外的线程操作：

```c
// 如需保证多步操作的原子性
msVmLock(vm);
MsValue* val = msGetGlobal(vm, "counter");
int64_t count = msToInt(vm, val);
msSetGlobal(vm, "counter", msInt(count + 1));
msVmUnlock(vm);
```

### 12.3 线程安全规则

| 规则 | 说明 |
|---|---|
| 不要在回调中调用阻塞操作 | 阻塞会持有 VM 锁，导致死锁 |
| 不要跨线程传递 `MsValue*` | `MsValue*` 与特定 VM 绑定 |
| 不要缓存 `MsValue*` 到全局变量 | GC 可能随时回收，除非已 root |
| 使用 `msCallAsync` 进行异步操作 | 避免在 VM 线程中阻塞 |

### 12.4 原子操作

需要原子操作时，使用 C11 `<stdatomic.h>`：

```c
#include <stdatomic.h>

static atomic_int globalCounter = 0;

int nextId(void) {
  return atomic_fetch_add(&globalCounter, 1);
}
```

### 12.5 线程局部存储

使用 `_Thread_local`（C11）：

```c
static _Thread_local MsVM* currentVm = NULL;
```

---

## 13 平台兼容性

### 13.1 平台抽象宏

```c
// 平台检测
#ifdef _WIN32
  #define MS_PLATFORM_WINDOWS 1
#elif defined(__linux__)
  #define MS_PLATFORM_LINUX 1
#elif defined(__APPLE__)
  #define MS_PLATFORM_MACOS 1
#endif

// 导出符号
#ifdef _WIN32
  #ifdef MSLANG_BUILDING
    #define MS_API __declspec(dllexport)
  #else
    #define MS_API __declspec(dllimport)
  #endif
#else
  #define MS_API __attribute__((visibility("default")))
#endif
```

### 13.2 跨平台规则

- 使用标准 C 库函数，避免平台特定 API
- 路径分隔符使用 `/`（Windows 也接受 `/`）
- 条件编译块尽量简短且集中

```c
// 正确：集中处理平台差异
#ifdef _WIN32
  #include <windows.h>
  #define MS_PATH_SEPARATOR "/"
  #define MS_PATH_MAX MAX_PATH
#else
  #include <limits.h>
  #include <unistd.h>
  #define MS_PATH_SEPARATOR "/"
  #define MS_PATH_MAX PATH_MAX
#endif
```

### 13.3 编译器扩展

允许使用以下广泛支持的编译器扩展（通过 `__has_attribute` 或 `__has_builtin` 检测）：

| 扩展 | 用途 |
|---|---|
| `__attribute__((unused))` | 抑制未使用参数警告 |
| `__attribute__((format(printf, ...)))` | 格式化字符串编译期检查 |
| `__attribute__((noreturn))` | 标记不返回的函数 |
| `__attribute__((aligned(n)))` | 对齐控制 |
| `__builtin_expect` | 分支预测提示 |

```c
// 安全使用编译器扩展
#if defined(__GNUC__) || defined(__clang__)
  #define MS_PRINTF_FORMAT(fmt, args) \
      __attribute__((format(printf, fmt, args)))
  #define MS_UNREACHABLE() __builtin_unreachable()
#else
  #define MS_PRINTF_FORMAT(fmt, args)
  #define MS_UNREACHABLE() abort()
#endif
```

---

## 14 构建系统约定

### 14.1 CMake 规范

使用 CMake 作为构建系统。推荐的 `CMakeLists.txt` 结构：

```cmake
cmake_minimum_required(VERSION 3.16)
project(myextension LANGUAGES C)

set(CMAKE_C_STANDARD 17)
set(CMAKE_C_STANDARD_REQUIRED ON)
set(CMAKE_C_EXTENSIONS OFF)

find_package(mslang REQUIRED)

add_library(myextension SHARED myextension.c)
target_link_libraries(myextension PRIVATE mslang)
```

### 14.2 编译选项

推荐的编译器警告级别：

```cmake
# GCC / Clang
target_compile_options(myextension PRIVATE
  -Wall
  -Wextra
  -Wpedantic
  -Werror
  -Wconversion
  -Wshadow
  -Wstrict-prototypes
  -Wold-style-definition
  -Wmissing-prototypes
)

# MSVC
target_compile_options(myextension PRIVATE
  /W4
  /WX
  /permissive-
)
```

### 14.3 必须零警告

代码必须在最高警告级别下零警告编译。`-Werror`（GCC/Clang）或 `/WX`（MSVC）将警告视为错误。

### 14.4 编译定义

```cmake
# 公开定义（使用者也需要）
target_compile_definitions(myextension PUBLIC
  MSLANG_BUILDING
)

# 私有定义（仅编译自身时使用）
target_compile_definitions(myextension PRIVATE
  _CRT_SECURE_NO_WARNINGS  # MSVC 安全警告
)
```

---

## 15 安全编码

### 15.1 缓冲区溢出防护

```c
// 使用 snprintf 代替 sprintf
char buf[256];
snprintf(buf, sizeof(buf), "value: %d", value);

// 使用 strncat 代替 strcat（或更推荐：完全避免固定缓冲区）
char buf[256] = "prefix: ";
strncat(buf, suffix, sizeof(buf) - strlen(buf) - 1);

// 使用安全的字符串长度跟踪
size_t remaining = sizeof(buf) - strlen(buf) - 1;
```

### 15.2 整数溢出检查

```c
// 乘法溢出检查
bool safeMultiply(size_t a, size_t b, size_t* result) {
  if (a != 0 && b > SIZE_MAX / a) {
    return false;
  }
  *result = a * b;
  return true;
}

// 加法溢出检查
bool safeAdd(size_t a, size_t b, size_t* result) {
  if (a > SIZE_MAX - b) {
    return false;
  }
  *result = a + b;
  return true;
}
```

### 15.3 输入验证

所有来自外部输入的数据（用户输入、文件内容、网络数据）必须验证：

```c
static MsValue* safeFunction(MsVM* vm, MsValue* const* args, int nargs) {
  if (nargs < 1) {
    return msThrowValueError(vm, "expected at least 1 argument");
  }
  if (!msIsString(args[0])) {
    return msThrowTypeError(vm, "string", "other");
  }

  size_t len = msStringLen(vm, args[0]);
  if (len > MAX_ALLOWED_LENGTH) {
    return msThrowValueError(vm, "input too long: %zu bytes", len);
  }

  // 安全处理
  // ...
}
```

### 15.4 格式化字符串安全

禁止将用户输入直接作为格式化字符串：

```c
// 错误：用户输入作为格式化字符串
const char* userInput = msToString(vm, args[0]);
printf(userInput);  // 格式化字符串漏洞

// 正确
printf("%s", userInput);
```

### 15.5 安全函数对照表

| 禁止使用 | 安全替代 |
|---|---|
| `sprintf` | `snprintf` |
| `vsprintf` | `vsnprintf` |
| `strcat` | `strncat` 或手动跟踪长度 |
| `strcpy` | `strncpy` + 手动 null 终止，或 `snprintf` |
| `gets` | `fgets` |
| `scanf("%s", buf)` | `scanf("%255s", buf)`（指定宽度） |
| `strlen` + `malloc` | `calloc` 或检查溢出后的 `malloc` |

---

## 16 完整示例

### 16.1 头文件示例

```c
// mathext.h
#pragma once

#include <mslang.h>

// 计算阶乘。
//
// 参数：
//   n - 非负整数
//
// 返回值：
//   n 的阶乘，如果 n 为负数或结果溢出则抛出异常
MsValue* mathFactorial(MsVM* vm, MsValue* const* args, int nargs);

// 判断是否为质数。
//
// 参数：
//   n - 待判断的整数
//
// 返回值：
//   MS_TRUE 或 MS_FALSE
MsValue* mathIsPrime(MsVM* vm, MsValue* const* args, int nargs);
```

### 16.2 源文件示例

```c
// mathext.c
#include "mathext.h"

#include <stdint.h>

// 模块入口，导出函数定义表
static const MsFuncDef mathFunctions[] = {
  {"factorial", mathFactorial},
  {"isPrime",   mathIsPrime},
  {NULL, NULL}
};

MsValue* mathFactorial(MsVM* vm, MsValue* const* args, int nargs) {
  if (nargs < 1 || !msIsInt(args[0])) {
    return msThrowTypeError(vm, "int", "other");
  }

  int64_t n = msToInt(vm, args[0]);
  if (n < 0) {
    return msThrowValueError(vm, "factorial requires non-negative integer");
  }

  int64_t result = 1;
  for (int64_t i = 2; i <= n; i++) {
    if (result > INT64_MAX / i) {
      return msThrowRuntimeError(vm, "integer overflow in factorial");
    }
    result *= i;
  }

  return msInt(result);
}

MsValue* mathIsPrime(MsVM* vm, MsValue* const* args, int nargs) {
  if (nargs < 1 || !msIsInt(args[0])) {
    return msThrowTypeError(vm, "int", "other");
  }

  int64_t n = msToInt(vm, args[0]);
  if (n < 2) {
    return msBoolVal(MS_FALSE);
  }
  if (n < 4) {
    return msBoolVal(MS_TRUE);
  }
  if (n % 2 == 0 || n % 3 == 0) {
    return msBoolVal(MS_FALSE);
  }

  for (int64_t i = 5; i * i <= n; i += 6) {
    if (n % i == 0 || n % (i + 2) == 0) {
      return msBoolVal(MS_FALSE);
    }
  }

  return msBoolVal(MS_TRUE);
}

// 模块入口点
MS_MODULE_INIT const MsModuleDef* msModuleInit(MsVM* vm) {
  static const MsModuleDef moduleDef = {
    .name = "mathext",
    .methods = mathFunctions,
    .consts = NULL,
  };
  return &moduleDef;
}
```

### 16.3 嵌入示例

```c
// main.c
#include <mslang.h>
#include <stdio.h>
#include <stdlib.h>

int main(void) {
  MsVM* vm = msVmNew();
  if (!vm) {
    fprintf(stderr, "failed to create VM\n");
    return 1;
  }

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
  MsValue* args[] = {arg};
  MsValue* result = msCall(vm, fib, args, 1);

  if (!msErrOccurred(vm)) {
    printf("fibonacci(10) = %ld\n", (long)msToInt(vm, result));
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
