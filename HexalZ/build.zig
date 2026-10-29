//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

const std = @import("std");

pub fn build(b: *std.Build) void {
    const target = b.standardTargetOptions(.{});
    const optimize = b.option(
        std.builtin.Optimize,
        "optimize",
        "Prioritize performance, safety, or binary size",
    ) orelse .fast;
    const release = optimize != .debug;
    const lua_flags: []const []const u8 = if (target.result.os.tag == .windows)
        &.{ "-std=c99", "-DLUA_USE_WINDOWS" }
    else
        &.{ "-std=c99", "-DLUA_USE_POSIX" };

    const mod = b.addModule("hexal", .{
        .root_source_file = b.path("src/main.zig"),
        .target = target,
        .optimize = optimize,
        .strip = release,
        .omit_frame_pointer = release,
        .error_tracing = !release,
        .unwind_tables = null,
        .link_libc = true,
    });

    mod.addIncludePath(b.path("third_party/lua"));
    mod.addCSourceFiles(.{
        .root = b.path("third_party/lua"),
        .files = &.{
            "lapi.c",     "lauxlib.c", "lbaselib.c", "lcode.c",   "lcorolib.c",
            "lctype.c",   "ldblib.c",  "ldebug.c",   "ldo.c",     "ldump.c",
            "lfunc.c",    "lgc.c",     "linit.c",    "liolib.c",  "llex.c",
            "lmathlib.c", "lmem.c",    "loadlib.c",  "lobject.c", "lopcodes.c",
            "loslib.c",   "lparser.c", "lstate.c",   "lstring.c", "lstrlib.c",
            "ltable.c",   "ltablib.c", "ltm.c",      "lundump.c", "lutf8lib.c",
            "lvm.c",      "lzio.c",
        },
        .flags = lua_flags,
    });

    const linkage: std.builtin.LinkMode = if (target.result.os.tag == .macos)
        .dynamic
    else
        .static;

    const exe = b.addExecutable(.{
        .name = "hexal",
        .root_module = mod,
        .linkage = linkage,
    });
    b.installArtifact(exe);
    // Grammars are data, not code: users edit and add `.hexdef` files without
    // rebuilding. Install them next to the binary so a relocated `zig-out`
    // keeps language recognition working from any working directory.
    b.installDirectory(.{
        .source_dir = b.path("data"),
        .install_dir = .prefix,
        .install_subdir = "data",
    });
    // Keep the portable data directory next to the executable as well, so a
    // binary copied out of `zig-out/bin` still finds grammars, themes and
    // plugins without depending on the current working directory.
    b.installDirectory(.{
        .source_dir = b.path("data"),
        .install_dir = .bin,
        .install_subdir = "data",
    });

    const run_cmd = b.addRunArtifact(exe);
    run_cmd.step.dependOn(b.getInstallStep());
    run_cmd.addPassthruArgs();
    const run_step = b.step("run", "Run Hexal");
    run_step.dependOn(&run_cmd.step);

    const test_mod = b.addModule("hexal-tests", .{
        .root_source_file = b.path("src/tests.zig"),
        .target = target,
        .optimize = optimize,
        .link_libc = true,
    });
    test_mod.addIncludePath(b.path("third_party/lua"));
    test_mod.addCSourceFiles(.{
        .root = b.path("third_party/lua"),
        .files = &.{
            "lapi.c",     "lauxlib.c", "lbaselib.c", "lcode.c",   "lcorolib.c",
            "lctype.c",   "ldblib.c",  "ldebug.c",   "ldo.c",     "ldump.c",
            "lfunc.c",    "lgc.c",     "linit.c",    "liolib.c",  "llex.c",
            "lmathlib.c", "lmem.c",    "loadlib.c",  "lobject.c", "lopcodes.c",
            "loslib.c",   "lparser.c", "lstate.c",   "lstring.c", "lstrlib.c",
            "ltable.c",   "ltablib.c", "ltm.c",      "lundump.c", "lutf8lib.c",
            "lvm.c",      "lzio.c",
        },
        .flags = lua_flags,
    });
    const tests = b.addTest(.{ .root_module = test_mod });
    const run_tests = b.addRunArtifact(tests);
    const test_step = b.step("test", "Run the Hexal tests");
    test_step.dependOn(&run_tests.step);
}
