use super::address::{PhysPageNum, VirtPageNum, PageTableEntry, PTEFlags, PhysAddr, VirtAddr};
use super::frame_alloc::{frame_alloc, FrameTracker};
use alloc::vec;
use alloc::vec::Vec;

pub struct PageTable {
    root_ppn: PhysPageNum,
    frames: Vec<FrameTracker>,
}

impl PageTable {
    pub fn new() -> Self {
        let frame = frame_alloc().unwrap();
        PageTable {
            root_ppn: frame.ppn,
            frames: vec![frame],
        }
    }

    /// Create a PageTable from an existing satp value (for reading another address space)
    pub fn from_token(satp: usize) -> Self {
        Self {
            root_ppn: PhysPageNum::from(satp & ((1usize << 44) - 1)),
            frames: Vec::new(),
        }
    }

    pub fn token(&self) -> usize {
        8usize << 60 | self.root_ppn.0
    }

    fn find_pte_create(&mut self, vpn: VirtPageNum) -> Option<&mut PageTableEntry> {
        let idxs = vpn.indexes();
        let mut ppn = self.root_ppn;
        for (i, &idx) in idxs.iter().enumerate() {
            let pte = &mut ppn.get_pte_array()[idx];
            if i == 2 {
                return Some(pte);
            }
            if !pte.is_valid() {
                let frame = frame_alloc().unwrap();
                *pte = PageTableEntry::new(frame.ppn, PTEFlags::V);
                self.frames.push(frame);
            }
            ppn = pte.ppn();
        }
        None
    }

    fn find_pte(&self, vpn: VirtPageNum) -> Option<&mut PageTableEntry> {
        let idxs = vpn.indexes();
        let mut ppn = self.root_ppn;
        for (i, &idx) in idxs.iter().enumerate() {
            let pte = &mut ppn.get_pte_array()[idx];
            if i == 2 {
                return Some(pte);
            }
            if !pte.is_valid() {
                return None;
            }
            ppn = pte.ppn();
        }
        None
    }

    pub fn map(&mut self, vpn: VirtPageNum, ppn: PhysPageNum, flags: PTEFlags) {
        let pte = self.find_pte_create(vpn).unwrap();
        if pte.is_valid() {
            // Page already mapped (e.g. overlapping ELF segments at page boundary)
            // Update flags to be the union of old and new
            let old_flags = pte.flags();
            *pte = PageTableEntry::new(pte.ppn(), old_flags | flags | PTEFlags::V);
            return;
        }
        *pte = PageTableEntry::new(ppn, flags | PTEFlags::V);
    }

    #[allow(dead_code)]
    pub fn unmap(&mut self, vpn: VirtPageNum) {
        let pte = self.find_pte(vpn).unwrap();
        assert!(pte.is_valid(), "vpn {:?} is invalid before unmap", vpn);
        *pte = PageTableEntry::empty();
    }

    #[allow(dead_code)]
    pub fn translate(&self, vpn: VirtPageNum) -> Option<PageTableEntry> {
        self.find_pte(vpn).map(|pte| *pte)
    }

    /// Get a mutable reference to the PTE for a given VPN.
    /// Used by COW page fault handler to modify PTE flags in-place.
    pub fn translate_mut(&mut self, vpn: VirtPageNum) -> Option<&mut PageTableEntry> {
        self.find_pte(vpn)
    }

    pub fn translate_va(&self, va: VirtAddr) -> Option<PhysAddr> {
        self.find_pte(va.floor()).map(|pte| {
            let aligned_pa: PhysAddr = pte.ppn().into();
            let offset = va.page_offset();
            let pa: usize = aligned_pa.0 + offset;
            pa.into()
        })
    }
}

/// Read a buffer from user space given a token (satp) and a user-space pointer
pub fn translated_byte_buffer(
    token: usize,
    ptr: *const u8,
    len: usize,
) -> Vec<&'static mut [u8]> {
    let page_table = PageTable::from_token(token);
    let mut start = ptr as usize;
    let end = start + len;
    let mut v = Vec::new();
    while start < end {
        let start_va = VirtAddr::from(start);
        let mut vpn = start_va.floor();
        let ppn = page_table.translate(vpn).unwrap().ppn();
        vpn = VirtPageNum(vpn.0 + 1);
        let mut end_va: VirtAddr = vpn.into();
        if end_va.0 > end {
            end_va = VirtAddr::from(end);
        }
        let pa_start = PhysAddr::from(ppn).0 + start_va.page_offset();
        v.push(unsafe {
            core::slice::from_raw_parts_mut(pa_start as *mut u8, end_va.0 - start)
        });
        start = end_va.0;
    }
    v
}

/// Read a string from user space
#[allow(dead_code)]
pub fn translated_str(token: usize, ptr: *const u8) -> alloc::string::String {
    let page_table = PageTable::from_token(token);
    let mut string = alloc::string::String::new();
    let mut va = ptr as usize;
    loop {
        let ch: u8 = unsafe {
            *(page_table
                .translate_va(VirtAddr::from(va))
                .unwrap()
                .0 as *const u8)
        };
        if ch == 0 {
            break;
        }
        string.push(ch as char);
        va += 1;
    }
    string
}

/// Get a reference to a value in user space
#[allow(dead_code)]
pub fn translated_ref<T>(token: usize, ptr: *const T) -> &'static T {
    let page_table = PageTable::from_token(token);
    page_table
        .translate_va(VirtAddr::from(ptr as usize))
        .map(|pa| unsafe { (pa.0 as *const T).as_ref().unwrap() })
        .unwrap()
}

/// Get a mutable reference to a value in user space
pub fn translated_refmut<T>(token: usize, ptr: *mut T) -> &'static mut T {
    let page_table = PageTable::from_token(token);
    page_table
        .translate_va(VirtAddr::from(ptr as usize))
        .map(|pa| unsafe { (pa.0 as *mut T).as_mut().unwrap() })
        .unwrap()
}
