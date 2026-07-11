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
    /* Note: relies on MS_GC_FULL synchronously collecting unreachable objects.
       If the GC is incremental, increase the collect count or loop until
       finalizer_called is set. */
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
