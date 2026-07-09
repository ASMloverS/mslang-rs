/*
 * msStringFmt C va_list shim (task 67).
 *
 * Rust stable cannot read C variadic arguments (va_list).
 * This C wrapper uses vsnprintf to format, then calls Rust's msStringn.
 */

#include <stdio.h>
#include <stdarg.h>
#include <stdlib.h>
#include <string.h>

/* msStringn is exported by Rust via #[no_mangle] pub extern "C".
 * Declared as void* to avoid pulling in Rust type headers. */
extern void* msStringn(void* vm, const char* str, size_t len);

/* msStringFmt: printf-style string creation.
 * Implemented in C because Rust stable has no va_list support. */
void* msStringFmt(void* vm, const char* fmt, ...) {
    char stack_buf[1024];
    va_list ap;
    va_start(ap, fmt);
    int written = vsnprintf(stack_buf, sizeof(stack_buf), fmt, ap);
    va_end(ap);

    if (written < 0) {
        return msStringn(vm, "", 0);
    }

    size_t len = (size_t)written;
    if (len < sizeof(stack_buf)) {
        /* Result fits in stack buffer. */
        return msStringn(vm, stack_buf, len);
    }

    /* Result exceeds 1024 bytes; retry with heap buffer. */
    char* heap_buf = (char*)malloc(len + 1);
    if (!heap_buf) {
        return msStringn(vm, stack_buf, sizeof(stack_buf) - 1);
    }
    va_start(ap, fmt);
    vsnprintf(heap_buf, len + 1, fmt, ap);
    va_end(ap);
    void* result = msStringn(vm, heap_buf, len);
    free(heap_buf);
    return result;
}
