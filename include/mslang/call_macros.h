#ifndef MS_CALL_MACROS_H
#define MS_CALL_MACROS_H

#define msCall0(vm, f) msCall((vm), (f), NULL, 0)

#ifdef __GNUC__
#define msCall1(vm, f, a) __extension__ ({                    \
    MsValue* _args[] = {(a)};                                  \
    msCall((vm), (f), _args, 1);                               \
})
#define msCall2(vm, f, a, b) __extension__ ({                  \
    MsValue* _args[] = {(a), (b)};                             \
    msCall((vm), (f), _args, 2);                               \
})
#define msCall3(vm, f, a, b, c) __extension__ ({               \
    MsValue* _args[] = {(a), (b), (c)};                        \
    msCall((vm), (f), _args, 3);                               \
})
#else
#define msCall1(vm, f, a) msCall((vm), (f), (MsValue* const[]){(a)}, 1)
#define msCall2(vm, f, a, b) msCall((vm), (f), (MsValue* const[]){(a), (b)}, 2)
#define msCall3(vm, f, a, b, c) msCall((vm), (f), (MsValue* const[]){(a), (b), (c)}, 3)
#endif

#endif /* MS_CALL_MACROS_H */
