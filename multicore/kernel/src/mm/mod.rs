pub mod address;
pub mod frame_alloc;
pub mod heap_alloc;
pub mod memory_set;
pub mod page_table;

pub use address::*;
pub use frame_alloc::{frame_alloc, FrameTracker};
pub use memory_set::MemorySet;
pub use page_table::{translated_byte_buffer, translated_refmut};

use crate::sync::SpinLock;

static KERNEL_SPACE: SpinLock<Option<MemorySet>> = SpinLock::new(None);

pub fn init() {
    heap_alloc::init_heap();
    frame_alloc::init_frame_allocator();
    let kernel_space = MemorySet::new_kernel();
    kernel_space.activate();
    *KERNEL_SPACE.lock() = Some(kernel_space);
    crate::println!("[kernel] memory management initialized");
}

pub fn kernel_token() -> usize {
    KERNEL_SPACE.lock().as_ref().unwrap().token()
}
