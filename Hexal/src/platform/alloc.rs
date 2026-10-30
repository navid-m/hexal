//! Global allocator with a small cache for large, short-lived buffers.
//!
//! The editor regularly allocates multi-megabyte plugin snapshots and
//! whole-buffer replacements. Returning those to the system allocator causes
//! repeated `mmap`/`munmap` and page faults. Large blocks are kept in a small
//! exact-size cache; small allocations go straight to the system allocator so
//! startup does not pay for an arena.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicBool, Ordering};

const LARGE_ALLOCATION: usize = 512 * 1024;
const MAX_CACHED_BYTES: usize = 256 * 1024 * 1024;

struct Block {
    pointer: *mut u8,
    length: usize,
}

fn mapped_length(size: usize) -> usize {
    let page = 4096usize;
    (size + page - 1) / page * page
}

pub struct LargeCacheAllocator {
    lock: AtomicBool,
    blocks: UnsafeCell<Vec<Block>>,
    bytes: UnsafeCell<usize>,
}

unsafe impl Sync for LargeCacheAllocator {}

impl LargeCacheAllocator {
    pub const fn new() -> LargeCacheAllocator {
        LargeCacheAllocator {
            lock: AtomicBool::new(false),
            blocks: UnsafeCell::new(Vec::new()),
            bytes: UnsafeCell::new(0),
        }
    }

    fn lock(&self) {
        while self
            .lock
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            std::hint::spin_loop();
        }
    }

    fn unlock(&self) {
        self.lock.store(false, Ordering::Release);
    }

    unsafe fn take(&self, length: usize) -> Option<*mut u8> {
        self.lock();
        let blocks = unsafe { &mut *self.blocks.get() };
        let index = blocks.iter().rposition(|block| block.length == length);
        let result = index.map(|index| {
            let block = blocks.swap_remove(index);
            unsafe { *self.bytes.get() -= block.length };
            block.pointer
        });
        self.unlock();
        result
    }

    unsafe fn store(&self, pointer: *mut u8, length: usize) -> bool {
        if length > MAX_CACHED_BYTES {
            return false;
        }
        self.lock();
        let bytes = unsafe { &mut *self.bytes.get() };
        if *bytes + length > MAX_CACHED_BYTES {
            self.unlock();
            return false;
        }
        unsafe {
            (*self.blocks.get()).push(Block { pointer, length });
            *bytes += length;
        }
        self.unlock();
        true
    }

    unsafe fn map(length: usize) -> *mut u8 {
        let pointer = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                length,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                -1,
                0,
            )
        };
        if pointer == libc::MAP_FAILED {
            return std::ptr::null_mut();
        }
        #[cfg(target_os = "linux")]
        unsafe {
            libc::madvise(pointer, length, libc::MADV_HUGEPAGE)
        };
        pointer as *mut u8
    }
}

unsafe impl GlobalAlloc for LargeCacheAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if layout.size() >= LARGE_ALLOCATION && layout.align() <= 4096 {
            let length = mapped_length(layout.size());
            if let Some(pointer) = unsafe { self.take(length) } {
                return pointer;
            }
            let pointer = unsafe { Self::map(length) };
            if !pointer.is_null() {
                return pointer;
            }
            return std::ptr::null_mut();
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        if layout.size() >= LARGE_ALLOCATION && layout.align() <= 4096 {
            let length = mapped_length(layout.size());
            if unsafe { self.store(pointer, length) } {
                return;
            }
            unsafe { libc::munmap(pointer as *mut libc::c_void, length) };
            return;
        }
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if layout.size() < LARGE_ALLOCATION && new_size < LARGE_ALLOCATION {
            return unsafe { System.realloc(pointer, layout, new_size) };
        }
        let new_layout = unsafe { Layout::from_size_align_unchecked(new_size, layout.align()) };
        let replacement = unsafe { self.alloc(new_layout) };
        if !replacement.is_null() {
            unsafe {
                std::ptr::copy_nonoverlapping(pointer, replacement, layout.size().min(new_size));
            }
            unsafe { self.dealloc(pointer, layout) };
        }
        replacement
    }
}
