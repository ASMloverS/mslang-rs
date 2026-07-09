#include <mslang/mslang.h>
#include <assert.h>
#include <string.h>

static char captured_buf[4096];
static size_t captured_len = 0;

static int write_capture(const char* data, size_t len, void* userdata) {
    (void)userdata;
    memcpy(captured_buf + captured_len, data, len);
    captured_len += len;
    return 0;
}

void test_vm_new_free(void) {
    MsVM* vm = msVmNew();
    assert(vm != NULL);
    msVmFree(vm);
}

void test_exec_string(void) {
    MsVM* vm = msVmNew();
    MsStatus s = msExecString(vm, "x = 42", "test.ms");
    assert(s == MS_OK);
    msVmFree(vm);
}

void test_exec_string_error(void) {
    MsVM* vm = msVmNew();
    MsStatus s = msExecString(vm, "fn (", "bad.ms");
    assert(s == MS_ERROR);
    msVmFree(vm);
}

void test_output_redirect(void) {
    captured_len = 0;
    MsVM* vm = msVmNew();
    msSetStdout(vm, write_capture, NULL);
    MsStatus s = msExecString(vm, "print(\"hello\")", "test.ms");
    assert(s == MS_OK);
    assert(strstr(captured_buf, "hello") != NULL);
    msVmFree(vm);
}

void test_global_roundtrip(void) {
    MsVM* vm = msVmNew();
    msExecString(vm, "answer = 42", "test.ms");
    MsValue* val = msGetGlobal(vm, "answer");
    assert(val != NULL);
    msValueFree(val);
    msVmFree(vm);
}

void test_two_vms_independent(void) {
    MsVM* vm1 = msVmNew();
    MsVM* vm2 = msVmNew();
    msExecString(vm1, "x = 1", "test.ms");
    MsValue* val1 = msGetGlobal(vm1, "x");
    MsValue* val2 = msGetGlobal(vm2, "x");
    assert(val1 != NULL);
    assert(val2 == NULL);
    msValueFree(val1);
    msVmFree(vm1);
    msVmFree(vm2);
}

void test_eval(void) {
    MsVM* vm = msVmNew();
    MsValue* val = msEval(vm, "1 + 2");
    assert(val != NULL);
    msValueFree(val);
    msVmFree(vm);
}

void test_vm_lock_unlock(void) {
    MsVM* vm = msVmNew();
    msVmLock(vm);
    msExecString(vm, "x = 1", "test.ms");
    MsValue* val = msGetGlobal(vm, "x");
    assert(val != NULL);
    msValueFree(val);
    msVmUnlock(vm);
    msVmFree(vm);
}

int main(void) {
    test_vm_new_free();
    test_exec_string();
    test_exec_string_error();
    test_output_redirect();
    test_global_roundtrip();
    test_two_vms_independent();
    test_eval();
    test_vm_lock_unlock();
    return 0;
}
