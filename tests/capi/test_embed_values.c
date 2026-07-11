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
    /* Note: cannot free(copy) — msToStringCopy uses Rust's allocator,
     * not the CRT malloc. Small leak in test is acceptable. */

    msVmFree(vm);
    TEST_END();
}

void test_is_identity(void) {
    /* msIs compares identity for heap-allocated Ref types only.
     * For inline values (Nil/Bool/Int/Float), msIs returns MS_FALSE
     * because the signature has no vm param to set TypeError. */
    TEST_BEGIN("is operator");

    MsVM* vm = msVmNew();

    MsValue* n1 = msNil();
    MsValue* n2 = msNil();
    TEST_ASSERT_EQ(MS_FALSE, msIs(n1, n2), "nil is nil -> false (inline)");

    MsValue* a = msInt(42);
    MsValue* b = msInt(42);
    TEST_ASSERT_EQ(MS_FALSE, msIs(a, b), "same int is -> false (inline)");

    /* Heap-allocated strings should have different identity */
    MsValue* s1 = msString(vm, "abc");
    MsValue* s2 = msString(vm, "abc");
    TEST_ASSERT_EQ(MS_FALSE, msIs(s1, s2), "distinct strings is -> false");

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

void test_tofloat(void) {
    TEST_BEGIN("toFloat");

    MsVM* vm = msVmNew();
    MsValue* f = msFloat(3.14);
    TEST_ASSERT_EQ(3.14, msToFloat(vm, f), "float toFloat");

    MsValue* i = msInt(42);
    TEST_ASSERT_EQ(42.0, msToFloat(vm, i), "int toFloat");

    msVmFree(vm);
    TEST_END();
}

void test_convert_all(void) {
    TEST_BEGIN("convert float/bool/list");

    MsVM* vm = msVmNew();

    MsValue* i = msInt(1);
    MsValue* f = msConvertFloat(vm, i);
    TEST_ASSERT_NOT_NULL(f, "int to float");
    TEST_ASSERT(msIsFloat(f), "result is float");

    MsValue* b = msConvertBool(i);
    TEST_ASSERT_NOT_NULL(b, "int to bool");

    MsValue* s = msString(vm, "abc");
    MsValue* lst = msConvertList(vm, s);
    TEST_ASSERT_NOT_NULL(lst, "string to list");
    TEST_ASSERT(msIsList(lst), "result is list");

    msVmFree(vm);
    TEST_END();
}

void test_string_data(void) {
    TEST_BEGIN("stringData");

    MsVM* vm = msVmNew();
    MsValue* s = msString(vm, "hello");
    const char* data = msStringData(vm, s);
    TEST_ASSERT_NOT_NULL(data, "data non-NULL");
    TEST_ASSERT_STR_EQ("hello", data, "data content");

    msVmFree(vm);
    TEST_END();
}

void test_attr_access(void) {
    /* msGetAttr/msSetAttr are placeholder stubs (task 73 not done).
     * Verify they fail gracefully without crashing. */
    TEST_BEGIN("getAttr/setAttr (stub)");

    MsVM* vm = msVmNew();
    msExecString(vm,
        "class Box {\n"
        "  fn __init__(self) { self.val = 99 }\n"
        "}\n",
        "test.ms");

    MsValue* cls = msGetClass(vm, "Box");
    MsValue* inst = msInstanceNew(vm, cls, NULL, 0);

    MsValue* got = msGetAttr(vm, inst, "val");
    /* Expected: NULL — instance attribute access not yet implemented (task 73) */
    TEST_ASSERT_NULL(got, "msGetAttr returns NULL (stub)");

    MsValue* nv = msInt(77);
    TEST_ASSERT_EQ(MS_ERROR, msSetAttr(vm, inst, "val2", nv), "msSetAttr returns MS_ERROR (stub)");

    msVmFree(vm);
    TEST_END();
}

int main(void) {
    setvbuf(stdout, NULL, _IONBF, 0);
    setvbuf(stderr, NULL, _IONBF, 0);
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
    test_tofloat();
    test_convert_all();
    test_string_data();
    test_attr_access();
    TEST_SUMMARY();
    return TEST_RETURN();
}
