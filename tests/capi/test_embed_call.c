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
