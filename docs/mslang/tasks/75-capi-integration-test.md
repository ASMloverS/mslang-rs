# C API — 集成测试（嵌入 + 扩展端到端）

## 所属阶段

Phase 6 — 模块系统 + 标准库

## 前置任务

- 65-capi-infrastructure（cbindgen + 手写类型头文件 + 构建集成）
- 66-capi-vm（VM 生命周期：msVmNew、msVmFree、msExecString、msGetGlobal 等）
- 67-capi-value-creation（值创建与类型判断：msInt、msString、msRoot、msTypeof 等）
- 68-capi-value-convert（值转换与比较：msToInt、msToString、msEq、msLt 等）
- 69-capi-collections（集合操作：List/Dict/Tuple/Set 增删改查 + 迭代器）
- 70-capi-call（函数调用：msCall、msCall0-msCall3、MsCFunction 桥接）
- 71-capi-error（异常处理：msThrow*、msErrOccurred、msErrFetch、msTry）
- 72-capi-module（模块注册：msRegisterModule、msModuleNew、动态加载）
- 73-capi-class（Class 操作：msGetClass、msInstanceNew、msClassDefine）
- 74-capi-gc（GC 交互：msWriteBarrier、msOnFinalize、msGcCollect、msGcStats）

## 目标

搭建 C API 集成测试基础设施（`tests/capi/` + CMakeLists.txt + build.rs 集成），编写端到端集成测试覆盖两大场景：

1. **嵌入端到端**：C 程序创建 VM → 执行脚本 → 调用脚本函数 → 操作值（创建/转换/比较/集合操作）→ 处理异常 → GC 交互 → 销毁 VM
2. **扩展端到端**：C 编写扩展模块（`.dll`/`.so`）→ mslang 脚本 `import` → 调用 C 函数 → 验证结果

集成到 CI 流水线，确保全部 C API（Task 65-74）在 Linux/macOS/Windows 三平台协同工作。

## 设计规格

参照 [13-capi.md](../13-capi.md) § 完整嵌入示例 与 § 完整扩展模块示例。

### 目录结构

```
tests/capi/
├── CMakeLists.txt               # CMake 构建 C 测试可执行文件
├── common.h                     # 共享测试框架（宏 + 辅助函数）
├── test_embed_basic.c           # VM 生命周期 + 基础执行
├── test_embed_values.c          # 值创建、转换、比较
├── test_embed_collections.c     # List/Dict/Tuple/Set 操作
├── test_embed_call.c            # 函数调用
├── test_embed_error.c           # 异常处理
├── test_embed_class.c           # Class 操作
├── test_embed_gc.c              # GC 交互
├── test_extension.c             # C 扩展模块源码（编译为 .dll/.so）
├── test_import_extension.ms     # 测试脚本：import 扩展模块
└── test_full_lifecycle.c        # 完整生命周期测试（13-capi.md 示例增强版）
```

### 构建集成方案

**方案 A — cc crate（简单测试）**：在 `tests/` 下添加 `build.rs`，使用 `cc` crate 编译 C 测试文件并链接到 mslang cdylib。`cargo test` 自动触发构建。

**方案 B — CMakeLists.txt（复杂测试）**：跨平台 CMake 脚本，处理 include 路径、链接库路径、平台差异（Windows `.dll` vs Linux `.so`）。CI 中独立步骤运行。

**推荐**：方案 A 用于大部分测试；方案 B 用于扩展模块动态加载测试。

## 实现细节

### 1. common.h — 测试框架

```c
#ifndef TEST_COMMON_H
#define TEST_COMMON_H

#include <mslang.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int tests_passed = 0;
static int tests_failed = 0;

#define TEST_ASSERT(cond, msg) do {                             \
    if (!(cond)) {                                              \
        fprintf(stderr, "  FAIL: %s at %s:%d\n",               \
                msg, __FILE__, __LINE__);                       \
        tests_failed++;                                         \
    } else {                                                    \
        tests_passed++;                                         \
    }                                                           \
} while (0)

#define TEST_ASSERT_EQ(expected, actual, msg) do {              \
    long _e = (long)(expected);                                  \
    long _a = (long)(actual);                                    \
    if (_e != _a) {                                              \
        fprintf(stderr, "  FAIL: %s (expected %ld, got %ld) "   \
                "at %s:%d\n", msg, _e, _a, __FILE__, __LINE__); \
        tests_failed++;                                          \
    } else {                                                     \
        tests_passed++;                                          \
    }                                                            \
} while (0)

#define TEST_ASSERT_NOT_NULL(ptr, msg) do {                     \
    if ((ptr) == NULL) {                                         \
        fprintf(stderr, "  FAIL: %s (got NULL) at %s:%d\n",     \
                msg, __FILE__, __LINE__);                        \
        tests_failed++;                                          \
    } else {                                                     \
        tests_passed++;                                          \
    }                                                            \
} while (0)

#define TEST_ASSERT_NULL(ptr, msg) do {                          \
    if ((ptr) != NULL) {                                         \
        fprintf(stderr, "  FAIL: %s (expected NULL) at %s:%d\n", \
                msg, __FILE__, __LINE__);                        \
        tests_failed++;                                          \
    } else {                                                     \
        tests_passed++;                                          \
    }                                                            \
} while (0)

#define TEST_ASSERT_STR_EQ(expected, actual, msg) do {           \
    const char* _e = (expected);                                  \
    const char* _a = (actual);                                    \
    if (_e == NULL || _a == NULL || strcmp(_e, _a) != 0) {        \
        fprintf(stderr, "  FAIL: %s (expected \"%s\", got \"%s\") "\
                "at %s:%d\n", msg,                                \
                _e ? _e : "(null)",                               \
                _a ? _a : "(null)",                               \
                __FILE__, __LINE__);                              \
        tests_failed++;                                           \
    } else {                                                      \
        tests_passed++;                                           \
    }                                                             \
} while (0)

#define TEST_BEGIN(name) \
    fprintf(stdout, "  %-40s ", name);

#define TEST_END() \
    fprintf(stdout, "ok\n");

#define TEST_SUMMARY() do {                                       \
    fprintf(stdout, "\n--- results: %d passed, %d failed ---\n",  \
            tests_passed, tests_failed);                          \
} while (0)

#define TEST_RETURN() (tests_failed > 0 ? 1 : 0)

static char captured_buf[8192];
static size_t captured_len = 0;

static void captured_reset(void) {
    memset(captured_buf, 0, sizeof(captured_buf));
    captured_len = 0;
}

static int write_capture(const char* data, size_t len, void* userdata) {
    (void)userdata;
    if (captured_len + len >= sizeof(captured_buf)) {
        len = sizeof(captured_buf) - captured_len - 1;
    }
    if (len > 0) {
        memcpy(captured_buf + captured_len, data, len);
        captured_len += len;
    }
    return 0;
}

static int captured_contains(const char* needle) {
    return strstr(captured_buf, needle) != NULL;
}

#endif /* TEST_COMMON_H */
```

### 2. test_embed_basic.c — VM 生命周期 + 基础执行

```c
#include "common.h"

void test_vm_new_free(void) {
    TEST_BEGIN("vm new/free");

    MsVM* vm = msVmNew();
    TEST_ASSERT_NOT_NULL(vm, "msVmNew returns non-NULL");
    msVmFree(vm);

    msVmFree(NULL);

    TEST_END();
}

void test_exec_string_simple(void) {
    TEST_BEGIN("exec string simple");

    MsVM* vm = msVmNew();
    MsStatus s = msExecString(vm, "x = 42", "test.ms");
    TEST_ASSERT_EQ(MS_OK, s, "exec 'x = 42' succeeds");

    MsValue* val = msGetGlobal(vm, "x");
    TEST_ASSERT_NOT_NULL(val, "get global 'x'");
    TEST_ASSERT_EQ(MS_TYPE_INT, msTypeof(val), "x is int");
    TEST_ASSERT_EQ(42, msToInt(vm, val), "x == 42");

    msVmFree(vm);
    TEST_END();
}

void test_exec_string_syntax_error(void) {
    TEST_BEGIN("exec string syntax error");

    MsVM* vm = msVmNew();
    MsStatus s = msExecString(vm, "fn (", "bad.ms");
    TEST_ASSERT_EQ(MS_ERROR, s, "syntax error returns MS_ERROR");

    msVmFree(vm);
    TEST_END();
}

void test_global_roundtrip(void) {
    TEST_BEGIN("global set/get/del");

    MsVM* vm = msVmNew();
    msExecString(vm, "answer = 42", "test.ms");

    MsValue* val = msGetGlobal(vm, "answer");
    TEST_ASSERT_NOT_NULL(val, "get 'answer'");
    TEST_ASSERT_EQ(42, msToInt(vm, val), "answer == 42");

    msDelGlobal(vm, "answer");
    MsValue* gone = msGetGlobal(vm, "answer");
    TEST_ASSERT_NULL(gone, "deleted global returns NULL");

    msVmFree(vm);
    TEST_END();
}

void test_output_redirect(void) {
    TEST_BEGIN("output redirect");

    MsVM* vm = msVmNew();
    msSetStdout(vm, write_capture, NULL);
    captured_reset();

    msExecString(vm, "print(\"hello mslang\")", "test.ms");
    TEST_ASSERT(captured_contains("hello mslang"), "stdout captured");

    msVmFree(vm);
    TEST_END();
}

void test_two_vms_independent(void) {
    TEST_BEGIN("two VMs independent");

    MsVM* vm1 = msVmNew();
    MsVM* vm2 = msVmNew();

    msExecString(vm1, "x = 1", "test.ms");
    msExecString(vm2, "y = 2", "test.ms");

    MsValue* x1 = msGetGlobal(vm1, "x");
    MsValue* x2 = msGetGlobal(vm2, "x");
    MsValue* y1 = msGetGlobal(vm1, "y");
    MsValue* y2 = msGetGlobal(vm2, "y");

    TEST_ASSERT_NOT_NULL(x1, "vm1 has x");
    TEST_ASSERT_NULL(x2, "vm2 has no x");
    TEST_ASSERT_NULL(y1, "vm1 has no y");
    TEST_ASSERT_NOT_NULL(y2, "vm2 has y");

    msVmFree(vm1);
    msVmFree(vm2);
    TEST_END();
}

void test_module_path(void) {
    TEST_BEGIN("add module path");

    MsVM* vm = msVmNew();
    msAddModulePath(vm, "/test/path");
    msAddModulePath(vm, "/another/path");
    msVmFree(vm);

    TEST_END();
}

void test_eval_expression(void) {
    TEST_BEGIN("eval expression");

    MsVM* vm = msVmNew();
    MsValue* result = msEval(vm, "2 + 3 * 4");
    TEST_ASSERT_NOT_NULL(result, "eval returns non-NULL");
    TEST_ASSERT_EQ(MS_TYPE_INT, msTypeof(result), "result is int");
    TEST_ASSERT_EQ(14, msToInt(vm, result), "2 + 3 * 4 == 14");

    msVmFree(vm);
    TEST_END();
}

int main(void) {
    fprintf(stdout, "test_embed_basic:\n");
    test_vm_new_free();
    test_exec_string_simple();
    test_exec_string_syntax_error();
    test_global_roundtrip();
    test_output_redirect();
    test_two_vms_independent();
    test_module_path();
    test_eval_expression();
    TEST_SUMMARY();
    return TEST_RETURN();
}
```

### 3. test_embed_values.c — 值创建、转换、比较

```c
#include "common.h"

void test_create_primitives(void) {
    TEST_BEGIN("create primitives");

    MsVM* vm = msVmNew();

    MsValue* i = msInt(42);
    TEST_ASSERT_EQ(MS_TYPE_INT, msTypeof(i), "int type");
    TEST_ASSERT(msIsInt(i), "msIsInt");
    TEST_ASSERT(msIsNumber(i), "msIsNumber");
    TEST_ASSERT_EQ(42, msToInt(vm, i), "msToInt");

    MsValue* f = msFloat(3.14);
    TEST_ASSERT_EQ(MS_TYPE_FLOAT, msTypeof(f), "float type");
    TEST_ASSERT(msIsFloat(f), "msIsFloat");
    TEST_ASSERT(msIsNumber(f), "msIsNumber for float");

    MsValue* neg = msInt(-100);
    TEST_ASSERT_EQ(-100, msToInt(vm, neg), "negative int");

    MsValue* dbl = msFloat(1e10);
    TEST_ASSERT(dbl != NULL, "large float created");

    MsValue* zero = msInt(0);
    TEST_ASSERT_EQ(0, msToInt(vm, zero), "zero int");

    msVmFree(vm);
    TEST_END();
}

void test_create_string(void) {
    TEST_BEGIN("create string");

    MsVM* vm = msVmNew();

    MsValue* s = msString(vm, "hello world");
    TEST_ASSERT_EQ(MS_TYPE_STRING, msTypeof(s), "string type");
    TEST_ASSERT(msIsString(s), "msIsString");
    TEST_ASSERT(!msIsInt(s), "string is not int");

    const char* data = msToString(vm, s);
    TEST_ASSERT_STR_EQ("hello world", data, "msToString");

    size_t slen = msStringLen(vm, s);
    TEST_ASSERT_EQ(11, (long)slen, "string len");

    MsValue* empty = msString(vm, "");
    TEST_ASSERT_EQ(0, (long)msStringLen(vm, empty), "empty string len");

    msVmFree(vm);
    TEST_END();
}

void test_create_stringn(void) {
    TEST_BEGIN("create stringn with embedded null");

    MsVM* vm = msVmNew();
    const char raw[] = "ab\x00""cd";
    MsValue* s = msStringn(vm, raw, 5);
    TEST_ASSERT_NOT_NULL(s, "stringn created");
    TEST_ASSERT_EQ(5, (long)msStringLen(vm, s), "stringn len == 5");

    msVmFree(vm);
    TEST_END();
}

void test_nil_and_bool(void) {
    TEST_BEGIN("nil and bool");

    MsValue* nil = msNil();
    TEST_ASSERT(msIsNil(nil), "is nil");
    TEST_ASSERT(!msIsBool(nil), "nil is not bool");
    TEST_ASSERT_EQ(MS_TYPE_NIL, msTypeof(nil), "nil type");
    TEST_ASSERT_EQ(MS_FALSE, msToBool(nil), "nil to bool == false");

    MsValue* t = msBoolVal(1);
    TEST_ASSERT(msIsBool(t), "is bool");
    TEST_ASSERT_EQ(MS_TRUE, msToBool(t), "true to bool");

    MsValue* f = msBoolVal(0);
    TEST_ASSERT_EQ(MS_FALSE, msToBool(f), "false to bool");

    TEST_END();
}

void test_to_bool_truthy(void) {
    TEST_BEGIN("to bool truthy rules");

    MsVM* vm = msVmNew();

    TEST_ASSERT_EQ(MS_FALSE, msToBool(msNil()), "nil is falsy");
    TEST_ASSERT_EQ(MS_FALSE, msToBool(msBoolVal(0)), "false is falsy");
    TEST_ASSERT_EQ(MS_TRUE, msToBool(msBoolVal(1)), "true is truthy");
    TEST_ASSERT_EQ(MS_FALSE, msToBool(msInt(0)), "0 is falsy");
    TEST_ASSERT_EQ(MS_TRUE, msToBool(msInt(1)), "1 is truthy");
    TEST_ASSERT_EQ(MS_TRUE, msToBool(msInt(-1)), "-1 is truthy");
    TEST_ASSERT_EQ(MS_FALSE, msToBool(msFloat(0.0)), "0.0 is falsy");
    TEST_ASSERT_EQ(MS_TRUE, msToBool(msFloat(0.1)), "0.1 is truthy");

    MsValue* s = msString(vm, "hello");
    TEST_ASSERT_EQ(MS_TRUE, msToBool(s), "non-empty string is truthy");

    MsValue* empty_s = msString(vm, "");
    TEST_ASSERT_EQ(MS_FALSE, msToBool(empty_s), "empty string is falsy");

    msVmFree(vm);
    TEST_END();
}

void test_comparison(void) {
    TEST_BEGIN("comparison operators");

    MsVM* vm = msVmNew();

    MsValue* a = msInt(10);
    MsValue* b = msInt(20);
    MsValue* c = msInt(10);

    TEST_ASSERT_EQ(MS_TRUE, msEq(vm, a, c), "10 == 10");
    TEST_ASSERT_EQ(MS_FALSE, msEq(vm, a, b), "10 != 20");
    TEST_ASSERT_EQ(MS_TRUE, msLt(vm, a, b), "10 < 20");
    TEST_ASSERT_EQ(MS_FALSE, msLt(vm, b, a), "!(20 < 10)");
    TEST_ASSERT_EQ(MS_TRUE, msLe(vm, a, c), "10 <= 10");
    TEST_ASSERT_EQ(MS_TRUE, msGt(vm, b, a), "20 > 10");
    TEST_ASSERT_EQ(MS_TRUE, msGe(vm, a, c), "10 >= 10");

    MsValue* s1 = msString(vm, "abc");
    MsValue* s2 = msString(vm, "abd");
    TEST_ASSERT_EQ(MS_TRUE, msLt(vm, s1, s2), "abc < abd");

    msVmFree(vm);
    TEST_END();
}

void test_hash(void) {
    TEST_BEGIN("hash consistency");

    MsVM* vm = msVmNew();

    MsValue* a = msInt(42);
    MsValue* b = msInt(42);
    TEST_ASSERT_EQ(msHash(vm, a), msHash(vm, b), "same int same hash");

    MsValue* s1 = msString(vm, "hello");
    MsValue* s2 = msString(vm, "hello");
    TEST_ASSERT_EQ(msHash(vm, s1), msHash(vm, s2), "same string same hash");

    msVmFree(vm);
    TEST_END();
}

void test_explicit_conversion(void) {
    TEST_BEGIN("explicit type conversion");

    MsVM* vm = msVmNew();

    MsValue* f = msFloat(3.0);
    MsValue* i = msConvertInt(vm, f);
    TEST_ASSERT_NOT_NULL(i, "float to int");
    TEST_ASSERT_EQ(3, msToInt(vm, i), "3.0 -> 3");

    MsValue* n = msInt(42);
    MsValue* s = msConvertStr(vm, n);
    TEST_ASSERT_NOT_NULL(s, "int to str");
    TEST_ASSERT(msIsString(s), "result is string");

    msVmFree(vm);
    TEST_END();
}

void test_root_unroot(void) {
    TEST_BEGIN("root/unroot lifecycle");

    MsVM* vm = msVmNew();

    MsValue* s = msString(vm, "rooted");
    MsValue* r = msRoot(vm, s);
    TEST_ASSERT(r == s, "msRoot returns same pointer");

    msUnroot(vm, s);

    msRoot(vm, s);
    msUnroot(vm, s);

    msVmFree(vm);
    TEST_END();
}

void test_string_concat(void) {
    TEST_BEGIN("string concat");

    MsVM* vm = msVmNew();
    MsValue* a = msString(vm, "hello ");
    MsValue* b = msString(vm, "world");
    MsValue* c = msStringConcat(vm, a, b);
    TEST_ASSERT_STR_EQ("hello world", msToString(vm, c), "concat result");

    msVmFree(vm);
    TEST_END();
}

void test_string_slice(void) {
    TEST_BEGIN("string slice");

    MsVM* vm = msVmNew();
    MsValue* s = msString(vm, "hello world");
    MsValue* sub = msStringSlice(vm, s, 0, 5);
    TEST_ASSERT_STR_EQ("hello", msToString(vm, sub), "slice [0:5]");

    msVmFree(vm);
    TEST_END();
}

void test_to_string_copy(void) {
    TEST_BEGIN("toStringCopy");

    MsVM* vm = msVmNew();
    MsValue* s = msString(vm, "owned");
    char* copy = msToStringCopy(vm, s);
    TEST_ASSERT_NOT_NULL(copy, "copy non-NULL");
    TEST_ASSERT_STR_EQ("owned", copy, "copy content");
    free(copy);

    msVmFree(vm);
    TEST_END();
}

void test_is_identity(void) {
    TEST_BEGIN("is operator");

    MsVM* vm = msVmNew();

    MsValue* n1 = msNil();
    MsValue* n2 = msNil();
    TEST_ASSERT_EQ(MS_TRUE, msIs(n1, n2), "nil is nil");

    MsValue* a = msInt(42);
    MsValue* b = msInt(42);
    TEST_ASSERT_EQ(MS_TRUE, msIs(a, b), "same int is");

    msVmFree(vm);
    TEST_END();
}

void test_string_fmt(void) {
    TEST_BEGIN("stringFmt");

    MsVM* vm = msVmNew();
    MsValue* s = msStringFmt(vm, "%d + %d = %d", 1, 2, 3);
    TEST_ASSERT_NOT_NULL(s, "fmt non-NULL");
    TEST_ASSERT(msIsString(s), "fmt is string");

    msVmFree(vm);
    TEST_END();
}

int main(void) {
    fprintf(stdout, "test_embed_values:\n");
    test_create_primitives();
    test_create_string();
    test_create_stringn();
    test_nil_and_bool();
    test_to_bool_truthy();
    test_comparison();
    test_hash();
    test_explicit_conversion();
    test_root_unroot();
    test_string_concat();
    test_string_slice();
    test_to_string_copy();
    test_is_identity();
    test_string_fmt();
    TEST_SUMMARY();
    return TEST_RETURN();
}
```

### 4. test_embed_collections.c — List/Dict/Tuple/Set 操作

```c
#include "common.h"

void test_list_basic(void) {
    TEST_BEGIN("list basic");

    MsVM* vm = msVmNew();
    MsValue* list = msListNew(vm);
    TEST_ASSERT(msIsList(list), "is list");
    TEST_ASSERT_EQ(0, msListLen(vm, list), "new list empty");

    MsValue* a = msInt(10);
    MsValue* b = msInt(20);
    MsValue* c = msInt(30);

    TEST_ASSERT_EQ(MS_OK, msListPush(vm, list, a), "push a");
    TEST_ASSERT_EQ(MS_OK, msListPush(vm, list, b), "push b");
    TEST_ASSERT_EQ(MS_OK, msListPush(vm, list, c), "push c");
    TEST_ASSERT_EQ(3, msListLen(vm, list), "len == 3");

    TEST_ASSERT_EQ(10, msToInt(vm, msListGet(vm, list, 0)), "list[0] == 10");
    TEST_ASSERT_EQ(20, msToInt(vm, msListGet(vm, list, 1)), "list[1] == 20");
    TEST_ASSERT_EQ(30, msToInt(vm, msListGet(vm, list, 2)), "list[2] == 30");

    TEST_ASSERT_EQ(10, msToInt(vm, msListGet(vm, list, -3)), "list[-3] == 10");

    MsValue* popped = msListPop(vm, list);
    TEST_ASSERT_EQ(30, msToInt(vm, popped), "popped == 30");
    TEST_ASSERT_EQ(2, msListLen(vm, list), "len after pop == 2");

    msVmFree(vm);
    TEST_END();
}

void test_list_set_insert(void) {
    TEST_BEGIN("list set/insert");

    MsVM* vm = msVmNew();
    MsValue* list = msListNew(vm);
    MsValue* a = msInt(1);
    msListPush(vm, list, a);
    msListPush(vm, list, a);

    MsValue* val = msInt(99);
    TEST_ASSERT_EQ(MS_OK, msListSet(vm, list, 0, val), "set [0]");
    TEST_ASSERT_EQ(99, msToInt(vm, msListGet(vm, list, 0)), "list[0] == 99");

    MsValue* ins = msInt(50);
    TEST_ASSERT_EQ(MS_OK, msListInsert(vm, list, 1, ins), "insert at 1");
    TEST_ASSERT_EQ(3, msListLen(vm, list), "len after insert == 3");
    TEST_ASSERT_EQ(50, msToInt(vm, msListGet(vm, list, 1)), "list[1] == 50");

    msVmFree(vm);
    TEST_END();
}

void test_list_from(void) {
    TEST_BEGIN("list from array");

    MsVM* vm = msVmNew();
    MsValue* items[] = { msInt(1), msInt(2), msInt(3) };
    MsValue* list = msListFrom(vm, items, 3);

    TEST_ASSERT_NOT_NULL(list, "listFrom non-NULL");
    TEST_ASSERT_EQ(3, msListLen(vm, list), "listFrom len == 3");
    TEST_ASSERT_EQ(2, msToInt(vm, msListGet(vm, list, 1)), "listFrom[1] == 2");

    msVmFree(vm);
    TEST_END();
}

void test_list_contains(void) {
    TEST_BEGIN("list contains");

    MsVM* vm = msVmNew();
    MsValue* items[] = { msInt(1), msInt(2), msInt(3) };
    MsValue* list = msListFrom(vm, items, 3);

    TEST_ASSERT_EQ(MS_TRUE, msListContains(vm, list, msInt(2)), "contains 2");
    TEST_ASSERT_EQ(MS_FALSE, msListContains(vm, list, msInt(99)), "!contains 99");

    msVmFree(vm);
    TEST_END();
}

void test_list_slice(void) {
    TEST_BEGIN("list slice");

    MsVM* vm = msVmNew();
    MsValue* items[] = { msInt(0), msInt(1), msInt(2), msInt(3), msInt(4) };
    MsValue* list = msListFrom(vm, items, 5);

    MsValue* sub = msListSlice(vm, list, 1, 4, 1);
    TEST_ASSERT_NOT_NULL(sub, "slice non-NULL");
    TEST_ASSERT_EQ(3, msListLen(vm, sub), "slice len == 3");
    TEST_ASSERT_EQ(1, msToInt(vm, msListGet(vm, sub, 0)), "slice[0] == 1");
    TEST_ASSERT_EQ(3, msToInt(vm, msListGet(vm, sub, 2)), "slice[2] == 3");

    msVmFree(vm);
    TEST_END();
}

void test_dict_basic(void) {
    TEST_BEGIN("dict basic");

    MsVM* vm = msVmNew();
    MsValue* dict = msDictNew(vm);
    TEST_ASSERT(msIsDict(dict), "is dict");
    TEST_ASSERT_EQ(0, msDictLen(vm, dict), "new dict empty");

    MsValue* k1 = msString(vm, "name");
    MsValue* v1 = msString(vm, "mslang");
    TEST_ASSERT_EQ(MS_OK, msDictSet(vm, dict, k1, v1), "dict set name");

    MsValue* k2 = msString(vm, "version");
    MsValue* v2 = msInt(1);
    TEST_ASSERT_EQ(MS_OK, msDictSet(vm, dict, k2, v2), "dict set version");

    TEST_ASSERT_EQ(2, msDictLen(vm, dict), "dict len == 2");

    MsValue* got = msDictGet(vm, dict, k1);
    TEST_ASSERT_NOT_NULL(got, "dict get name");
    TEST_ASSERT_STR_EQ("mslang", msToString(vm, got), "name == mslang");

    TEST_ASSERT_EQ(MS_TRUE, msDictContains(vm, dict, k1), "contains name");
    TEST_ASSERT_EQ(MS_FALSE, msDictContains(vm, dict, msString(vm, "nope")), "!contains nope");

    msDictRemove(vm, dict, k1);
    TEST_ASSERT_EQ(1, msDictLen(vm, dict), "after remove len == 1");
    TEST_ASSERT_NULL(msDictGet(vm, dict, k1), "removed key get NULL");

    msVmFree(vm);
    TEST_END();
}

void test_dict_from(void) {
    TEST_BEGIN("dict from pairs");

    MsVM* vm = msVmNew();
    MsValue* k1 = msString(vm, "x");
    MsValue* v1 = msInt(10);
    MsValue* k2 = msString(vm, "y");
    MsValue* v2 = msInt(20);
    MsValue* pairs[] = { k1, v1, k2, v2 };
    MsValue* dict = msDictFrom(vm, pairs, 2);

    TEST_ASSERT_EQ(2, msDictLen(vm, dict), "dictFrom len == 2");
    TEST_ASSERT_EQ(10, msToInt(vm, msDictGet(vm, dict, k1)), "dictFrom[x] == 10");

    msVmFree(vm);
    TEST_END();
}

void test_dict_keys_values_items(void) {
    TEST_BEGIN("dict keys/values/items");

    MsVM* vm = msVmNew();
    MsValue* k1 = msString(vm, "a");
    MsValue* v1 = msInt(1);
    MsValue* k2 = msString(vm, "b");
    MsValue* v2 = msInt(2);
    MsValue* pairs[] = { k1, v1, k2, v2 };
    MsValue* dict = msDictFrom(vm, pairs, 2);

    MsValue* keys = msDictKeys(vm, dict);
    TEST_ASSERT(msIsList(keys), "keys is list");
    TEST_ASSERT_EQ(2, msListLen(vm, keys), "keys len == 2");

    MsValue* values = msDictValues(vm, dict);
    TEST_ASSERT(msIsList(values), "values is list");
    TEST_ASSERT_EQ(2, msListLen(vm, values), "values len == 2");

    MsValue* items = msDictItems(vm, dict);
    TEST_ASSERT(msIsList(items), "items is list");
    TEST_ASSERT_EQ(2, msListLen(vm, items), "items len == 2");

    msVmFree(vm);
    TEST_END();
}

void test_dict_get_default(void) {
    TEST_BEGIN("dict get default");

    MsVM* vm = msVmNew();
    MsValue* dict = msDictNew(vm);
    MsValue* k = msString(vm, "absent");
    MsValue* def = msInt(999);

    MsValue* got = msDictGetDefault(vm, dict, k, def);
    TEST_ASSERT_EQ(999, msToInt(vm, got), "get default returns 999");

    msVmFree(vm);
    TEST_END();
}

void test_tuple_basic(void) {
    TEST_BEGIN("tuple basic");

    MsVM* vm = msVmNew();
    MsValue* items[] = { msInt(10), msInt(20), msInt(30) };
    MsValue* tup = msTupleFrom(vm, items, 3);

    TEST_ASSERT(msIsTuple(tup), "is tuple");
    TEST_ASSERT_EQ(3, msTupleLen(vm, tup), "tuple len == 3");
    TEST_ASSERT_EQ(20, msToInt(vm, msTupleGet(vm, tup, 1)), "tuple[1] == 20");
    TEST_ASSERT_EQ(30, msToInt(vm, msTupleGet(vm, tup, -1)), "tuple[-1] == 30");

    msVmFree(vm);
    TEST_END();
}

void test_tuple_unpack(void) {
    TEST_BEGIN("tuple unpack");

    MsVM* vm = msVmNew();
    MsValue* items[] = { msInt(1), msInt(2), msInt(3) };
    MsValue* tup = msTupleFrom(vm, items, 3);

    MsValue** unpacked = NULL;
    int count = 0;
    MsStatus s = msTupleUnpack(vm, tup, &unpacked, &count);
    TEST_ASSERT_EQ(MS_OK, s, "unpack succeeds");
    TEST_ASSERT_EQ(3, count, "unpack count == 3");
    TEST_ASSERT_EQ(1, msToInt(vm, unpacked[0]), "unpacked[0] == 1");
    TEST_ASSERT_EQ(3, msToInt(vm, unpacked[2]), "unpacked[2] == 3");

    free(unpacked);
    msVmFree(vm);
    TEST_END();
}

void test_set_basic(void) {
    TEST_BEGIN("set basic");

    MsVM* vm = msVmNew();
    MsValue* set = msSetNew(vm);
    TEST_ASSERT(msIsSet(set), "is set");
    TEST_ASSERT_EQ(0, msSetLen(vm, set), "new set empty");

    MsValue* a = msInt(1);
    MsValue* b = msInt(2);
    MsValue* c = msInt(1);

    TEST_ASSERT_EQ(MS_OK, msSetAdd(vm, set, a), "add 1");
    TEST_ASSERT_EQ(MS_OK, msSetAdd(vm, set, b), "add 2");
    TEST_ASSERT_EQ(MS_OK, msSetAdd(vm, set, c), "add 1 again");
    TEST_ASSERT_EQ(2, msSetLen(vm, set), "set dedup len == 2");

    TEST_ASSERT_EQ(MS_TRUE, msSetContains(vm, set, a), "contains 1");
    TEST_ASSERT_EQ(MS_TRUE, msSetContains(vm, set, b), "contains 2");
    TEST_ASSERT_EQ(MS_FALSE, msSetContains(vm, set, msInt(99)), "!contains 99");

    msSetRemove(vm, set, a);
    TEST_ASSERT_EQ(1, msSetLen(vm, set), "after remove len == 1");

    msVmFree(vm);
    TEST_END();
}

void test_iterator(void) {
    TEST_BEGIN("iterator");

    MsVM* vm = msVmNew();
    MsValue* items[] = { msInt(10), msInt(20), msInt(30) };
    MsValue* list = msListFrom(vm, items, 3);

    MsValue* iter = msIter(vm, list);
    TEST_ASSERT_NOT_NULL(iter, "iterator non-NULL");

    int sum = 0;
    MsValue* val = NULL;
    while (msNext(vm, iter, &val) == MS_OK) {
        sum += (int)msToInt(vm, val);
    }
    TEST_ASSERT_EQ(60, sum, "iterated sum == 60");

    msVmFree(vm);
    TEST_END();
}

void test_generic_len(void) {
    TEST_BEGIN("generic len");

    MsVM* vm = msVmNew();

    MsValue* items[] = { msInt(1), msInt(2) };
    MsValue* list = msListFrom(vm, items, 2);
    TEST_ASSERT_EQ(2, (long)msLen(vm, list), "len(list) == 2");

    MsValue* s = msString(vm, "hello");
    TEST_ASSERT_EQ(5, (long)msLen(vm, s), "len(str) == 5");

    MsValue* dict = msDictNew(vm);
    msDictSet(vm, dict, msString(vm, "k"), msInt(1));
    TEST_ASSERT_EQ(1, (long)msLen(vm, dict), "len(dict) == 1");

    msVmFree(vm);
    TEST_END();
}

void test_repr(void) {
    TEST_BEGIN("repr");

    MsVM* vm = msVmNew();
    MsValue* i = msInt(42);
    MsValue* r = msRepr(vm, i);
    TEST_ASSERT_NOT_NULL(r, "repr non-NULL");
    TEST_ASSERT(msIsString(r), "repr is string");

    msVmFree(vm);
    TEST_END();
}

void test_getitem_setitem(void) {
    TEST_BEGIN("getitem/setitem");

    MsVM* vm = msVmNew();
    MsValue* list = msListNew(vm);
    msListPush(vm, list, msInt(0));

    MsValue* idx = msInt(0);
    MsValue* got = msGetItem(vm, list, idx);
    TEST_ASSERT_EQ(0, msToInt(vm, got), "getitem [0] == 0");

    MsValue* new_val = msInt(99);
    msSetItem(vm, list, idx, new_val);
    TEST_ASSERT_EQ(99, msToInt(vm, msGetItem(vm, list, idx)), "setitem [0] == 99");

    msVmFree(vm);
    TEST_END();
}

int main(void) {
    fprintf(stdout, "test_embed_collections:\n");
    test_list_basic();
    test_list_set_insert();
    test_list_from();
    test_list_contains();
    test_list_slice();
    test_dict_basic();
    test_dict_from();
    test_dict_keys_values_items();
    test_dict_get_default();
    test_tuple_basic();
    test_tuple_unpack();
    test_set_basic();
    test_iterator();
    test_generic_len();
    test_repr();
    test_getitem_setitem();
    TEST_SUMMARY();
    return TEST_RETURN();
}
```

### 5. test_embed_call.c — 函数调用

```c
#include "common.h"

void test_call_zero_args(void) {
    TEST_BEGIN("call zero args");

    MsVM* vm = msVmNew();
    msExecString(vm, "fn fortytwo() { return 42 }", "test.ms");
    MsValue* fn = msGetGlobal(vm, "fortytwo");
    msRoot(vm, fn);

    MsValue* result = msCall0(vm, fn);
    TEST_ASSERT_NOT_NULL(result, "call result non-NULL");
    TEST_ASSERT_EQ(42, msToInt(vm, result), "fortytwo() == 42");

    msUnroot(vm, fn);
    msVmFree(vm);
    TEST_END();
}

void test_call_with_args(void) {
    TEST_BEGIN("call with args");

    MsVM* vm = msVmNew();
    msExecString(vm, "fn add(a, b) { return a + b }", "test.ms");
    MsValue* fn = msGetGlobal(vm, "add");
    msRoot(vm, fn);

    MsValue* a = msInt(3);
    MsValue* b = msInt(4);
    MsValue* result = msCall2(vm, fn, a, b);
    TEST_ASSERT_NOT_NULL(result, "call2 non-NULL");
    TEST_ASSERT_EQ(7, msToInt(vm, result), "add(3, 4) == 7");

    msUnroot(vm, fn);
    msVmFree(vm);
    TEST_END();
}

void test_call1(void) {
    TEST_BEGIN("call1");

    MsVM* vm = msVmNew();
    msExecString(vm, "fn double(x) { return x * 2 }", "test.ms");
    MsValue* fn = msGetGlobal(vm, "double");
    msRoot(vm, fn);

    MsValue* result = msCall1(vm, fn, msInt(21));
    TEST_ASSERT_EQ(42, msToInt(vm, result), "double(21) == 42");

    msUnroot(vm, fn);
    msVmFree(vm);
    TEST_END();
}

void test_call3(void) {
    TEST_BEGIN("call3");

    MsVM* vm = msVmNew();
    msExecString(vm, "fn sum3(a, b, c) { return a + b + c }", "test.ms");
    MsValue* fn = msGetGlobal(vm, "sum3");
    msRoot(vm, fn);

    MsValue* result = msCall3(vm, fn, msInt(10), msInt(20), msInt(30));
    TEST_ASSERT_EQ(60, msToInt(vm, result), "sum3(10,20,30) == 60");

    msUnroot(vm, fn);
    msVmFree(vm);
    TEST_END();
}

void test_call_recursive_fibonacci(void) {
    TEST_BEGIN("call recursive fibonacci");

    MsVM* vm = msVmNew();
    msExecString(vm,
        "fn fibonacci(n) {\n"
        "  if n <= 1 { return n }\n"
        "  return fibonacci(n - 1) + fibonacci(n - 2)\n"
        "}\n",
        "test.ms");

    MsValue* fib = msGetGlobal(vm, "fibonacci");
    msRoot(vm, fib);

    MsValue* result = msCall1(vm, fib, msInt(10));
    TEST_ASSERT_NOT_NULL(result, "fib(10) non-NULL");
    TEST_ASSERT_EQ(55, msToInt(vm, result), "fibonacci(10) == 55");

    msUnroot(vm, fib);
    msVmFree(vm);
    TEST_END();
}

void test_call_returns_string(void) {
    TEST_BEGIN("call returns string");

    MsVM* vm = msVmNew();
    msExecString(vm, "fn greet(name) { return \"hello \" + name }", "test.ms");
    MsValue* fn = msGetGlobal(vm, "greet");
    msRoot(vm, fn);

    MsValue* result = msCall1(vm, fn, msString(vm, "world"));
    TEST_ASSERT(msIsString(result), "result is string");
    TEST_ASSERT_STR_EQ("hello world", msToString(vm, result), "greet result");

    msUnroot(vm, fn);
    msVmFree(vm);
    TEST_END();
}

void test_call_returns_list(void) {
    TEST_BEGIN("call returns list");

    MsVM* vm = msVmNew();
    msExecString(vm, "fn mklist() { return [1, 2, 3] }", "test.ms");
    MsValue* fn = msGetGlobal(vm, "mklist");
    msRoot(vm, fn);

    MsValue* result = msCall0(vm, fn);
    TEST_ASSERT(msIsList(result), "result is list");
    TEST_ASSERT_EQ(3, msListLen(vm, result), "list len == 3");

    msUnroot(vm, fn);
    msVmFree(vm);
    TEST_END();
}

void test_call_closure(void) {
    TEST_BEGIN("call closure");

    MsVM* vm = msVmNew();
    msExecString(vm,
        "fn make_adder(x) {\n"
        "  return fn(y) { return x + y }\n"
        "}\n",
        "test.ms");

    MsValue* make = msGetGlobal(vm, "make_adder");
    msRoot(vm, make);

    MsValue* adder = msCall1(vm, make, msInt(10));
    TEST_ASSERT_NOT_NULL(adder, "adder non-NULL");
    msRoot(vm, adder);

    MsValue* result = msCall1(vm, adder, msInt(5));
    TEST_ASSERT_EQ(15, msToInt(vm, result), "adder(5) == 15");

    msUnroot(vm, adder);
    msUnroot(vm, make);
    msVmFree(vm);
    TEST_END();
}

void test_call_exception(void) {
    TEST_BEGIN("call exception");

    MsVM* vm = msVmNew();
    msExecString(vm, "fn boom() { throw ValueError(\"exploded\") }", "test.ms");
    MsValue* fn = msGetGlobal(vm, "boom");
    msRoot(vm, fn);

    MsValue* result = msCall0(vm, fn);
    TEST_ASSERT_NULL(result, "throwing call returns NULL");
    TEST_ASSERT(msErrOccurred(vm), "error occurred");

    MsValue* err = msErrFetch(vm);
    TEST_ASSERT_NOT_NULL(err, "err non-NULL");
    const char* type = msErrTypeName(vm, err);
    TEST_ASSERT_STR_EQ("ValueError", type, "error type");
    const char* msg = msErrMessage(vm, err);
    TEST_ASSERT(strstr(msg, "exploded") != NULL, "error message");

    msUnroot(vm, err);
    msUnroot(vm, fn);
    msVmFree(vm);
    TEST_END();
}

void test_call_non_callable(void) {
    TEST_BEGIN("call non-callable");

    MsVM* vm = msVmNew();
    MsValue* not_fn = msInt(42);
    MsValue* result = msCall0(vm, not_fn);
    TEST_ASSERT_NULL(result, "calling int returns NULL");

    msVmFree(vm);
    TEST_END();
}

void test_call_null_vm(void) {
    TEST_BEGIN("call null vm");

    MsValue* result = msCall(NULL, NULL, NULL, 0);
    TEST_ASSERT_NULL(result, "null vm returns NULL");

    TEST_END();
}

void test_call_script_callback(void) {
    TEST_BEGIN("call script callback from C");

    MsVM* vm = msVmNew();
    msExecString(vm,
        "fn transform(lst) {\n"
        "  result = []\n"
        "  for item in lst {\n"
        "    result.push(item * 2)\n"
        "  }\n"
        "  return result\n"
        "}\n",
        "test.ms");

    MsValue* fn = msGetGlobal(vm, "transform");
    msRoot(vm, fn);

    MsValue* items[] = { msInt(1), msInt(2), msInt(3) };
    MsValue* input_list = msListFrom(vm, items, 3);
    MsValue* result = msCall1(vm, fn, input_list);

    TEST_ASSERT(msIsList(result), "result is list");
    TEST_ASSERT_EQ(3, msListLen(vm, result), "result len == 3");
    TEST_ASSERT_EQ(2, msToInt(vm, msListGet(vm, result, 0)), "result[0] == 2");
    TEST_ASSERT_EQ(4, msToInt(vm, msListGet(vm, result, 1)), "result[1] == 4");
    TEST_ASSERT_EQ(6, msToInt(vm, msListGet(vm, result, 2)), "result[2] == 6");

    msUnroot(vm, fn);
    msVmFree(vm);
    TEST_END();
}

int main(void) {
    fprintf(stdout, "test_embed_call:\n");
    test_call_zero_args();
    test_call_with_args();
    test_call1();
    test_call3();
    test_call_recursive_fibonacci();
    test_call_returns_string();
    test_call_returns_list();
    test_call_closure();
    test_call_exception();
    test_call_non_callable();
    test_call_null_vm();
    test_call_script_callback();
    TEST_SUMMARY();
    return TEST_RETURN();
}
```

### 6. test_embed_error.c — 异常处理

```c
#include "common.h"

void test_err_occurred_initial(void) {
    TEST_BEGIN("err initially false");

    MsVM* vm = msVmNew();
    TEST_ASSERT_EQ(MS_FALSE, msErrOccurred(vm), "no error initially");
    msVmFree(vm);

    TEST_END();
}

void test_throw_and_fetch(void) {
    TEST_BEGIN("throw and fetch");

    MsVM* vm = msVmNew();
    MsStatus s = msThrowValueError(vm, "bad value");
    TEST_ASSERT_EQ(MS_ERROR, s, "throw returns MS_ERROR");
    TEST_ASSERT_EQ(MS_TRUE, msErrOccurred(vm), "error occurred after throw");

    MsValue* err = msErrFetch(vm);
    TEST_ASSERT_NOT_NULL(err, "err non-NULL");
    TEST_ASSERT_EQ(MS_FALSE, msErrOccurred(vm), "error cleared after fetch");

    TEST_ASSERT_STR_EQ("ValueError", msErrTypeName(vm, err), "type name");
    TEST_ASSERT_STR_EQ("bad value", msErrMessage(vm, err), "message");

    msVmFree(vm);
    TEST_END();
}

void test_err_clear(void) {
    TEST_BEGIN("err clear");

    MsVM* vm = msVmNew();
    msThrowRuntimeError(vm, "oops");
    TEST_ASSERT_EQ(MS_TRUE, msErrOccurred(vm), "error set");
    msErrClear(vm);
    TEST_ASSERT_EQ(MS_FALSE, msErrOccurred(vm), "error cleared");

    msVmFree(vm);
    TEST_END();
}

void test_throw_type_error(void) {
    TEST_BEGIN("throw type error");

    MsVM* vm = msVmNew();
    msThrowTypeError(vm, "string", "int");

    MsValue* err = msErrFetch(vm);
    TEST_ASSERT_STR_EQ("TypeError", msErrTypeName(vm, err), "type");
    const char* msg = msErrMessage(vm, err);
    TEST_ASSERT(strstr(msg, "string") != NULL, "msg contains expected");
    TEST_ASSERT(strstr(msg, "int") != NULL, "msg contains actual");

    msVmFree(vm);
    TEST_END();
}

void test_throw_index_error(void) {
    TEST_BEGIN("throw index error");

    MsVM* vm = msVmNew();
    msThrowIndexError(vm, "out of bounds");
    MsValue* err = msErrFetch(vm);
    TEST_ASSERT_STR_EQ("IndexError", msErrTypeName(vm, err), "type");

    msVmFree(vm);
    TEST_END();
}

void test_throw_key_error(void) {
    TEST_BEGIN("throw key error");

    MsVM* vm = msVmNew();
    MsValue* key = msString(vm, "missing");
    msThrowKeyError(vm, key);

    MsValue* err = msErrFetch(vm);
    TEST_ASSERT_STR_EQ("KeyError", msErrTypeName(vm, err), "type");
    const char* msg = msErrMessage(vm, err);
    TEST_ASSERT(strstr(msg, "missing") != NULL, "msg contains key");

    msVmFree(vm);
    TEST_END();
}

void test_throw_io_error(void) {
    TEST_BEGIN("throw io error");

    MsVM* vm = msVmNew();
    msThrowIoError(vm, "file not found");
    MsValue* err = msErrFetch(vm);
    TEST_ASSERT_STR_EQ("IOError", msErrTypeName(vm, err), "type");

    msVmFree(vm);
    TEST_END();
}

void test_throw_runtime_error(void) {
    TEST_BEGIN("throw runtime error");

    MsVM* vm = msVmNew();
    msThrowRuntimeError(vm, "unexpected");
    MsValue* err = msErrFetch(vm);
    TEST_ASSERT_STR_EQ("RuntimeError", msErrTypeName(vm, err), "type");

    msVmFree(vm);
    TEST_END();
}

void test_throw_value(void) {
    TEST_BEGIN("throw value");

    MsVM* vm = msVmNew();
    msThrowValueError(vm, "original");
    MsValue* original = msErrFetch(vm);

    msThrowValue(vm, original);
    TEST_ASSERT_EQ(MS_TRUE, msErrOccurred(vm), "error set via throwValue");

    MsValue* rethrown = msErrFetch(vm);
    TEST_ASSERT_STR_EQ("ValueError", msErrTypeName(vm, rethrown), "rethrown type");

    msVmFree(vm);
    TEST_END();
}

void test_try_success(void) {
    TEST_BEGIN("try success");

    MsVM* vm = msVmNew();
    msExecString(vm, "fn add(a, b) { return a + b }", "test.ms");
    MsValue* fn = msGetGlobal(vm, "add");
    msRoot(vm, fn);

    MsValue* args[] = { msInt(3), msInt(4) };
    MsValue* result = NULL;
    MsStatus s = msTry(vm, fn, args, 2, &result);

    TEST_ASSERT_EQ(MS_OK, s, "try returns MS_OK");
    TEST_ASSERT_NOT_NULL(result, "result non-NULL");
    TEST_ASSERT_EQ(7, msToInt(vm, result), "add(3,4) == 7");

    msUnroot(vm, fn);
    msVmFree(vm);
    TEST_END();
}

void test_try_exception(void) {
    TEST_BEGIN("try exception");

    MsVM* vm = msVmNew();
    msExecString(vm, "fn boom() { throw RuntimeError(\"boom\") }", "test.ms");
    MsValue* fn = msGetGlobal(vm, "boom");
    msRoot(vm, fn);

    MsValue* result = NULL;
    MsStatus s = msTry(vm, fn, NULL, 0, &result);

    TEST_ASSERT_EQ(MS_ERROR, s, "try returns MS_ERROR");
    TEST_ASSERT_NULL(result, "result is NULL");
    TEST_ASSERT_EQ(MS_TRUE, msErrOccurred(vm), "error available");

    MsValue* err = msErrFetch(vm);
    TEST_ASSERT_NOT_NULL(err, "err non-NULL");
    TEST_ASSERT_STR_EQ("RuntimeError", msErrTypeName(vm, err), "error type");

    msUnroot(vm, err);
    msUnroot(vm, fn);
    msVmFree(vm);
    TEST_END();
}

void test_err_traceback(void) {
    TEST_BEGIN("err traceback");

    MsVM* vm = msVmNew();
    msThrowRuntimeError(vm, "test");
    MsValue* err = msErrFetch(vm);
    const char* tb = msErrTraceback(vm, err);
    TEST_ASSERT_NOT_NULL(tb, "traceback non-NULL");

    msVmFree(vm);
    TEST_END();
}

void test_err_cause_none(void) {
    TEST_BEGIN("err cause none");

    MsVM* vm = msVmNew();
    msThrowRuntimeError(vm, "test");
    MsValue* err = msErrFetch(vm);
    MsValue* cause = msErrCause(vm, err);
    TEST_ASSERT_NULL(cause, "no cause");

    msVmFree(vm);
    TEST_END();
}

void test_exec_syntax_error_catch(void) {
    TEST_BEGIN("exec syntax error catch");

    MsVM* vm = msVmNew();
    MsStatus s = msExecString(vm, "fn (", "bad.ms");
    TEST_ASSERT_EQ(MS_ERROR, s, "syntax error returns MS_ERROR");

    msVmFree(vm);
    TEST_END();
}

void test_throw_in_callback(void) {
    TEST_BEGIN("throw in C callback");

    MsVM* vm = msVmNew();

    MsValue* result = msThrowValueError(vm, "from callback");
    TEST_ASSERT_NULL(result, "throw returns NULL-like");
    TEST_ASSERT_EQ(MS_TRUE, msErrOccurred(vm), "error set");
    msErrClear(vm);

    msVmFree(vm);
    TEST_END();
}

int main(void) {
    fprintf(stdout, "test_embed_error:\n");
    test_err_occurred_initial();
    test_throw_and_fetch();
    test_err_clear();
    test_throw_type_error();
    test_throw_index_error();
    test_throw_key_error();
    test_throw_io_error();
    test_throw_runtime_error();
    test_throw_value();
    test_try_success();
    test_try_exception();
    test_err_traceback();
    test_err_cause_none();
    test_exec_syntax_error_catch();
    test_throw_in_callback();
    TEST_SUMMARY();
    return TEST_RETURN();
}
```

### 7. test_embed_class.c — Class 操作

```c
#include "common.h"

void test_get_class_and_instance(void) {
    TEST_BEGIN("get class and instance");

    MsVM* vm = msVmNew();
    msExecString(vm,
        "class Animal {\n"
        "  fn __init__(self, name) {\n"
        "    self.name = name\n"
        "  }\n"
        "  fn speak(self) {\n"
        "    return self.name + \" speaks\"\n"
        "  }\n"
        "}\n",
        "test.ms");

    MsValue* cls = msGetClass(vm, "Animal");
    TEST_ASSERT_NOT_NULL(cls, "get class Animal");
    TEST_ASSERT(msIsClass(cls), "is class");

    MsValue* args[] = { msString(vm, "Dog") };
    MsValue* inst = msInstanceNew(vm, cls, args, 1);
    TEST_ASSERT_NOT_NULL(inst, "instance created");
    TEST_ASSERT(msIsInstance(inst), "is instance");
    TEST_ASSERT_EQ(MS_TRUE, msIsInstance(vm, inst, cls), "inst is Animal");

    MsValue* name = msInstanceGet(vm, inst, "name");
    TEST_ASSERT_NOT_NULL(name, "inst.name non-NULL");
    TEST_ASSERT_STR_EQ("Dog", msToString(vm, name), "inst.name == Dog");

    msVmFree(vm);
    TEST_END();
}

void test_instance_set(void) {
    TEST_BEGIN("instance set");

    MsVM* vm = msVmNew();
    msExecString(vm,
        "class Point {\n"
        "  fn __init__(self) {\n"
        "    self.x = 0\n"
        "    self.y = 0\n"
        "  }\n"
        "}\n",
        "test.ms");

    MsValue* cls = msGetClass(vm, "Point");
    MsValue* inst = msInstanceNew(vm, cls, NULL, 0);

    msInstanceSet(vm, inst, "x", msInt(10));
    msInstanceSet(vm, inst, "y", msInt(20));

    TEST_ASSERT_EQ(10, msToInt(vm, msInstanceGet(vm, inst, "x")), "x == 10");
    TEST_ASSERT_EQ(20, msToInt(vm, msInstanceGet(vm, inst, "y")), "y == 20");

    msVmFree(vm);
    TEST_END();
}

void test_instance_method_call(void) {
    TEST_BEGIN("instance method call");

    MsVM* vm = msVmNew();
    msExecString(vm,
        "class Counter {\n"
        "  fn __init__(self) {\n"
        "    self.count = 0\n"
        "  }\n"
        "  fn inc(self) {\n"
        "    self.count = self.count + 1\n"
        "    return self.count\n"
        "  }\n"
        "}\n",
        "test.ms");

    MsValue* cls = msGetClass(vm, "Counter");
    MsValue* inst = msInstanceNew(vm, cls, NULL, 0);
    msRoot(vm, inst);

    MsValue* inc_method = msInstanceGet(vm, inst, "inc");
    TEST_ASSERT_NOT_NULL(inc_method, "get inc method");
    msRoot(vm, inc_method);

    MsValue* r1 = msCall1(vm, inc_method, inst);
    TEST_ASSERT_EQ(1, msToInt(vm, r1), "count == 1");

    MsValue* r2 = msCall1(vm, inc_method, inst);
    TEST_ASSERT_EQ(2, msToInt(vm, r2), "count == 2");

    msUnroot(vm, inc_method);
    msUnroot(vm, inst);
    msVmFree(vm);
    TEST_END();
}

void test_inheritance_isinstance(void) {
    TEST_BEGIN("inheritance isinstance");

    MsVM* vm = msVmNew();
    msExecString(vm,
        "class Base {}\n"
        "class Derived < Base {}\n",
        "test.ms");

    MsValue* base = msGetClass(vm, "Base");
    MsValue* derived = msGetClass(vm, "Derived");
    MsValue* inst = msInstanceNew(vm, derived, NULL, 0);

    TEST_ASSERT_EQ(MS_TRUE, msIsInstance(vm, inst, derived), "inst is Derived");
    TEST_ASSERT_EQ(MS_TRUE, msIsInstance(vm, inst, base), "inst is Base (inherited)");

    MsValue* base_inst = msInstanceNew(vm, base, NULL, 0);
    TEST_ASSERT_EQ(MS_FALSE, msIsInstance(vm, base_inst, derived), "base inst not Derived");

    msVmFree(vm);
    TEST_END();
}

static MsValue* c_greet(MsVM* vm, MsValue* const* args, int nargs) {
    if (nargs < 1) return msThrowValueError(vm, "need self");
    msInstanceSet(vm, args[0], "greeted", msBoolVal(1));
    return msStringFmt(vm, "Hello from C");
}

void test_class_define_from_c(void) {
    TEST_BEGIN("class define from C");

    MsVM* vm = msVmNew();
    MsValue* cls = msClassDefine(vm, "CGreeter", NULL);
    TEST_ASSERT_NOT_NULL(cls, "class defined");
    TEST_ASSERT(msIsClass(cls), "is class");

    MsStatus s = msClassAddMethod(vm, cls, "greet", c_greet);
    TEST_ASSERT_EQ(MS_OK, s, "add method");

    msSetGlobal(vm, "CGreeter", cls);

    msExecString(vm,
        "g = CGreeter()\n"
        "msg = g.greet()\n",
        "test.ms");

    MsValue* msg = msGetGlobal(vm, "msg");
    TEST_ASSERT_NOT_NULL(msg, "msg non-NULL");
    TEST_ASSERT_STR_EQ("Hello from C", msToString(vm, msg), "greet result");

    MsValue* g = msGetGlobal(vm, "g");
    MsValue* greeted = msInstanceGet(vm, g, "greeted");
    TEST_ASSERT_EQ(MS_TRUE, msToBool(greeted), "greeted set");

    msVmFree(vm);
    TEST_END();
}

void test_class_add_static(void) {
    TEST_BEGIN("class add static");

    MsVM* vm = msVmNew();
    MsValue* cls = msClassDefine(vm, "Math", NULL);
    msClassAddStatic(vm, cls, "PI", msFloat(3.14159));
    msSetGlobal(vm, "Math", cls);

    msExecString(vm, "pi = Math.PI", "test.ms");
    MsValue* pi = msGetGlobal(vm, "pi");
    TEST_ASSERT_NOT_NULL(pi, "static PI non-NULL");
    TEST_ASSERT(msIsFloat(pi), "PI is float");

    msVmFree(vm);
    TEST_END();
}

void test_get_class_nonexistent(void) {
    TEST_BEGIN("get class nonexistent");

    MsVM* vm = msVmNew();
    MsValue* cls = msGetClass(vm, "NoSuchClass");
    TEST_ASSERT_NULL(cls, "nonexistent class returns NULL");

    msVmFree(vm);
    TEST_END();
}

void test_instance_new_with_init_args(void) {
    TEST_BEGIN("instance new with init args");

    MsVM* vm = msVmNew();
    msExecString(vm,
        "class Vec2 {\n"
        "  fn __init__(self, x, y) {\n"
        "    self.x = x\n"
        "    self.y = y\n"
        "  }\n"
        "  fn mag(self) {\n"
        "    return self.x * self.x + self.y * self.y\n"
        "  }\n"
        "}\n",
        "test.ms");

    MsValue* cls = msGetClass(vm, "Vec2");
    MsValue* args[] = { msInt(3), msInt(4) };
    MsValue* v = msInstanceNew(vm, cls, args, 2);
    msRoot(vm, v);

    TEST_ASSERT_EQ(3, msToInt(vm, msInstanceGet(vm, v, "x")), "x == 3");
    TEST_ASSERT_EQ(4, msToInt(vm, msInstanceGet(vm, v, "y")), "y == 4");

    MsValue* mag_method = msInstanceGet(vm, v, "mag");
    msRoot(vm, mag_method);
    MsValue* mag = msCall1(vm, mag_method, v);
    TEST_ASSERT_EQ(25, msToInt(vm, mag), "3^2 + 4^2 == 25");

    msUnroot(vm, mag_method);
    msUnroot(vm, v);
    msVmFree(vm);
    TEST_END();
}

int main(void) {
    fprintf(stdout, "test_embed_class:\n");
    test_get_class_and_instance();
    test_instance_set();
    test_instance_method_call();
    test_inheritance_isinstance();
    test_class_define_from_c();
    test_class_add_static();
    test_get_class_nonexistent();
    test_instance_new_with_init_args();
    TEST_SUMMARY();
    return TEST_RETURN();
}
```

### 8. test_embed_gc.c — GC 交互

```c
#include "common.h"

void test_gc_collect(void) {
    TEST_BEGIN("gc collect all types");

    MsVM* vm = msVmNew();
    msGcCollect(vm, MS_GC_MINOR);
    msGcCollect(vm, MS_GC_MAJOR);
    msGcCollect(vm, MS_GC_FULL);

    msVmFree(vm);
    TEST_END();
}

void test_gc_enable_disable(void) {
    TEST_BEGIN("gc enable/disable");

    MsVM* vm = msVmNew();
    TEST_ASSERT_EQ(MS_TRUE, msGcIsEnabled(vm), "initially enabled");

    msGcEnable(vm, MS_FALSE);
    TEST_ASSERT_EQ(MS_FALSE, msGcIsEnabled(vm), "disabled");

    msGcEnable(vm, MS_TRUE);
    TEST_ASSERT_EQ(MS_TRUE, msGcIsEnabled(vm), "re-enabled");

    msVmFree(vm);
    TEST_END();
}

void test_gc_stats(void) {
    TEST_BEGIN("gc stats");

    MsVM* vm = msVmNew();
    MsGcStats s = msGcStats(vm);
    TEST_ASSERT_EQ(0, (long)s.minorGcCount, "initial minor 0");
    TEST_ASSERT_EQ(0, (long)s.majorGcCount, "initial major 0");

    msGcCollect(vm, MS_GC_FULL);

    s = msGcStats(vm);
    TEST_ASSERT(s.minorGcCount > 0 || s.majorGcCount > 0, "gc ran");

    msVmFree(vm);
    TEST_END();
}

void test_gc_stats_pause_time(void) {
    TEST_BEGIN("gc stats pause time");

    MsVM* vm = msVmNew();
    msExecString(vm,
        "for i in range(100) { x = [1, 2, 3, 4, 5] }",
        "test.ms");

    msGcCollect(vm, MS_GC_FULL);
    MsGcStats s = msGcStats(vm);
    TEST_ASSERT(s.lastPauseNs > 0, "lastPauseNs > 0");
    TEST_ASSERT(s.totalPauseNs >= s.lastPauseNs, "total >= last");

    msVmFree(vm);
    TEST_END();
}

void test_gc_threshold(void) {
    TEST_BEGIN("gc set threshold");

    MsVM* vm = msVmNew();
    msGcSetThreshold(vm, MS_GC_MAJOR, 3.0);
    msGcSetThreshold(vm, MS_GC_MINOR, 8.0);
    msGcSetThreshold(vm, MS_GC_FULL, 2.5);

    msGcSetThreshold(vm, MS_GC_MAJOR, 0.0);
    msGcSetThreshold(vm, MS_GC_MAJOR, -1.0);

    msVmFree(vm);
    TEST_END();
}

void test_gc_promotion_age(void) {
    TEST_BEGIN("gc set promotion age");

    MsVM* vm = msVmNew();
    msGcSetPromotionAge(vm, 1);
    msGcSetPromotionAge(vm, 3);
    msGcSetPromotionAge(vm, 0);
    msGcSetPromotionAge(vm, 10);

    msVmFree(vm);
    TEST_END();
}

void test_gc_threads(void) {
    TEST_BEGIN("gc set threads");

    MsVM* vm = msVmNew();
    msGcSetGcThreads(vm, 4);
    msGcSetGcThreads(vm, 8);
    msGcSetGcThreads(vm, 0);

    msVmFree(vm);
    TEST_END();
}

void test_write_barrier(void) {
    TEST_BEGIN("write barrier");

    MsVM* vm = msVmNew();
    msExecString(vm, "a = [1]\nb = [2]", "test.ms");
    MsValue* a = msGetGlobal(vm, "a");
    MsValue* b = msGetGlobal(vm, "b");

    msWriteBarrier(vm, a, b);

    msVmFree(vm);
    TEST_END();
}

static int finalizer_called = 0;
static void* finalizer_userdata = NULL;

static void test_finalizer_fn(MsVM* vm, MsValue* obj, void* userdata) {
    (void)vm;
    (void)obj;
    finalizer_called = 1;
    finalizer_userdata = userdata;
}

void test_finalizer(void) {
    TEST_BEGIN("finalizer");

    MsVM* vm = msVmNew();
    msExecString(vm, "obj = [1, 2, 3]", "test.ms");
    MsValue* obj = msGetGlobal(vm, "obj");
    TEST_ASSERT_NOT_NULL(obj, "obj non-NULL");

    int dummy = 42;
    MsStatus s = msOnFinalize(vm, obj, test_finalizer_fn, &dummy);
    TEST_ASSERT_EQ(MS_OK, s, "register finalizer");

    finalizer_called = 0;
    msDelGlobal(vm, "obj");
    msGcCollect(vm, MS_GC_FULL);
    msGcCollect(vm, MS_GC_FULL);

    TEST_ASSERT_EQ(1, finalizer_called, "finalizer called");
    TEST_ASSERT(&dummy == finalizer_userdata, "userdata correct");

    msVmFree(vm);
    TEST_END();
}

void test_gc_debug_mode(void) {
    TEST_BEGIN("gc debug mode");

    MsVM* vm = msVmNew();
    msGcSetDebug(vm, MS_TRUE);
    msGcCollect(vm, MS_GC_FULL);
    msGcSetDebug(vm, MS_FALSE);

    msVmFree(vm);
    TEST_END();
}

void test_gc_root_survives(void) {
    TEST_BEGIN("gc root survives collect");

    MsVM* vm = msVmNew();
    MsValue* s = msString(vm, "i must survive");
    msRoot(vm, s);

    msGcCollect(vm, MS_GC_FULL);
    msGcCollect(vm, MS_GC_FULL);

    const char* data = msToString(vm, s);
    TEST_ASSERT_STR_EQ("i must survive", data, "rooted value survives GC");

    msUnroot(vm, s);
    msVmFree(vm);
    TEST_END();
}

int main(void) {
    fprintf(stdout, "test_embed_gc:\n");
    test_gc_collect();
    test_gc_enable_disable();
    test_gc_stats();
    test_gc_stats_pause_time();
    test_gc_threshold();
    test_gc_promotion_age();
    test_gc_threads();
    test_write_barrier();
    test_finalizer();
    test_gc_debug_mode();
    test_gc_root_survives();
    TEST_SUMMARY();
    return TEST_RETURN();
}
```

### 9. test_extension.c — C 扩展模块

编译为动态库（`.dll` / `.so` / `.dylib`），供 mslang 脚本 `import`。

```c
#include <mslang.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static MsValue* fileio_read(MsVM* vm, MsValue* const* args, int nargs) {
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

static MsValue* fileio_write(MsVM* vm, MsValue* const* args, int nargs) {
    if (nargs < 2 || !msIsString(args[0]) || !msIsString(args[1])) {
        return msThrowTypeError(vm, "string, string", "other");
    }
    const char* path = msToString(vm, args[0]);
    const char* data = msToString(vm, args[1]);

    FILE* f = fopen(path, "wb");
    if (!f) {
        return msThrowIoError(vm, "cannot open for write: %s", path);
    }
    fputs(data, f);
    fclose(f);
    return msNil();
}

static MsValue* fileio_exists(MsVM* vm, MsValue* const* args, int nargs) {
    if (nargs < 1 || !msIsString(args[0])) {
        return msThrowTypeError(vm, "string", "other");
    }
    const char* path = msToString(vm, args[0]);
    FILE* f = fopen(path, "rb");
    if (f) {
        fclose(f);
        return msBoolVal(1);
    }
    return msBoolVal(0);
}

static MsValue* mathlib_add(MsVM* vm, MsValue* const* args, int nargs) {
    if (nargs < 2) {
        return msThrowValueError(vm, "need 2 args");
    }
    int64_t a = msToInt(vm, args[0]);
    int64_t b = msToInt(vm, args[1]);
    return msInt(a + b);
}

static MsValue* mathlib_mul(MsVM* vm, MsValue* const* args, int nargs) {
    if (nargs < 2) {
        return msThrowValueError(vm, "need 2 args");
    }
    int64_t a = msToInt(vm, args[0]);
    int64_t b = msToInt(vm, args[1]);
    return msInt(a * b);
}

static const MsFuncDef fileio_methods[] = {
    {"read",   fileio_read},
    {"write",  fileio_write},
    {"exists", fileio_exists},
    {NULL, NULL}
};

static const MsFuncDef mathlib_methods[] = {
    {"add", mathlib_add},
    {"mul", mathlib_mul},
    {NULL, NULL}
};

static const MsModuleDef fileio_def = {
    .name = "fileio",
    .methods = fileio_methods,
    .consts = NULL,
};

MS_MODULE_INIT const MsModuleDef* msModuleInit(MsVM* vm) {
    (void)vm;
    return &fileio_def;
}
```

### 10. test_import_extension.ms — 扩展模块测试脚本

```ms
import fileio

fn test_fileio() {
    fileio.write("/tmp/mslang_test.txt", "hello from mslang")
    content = fileio.read("/tmp/mslang_test.txt")
    assert content == "hello from mslang", "read/write roundtrip"

    exists = fileio.exists("/tmp/mslang_test.txt")
    assert exists == true, "file exists"

    not_exists = fileio.exists("/tmp/nonexistent_mslang_file.txt")
    assert not_exists == false, "nonexistent file"
}

test_fileio()
print("extension tests passed")
```

### 11. test_full_lifecycle.c — 完整生命周期测试

参照 [13-capi.md](../13-capi.md) § 完整嵌入示例 增强版，覆盖全部 C API 功能域。

```c
#include "common.h"

static char output_buf[8192];
static size_t output_len = 0;

static int lifecycle_write(const char* data, size_t len, void* userdata) {
    (void)userdata;
    if (output_len + len < sizeof(output_buf)) {
        memcpy(output_buf + output_len, data, len);
        output_len += len;
    }
    return 0;
}

static void lifecycle_reset_output(void) {
    memset(output_buf, 0, sizeof(output_buf));
    output_len = 0;
}

void test_fibonacci_embed(void) {
    TEST_BEGIN("fibonacci embedding (13-capi.md example)");

    MsVM* vm = msVmNew();

    const char* script =
        "fn fibonacci(n) {\n"
        "  if n <= 1 { return n }\n"
        "  return fibonacci(n - 1) + fibonacci(n - 2)\n"
        "}\n";

    MsStatus s = msExecString(vm, script, "fib.ms");
    TEST_ASSERT_EQ(MS_OK, s, "exec fibonacci script");

    if (s != MS_OK) {
        MsValue* err = msErrFetch(vm);
        fprintf(stderr, "  error: %s\n", msErrMessage(vm, err));
        msVmFree(vm);
        TEST_END();
        return;
    }

    MsValue* fib = msGetGlobal(vm, "fibonacci");
    TEST_ASSERT_NOT_NULL(fib, "get fibonacci fn");
    msRoot(vm, fib);

    MsValue* arg = msInt(10);
    msRoot(vm, arg);
    MsValue* result = msCall1(vm, fib, arg);

    TEST_ASSERT(!msErrOccurred(vm), "no error after call");
    TEST_ASSERT_NOT_NULL(result, "result non-NULL");
    TEST_ASSERT_EQ(55, msToInt(vm, result), "fibonacci(10) == 55");

    msUnroot(vm, result);
    msUnroot(vm, arg);
    msUnroot(vm, fib);
    msVmFree(vm);
    TEST_END();
}

void test_value_operations(void) {
    TEST_BEGIN("full value operations");

    MsVM* vm = msVmNew();

    MsValue* i = msInt(42);
    TEST_ASSERT_EQ(MS_TYPE_INT, msTypeof(i), "int type");
    TEST_ASSERT_EQ(42, msToInt(vm, i), "int val");
    TEST_ASSERT(msIsInt(i), "isInt");
    TEST_ASSERT(msIsNumber(i), "isNumber");

    MsValue* f = msFloat(2.718);
    TEST_ASSERT(msIsFloat(f), "isFloat");
    TEST_ASSERT(msIsNumber(f), "isNumber");

    MsValue* s = msString(vm, "hello");
    TEST_ASSERT(msIsString(s), "isString");
    TEST_ASSERT_STR_EQ("hello", msToString(vm, s), "string val");

    MsValue* n = msNil();
    TEST_ASSERT(msIsNil(n), "isNil");

    MsValue* b = msBoolVal(1);
    TEST_ASSERT(msIsBool(b), "isBool");

    TEST_ASSERT_EQ(MS_TRUE, msEq(vm, i, msInt(42)), "eq");
    TEST_ASSERT_EQ(MS_TRUE, msLt(vm, msInt(1), msInt(2)), "lt");

    MsValue* concat = msStringConcat(vm, msString(vm, "a"), msString(vm, "b"));
    TEST_ASSERT_STR_EQ("ab", msToString(vm, concat), "concat");

    msVmFree(vm);
    TEST_END();
}

void test_collection_workflow(void) {
    TEST_BEGIN("full collection workflow");

    MsVM* vm = msVmNew();

    MsValue* list = msListNew(vm);
    msListPush(vm, list, msInt(10));
    msListPush(vm, list, msInt(20));
    msListPush(vm, list, msInt(30));
    TEST_ASSERT_EQ(3, msListLen(vm, list), "list len");
    TEST_ASSERT_EQ(20, msToInt(vm, msListGet(vm, list, 1)), "list[1]");

    MsValue* dict = msDictNew(vm);
    msDictSet(vm, dict, msString(vm, "key"), msInt(99));
    TEST_ASSERT_EQ(1, msDictLen(vm, dict), "dict len");
    MsValue* got = msDictGet(vm, dict, msString(vm, "key"));
    TEST_ASSERT_EQ(99, msToInt(vm, got), "dict[key]");

    MsValue* items[] = { msInt(1), msInt(2) };
    MsValue* tup = msTupleFrom(vm, items, 2);
    TEST_ASSERT_EQ(2, msTupleLen(vm, tup), "tuple len");

    MsValue* set = msSetNew(vm);
    msSetAdd(vm, set, msInt(1));
    msSetAdd(vm, set, msInt(2));
    msSetAdd(vm, set, msInt(1));
    TEST_ASSERT_EQ(2, msSetLen(vm, set), "set dedup");

    msVmFree(vm);
    TEST_END();
}

void test_error_handling_workflow(void) {
    TEST_BEGIN("full error handling workflow");

    MsVM* vm = msVmNew();

    MsStatus se = msExecString(vm, "fn (", "bad.ms");
    TEST_ASSERT_EQ(MS_ERROR, se, "syntax error");

    msErrClear(vm);
    TEST_ASSERT_EQ(MS_FALSE, msErrOccurred(vm), "cleared");

    msThrowValueError(vm, "test error");
    TEST_ASSERT_EQ(MS_TRUE, msErrOccurred(vm), "error set");

    MsValue* err = msErrFetch(vm);
    TEST_ASSERT_NOT_NULL(err, "err non-NULL");
    TEST_ASSERT_STR_EQ("ValueError", msErrTypeName(vm, err), "type");

    msExecString(vm, "fn safe() { return 42 }", "test.ms");
    MsValue* fn = msGetGlobal(vm, "safe");
    msRoot(vm, fn);

    MsValue* try_result = NULL;
    MsStatus ts = msTry(vm, fn, NULL, 0, &try_result);
    TEST_ASSERT_EQ(MS_OK, ts, "try success");
    TEST_ASSERT_EQ(42, msToInt(vm, try_result), "try result");

    msUnroot(vm, fn);
    msVmFree(vm);
    TEST_END();
}

void test_gc_interaction_workflow(void) {
    TEST_BEGIN("full GC workflow");

    MsVM* vm = msVmNew();

    MsValue* rooted = msString(vm, "survivor");
    msRoot(vm, rooted);

    msExecString(vm,
        "for i in range(200) {\n"
        "  x = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]\n"
        "}\n",
        "test.ms");

    msGcCollect(vm, MS_GC_FULL);

    TEST_ASSERT_STR_EQ("survivor", msToString(vm, rooted), "rooted survived GC");

    MsGcStats stats = msGcStats(vm);
    TEST_ASSERT(stats.totalPauseNs > 0, "gc pause recorded");

    msGcEnable(vm, MS_FALSE);
    TEST_ASSERT_EQ(MS_FALSE, msGcIsEnabled(vm), "gc disabled");
    msGcEnable(vm, MS_TRUE);
    TEST_ASSERT_EQ(MS_TRUE, msGcIsEnabled(vm), "gc re-enabled");

    msUnroot(vm, rooted);
    msVmFree(vm);
    TEST_END();
}

void test_class_interaction_workflow(void) {
    TEST_BEGIN("full class workflow");

    MsVM* vm = msVmNew();
    msExecString(vm,
        "class Shape {\n"
        "  fn __init__(self, name) {\n"
        "    self.name = name\n"
        "  }\n"
        "  fn describe(self) {\n"
        "    return \"I am a \" + self.name\n"
        "  }\n"
        "}\n",
        "test.ms");

    MsValue* shape_cls = msGetClass(vm, "Shape");
    TEST_ASSERT_NOT_NULL(shape_cls, "Shape class");

    MsValue* args[] = { msString(vm, "circle") };
    MsValue* inst = msInstanceNew(vm, shape_cls, args, 1);
    TEST_ASSERT(msIsInstance(inst), "is instance");

    MsValue* name_attr = msInstanceGet(vm, inst, "name");
    TEST_ASSERT_STR_EQ("circle", msToString(vm, name_attr), "name attr");

    msInstanceSet(vm, inst, "radius", msInt(5));
    MsValue* radius = msInstanceGet(vm, inst, "radius");
    TEST_ASSERT_EQ(5, msToInt(vm, radius), "radius attr");

    MsValue* desc_method = msInstanceGet(vm, inst, "describe");
    msRoot(vm, desc_method);
    msRoot(vm, inst);
    MsValue* desc = msCall1(vm, desc_method, inst);
    TEST_ASSERT_STR_EQ("I am a circle", msToString(vm, desc), "describe result");

    msUnroot(vm, desc_method);
    msUnroot(vm, inst);
    msVmFree(vm);
    TEST_END();
}

void test_module_registration(void) {
    TEST_BEGIN("module static registration");

    MsVM* vm = msVmNew();

    MsValue* mod = msModuleNew(vm, "mymod");
    TEST_ASSERT_NOT_NULL(mod, "module created");

    msModuleAddConst(vm, mod, "VERSION", msString(vm, "1.0"));

    msRegisterModuleValue(vm, mod);

    msExecString(vm,
        "import mymod\n"
        "v = mymod.VERSION\n",
        "test.ms");

    MsValue* v = msGetGlobal(vm, "v");
    TEST_ASSERT_NOT_NULL(v, "module const accessible");
    TEST_ASSERT_STR_EQ("1.0", msToString(vm, v), "module const value");

    msVmFree(vm);
    TEST_END();
}

void test_output_capture_full(void) {
    TEST_BEGIN("full output capture");

    MsVM* vm = msVmNew();
    msSetStdout(vm, lifecycle_write, NULL);
    lifecycle_reset_output();

    msExecString(vm, "print(\"line1\")\nprint(\"line2\")", "test.ms");
    TEST_ASSERT(strstr(output_buf, "line1") != NULL, "captured line1");
    TEST_ASSERT(strstr(output_buf, "line2") != NULL, "captured line2");

    msVmFree(vm);
    TEST_END();
}

void test_script_calls_c_function(void) {
    TEST_BEGIN("script calls registered C function");

    MsVM* vm = msVmNew();

    MsValue* mod = msModuleNew(vm, "calc");

    msModuleAddConst(vm, mod, "PI", msFloat(3.14159));
    msRegisterModuleValue(vm, mod);

    msExecString(vm,
        "import calc\n"
        "pi_val = calc.PI\n",
        "test.ms");

    MsValue* pi_val = msGetGlobal(vm, "pi_val");
    TEST_ASSERT_NOT_NULL(pi_val, "pi_val from C module");
    TEST_ASSERT(msIsFloat(pi_val), "PI is float");

    msVmFree(vm);
    TEST_END();
}

void test_thread_safety_two_vms(void) {
    TEST_BEGIN("two VMs concurrent (single-threaded smoke test)");

    MsVM* vm1 = msVmNew();
    MsVM* vm2 = msVmNew();

    msExecString(vm1, "x = 100", "test.ms");
    msExecString(vm2, "x = 200", "test.ms");

    MsValue* x1 = msGetGlobal(vm1, "x");
    MsValue* x2 = msGetGlobal(vm2, "x");

    TEST_ASSERT_EQ(100, msToInt(vm1, x1), "vm1.x == 100");
    TEST_ASSERT_EQ(200, msToInt(vm2, x2), "vm2.x == 200");

    msVmFree(vm1);
    msVmFree(vm2);
    TEST_END();
}

int main(void) {
    fprintf(stdout, "test_full_lifecycle:\n");
    test_fibonacci_embed();
    test_value_operations();
    test_collection_workflow();
    test_error_handling_workflow();
    test_gc_interaction_workflow();
    test_class_interaction_workflow();
    test_module_registration();
    test_output_capture_full();
    test_script_calls_c_function();
    test_thread_safety_two_vms();
    TEST_SUMMARY();
    return TEST_RETURN();
}
```

### 12. CMakeLists.txt

```cmake
cmake_minimum_required(VERSION 3.14)
project(mslang_capi_tests C)

set(CMAKE_C_STANDARD 11)
set(CMAKE_C_STANDARD_REQUIRED ON)

if(NOT MSLANG_INCLUDE_DIR)
    set(MSLANG_INCLUDE_DIR "${CMAKE_CURRENT_SOURCE_DIR}/../../include")
endif()

if(NOT MSLANG_LIB_DIR)
    if(WIN32)
        set(MSLANG_LIB_DIR "${CMAKE_CURRENT_SOURCE_DIR}/../../target/debug")
    else()
        set(MSLANG_LIB_DIR "${CMAKE_CURRENT_SOURCE_DIR}/../../target/debug")
    endif()
endif()

add_library(mslang_capi SHARED IMPORTED)
if(WIN32)
    set_target_properties(mslang_capi PROPERTIES
        IMPORTED_LOCATION "${MSLANG_LIB_DIR}/mslang.dll"
        IMPORTED_IMPLIB "${MSLANG_LIB_DIR}/mslang.dll.lib"
    )
else()
    set_target_properties(mslang_capi PROPERTIES
        IMPORTED_LOCATION "${MSLANG_LIB_DIR}/libmslang${CMAKE_SHARED_LIBRARY_SUFFIX}"
    )
endif()

set(TEST_SOURCES
    test_embed_basic
    test_embed_values
    test_embed_collections
    test_embed_call
    test_embed_error
    test_embed_class
    test_embed_gc
    test_full_lifecycle
)

foreach(test_name ${TEST_SOURCES})
    add_executable(${test_name} ${test_name}.c)
    target_include_directories(${test_name} PRIVATE ${MSLANG_INCLUDE_DIR} ${CMAKE_CURRENT_SOURCE_DIR})
    target_link_libraries(${test_name} mslang_capi)
    if(UNIX)
        target_link_libraries(${test_name} m dl pthread)
    endif()
    add_test(NAME ${test_name} COMMAND ${test_name})
endforeach()

add_library(test_extension SHARED test_extension.c)
target_include_directories(test_extension PRIVATE ${MSLANG_INCLUDE_DIR})
target_link_libraries(test_extension mslang_capi)
set_target_properties(test_extension PROPERTIES
    OUTPUT_NAME "fileio"
    PREFIX ""
    SUFFIX "${CMAKE_SHARED_MODULE_SUFFIX}"
)

enable_testing()
```

### 13. CI 集成

在 `.github/workflows/ci.yml` 中追加步骤（或创建 `.github/workflows/capi-test.yml`）：

```yaml
name: C API Integration Tests

on: [push, pull_request]

jobs:
  capi-test:
    strategy:
      matrix:
        os: [ubuntu-latest, macos-latest, windows-latest]
    runs-on: ${{ matrix.os }}

    steps:
      - uses: actions/checkout@v4

      - name: Install Rust
        uses: dtolnay/rust-toolchain@stable

      - name: Build mslang cdylib
        run: cargo build --features capi

      - name: Install CMake
        uses: jwlawson/actions-setup-cmake@v2

      - name: Configure CMake
        run: |
          cd tests/capi
          cmake -B build \
            -DMSLANG_INCLUDE_DIR=${{ github.workspace }}/include \
            -DMSLANG_LIB_DIR=${{ github.workspace }}/target/debug

      - name: Build C tests
        run: |
          cd tests/capi
          cmake --build build

      - name: Run C tests
        run: |
          cd tests/capi/build
          ctest --output-on-failure

      - name: Build extension module
        run: |
          cd tests/capi/build
          cmake --build . --target test_extension

      - name: Run extension test
        run: |
          cargo run --features capi -- test_import_extension.ms
```

## 验证标准

1. **编译无警告**：全部 C 测试程序在 Linux/macOS/Windows 三平台使用 `-Wall -Wextra` 编译无错误无警告
2. **全部测试通过**：所有 `test_embed_*` 和 `test_full_lifecycle` 程序在三个平台上全部断言通过
3. **嵌入示例验证**：`test_fibonacci_embed` 输出 `fibonacci(10) = 55`，与 13-capi.md § 完整嵌入示例 一致
4. **扩展模块验证**：`test_extension.c` 编译为 `fileio.dll`/`fileio.so`，`test_import_extension.ms` 执行成功
5. **内存安全**：ASan/MSan 运行无泄漏、无越界（Linux）
6. **线程安全**：`test_thread_safety_two_vms` 两个 VM 实例在独立线程中并发使用（可选扩展为多线程测试）
7. **CI 集成**：GitHub Actions 三平台 C API 测试步骤通过

## 测试用例

### 测试矩阵

| 测试程序 | 覆盖 API 域 | 断言数 | 来源任务 |
|---|---|---|---|
| test_embed_basic | VM 生命周期、执行、全局变量、输出重定向 | 19 | 65, 66 |
| test_embed_values | 值创建、类型判断、转换、比较、字符串操作 | 55+ | 67, 68 |
| test_embed_collections | List/Dict/Tuple/Set/迭代器/通用操作 | 60+ | 69 |
| test_embed_call | 函数调用（0-3 参数）、递归、闭包、异常 | 30+ | 70 |
| test_embed_error | 异常抛出/捕获/查询、try 模式 | 40+ | 71 |
| test_embed_class | Class 获取/实例化/属性/继承/C 侧定义 | 35+ | 73 |
| test_embed_gc | GC 收集/控制/统计/Finalizer/写屏障 | 25+ | 74 |
| test_extension | C 扩展模块动态加载端到端 | 3（脚本侧） | 72 |
| test_full_lifecycle | 全部 API 综合场景 | 60+ | 65-74 |

### 各测试预期结果

1. **test_embed_basic**：全部断言通过，`tests_failed == 0`
2. **test_embed_values**：全部断言通过，`tests_failed == 0`
3. **test_embed_collections**：全部断言通过，`tests_failed == 0`
4. **test_embed_call**：全部断言通过，`tests_failed == 0`
5. **test_embed_error**：全部断言通过，`tests_failed == 0`
6. **test_embed_class**：全部断言通过，`tests_failed == 0`
7. **test_embed_gc**：全部断言通过，`tests_failed == 0`
8. **test_extension**：mslang 执行 `test_import_extension.ms` 输出 `extension tests passed`
9. **test_full_lifecycle**：全部断言通过，`tests_failed == 0`，输出 `test_full_lifecycle: ... ok`

### 构建验证命令

```bash
# 1. 构建 mslang cdylib
cargo build --features capi

# 2. 编译 C 测试
cd tests/capi
cmake -B build -DMSLANG_INCLUDE_DIR=../../include -DMSLANG_LIB_DIR=../../target/debug
cmake --build build

# 3. 运行全部 C 测试
cd build
ctest --output-on-failure

# 4. 运行单独测试
./test_embed_basic
./test_embed_values
./test_embed_collections
./test_embed_call
./test_embed_error
./test_embed_class
./test_embed_gc
./test_full_lifecycle

# 5. 扩展模块测试
cmake --build . --target test_extension
cargo run --features capi -- ../test_import_extension.ms

# 6. 内存检查（Linux）
valgrind --leak-check=full ./test_full_lifecycle
```
