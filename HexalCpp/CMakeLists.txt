cmake_minimum_required(VERSION 3.20)

project(Hexal
    VERSION 0.1.0
    DESCRIPTION "A fast, modal, terminal-based text editor"
    LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

option(HEXAL_BUILD_TESTS "Build the Hexal unit tests" ON)
option(HEXAL_BUILD_BENCHMARKS "Build the Hexal microbenchmarks" ON)

find_package(Threads REQUIRED)

if(MSVC)
    add_compile_options(/utf-8 /W4 /permissive-)
else()
    add_compile_options(-Wall -Wextra -Wpedantic)
endif()

# ---------------------------------------------------------------------------
# Core library: buffer, document, command parsing and small utilities.
# Deliberately free of terminal / UI dependencies so it can be unit tested.
# ---------------------------------------------------------------------------
add_library(hexal_core
    src/core/piece_table.cpp
    src/core/document.cpp
    src/core/command.cpp
    src/util/filesystem.cpp
    src/util/unicode.cpp)

target_include_directories(hexal_core PUBLIC src)

# ---------------------------------------------------------------------------
# The editor executable.
# ---------------------------------------------------------------------------
add_executable(hexal
    src/main.cpp
    src/ui/editor.cpp
    src/ui/screen.cpp
    src/ui/renderer.cpp)

target_link_libraries(hexal PRIVATE hexal_core Threads::Threads)

# The terminal layer is the only platform-specific part of the MVP.
if(WIN32)
    target_sources(hexal PRIVATE src/platform/terminal_win32.cpp)
    if(MSVC)
        # Use wmain so command-line arguments and file names arrive as UTF-16
        # and can be converted to UTF-8 without going through the ANSI code page.
        set_target_properties(hexal PROPERTIES LINK_FLAGS "/ENTRY:wmainCRTStartup")
    endif()
else()
    target_sources(hexal PRIVATE src/platform/terminal_posix.cpp)
endif()

# ---------------------------------------------------------------------------
# Microbenchmarks. The UI cell grid and renderer are included so the benchmark
# can measure a representative viewport frame as well as core buffer work.
# ---------------------------------------------------------------------------
if(HEXAL_BUILD_BENCHMARKS)
    add_executable(hexal_bench
        benchmarks/bench_main.cpp
        src/ui/screen.cpp
        src/ui/renderer.cpp)

    target_link_libraries(hexal_bench PRIVATE hexal_core)
endif()

# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------
if(HEXAL_BUILD_TESTS)
    enable_testing()

    add_executable(hexal_tests
        tests/test_main.cpp)

    target_link_libraries(hexal_tests PRIVATE hexal_core)

    add_test(NAME hexal_tests COMMAND hexal_tests)
endif()

# ---------------------------------------------------------------------------
# Install rules (simple, useful and platform independent).
# ---------------------------------------------------------------------------
include(GNUInstallDirs)
install(TARGETS hexal RUNTIME DESTINATION ${CMAKE_INSTALL_BINDIR})
