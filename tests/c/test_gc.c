#include <mslang/mslang.h>
#include <assert.h>
#include <stdio.h>
#include <string.h>

static int finalizer_called = 0;
static void* finalizer_userdata = NULL;

static void my_finalizer(MsVM* vm, MsValue* obj, void* userdata) {
    (void)vm;
    (void)obj;
    finalizer_called = 1;
    finalizer_userdata = userdata;
}

static char captured_buf[4096];
static size_t captured_len = 0;

static int write_capture(const char* data, size_t len, void* userdata) {
    (void)userdata;
    if (captured_len + len > sizeof(captured_buf)) {
        len = sizeof(captured_buf) - captured_len;
    }
    if (len == 0) return 0;
    memcpy(captured_buf + captured_len, data, len);
    captured_len += len;
    return 0;
}

void test_gc_collect(void) {
    MsVM* vm = msVmNew();
    msGcCollect(vm, MS_GC_MINOR);
    msGcCollect(vm, MS_GC_MAJOR);
    msGcCollect(vm, MS_GC_FULL);
    msVmFree(vm);
}

void test_gc_enable_disable(void) {
    MsVM* vm = msVmNew();

    assert(msGcIsEnabled(vm) == MS_TRUE);

    msGcEnable(vm, MS_FALSE);
    assert(msGcIsEnabled(vm) == MS_FALSE);

    msGcEnable(vm, MS_TRUE);
    assert(msGcIsEnabled(vm) == MS_TRUE);

    msVmFree(vm);
}

void test_gc_stats(void) {
    MsVM* vm = msVmNew();

    MsGcStats s = msGcStats(vm);
    assert(s.minorGcCount == 0);
    assert(s.majorGcCount == 0);

    msGcCollect(vm, MS_GC_FULL);

    s = msGcStats(vm);
    assert(s.minorGcCount > 0 || s.majorGcCount > 0);

    msVmFree(vm);
}

void test_gc_stats_pause(void) {
    MsVM* vm = msVmNew();

    msSetStdout(vm, write_capture, NULL);
    msExecString(vm,
        (const int8_t*)"for i in range(100) { x = [1,2,3,4,5] }",
        (const int8_t*)"test.ms");

    msGcCollect(vm, MS_GC_FULL);

    MsGcStats s = msGcStats(vm);
    assert(s.lastPauseNs > 0);
    assert(s.totalPauseNs >= s.lastPauseNs);

    msVmFree(vm);
}

void test_finalizer(void) {
    finalizer_called = 0;
    finalizer_userdata = NULL;

    MsVM* vm = msVmNew();

    msExecString(vm, (const int8_t*)"obj = [1, 2, 3]", (const int8_t*)"test.ms");
    MsValue* obj = msGetGlobal(vm, (const int8_t*)"obj");
    assert(obj != NULL);

    int dummy_data = 42;
    MsStatus s = msOnFinalize(vm, obj, my_finalizer, &dummy_data);
    assert(s == MS_OK);

    msDelGlobal(vm, (const int8_t*)"obj");
    msGcCollect(vm, MS_GC_FULL);

    assert(finalizer_called == 1);
    assert(finalizer_userdata == &dummy_data);

    msVmFree(vm);
}

void test_write_barrier(void) {
    MsVM* vm = msVmNew();

    msExecString(vm, (const int8_t*)"a = [1]", (const int8_t*)"test.ms");
    msExecString(vm, (const int8_t*)"b = [2]", (const int8_t*)"test.ms");

    MsValue* a = msGetGlobal(vm, (const int8_t*)"a");
    MsValue* b = msGetGlobal(vm, (const int8_t*)"b");

    msWriteBarrier(vm, a, b);

    msVmFree(vm);
}

void test_gc_threshold(void) {
    MsVM* vm = msVmNew();

    msGcSetThreshold(vm, MS_GC_MAJOR, 3.0);
    msGcSetThreshold(vm, MS_GC_MINOR, 8.0);
    msGcSetThreshold(vm, MS_GC_FULL, 2.5);

    msVmFree(vm);
}

void test_gc_promotion_age(void) {
    MsVM* vm = msVmNew();

    msGcSetPromotionAge(vm, 1);
    msGcSetPromotionAge(vm, 3);

    msVmFree(vm);
}

void test_gc_threads(void) {
    MsVM* vm = msVmNew();

    msGcSetGcThreads(vm, 4);
    msGcSetGcThreads(vm, 8);

    msVmFree(vm);
}

void test_gc_debug(void) {
    MsVM* vm = msVmNew();

    msGcSetDebug(vm, MS_TRUE);
    msGcCollect(vm, MS_GC_FULL);
    msGcSetDebug(vm, MS_FALSE);

    msVmFree(vm);
}

void test_null_vm(void) {
    assert(msGcIsEnabled(NULL) == 0);

    msGcCollect(NULL, MS_GC_FULL);
    msGcEnable(NULL, 1);
    msGcSetThreshold(NULL, MS_GC_MAJOR, 2.0);
    msGcSetPromotionAge(NULL, 2);
    msGcSetGcThreads(NULL, 4);
    msGcSetDebug(NULL, 1);
    msWriteBarrier(NULL, NULL, NULL);

    MsGcStats s = msGcStats(NULL);
    assert(s.minorGcCount == 0);
}

void test_multi_vm_isolation(void) {
    MsVM* vm1 = msVmNew();
    MsVM* vm2 = msVmNew();

    msGcEnable(vm1, MS_FALSE);
    assert(msGcIsEnabled(vm1) == MS_FALSE);
    assert(msGcIsEnabled(vm2) == MS_TRUE);

    msGcCollect(vm1, MS_GC_MINOR);
    msGcCollect(vm2, MS_GC_MAJOR);

    MsGcStats s1 = msGcStats(vm1);
    MsGcStats s2 = msGcStats(vm2);
    assert(s1.minorGcCount > 0);
    assert(s2.majorGcCount > 0);
    assert(s1.majorGcCount == 0);
    assert(s2.minorGcCount == 0);

    msVmFree(vm1);
    msVmFree(vm2);
}

int main(void) {
    test_gc_collect();
    test_gc_enable_disable();
    test_gc_stats();
    test_gc_stats_pause();
    test_finalizer();
    test_write_barrier();
    test_gc_threshold();
    test_gc_promotion_age();
    test_gc_threads();
    test_gc_debug();
    test_null_vm();
    test_multi_vm_isolation();

    printf("all gc tests passed\n");
    return 0;
}
