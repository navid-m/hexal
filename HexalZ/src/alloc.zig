//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026
//!
//! Allocator tuned for the editor's large, short-lived buffers.
//!
//! Small allocations stay on the C allocator. Buffers of at least
//! `large_threshold` bytes are mapped on 2 MiB boundaries, marked as
//! candidates for transparent huge pages, and returned to a small free
//! cache instead of being unmapped. Plugin snapshots, bulk document
//! replacements and Lua string operands all follow this path, which keeps
//! repeated whole-buffer operations from paying page-fault costs for every
//! copy.

const std = @import("std");
const builtin = @import("builtin");
const Allocator = std.mem.Allocator;
const Alignment = std.mem.Alignment;

const page = std.heap.page_size_min;
const huge = 2 * 1024 * 1024;

pub const HugeAllocator = struct {
    /// Buffers below this size are left to the C allocator: they are cheap,
    /// numerous, and do not benefit from huge pages.
    pub const large_threshold = 512 * 1024;

    /// Keep at most this many bytes of unmapped-but-cached large buffers.
    const max_cached = 256 * 1024 * 1024;

    const Block = struct {
        ptr: [*]align(huge) u8,
        len: usize,
    };

    mutex: std.atomic.Mutex = .unlocked,
    cached: std.ArrayListUnmanaged(Block) = .empty,
    cached_bytes: usize = 0,

    fn lock(mutex: *std.atomic.Mutex) void {
        while (!mutex.tryLock()) std.atomic.spinLoopHint();
    }

    pub fn allocator(self: *HugeAllocator) Allocator {
        return .{ .ptr = @ptrCast(self), .vtable = &vtable };
    }

    pub fn deinit(self: *HugeAllocator) void {
        lock(&self.mutex);
        const blocks = self.cached;
        self.cached = .empty;
        self.cached_bytes = 0;
        self.mutex.unlock();

        for (blocks.items) |block| {
            std.heap.page_allocator.free(block.ptr[0..block.len]);
        }
        var mutable = blocks;
        mutable.deinit(std.heap.c_allocator);
    }

    fn largeLen(len: usize) usize {
        return std.mem.alignForward(usize, len, huge);
    }

    fn allocRaw(self: *HugeAllocator, len: usize, alignment: Alignment) ?[*]u8 {
        const align_bytes = alignment.toByteUnits();
        if (len < large_threshold or align_bytes > huge) {
            return std.heap.c_allocator.rawAlloc(len, alignment, @returnAddress());
        }

        const wanted = largeLen(len);
        lock(&self.mutex);
        var index = self.cached.items.len;
        while (index > 0) {
            index -= 1;
            const block = self.cached.items[index];
            if (block.len == wanted) {
                _ = self.cached.swapRemove(index);
                self.cached_bytes -= block.len;
                self.mutex.unlock();
                return block.ptr;
            }
        }
        self.mutex.unlock();

        const mapping = std.heap.page_allocator.alignedAlloc(
            u8,
            .fromByteUnits(huge),
            wanted,
        ) catch return null;
        // Transparent huge pages are a Linux facility; macOS (and other
        // targets) manage large-page promotion themselves, so only request
        // the hint where the kernel supports it.
        if (builtin.os.tag == .linux) {
            std.posix.madvise(mapping.ptr, mapping.len, std.posix.MADV.HUGEPAGE) catch {};
        }
        return mapping.ptr;
    }

    fn freeRaw(self: *HugeAllocator, memory: []u8, alignment: Alignment) void {
        const align_bytes = alignment.toByteUnits();
        if (memory.len < large_threshold or align_bytes > huge) {
            std.heap.c_allocator.rawFree(memory, alignment, @returnAddress());
            return;
        }

        const len = largeLen(memory.len);
        const ptr: [*]align(huge) u8 = @alignCast(memory.ptr);

        lock(&self.mutex);
        if (self.cached_bytes + len <= max_cached) {
            self.cached.append(std.heap.c_allocator, .{ .ptr = ptr, .len = len }) catch {
                self.mutex.unlock();
                std.heap.page_allocator.free(ptr[0..len]);
                return;
            };
            self.cached_bytes += len;
            self.mutex.unlock();
            return;
        }
        self.mutex.unlock();
        std.heap.page_allocator.free(ptr[0..len]);
    }

    fn sameLargeClass(a: usize, b: usize) bool {
        return a >= large_threshold and b >= large_threshold and largeLen(a) == largeLen(b);
    }

    fn alloc(
        context: *anyopaque,
        len: usize,
        alignment: Alignment,
        return_address: usize,
    ) ?[*]u8 {
        _ = return_address;
        const self: *HugeAllocator = @ptrCast(@alignCast(context));
        return self.allocRaw(len, alignment);
    }

    fn resize(
        context: *anyopaque,
        memory: []u8,
        alignment: Alignment,
        new_len: usize,
        return_address: usize,
    ) bool {
        _ = context;
        _ = return_address;
        if (sameLargeClass(memory.len, new_len)) return true;
        if (memory.len < large_threshold and new_len < large_threshold) {
            return std.heap.c_allocator.rawResize(memory, alignment, new_len, @returnAddress());
        }
        return false;
    }

    fn remap(
        context: *anyopaque,
        memory: []u8,
        alignment: Alignment,
        new_len: usize,
        return_address: usize,
    ) ?[*]u8 {
        _ = context;
        _ = return_address;
        if (sameLargeClass(memory.len, new_len)) return memory.ptr;
        if (memory.len < large_threshold and new_len < large_threshold) {
            return std.heap.c_allocator.rawRemap(memory, alignment, new_len, @returnAddress());
        }
        return null;
    }

    fn free(
        context: *anyopaque,
        memory: []u8,
        alignment: Alignment,
        return_address: usize,
    ) void {
        _ = return_address;
        const self: *HugeAllocator = @ptrCast(@alignCast(context));
        self.freeRaw(memory, alignment);
    }

    const vtable: Allocator.VTable = .{
        .alloc = alloc,
        .resize = resize,
        .remap = remap,
        .free = free,
    };
};

test "huge allocator reuses large blocks" {
    var huge_allocator = HugeAllocator{};
    defer huge_allocator.deinit();
    const allocator = huge_allocator.allocator();

    const first = try allocator.alloc(u8, 1024 * 1024);
    @memset(first, 0xaa);
    const address = @intFromPtr(first.ptr);
    allocator.free(first);

    const second = try allocator.alloc(u8, 1024 * 1024);
    defer allocator.free(second);
    try std.testing.expectEqual(address, @intFromPtr(second.ptr));
    try std.testing.expectEqual(@as(u8, 0xaa), second[0]);
}
