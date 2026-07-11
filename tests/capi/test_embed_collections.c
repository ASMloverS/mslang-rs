#include "common.h"

void test_list_basic(void) {
    TEST_BEGIN("list basic");

    MsVM* vm = msVmNew();
    MsValue* list = msListNew(vm);
    TEST_ASSERT(msIsList(list), "is list");
    TEST_ASSERT_EQ(0, msListLen(vm, list), "new list empty");

    MsValue* a = msInt(10);
    MsValue* b = msInt(20);
    MsValue* c = msInt(30);

    TEST_ASSERT_EQ(MS_OK, msListPush(vm, list, a), "push a");
    TEST_ASSERT_EQ(MS_OK, msListPush(vm, list, b), "push b");
    TEST_ASSERT_EQ(MS_OK, msListPush(vm, list, c), "push c");
    TEST_ASSERT_EQ(3, msListLen(vm, list), "len == 3");

    TEST_ASSERT_EQ(10, msToInt(vm, msListGet(vm, list, 0)), "list[0] == 10");
    TEST_ASSERT_EQ(20, msToInt(vm, msListGet(vm, list, 1)), "list[1] == 20");
    TEST_ASSERT_EQ(30, msToInt(vm, msListGet(vm, list, 2)), "list[2] == 30");

    TEST_ASSERT_EQ(10, msToInt(vm, msListGet(vm, list, -3)), "list[-3] == 10");

    MsValue* popped = msListPop(vm, list);
    TEST_ASSERT_EQ(30, msToInt(vm, popped), "popped == 30");
    TEST_ASSERT_EQ(2, msListLen(vm, list), "len after pop == 2");

    msVmFree(vm);
    TEST_END();
}

void test_list_set_insert(void) {
    TEST_BEGIN("list set/insert");

    MsVM* vm = msVmNew();
    MsValue* list = msListNew(vm);
    MsValue* a = msInt(1);
    msListPush(vm, list, a);
    msListPush(vm, list, a);

    MsValue* val = msInt(99);
    TEST_ASSERT_EQ(MS_OK, msListSet(vm, list, 0, val), "set [0]");
    TEST_ASSERT_EQ(99, msToInt(vm, msListGet(vm, list, 0)), "list[0] == 99");

    MsValue* ins = msInt(50);
    TEST_ASSERT_EQ(MS_OK, msListInsert(vm, list, 1, ins), "insert at 1");
    TEST_ASSERT_EQ(3, msListLen(vm, list), "len after insert == 3");
    TEST_ASSERT_EQ(50, msToInt(vm, msListGet(vm, list, 1)), "list[1] == 50");

    msVmFree(vm);
    TEST_END();
}

void test_list_from(void) {
    TEST_BEGIN("list from array");

    MsVM* vm = msVmNew();
    MsValue* items[] = { msInt(1), msInt(2), msInt(3) };
    MsValue* list = msListFrom(vm, items, 3);

    TEST_ASSERT_NOT_NULL(list, "listFrom non-NULL");
    TEST_ASSERT_EQ(3, msListLen(vm, list), "listFrom len == 3");
    TEST_ASSERT_EQ(2, msToInt(vm, msListGet(vm, list, 1)), "listFrom[1] == 2");

    msVmFree(vm);
    TEST_END();
}

void test_list_contains(void) {
    TEST_BEGIN("list contains");

    MsVM* vm = msVmNew();
    MsValue* items[] = { msInt(1), msInt(2), msInt(3) };
    MsValue* list = msListFrom(vm, items, 3);

    TEST_ASSERT_EQ(MS_TRUE, msListContains(vm, list, msInt(2)), "contains 2");
    TEST_ASSERT_EQ(MS_FALSE, msListContains(vm, list, msInt(99)), "!contains 99");

    msVmFree(vm);
    TEST_END();
}

void test_list_slice(void) {
    TEST_BEGIN("list slice");

    MsVM* vm = msVmNew();
    MsValue* items[] = { msInt(0), msInt(1), msInt(2), msInt(3), msInt(4) };
    MsValue* list = msListFrom(vm, items, 5);

    MsValue* sub = msListSlice(vm, list, 1, 4, 1);
    TEST_ASSERT_NOT_NULL(sub, "slice non-NULL");
    TEST_ASSERT_EQ(3, msListLen(vm, sub), "slice len == 3");
    TEST_ASSERT_EQ(1, msToInt(vm, msListGet(vm, sub, 0)), "slice[0] == 1");
    TEST_ASSERT_EQ(3, msToInt(vm, msListGet(vm, sub, 2)), "slice[2] == 3");

    msVmFree(vm);
    TEST_END();
}

void test_dict_basic(void) {
    TEST_BEGIN("dict basic");

    MsVM* vm = msVmNew();
    MsValue* dict = msDictNew(vm);
    TEST_ASSERT(msIsDict(dict), "is dict");
    TEST_ASSERT_EQ(0, msDictLen(vm, dict), "new dict empty");

    MsValue* k1 = msString(vm, "name");
    MsValue* v1 = msString(vm, "mslang");
    TEST_ASSERT_EQ(MS_OK, msDictSet(vm, dict, k1, v1), "dict set name");

    MsValue* k2 = msString(vm, "version");
    MsValue* v2 = msInt(1);
    TEST_ASSERT_EQ(MS_OK, msDictSet(vm, dict, k2, v2), "dict set version");

    TEST_ASSERT_EQ(2, msDictLen(vm, dict), "dict len == 2");

    MsValue* got = msDictGet(vm, dict, k1);
    TEST_ASSERT_NOT_NULL(got, "dict get name");
    TEST_ASSERT_STR_EQ("mslang", msToString(vm, got), "name == mslang");

    TEST_ASSERT_EQ(MS_TRUE, msDictContains(vm, dict, k1), "contains name");
    TEST_ASSERT_EQ(MS_FALSE, msDictContains(vm, dict, msString(vm, "nope")), "!contains nope");

    msDictRemove(vm, dict, k1);
    TEST_ASSERT_EQ(1, msDictLen(vm, dict), "after remove len == 1");
    TEST_ASSERT_NULL(msDictGet(vm, dict, k1), "removed key get NULL");

    msVmFree(vm);
    TEST_END();
}

void test_dict_from(void) {
    TEST_BEGIN("dict from pairs");

    MsVM* vm = msVmNew();
    MsValue* k1 = msString(vm, "x");
    MsValue* v1 = msInt(10);
    MsValue* k2 = msString(vm, "y");
    MsValue* v2 = msInt(20);
    MsValue* pairs[] = { k1, v1, k2, v2 };
    MsValue* dict = msDictFrom(vm, pairs, 2);

    TEST_ASSERT_EQ(2, msDictLen(vm, dict), "dictFrom len == 2");
    TEST_ASSERT_EQ(10, msToInt(vm, msDictGet(vm, dict, k1)), "dictFrom[x] == 10");

    msVmFree(vm);
    TEST_END();
}

void test_dict_keys_values_items(void) {
    TEST_BEGIN("dict keys/values/items");

    MsVM* vm = msVmNew();
    MsValue* k1 = msString(vm, "a");
    MsValue* v1 = msInt(1);
    MsValue* k2 = msString(vm, "b");
    MsValue* v2 = msInt(2);
    MsValue* pairs[] = { k1, v1, k2, v2 };
    MsValue* dict = msDictFrom(vm, pairs, 2);

    MsValue* keys = msDictKeys(vm, dict);
    TEST_ASSERT(msIsList(keys), "keys is list");
    TEST_ASSERT_EQ(2, msListLen(vm, keys), "keys len == 2");

    MsValue* values = msDictValues(vm, dict);
    TEST_ASSERT(msIsList(values), "values is list");
    TEST_ASSERT_EQ(2, msListLen(vm, values), "values len == 2");

    MsValue* items = msDictItems(vm, dict);
    TEST_ASSERT(msIsList(items), "items is list");
    TEST_ASSERT_EQ(2, msListLen(vm, items), "items len == 2");

    msVmFree(vm);
    TEST_END();
}

void test_dict_get_default(void) {
    TEST_BEGIN("dict get default");

    MsVM* vm = msVmNew();
    MsValue* dict = msDictNew(vm);
    MsValue* k = msString(vm, "absent");
    MsValue* def = msInt(999);

    MsValue* got = msDictGetDefault(vm, dict, k, def);
    TEST_ASSERT_EQ(999, msToInt(vm, got), "get default returns 999");

    msVmFree(vm);
    TEST_END();
}

void test_tuple_basic(void) {
    TEST_BEGIN("tuple basic");

    MsVM* vm = msVmNew();
    MsValue* items[] = { msInt(10), msInt(20), msInt(30) };
    MsValue* tup = msTupleFrom(vm, items, 3);

    TEST_ASSERT(msIsTuple(tup), "is tuple");
    TEST_ASSERT_EQ(3, msTupleLen(vm, tup), "tuple len == 3");
    TEST_ASSERT_EQ(20, msToInt(vm, msTupleGet(vm, tup, 1)), "tuple[1] == 20");
    TEST_ASSERT_EQ(30, msToInt(vm, msTupleGet(vm, tup, -1)), "tuple[-1] == 30");

    msVmFree(vm);
    TEST_END();
}

void test_tuple_unpack(void) {
    TEST_BEGIN("tuple unpack");

    MsVM* vm = msVmNew();
    MsValue* items[] = { msInt(1), msInt(2), msInt(3) };
    MsValue* tup = msTupleFrom(vm, items, 3);

    MsValue** unpacked = NULL;
    int count = 0;
    MsStatus s = msTupleUnpack(vm, tup, &unpacked, &count);
    TEST_ASSERT_EQ(MS_OK, s, "unpack succeeds");
    TEST_ASSERT_EQ(3, count, "unpack count == 3");
    TEST_ASSERT_EQ(1, msToInt(vm, unpacked[0]), "unpacked[0] == 1");
    TEST_ASSERT_EQ(3, msToInt(vm, unpacked[2]), "unpacked[2] == 3");

    msTupleUnpackFree(unpacked, count);
    msVmFree(vm);
    TEST_END();
}

void test_set_basic(void) {
    TEST_BEGIN("set basic");

    MsVM* vm = msVmNew();
    MsValue* set = msSetNew(vm);
    TEST_ASSERT(msIsSet(set), "is set");
    TEST_ASSERT_EQ(0, msSetLen(vm, set), "new set empty");

    MsValue* a = msInt(1);
    MsValue* b = msInt(2);
    MsValue* c = msInt(1);

    TEST_ASSERT_EQ(MS_OK, msSetAdd(vm, set, a), "add 1");
    TEST_ASSERT_EQ(MS_OK, msSetAdd(vm, set, b), "add 2");
    TEST_ASSERT_EQ(MS_OK, msSetAdd(vm, set, c), "add 1 again");
    TEST_ASSERT_EQ(2, msSetLen(vm, set), "set dedup len == 2");

    TEST_ASSERT_EQ(MS_TRUE, msSetContains(vm, set, a), "contains 1");
    TEST_ASSERT_EQ(MS_TRUE, msSetContains(vm, set, b), "contains 2");
    TEST_ASSERT_EQ(MS_FALSE, msSetContains(vm, set, msInt(99)), "!contains 99");

    msSetRemove(vm, set, a);
    TEST_ASSERT_EQ(1, msSetLen(vm, set), "after remove len == 1");

    msVmFree(vm);
    TEST_END();
}

void test_iterator(void) {
    /* msIter/msNext are placeholder stubs (not yet implemented).
     * Verify they fail gracefully without crashing. */
    TEST_BEGIN("iterator (stub)");

    MsVM* vm = msVmNew();
    MsValue* items[] = { msInt(10), msInt(20), msInt(30) };
    MsValue* list = msListFrom(vm, items, 3);

    MsValue* iter = msIter(vm, list);
    /* Expected: NULL — iterator protocol not yet implemented */
    TEST_ASSERT_NULL(iter, "msIter returns NULL (stub)");

    MsValue* out = NULL;
    MsStatus s = msNext(vm, list, &out);
    TEST_ASSERT_EQ(MS_ERROR, s, "msNext returns MS_ERROR (stub)");

    msVmFree(vm);
    TEST_END();
}

void test_generic_len(void) {
    TEST_BEGIN("generic len");

    MsVM* vm = msVmNew();

    MsValue* items[] = { msInt(1), msInt(2) };
    MsValue* list = msListFrom(vm, items, 2);
    TEST_ASSERT_EQ(2, (long)msLen(vm, list), "len(list) == 2");

    MsValue* s = msString(vm, "hello");
    TEST_ASSERT_EQ(5, (long)msLen(vm, s), "len(str) == 5");

    MsValue* dict = msDictNew(vm);
    msDictSet(vm, dict, msString(vm, "k"), msInt(1));
    TEST_ASSERT_EQ(1, (long)msLen(vm, dict), "len(dict) == 1");

    msVmFree(vm);
    TEST_END();
}

void test_repr(void) {
    TEST_BEGIN("repr");

    MsVM* vm = msVmNew();
    MsValue* i = msInt(42);
    MsValue* r = msRepr(vm, i);
    TEST_ASSERT_NOT_NULL(r, "repr non-NULL");
    TEST_ASSERT(msIsString(r), "repr is string");

    msVmFree(vm);
    TEST_END();
}

void test_getitem_setitem(void) {
    /* msGetItem/msSetItem are placeholder stubs (task 69 not done).
     * Verify they fail gracefully without crashing. */
    TEST_BEGIN("getitem/setitem (stub)");

    MsVM* vm = msVmNew();
    MsValue* list = msListNew(vm);
    msListPush(vm, list, msInt(0));

    MsValue* idx = msInt(0);
    MsValue* got = msGetItem(vm, list, idx);
    /* Expected: NULL — generic getitem not yet implemented */
    TEST_ASSERT_NULL(got, "msGetItem returns NULL (stub)");

    MsValue* new_val = msInt(99);
    msSetItem(vm, list, idx, new_val);
    /* Verify list unchanged */
    TEST_ASSERT_EQ(0, msToInt(vm, msListGet(vm, list, 0)), "list[0] still 0");

    msVmFree(vm);
    TEST_END();
}

int main(void) {
    fprintf(stdout, "test_embed_collections:\n");
    test_list_basic();
    test_list_set_insert();
    test_list_from();
    test_list_contains();
    test_list_slice();
    test_dict_basic();
    test_dict_from();
    test_dict_keys_values_items();
    test_dict_get_default();
    test_tuple_basic();
    test_tuple_unpack();
    test_set_basic();
    test_iterator();
    test_generic_len();
    test_repr();
    test_getitem_setitem();
    TEST_SUMMARY();
    return TEST_RETURN();
}
