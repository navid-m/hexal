//! Test aggregator: importing a module includes its `test` blocks.
const std = @import("std");

test {
    _ = @import("alloc.zig");
    _ = @import("document.zig");
    _ = @import("files.zig");
    _ = @import("json.zig");
    _ = @import("lua.zig");
    _ = @import("piece_table.zig");
    _ = @import("regex.zig");
    _ = @import("screen.zig");
    _ = @import("syntax.zig");
    _ = @import("terminal.zig");
    _ = @import("theme.zig");
    _ = @import("util.zig");
}
