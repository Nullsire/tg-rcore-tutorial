use super::address::PhysPageNum;
use crate::config::MEMORY_END;
use crate::sync::SpinLock;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;

trait FrameAllocator {
    fn alloc(&mut self) -> Option<PhysPageNum>;
    fn dealloc(&mut self, ppn: PhysPageNum);
}

struct StackFrameAllocator {
    current: usize,
    end: usize,
    recycled: Vec<usize>,
}

impl StackFrameAllocator {
    #[allow(dead_code)]
    fn new() -> Self {
        StackFrameAllocator {
            current: 0,
            end: 0,
            recycled: Vec::new(),
        }
    }

    fn init(&mut self, l: PhysPageNum, r: PhysPageNum) {
        self.current = l.0;
        self.end = r.0;
    }
}

impl FrameAllocator for StackFrameAllocator {
    fn alloc(&mut self) -> Option<PhysPageNum> {
        if let Some(ppn) = self.recycled.pop() {
            Some(ppn.into())
        } else if self.current == self.end {
            None
        } else {
            self.current += 1;
            Some((self.current - 1).into())
        }
    }

    fn dealloc(&mut self, ppn: PhysPageNum) {
        let ppn_val = ppn.0;
        if ppn_val >= self.current || self.recycled.contains(&ppn_val) {
            panic!("Frame ppn={:#x} has not been allocated!", ppn_val);
        }
        self.recycled.push(ppn_val);
    }
}

static FRAME_ALLOCATOR: SpinLock<StackFrameAllocator> =
    SpinLock::new(StackFrameAllocator {
        current: 0,
        end: 0,
        recycled: Vec::new(),
    });

pub fn init_frame_allocator() {
    extern "C" {
        fn ekernel();
    }
    let start = PhysPageNum::from(
        (ekernel as *const () as usize + crate::config::PAGE_SIZE - 1) / crate::config::PAGE_SIZE,
    );
    let end = PhysPageNum::from(MEMORY_END / crate::config::PAGE_SIZE);
    FRAME_ALLOCATOR.lock().init(start, end);
    crate::println!(
        "[kernel] frame allocator: [{:#x}, {:#x})",
        usize::from(start),
        usize::from(end)
    );
}

pub struct FrameTracker {
    pub ppn: PhysPageNum,
}

impl FrameTracker {
    pub fn new(ppn: PhysPageNum) -> Self {
        // Zero the page
        let bytes = ppn.get_bytes_array();
        for b in bytes.iter_mut() {
            *b = 0;
        }
        Self { ppn }
    }

    /// Create a FrameTracker from an existing ppn without zeroing.
    /// Used when taking exclusive ownership of a COW frame (refcount==1).
    pub fn from_existing_ppn(ppn: PhysPageNum) -> Self {
        Self { ppn }
    }
}

impl Drop for FrameTracker {
    fn drop(&mut self) {
        frame_dealloc(self.ppn);
    }
}

pub fn frame_alloc() -> Option<FrameTracker> {
    FRAME_ALLOCATOR
        .lock()
        .alloc()
        .map(FrameTracker::new)
}

fn frame_dealloc(ppn: PhysPageNum) {
    FRAME_ALLOCATOR.lock().dealloc(ppn);
}

// ---- COW frame reference counting ----

/// Global reference count table for physical frames shared via COW.
/// Key = ppn value, Value = reference count.
static FRAME_REFCOUNT: SpinLock<BTreeMap<usize, usize>> =
    SpinLock::new(BTreeMap::new());

/// Increment the reference count for a frame. Returns the new count.
pub fn frame_refcount_inc(ppn: PhysPageNum) -> usize {
    let mut map = FRAME_REFCOUNT.lock();
    let count = map.entry(ppn.0).or_insert(1);
    *count += 1;
    *count
}

/// Decrement the reference count for a frame. Returns the new count.
/// If the count drops to 0, the entry is removed and the frame is freed.
pub fn frame_refcount_dec(ppn: PhysPageNum) -> usize {
    let mut map = FRAME_REFCOUNT.lock();
    if let Some(count) = map.get_mut(&ppn.0) {
        *count -= 1;
        if *count == 0 {
            map.remove(&ppn.0);
            drop(map); // release lock before dealloc
            frame_dealloc(ppn);
            return 0;
        }
        *count
    } else {
        // Not in refcount table — shouldn't happen for COW frames
        0
    }
}

/// Get the current reference count for a frame. Returns 0 if not tracked.
pub fn frame_refcount_get(ppn: PhysPageNum) -> usize {
    FRAME_REFCOUNT.lock().get(&ppn.0).copied().unwrap_or(0)
}

/// Register a frame in the refcount table with initial count 1.
/// Called when a frame first becomes COW-shared.
pub fn frame_refcount_register(ppn: PhysPageNum) {
    let mut map = FRAME_REFCOUNT.lock();
    map.entry(ppn.0).or_insert(1);
}

/// Remove a frame from the refcount table without freeing it.
/// Used when taking exclusive ownership of a COW frame (refcount==1).
/// Returns true if the frame was in the table.
pub fn frame_refcount_take(ppn: PhysPageNum) -> bool {
    FRAME_REFCOUNT.lock().remove(&ppn.0).is_some()
}
