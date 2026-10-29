const std = @import("std");
const files = @import("files.zig");

const io = std.testing.io;

test "bench search" {
    const allocator = std.heap.c_allocator;
    var list: std.ArrayList([]u8) = .empty;
    defer {
        files.clearFiles(allocator, &list);
        list.deinit(allocator);
    }
    const cancel: std.atomic.Value(bool) = .init(false);

    var scan_start = std.Io.Clock.Timestamp.now(io, .awake);
    try files.scan(allocator, io, &list, 20000, &cancel);
    const scan_ns = scan_start.durationTo(std.Io.Clock.Timestamp.now(io, .awake)).raw.nanoseconds;

    var total_bytes: u64 = 0;
    var max_size: u64 = 0;
    for (list.items) |path| {
        const st = std.Io.Dir.cwd().statFile(io, path, .{}) catch continue;
        total_bytes += st.size;
        max_size = @max(max_size, st.size);
    }

    var hits: std.ArrayList(files.SearchHit) = .empty;
    defer {
        files.clearHits(allocator, &hits);
        hits.deinit(allocator);
    }

    try files.search(allocator, io, list.items, "Hello, world", &hits, 500, &cancel);
    var best: u64 = std.math.maxInt(u64);
    for (0..5) |_| {
        const start = std.Io.Clock.Timestamp.now(io, .awake);
        try files.search(allocator, io, list.items, "Hello, world", &hits, 500, &cancel);
        const elapsed = start.durationTo(std.Io.Clock.Timestamp.now(io, .awake)).raw.nanoseconds;
        best = @min(best, @as(u64, @intCast(elapsed)));
    }
    std.debug.print(
        "BENCH zig files={} bytes={} max={} scan_ms={d:.2} search_ms={d:.2} hits={}\n",
        .{
            list.items.len,
            total_bytes,
            max_size,
            @as(f64, @floatFromInt(scan_ns)) / 1e6,
            @as(f64, @floatFromInt(best)) / 1e6,
            hits.items.len,
        },
    );
}
