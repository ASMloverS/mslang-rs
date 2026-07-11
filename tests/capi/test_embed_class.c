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
    /* msIsInstanceType does not exist — use msTypeof check */
    TEST_ASSERT(msTypeof(inst) == MS_TYPE_INSTANCE, "is instance");
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
    if (nargs < 1) { msThrowValueError(vm, "need self"); return NULL; }
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
