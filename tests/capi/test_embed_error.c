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

    msUnroot(vm, err);
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

    msUnroot(vm, err);
    msVmFree(vm);
    TEST_END();
}

void test_throw_index_error(void) {
    TEST_BEGIN("throw index error");

    MsVM* vm = msVmNew();
    msThrowIndexError(vm, "out of bounds");
    MsValue* err = msErrFetch(vm);
    TEST_ASSERT_STR_EQ("IndexError", msErrTypeName(vm, err), "type");

    msUnroot(vm, err);
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

    msUnroot(vm, err);
    msVmFree(vm);
    TEST_END();
}

void test_throw_io_error(void) {
    TEST_BEGIN("throw io error");

    MsVM* vm = msVmNew();
    msThrowIoError(vm, "file not found");
    MsValue* err = msErrFetch(vm);
    TEST_ASSERT_STR_EQ("IOError", msErrTypeName(vm, err), "type");

    msUnroot(vm, err);
    msVmFree(vm);
    TEST_END();
}

void test_throw_runtime_error(void) {
    TEST_BEGIN("throw runtime error");

    MsVM* vm = msVmNew();
    msThrowRuntimeError(vm, "unexpected");
    MsValue* err = msErrFetch(vm);
    TEST_ASSERT_STR_EQ("RuntimeError", msErrTypeName(vm, err), "type");

    msUnroot(vm, err);
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

    msUnroot(vm, rethrown);
    msUnroot(vm, original);
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

    msUnroot(vm, err);
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

    msUnroot(vm, err);
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

    MsStatus s = msThrowValueError(vm, "from callback");
    TEST_ASSERT_EQ(MS_ERROR, s, "throw returns MS_ERROR");
    TEST_ASSERT_EQ(MS_TRUE, msErrOccurred(vm), "error set");
    msErrClear(vm);

    msVmFree(vm);
    TEST_END();
}

void test_throw_generic(void) {
    TEST_BEGIN("throw generic");

    MsVM* vm = msVmNew();
    /* msThrow takes (vm, type, msg) — no varargs, pre-format */
    MsStatus s = msThrow(vm, "CustomError", "custom message 42");
    TEST_ASSERT_EQ(MS_ERROR, s, "throw returns MS_ERROR");

    MsValue* err = msErrFetch(vm);
    TEST_ASSERT_STR_EQ("CustomError", msErrTypeName(vm, err), "type name");

    msUnroot(vm, err);
    msVmFree(vm);
    TEST_END();
}

void test_throw_rethrow(void) {
    TEST_BEGIN("throw rethrow");

    MsVM* vm = msVmNew();
    msThrowValueError(vm, "first");
    MsValue* first = msErrFetch(vm);

    msThrowValue(vm, first);
    MsStatus s = msThrowRethrow(vm);
    TEST_ASSERT_EQ(MS_ERROR, s, "rethrow returns MS_ERROR");
    TEST_ASSERT_EQ(MS_TRUE, msErrOccurred(vm), "error present after rethrow");

    MsValue* err = msErrFetch(vm);
    msUnroot(vm, err);
    msUnroot(vm, first);
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
    test_throw_generic();
    test_throw_rethrow();
    TEST_SUMMARY();
    return TEST_RETURN();
}
