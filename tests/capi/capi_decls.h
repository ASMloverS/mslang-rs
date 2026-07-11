#ifndef CAPI_DECLS_H
#define CAPI_DECLS_H

/*
 * Forward declarations for all C API functions.
 *
 * The cbindgen-generated headers wrap function declarations in
 * #if (defined(MS_CAPI_ENABLED)) guards.  Defining that macro causes
 * struct redefinition errors across multiple generated headers (vm.h,
 * error.h, module.h, etc. all define MsFuncDef/MsConstDef/MsModuleDef).
 *
 * Instead of using the macro, we include <mslang.h> normally (which
 * provides types, structs, and constants) and supply the function
 * prototypes here.
 *
 * Declarations extracted from include/mslang/vm.h (cbindgen-generated).
 */

/* ── VM lifecycle ── */
MsVM *msVmNew(void);
void msVmFree(MsVM *vm);
void msVmLock(MsVM *vm);
void msVmUnlock(MsVM *vm);

/* ── Execution ── */
MsStatus msExecString(MsVM *vm, const char *source, const char *filename);
MsStatus msExecFile(MsVM *vm, const char *path);
MsValue *msEval(MsVM *vm, const char *expr);

/* ── Globals ── */
MsValue *msGetGlobal(MsVM *vm, const char *name);
MsStatus msSetGlobal(MsVM *vm, const char *name, MsValue *val);
void msDelGlobal(MsVM *vm, const char *name);

/* ── Args / module path / output ── */
void msSetArgs(MsVM *vm, int argc, const char *const *argv);
void msAddModulePath(MsVM *vm, const char *path);
void msSetStdout(MsVM *vm, MsWriteFn fn, void *userdata);
void msSetStderr(MsVM *vm, MsWriteFn fn, void *userdata);

/* ── Value creation ── */
MsValue *msInt(int64_t val);
MsValue *msFloat(double val);
MsValue *msString(MsVM *vm, const char *str);
MsValue *msStringn(MsVM *vm, const char *str, size_t len);
MsValue *msNil(void);
MsValue *msBoolVal(int val);

/* ── Type checks ── */
MsType msTypeof(MsValue *val);
int msIsNil(MsValue *val);
int msIsBool(MsValue *val);
int msIsInt(MsValue *val);
int msIsFloat(MsValue *val);
int msIsNumber(MsValue *val);
int msIsString(MsValue *val);
int msIsList(MsValue *val);
int msIsDict(MsValue *val);
int msIsTuple(MsValue *val);
int msIsSet(MsValue *val);
int msIsClass(MsValue *val);
int msIsInstance(MsVM *vm, MsValue *obj, MsValue *cls);
int msIs(MsValue *a, MsValue *b);

/* ── Value extraction ── */
int64_t msToInt(MsVM *vm, MsValue *val);
double msToFloat(MsVM *vm, MsValue *val);
int msToBool(MsValue *val);
const char *msToString(MsVM *vm, MsValue *val);
char *msToStringCopy(MsVM *vm, MsValue *val);

/* ── String ops ── */
size_t msStringLen(MsVM *vm, MsValue *str);
const char *msStringData(MsVM *vm, MsValue *str);
MsValue *msStringConcat(MsVM *vm, MsValue *a, MsValue *b);
MsValue *msStringSlice(MsVM *vm, MsValue *str, int start, int end);

/* ── Comparison ── */
int msEq(MsVM *vm, MsValue *a, MsValue *b);
int msLt(MsVM *vm, MsValue *a, MsValue *b);
int msLe(MsVM *vm, MsValue *a, MsValue *b);
int msGt(MsVM *vm, MsValue *a, MsValue *b);
int msGe(MsVM *vm, MsValue *a, MsValue *b);
int64_t msHash(MsVM *vm, MsValue *val);

/* ── Conversion ── */
MsValue *msConvertInt(MsVM *vm, MsValue *val);
MsValue *msConvertFloat(MsVM *vm, MsValue *val);
MsValue *msConvertStr(MsVM *vm, MsValue *val);
MsValue *msConvertBool(MsValue *val);
MsValue *msConvertList(MsVM *vm, MsValue *val);

/* ── Rooting ── */
MsValue *msRoot(MsVM *vm, MsValue *val);
void msUnroot(MsVM *vm, MsValue *val);
void msValueFree(MsValue *val);

/* ── List ── */
MsValue *msListNew(MsVM *vm);
MsValue *msListFrom(MsVM *vm, MsValue *const *items, int count);
int msListLen(MsVM *vm, MsValue *list);
MsValue *msListGet(MsVM *vm, MsValue *list, int index);
MsStatus msListSet(MsVM *vm, MsValue *list, int index, MsValue *val);
MsStatus msListPush(MsVM *vm, MsValue *list, MsValue *val);
MsStatus msListInsert(MsVM *vm, MsValue *list, int index, MsValue *val);
MsValue *msListPop(MsVM *vm, MsValue *list);
int msListContains(MsVM *vm, MsValue *list, MsValue *val);
MsValue *msListSlice(MsVM *vm, MsValue *list, int start, int end, int step);

/* ── Dict ── */
MsValue *msDictNew(MsVM *vm);
MsValue *msDictFrom(MsVM *vm, MsValue *const *pairs, int count);
int msDictLen(MsVM *vm, MsValue *dict);
MsValue *msDictGet(MsVM *vm, MsValue *dict, MsValue *key);
MsValue *msDictGetDefault(MsVM *vm, MsValue *dict, MsValue *key, MsValue *def);
MsStatus msDictSet(MsVM *vm, MsValue *dict, MsValue *key, MsValue *val);
MsStatus msDictRemove(MsVM *vm, MsValue *dict, MsValue *key);
int msDictContains(MsVM *vm, MsValue *dict, MsValue *key);
MsValue *msDictKeys(MsVM *vm, MsValue *dict);
MsValue *msDictValues(MsVM *vm, MsValue *dict);
MsValue *msDictItems(MsVM *vm, MsValue *dict);

/* ── Tuple ── */
MsValue *msTupleFrom(MsVM *vm, MsValue *const *items, int count);
int msTupleLen(MsVM *vm, MsValue *tup);
MsValue *msTupleGet(MsVM *vm, MsValue *tup, int index);
MsStatus msTupleUnpack(MsVM *vm, MsValue *tup, MsValue ***items, int *count);
void msTupleUnpackFree(MsValue **items, int count);

/* ── Set ── */
MsValue *msSetNew(MsVM *vm);
int msSetLen(MsVM *vm, MsValue *set);
MsStatus msSetAdd(MsVM *vm, MsValue *set, MsValue *val);
MsStatus msSetRemove(MsVM *vm, MsValue *set, MsValue *val);
int msSetContains(MsVM *vm, MsValue *set, MsValue *val);

/* ── Iteration ── */
MsValue *msIter(MsVM *vm, MsValue *iterable);
MsStatus msNext(MsVM *vm, MsValue *iterator, MsValue **out);

/* ── Generic ── */
int64_t msLen(MsVM *vm, MsValue *val);
MsValue *msRepr(MsVM *vm, MsValue *val);
MsValue *msGetItem(MsVM *vm, MsValue *obj, MsValue *key);
MsStatus msSetItem(MsVM *vm, MsValue *obj, MsValue *key, MsValue *val);

/* ── Attr ── */
MsValue *msGetAttr(MsVM *vm, MsValue *obj, const char *attr);
MsStatus msSetAttr(MsVM *vm, MsValue *obj, const char *attr, MsValue *val);

/* ── Call ── */
MsValue *msCall(MsVM *vm, MsValue *func, MsValue *const *args, int nargs);
MsStatus msTry(MsVM *vm, MsValue *func, MsValue *const *args, int nargs, MsValue **result);

/* ── Error handling ── */
int msErrOccurred(MsVM *vm);
MsValue *msErrFetch(MsVM *vm);
void msErrClear(MsVM *vm);
const char *msErrTypeName(MsVM *vm, MsValue *err);
const char *msErrMessage(MsVM *vm, MsValue *err);
const char *msErrTraceback(MsVM *vm, MsValue *err);
MsValue *msErrCause(MsVM *vm, MsValue *err);
MsStatus msThrow(MsVM *vm, const char *type, const char *msg);
MsStatus msThrowValue(MsVM *vm, MsValue *err);
MsStatus msThrowRethrow(MsVM *vm);
MsStatus msThrowTypeError(MsVM *vm, const char *expected, const char *actual);
MsStatus msThrowValueError(MsVM *vm, const char *msg);
MsStatus msThrowIndexError(MsVM *vm, const char *msg);
MsStatus msThrowKeyError(MsVM *vm, MsValue *key);
MsStatus msThrowRuntimeError(MsVM *vm, const char *msg);
MsStatus msThrowIoError(MsVM *vm, const char *msg);

/* ── Class ── */
MsValue *msGetClass(MsVM *vm, const char *name);
MsValue *msInstanceNew(MsVM *vm, MsValue *cls, MsValue *const *args, int nargs);
MsValue *msInstanceGet(MsVM *vm, MsValue *obj, const char *attr);
MsStatus msInstanceSet(MsVM *vm, MsValue *obj, const char *attr, MsValue *val);
MsValue *msClassDefine(MsVM *vm, const char *name, MsValue *parent);
MsStatus msClassAddMethod(MsVM *vm, MsValue *cls, const char *name, MsCFunction method);
MsStatus msClassAddStatic(MsVM *vm, MsValue *cls, const char *name, MsValue *val);

/* ── Module ── */
MsStatus msRegisterModule(MsVM *vm, const MsModuleDef *def);
MsValue *msModuleNew(MsVM *vm, const char *name);
MsStatus msModuleAddFunc(MsVM *vm, MsValue *mod, const char *name, MsCFunction fn);
MsStatus msModuleAddConst(MsVM *vm, MsValue *mod, const char *name, MsValue *val);
MsStatus msRegisterModuleValue(MsVM *vm, MsValue *mod);

/* ── GC ── */
void msGcCollect(MsVM *vm, MsGcType type);
int msGcIsEnabled(MsVM *vm);
void msGcEnable(MsVM *vm, int enable);
MsGcStats msGcStats(MsVM *vm);
void msGcSetThreshold(MsVM *vm, MsGcType type, double threshold);
void msGcSetPromotionAge(MsVM *vm, unsigned int age);
void msGcSetGcThreads(MsVM *vm, unsigned int threads);
void msGcSetDebug(MsVM *vm, int enable);
void msWriteBarrier(MsVM *vm, MsValue *parent, MsValue *child);
MsStatus msOnFinalize(MsVM *vm, MsValue *obj, MsFinalizerFn fn, void *userdata);

#endif /* CAPI_DECLS_H */
