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

void test_set_args(void) {
    TEST_BEGIN("set args");

    MsVM* vm = msVmNew();
    const char* argv[] = { "mslang", "arg1", "arg2" };
    msSetArgs(vm, 3, argv);
    msVmFree(vm);

    TEST_END();
}

void test_set_stderr(void) {
    TEST_BEGIN("stderr redirect");

    MsVM* vm = msVmNew();
    msSetStderr(vm, write_capture, NULL);
    captured_reset();
    msExecString(vm, "printerr(\"warn\")", "test.ms");
    msVmFree(vm);

    TEST_END();
}

void test_vm_lock_unlock(void) {
    TEST_BEGIN("vm lock/unlock");

    MsVM* vm = msVmNew();
    msVmLock(vm);
    msExecString(vm, "x = 1", "test.ms");
    msVmUnlock(vm);
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
    test_set_args();
    test_set_stderr();
    test_vm_lock_unlock();
    TEST_SUMMARY();
    return TEST_RETURN();
}
