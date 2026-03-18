use core::ptr::NonNull;
use virtio_drivers::{BufferDirection, Hal, PhysAddr};

pub struct VirtioHal;

const PAGE_SIZE: usize = 4096;
#[repr(align(4096))]
struct DmaPool([u8; 2048 * PAGE_SIZE]);
static mut DMA_POOL: DmaPool = DmaPool([0; 2048 * PAGE_SIZE]);
static mut DMA_POOL_ALLOC_PAGES: usize = 0;

unsafe impl Hal for VirtioHal {
    fn dma_alloc(pages: usize, _dir: BufferDirection) -> (PhysAddr, NonNull<u8>) {
        unsafe {
            let alloc_base = core::ptr::addr_of_mut!(DMA_POOL.0)
                .cast::<u8>()
                .add(DMA_POOL_ALLOC_PAGES * PAGE_SIZE);
            core::ptr::write_bytes(alloc_base, 0, pages * PAGE_SIZE);
            DMA_POOL_ALLOC_PAGES += pages;
            (alloc_base as usize, NonNull::new(alloc_base).unwrap())
        }
    }
    unsafe fn dma_dealloc(_paddr: PhysAddr, _vaddr: NonNull<u8>, _pages: usize) -> i32 {
        0
    }
    unsafe fn mmio_phys_to_virt(paddr: PhysAddr, _size: usize) -> NonNull<u8> {
        NonNull::new(paddr as *mut u8).unwrap()
    }
    unsafe fn share(buffer: NonNull<[u8]>, _direction: BufferDirection) -> PhysAddr {
        buffer.as_ptr() as *mut u8 as usize
    }
    unsafe fn unshare(_paddr: PhysAddr, _buffer: NonNull<[u8]>, _direction: BufferDirection) {}
}
