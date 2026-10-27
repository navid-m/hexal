//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

const std = @import("std");

pub fn build(b: *std.Build) void {
    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});
    const release = optimize != .debug;

    const mod = b.addModule("hexalz", .{
        .root_source_file = b.path("src/main.zig"),
        .target = target,
        .optimize = optimize,
        .strip = release,
        .single_threaded = true,
        .omit_frame_pointer = release,
        .error_tracing = !release,
        .unwind_tables = if (release) .none else null,
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
        .flags = &.{ "-std=c99", "-DLUA_USE_POSIX" },
    });

    const exe = b.addExecutable(.{
        .name = "hexalz",
        .root_module = mod,
        .linkage = .static,
    });
    b.installArtifact(exe);

    const run_cmd = b.addRunArtifact(exe);
    run_cmd.step.dependOn(b.getInstallStep());
    run_cmd.addPassthruArgs();
    const run_step = b.step("run", "Run HexalZ");
    run_step.dependOn(&run_cmd.step);

    const tests = b.addTest(.{ .root_module = mod });
    const run_tests = b.addRunArtifact(tests);
    const test_step = b.step("test", "Run the HexalZ tests");
    test_step.dependOn(&run_tests.step);
}
