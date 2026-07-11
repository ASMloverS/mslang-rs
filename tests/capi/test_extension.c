#include <mslang.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* cbindgen generates `struct MsFuncDef` without typedef; provide for C use. */
typedef struct MsFuncDef MsFuncDef;
typedef struct MsConstDef MsConstDef;
typedef struct MsModuleDef MsModuleDef;

/* Function prototypes for the extension module. */
#include "capi_decls.h"

static MsValue* fileio_read(MsVM* vm, MsValue* const* args, int nargs) {
    if (nargs < 1 || !msIsString(args[0])) {
        msThrowTypeError(vm, "string", "other");
        return NULL;
    }
    const char* path = msToString(vm, args[0]);

    FILE* f = fopen(path, "rb");
    if (!f) {
        msThrowIoError(vm, "cannot open file");
        return NULL;
    }

    fseek(f, 0, SEEK_END);
    long size = ftell(f);
    if (size < 0) { fclose(f); msThrowIoError(vm, "ftell failed"); return NULL; }
    fseek(f, 0, SEEK_SET);

    char* buf = malloc((size_t)size + 1);
    if (!buf) { fclose(f); msThrowIoError(vm, "out of memory"); return NULL; }
    size_t actual = fread(buf, 1, (size_t)size, f);
    buf[actual] = '\0';
    fclose(f);

    MsValue* result = msStringn(vm, buf, actual);
    free(buf);
    return result;
}

static MsValue* fileio_write(MsVM* vm, MsValue* const* args, int nargs) {
    if (nargs < 2 || !msIsString(args[0]) || !msIsString(args[1])) {
        msThrowTypeError(vm, "string, string", "other");
        return NULL;
    }
    const char* path = msToString(vm, args[0]);
    const char* data = msToString(vm, args[1]);

    FILE* f = fopen(path, "wb");
    if (!f) {
        msThrowIoError(vm, "cannot open for write");
        return NULL;
    }
    fputs(data, f);
    fclose(f);
    return msNil();
}

static MsValue* fileio_exists(MsVM* vm, MsValue* const* args, int nargs) {
    if (nargs < 1 || !msIsString(args[0])) {
        msThrowTypeError(vm, "string", "other");
        return NULL;
    }
    const char* path = msToString(vm, args[0]);
    FILE* f = fopen(path, "rb");
    if (f) {
        fclose(f);
        return msBoolVal(1);
    }
    return msBoolVal(0);
}

static const MsFuncDef fileio_methods[] = {
    {"read",   fileio_read},
    {"write",  fileio_write},
    {"exists", fileio_exists},
    {NULL, NULL}
};

static const MsModuleDef fileio_def = {
    .name = "fileio",
    .methods = fileio_methods,
    .consts = NULL,
};

MS_MODULE_INIT const MsModuleDef* msModuleInit(MsVM* vm) {
    (void)vm;
    return &fileio_def;
}
