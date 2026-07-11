#ifndef TEST_COMMON_H
#define TEST_COMMON_H

#include <mslang.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* msStringFmt is implemented in C (vsnprintf_shim.c) but not declared in
 * the cbindgen-generated headers. Forward-declare it here. */
extern MsValue* msStringFmt(MsVM* vm, const char* fmt, ...);

/* cbindgen generates `struct MsFuncDef` etc. without typedefs. In C (not C++),
 * these require the `struct` keyword. Provide typedefs for convenience. */
typedef struct MsFuncDef MsFuncDef;
typedef struct MsConstDef MsConstDef;
typedef struct MsModuleDef MsModuleDef;

/* Function prototypes — cbindgen guards these behind MS_CAPI_ENABLED which
 * we cannot define without causing struct redefinition errors. Supply them
 * here instead. */
#include "capi_decls.h"

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
    if (_e != _a) {                                             \
        fprintf(stderr, "  FAIL: %s (expected %ld, got %ld) "   \
                "at %s:%d\n", msg, _e, _a, __FILE__, __LINE__); \
        tests_failed++;                                         \
    } else {                                                     \
        tests_passed++;                                         \
    }                                                            \
} while (0)

#define TEST_ASSERT_NOT_NULL(ptr, msg) do {                     \
    if ((ptr) == NULL) {                                         \
        fprintf(stderr, "  FAIL: %s (got NULL) at %s:%d\n",     \
                msg, __FILE__, __LINE__);                        \
        tests_failed++;                                         \
    } else {                                                     \
        tests_passed++;                                         \
    }                                                            \
} while (0)

#define TEST_ASSERT_NULL(ptr, msg) do {                          \
    if ((ptr) != NULL) {                                         \
        fprintf(stderr, "  FAIL: %s (expected NULL) at %s:%d\n", \
                msg, __FILE__, __LINE__);                        \
        tests_failed++;                                          \
    } else {                                                     \
        tests_passed++;                                         \
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
