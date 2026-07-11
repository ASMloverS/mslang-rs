cmake_minimum_required(VERSION 3.14)
project(mslang_capi_tests C)

set(CMAKE_C_STANDARD 11)
set(CMAKE_C_STANDARD_REQUIRED ON)

if(NOT MSLANG_INCLUDE_DIR)
    set(MSLANG_INCLUDE_DIR "${CMAKE_CURRENT_SOURCE_DIR}/../../include")
endif()

if(NOT MSLANG_LIB_DIR)
    if(WIN32)
        set(MSLANG_LIB_DIR "${CMAKE_CURRENT_SOURCE_DIR}/../../target/debug")
    else()
        set(MSLANG_LIB_DIR "${CMAKE_CURRENT_SOURCE_DIR}/../../target/debug")
    endif()
endif()

add_library(mslang_capi SHARED IMPORTED)
if(WIN32)
    set_target_properties(mslang_capi PROPERTIES
        IMPORTED_LOCATION "${MSLANG_LIB_DIR}/mslang.dll"
        IMPORTED_IMPLIB "${MSLANG_LIB_DIR}/mslang.dll.lib"
    )
else()
    set_target_properties(mslang_capi PROPERTIES
        IMPORTED_LOCATION "${MSLANG_LIB_DIR}/libmslang${CMAKE_SHARED_LIBRARY_SUFFIX}"
    )
endif()

set(TEST_SOURCES
    test_embed_basic
    test_embed_values
    test_embed_collections
    test_embed_call
    test_embed_error
    test_embed_class
    test_embed_gc
    test_full_lifecycle
)

foreach(test_name ${TEST_SOURCES})
    add_executable(${test_name} ${test_name}.c)
    target_include_directories(${test_name} PRIVATE
        ${MSLANG_INCLUDE_DIR}/mslang
        ${MSLANG_INCLUDE_DIR}
        ${CMAKE_CURRENT_SOURCE_DIR}
    )
    target_link_libraries(${test_name} mslang_capi)
    if(UNIX)
        target_link_libraries(${test_name} m dl pthread)
    endif()
    add_test(NAME ${test_name} COMMAND ${test_name})
endforeach()

add_library(test_extension SHARED test_extension.c)
target_include_directories(test_extension PRIVATE
    ${MSLANG_INCLUDE_DIR}/mslang
    ${MSLANG_INCLUDE_DIR}
)
target_link_libraries(test_extension mslang_capi)
set_target_properties(test_extension PROPERTIES
    OUTPUT_NAME "fileio"
    PREFIX ""
    SUFFIX "${CMAKE_SHARED_MODULE_SUFFIX}"
)

enable_testing()
