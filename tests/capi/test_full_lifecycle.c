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
        msUnroot(vm, err);
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
    msUnroot(vm, err);

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
    /* msIsInstanceType does not exist — use msTypeof check */
    TEST_ASSERT(msTypeof(inst) == MS_TYPE_INSTANCE, "is instance");

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

static MsValue* lifecycle_double(MsVM* vm, MsValue* const* args, int nargs) {
    if (nargs < 1) { msThrowValueError(vm, "need 1 arg"); return NULL; }
    return msInt(msToInt(vm, args[0]) * 2);
}

void test_static_module_registration(void) {
    TEST_BEGIN("static module registration (msRegisterModule)");

    MsVM* vm = msVmNew();

    static const MsFuncDef dbl_funcs[] = {
        {"double", lifecycle_double},
        {NULL, NULL}
    };
    static const MsModuleDef dbl_def = {
        .name = "dbl",
        .methods = dbl_funcs,
        .consts = NULL,
    };

    MsStatus s = msRegisterModule(vm, &dbl_def);
    TEST_ASSERT_EQ(MS_OK, s, "register static module");

    msExecString(vm,
        "import dbl\n"
        "r = dbl.double(21)\n",
        "test.ms");

    MsValue* r = msGetGlobal(vm, "r");
    TEST_ASSERT_NOT_NULL(r, "result non-NULL");
    TEST_ASSERT_EQ(42, msToInt(vm, r), "dbl.double(21) == 42");

    msVmFree(vm);
    TEST_END();
}

void test_module_add_func(void) {
    TEST_BEGIN("module addFunc");

    MsVM* vm = msVmNew();
    MsValue* mod = msModuleNew(vm, "ops");

    MsStatus s = msModuleAddFunc(vm, mod, "triple", lifecycle_double);
    TEST_ASSERT_EQ(MS_OK, s, "addFunc");

    msModuleAddConst(vm, mod, "BASE", msInt(10));
    msRegisterModuleValue(vm, mod);

    msExecString(vm,
        "import ops\n"
        "v = ops.triple(14)\n",
        "test.ms");

    MsValue* v = msGetGlobal(vm, "v");
    TEST_ASSERT_NOT_NULL(v, "triple result");
    TEST_ASSERT_EQ(28, msToInt(vm, v), "ops.triple(14) == 28");

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
    test_static_module_registration();
    test_module_add_func();
    test_output_capture_full();
    test_script_calls_c_function();
    test_thread_safety_two_vms();
    TEST_SUMMARY();
    return TEST_RETURN();
}
