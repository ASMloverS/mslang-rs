#include <mslang/mslang.h>
#include <stdio.h>

int main(void) {
    printf("mslang version: %s\n", MS_VERSION);
    printf("version at least 0.1.0: %d\n", MS_VERSION_AT_LEAST(0, 1, 0));

    MsType t = MS_TYPE_INT;
    MsStatus s = MS_OK;
    MsGcType g = MS_GC_MINOR;
    MsFutureState f = MS_FUTURE_PENDING;

    (void)t;
    (void)s;
    (void)g;
    (void)f;

    int truthy = MS_TRUE;
    int falsy = MS_FALSE;
    (void)truthy;
    (void)falsy;

    printf("all types ok\n");
    return 0;
}
