use alloc::alloc::handle_alloc_error;
use core::{
    alloc::{GlobalAlloc, Layout},
    cell::UnsafeCell,
    ptr::NonNull,
};
use customizable_buddy::{BuddyAllocator, LinkedListBuddy, UsizeBuddy};

/// 初始化用户态全局分配器。
///
/// 教学说明：用户程序同样需要 `alloc` 支持，因此也要在启动时初始化一个小堆。
struct StaticCell<T> {
    inner: UnsafeCell<T>,
}

unsafe impl<T> Sync for StaticCell<T> {}

impl<T> StaticCell<T> {
    const fn new(value: T) -> Self {
        Self {
            inner: UnsafeCell::new(value),
        }
    }

    #[inline]
    fn get(&self) -> *mut T {
        self.inner.get()
    }
}

// 64 KiB initial heap — keeps BSS small.
// Extended at runtime via mmap syscall to 16 MiB.
const MEMORY_SIZE: usize = 64 << 10;
static MEMORY: StaticCell<[u8; MEMORY_SIZE]> = StaticCell::new([0u8; MEMORY_SIZE]);

const HEAP_ADDR_BASE: usize = 0x1000_0000;
const PAGE_SIZE: usize = 4096;
const HEAP_GROW_MIN: usize = 2 << 20;
const HEAP_INITIAL_MAP: usize = 4 << 20;
static NEXT_HEAP_ADDR: StaticCell<usize> = StaticCell::new(HEAP_ADDR_BASE);

#[inline]
const fn page_align_up(size: usize) -> usize {
    (size + (PAGE_SIZE - 1)) & !(PAGE_SIZE - 1)
}

fn grow_heap(min_need: usize) -> bool {
    let mut grow = page_align_up(min_need.max(HEAP_GROW_MIN));
    if !grow.is_power_of_two() {
        grow = grow.next_power_of_two();
    }

    // keep growth chunks practical even for very large one-shot requests
    if grow > (128 << 20) {
        grow = page_align_up(min_need);
    }

    let addr = unsafe { *NEXT_HEAP_ADDR.get() };
    let ret = tg_syscall::mmap(addr, grow, 3); // prot = RW
    if ret < 0 {
        return false;
    }

    unsafe {
        heap_mut().transfer(NonNull::new_unchecked(addr as *mut u8), grow);
        *NEXT_HEAP_ADDR.get() = addr + grow;
    }
    true
}

pub fn init() {
    unsafe {
        heap_mut().init(
            core::mem::size_of::<usize>().trailing_zeros() as _,
            NonNull::new((*MEMORY.get()).as_mut_ptr()).unwrap(),
        );
        heap_mut().transfer(
            NonNull::new_unchecked((*MEMORY.get()).as_mut_ptr()),
            MEMORY_SIZE,
        );
    }

    // Map a larger initial heap for memory-heavy apps (e.g. Doom), then grow on demand.
    let _ = grow_heap(HEAP_INITIAL_MAP);
}

type MutAllocator<const N: usize> = BuddyAllocator<N, UsizeBuddy, LinkedListBuddy>;
static HEAP: StaticCell<MutAllocator<32>> = StaticCell::new(MutAllocator::new());

#[inline]
fn heap_mut() -> &'static mut MutAllocator<32> {
    unsafe { &mut *HEAP.get() }
}

struct Global;

#[global_allocator]
static GLOBAL: Global = Global;

unsafe impl GlobalAlloc for Global {
    #[inline]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // 分配失败直接走 Rust 统一错误处理（通常会 panic）。
        if let Ok((ptr, _)) = heap_mut().allocate_layout::<u8>(layout) {
            ptr.as_ptr()
        } else {
            // Try to extend user heap lazily via mmap, then retry once.
            let need = page_align_up(layout.size().saturating_add(layout.align()));
            if grow_heap(need) {
                if let Ok((ptr, _)) = heap_mut().allocate_layout::<u8>(layout) {
                    return ptr.as_ptr();
                }
            }
            handle_alloc_error(layout)
        }
    }

    #[inline]
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { heap_mut().deallocate_layout(NonNull::new(ptr).unwrap(), layout) }
    }
}
