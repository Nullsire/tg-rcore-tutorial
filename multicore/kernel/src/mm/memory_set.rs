use super::address::*;
use super::frame_alloc::{
    frame_alloc, frame_refcount_dec, frame_refcount_get, frame_refcount_inc,
    frame_refcount_register, frame_refcount_take, FrameTracker,
};
use super::page_table::PageTable;
use crate::config::*;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use bitflags::bitflags;
use core::arch::asm;

bitflags! {
    #[derive(Copy, Clone)]
    pub struct MapPermission: u8 {
        const R = 1 << 1;
        const W = 1 << 2;
        const X = 1 << 3;
        const U = 1 << 4;
    }
}

#[derive(Copy, Clone, PartialEq, Debug)]
pub enum MapType {
    Identical,
    Framed,
}

pub struct MapArea {
    vpn_range: core::ops::Range<VirtPageNum>,
    data_frames: BTreeMap<VirtPageNum, FrameTracker>,
    /// COW-shared frames: we reference the physical page but don't own it
    /// exclusively. Refcount is tracked in the global FRAME_REFCOUNT table.
    cow_frames: BTreeMap<VirtPageNum, PhysPageNum>,
    map_type: MapType,
    map_perm: MapPermission,
}

impl MapArea {
    pub fn new(
        start_va: VirtAddr,
        end_va: VirtAddr,
        map_type: MapType,
        map_perm: MapPermission,
    ) -> Self {
        let start_vpn = start_va.floor();
        let end_vpn = end_va.ceil();
        Self {
            vpn_range: start_vpn..end_vpn,
            data_frames: BTreeMap::new(),
            cow_frames: BTreeMap::new(),
            map_type,
            map_perm,
        }
    }

    pub fn map_one(&mut self, page_table: &mut PageTable, vpn: VirtPageNum) {
        let ppn: PhysPageNum;
        match self.map_type {
            MapType::Identical => {
                ppn = PhysPageNum(vpn.0);
            }
            MapType::Framed => {
                if let Some(pte) = page_table.translate(vpn) {
                    if pte.is_valid() {
                        let pte_flags = PTEFlags::from_bits(self.map_perm.bits()).unwrap();
                        page_table.map(vpn, pte.ppn(), pte.flags() | pte_flags);
                        return;
                    }
                }
                let frame = frame_alloc().unwrap();
                ppn = frame.ppn;
                self.data_frames.insert(vpn, frame);
            }
        }
        let pte_flags = PTEFlags::from_bits(self.map_perm.bits()).unwrap();
        page_table.map(vpn, ppn, pte_flags);
    }

    #[allow(dead_code)]
    pub fn unmap_one(&mut self, page_table: &mut PageTable, vpn: VirtPageNum) {
        if self.map_type == MapType::Framed {
            if self.data_frames.remove(&vpn).is_none() {
                // Not in data_frames — check cow_frames
                if let Some(ppn) = self.cow_frames.remove(&vpn) {
                    frame_refcount_dec(ppn);
                }
            }
        }
        page_table.unmap(vpn);
    }

    pub fn map(&mut self, page_table: &mut PageTable) {
        for vpn in self.vpn_range.clone() {
            self.map_one(page_table, vpn);
        }
    }

    #[allow(dead_code)]
    pub fn unmap(&mut self, page_table: &mut PageTable) {
        for vpn in self.vpn_range.clone() {
            self.unmap_one(page_table, vpn);
        }
    }

    pub fn copy_data(&mut self, page_table: &mut PageTable, data: &[u8], page_offset: usize) {
        assert_eq!(self.map_type, MapType::Framed);
        let mut start: usize = 0;
        let mut current_vpn = self.vpn_range.start;
        let len = data.len();
        // First page may need an offset (when start_va is not page-aligned)
        let mut offset_in_page = page_offset;
        loop {
            let copy_len = (PAGE_SIZE - offset_in_page).min(len - start);
            let src = &data[start..start + copy_len];
            let dst = &mut page_table
                .translate(current_vpn)
                .unwrap()
                .ppn()
                .get_bytes_array()[offset_in_page..offset_in_page + copy_len];
            dst.copy_from_slice(src);
            start += copy_len;
            if start >= len {
                break;
            }
            offset_in_page = 0; // subsequent pages start at offset 0
            current_vpn = VirtPageNum(current_vpn.0 + 1);
        }
    }
}

impl Drop for MapArea {
    fn drop(&mut self) {
        // Decrement refcounts for COW-shared frames.
        // data_frames are freed automatically via FrameTracker::Drop.
        for &ppn in self.cow_frames.values() {
            frame_refcount_dec(ppn);
        }
    }
}

pub struct MemorySet {
    page_table: PageTable,
    areas: Vec<MapArea>,
}

impl MemorySet {
    pub fn new_bare() -> Self {
        Self {
            page_table: PageTable::new(),
            areas: Vec::new(),
        }
    }

    pub fn token(&self) -> usize {
        self.page_table.token()
    }

    fn push(&mut self, map_area: MapArea, data: Option<&[u8]>) {
        self.push_with_offset(map_area, data, 0);
    }

    fn push_with_offset(&mut self, mut map_area: MapArea, data: Option<&[u8]>, page_offset: usize) {
        map_area.map(&mut self.page_table);
        if let Some(data) = data {
            map_area.copy_data(&mut self.page_table, data, page_offset);
        }
        self.areas.push(map_area);
    }

    #[allow(dead_code)]
    pub fn insert_framed_area(
        &mut self,
        start_va: VirtAddr,
        end_va: VirtAddr,
        permission: MapPermission,
    ) {
        self.push(
            MapArea::new(start_va, end_va, MapType::Framed, permission),
            None,
        );
    }

    #[allow(dead_code)]
    pub fn remove_area_with_start_vpn(&mut self, start_vpn: VirtPageNum) {
        if let Some((idx, area)) = self
            .areas
            .iter_mut()
            .enumerate()
            .find(|(_, area)| area.vpn_range.start == start_vpn)
        {
            area.unmap(&mut self.page_table);
            self.areas.remove(idx);
        }
    }

    /// Create kernel address space
    pub fn new_kernel() -> Self {
        let mut memory_set = Self::new_bare();
        // Map kernel sections
        extern "C" {
            fn stext();
            fn etext();
            fn srodata();
            fn erodata();
            fn sdata();
            fn edata();
            fn sbss_with_stack();
            fn ebss();
            fn ekernel();
        }
        crate::println!("[kernel] mapping .text [{:#x}, {:#x})", stext as *const () as usize, etext as *const () as usize);
        memory_set.push(
            MapArea::new(
                (stext as *const () as usize).into(),
                (etext as *const () as usize).into(),
                MapType::Identical,
                MapPermission::R | MapPermission::X,
            ),
            None,
        );
        crate::println!("[kernel] mapping .rodata [{:#x}, {:#x})", srodata as *const () as usize, erodata as *const () as usize);
        memory_set.push(
            MapArea::new(
                (srodata as *const () as usize).into(),
                (erodata as *const () as usize).into(),
                MapType::Identical,
                MapPermission::R,
            ),
            None,
        );
        crate::println!("[kernel] mapping .data [{:#x}, {:#x})", sdata as *const () as usize, edata as *const () as usize);
        memory_set.push(
            MapArea::new(
                (sdata as *const () as usize).into(),
                (edata as *const () as usize).into(),
                MapType::Identical,
                MapPermission::R | MapPermission::W,
            ),
            None,
        );
        crate::println!(
            "[kernel] mapping .bss [{:#x}, {:#x})",
            sbss_with_stack as *const () as usize, ebss as *const () as usize
        );
        memory_set.push(
            MapArea::new(
                (sbss_with_stack as *const () as usize).into(),
                (ebss as *const () as usize).into(),
                MapType::Identical,
                MapPermission::R | MapPermission::W,
            ),
            None,
        );
        // Map remaining physical memory
        crate::println!(
            "[kernel] mapping physical memory [{:#x}, {:#x})",
            ekernel as *const () as usize, MEMORY_END
        );
        memory_set.push(
            MapArea::new(
                (ekernel as *const () as usize).into(),
                MEMORY_END.into(),
                MapType::Identical,
                MapPermission::R | MapPermission::W,
            ),
            None,
        );
        // Map MMIO regions
        for &(start, len) in &[
            (UART_BASE, 0x1000),
            (PLIC_BASE, 0x400000),
            (VIRTIO0_BASE, 0x1000),
        ] {
            memory_set.push(
                MapArea::new(
                    start.into(),
                    (start + len).into(),
                    MapType::Identical,
                    MapPermission::R | MapPermission::W,
                ),
                None,
            );
        }
        memory_set
    }

    /// Map kernel space into this memory set (for user page tables)
    /// This allows trap handlers to execute when CPU is in S-mode with user's page table
    fn map_kernel_space(&mut self) {
        #[allow(dead_code)]
        extern "C" {
            fn stext();
            fn etext();
            fn srodata();
            fn erodata();
            fn sdata();
            fn edata();
            fn sbss_with_stack();
            fn ebss();
            fn ekernel();
        }
        // Map kernel text (R+X, no U flag)
        self.push(
            MapArea::new(
                (stext as *const () as usize).into(),
                (etext as *const () as usize).into(),
                MapType::Identical,
                MapPermission::R | MapPermission::X,
            ),
            None,
        );
        // Map kernel rodata
        self.push(
            MapArea::new(
                (srodata as *const () as usize).into(),
                (erodata as *const () as usize).into(),
                MapType::Identical,
                MapPermission::R,
            ),
            None,
        );
        // Map kernel data + bss
        self.push(
            MapArea::new(
                (sdata as *const () as usize).into(),
                (ebss as *const () as usize).into(),
                MapType::Identical,
                MapPermission::R | MapPermission::W,
            ),
            None,
        );
        // Map remaining physical memory
        self.push(
            MapArea::new(
                (ekernel as *const () as usize).into(),
                MEMORY_END.into(),
                MapType::Identical,
                MapPermission::R | MapPermission::W,
            ),
            None,
        );
        // Map MMIO
        for &(start, len) in &[
            (UART_BASE, 0x1000),
            (PLIC_BASE, 0x400000),
            (VIRTIO0_BASE, 0x1000),
        ] {
            self.push(
                MapArea::new(
                    start.into(),
                    (start + len).into(),
                    MapType::Identical,
                    MapPermission::R | MapPermission::W,
                ),
                None,
            );
        }
    }

    /// Create user address space from ELF data
    pub fn from_elf(elf_data: &[u8]) -> (Self, usize, usize) {
        let mut memory_set = Self::new_bare();
        // Map kernel space into user page table for trap handling
        memory_set.map_kernel_space();
        let elf = xmas_elf::ElfFile::new(elf_data).unwrap();
        let elf_header = elf.header;
        let magic = elf_header.pt1.magic;
        assert_eq!(magic, [0x7f, 0x45, 0x4c, 0x46], "invalid elf!");
        let ph_count = elf_header.pt2.ph_count();
        let mut max_end_vpn = VirtPageNum(0);
        for i in 0..ph_count {
            let ph = elf.program_header(i).unwrap();
            if ph.get_type().unwrap() == xmas_elf::program::Type::Load {
                let start_va: VirtAddr = (ph.virtual_addr() as usize).into();
                let end_va: VirtAddr = ((ph.virtual_addr() + ph.mem_size()) as usize).into();
                let mut map_perm = MapPermission::U;
                let ph_flags = ph.flags();
                if ph_flags.is_read() {
                    map_perm |= MapPermission::R;
                }
                if ph_flags.is_write() {
                    map_perm |= MapPermission::W;
                }
                if ph_flags.is_execute() {
                    map_perm |= MapPermission::X;
                }
                let map_area = MapArea::new(start_va, end_va, MapType::Framed, map_perm);
                max_end_vpn = map_area.vpn_range.end;
                let page_offset = start_va.page_offset();
                memory_set.push_with_offset(
                    map_area,
                    Some(&elf.input[ph.offset() as usize..(ph.offset() + ph.file_size()) as usize]),
                    page_offset,
                );
            }
        }
        // Map user stack
        let max_end_va: VirtAddr = max_end_vpn.into();
        let mut user_stack_bottom: usize = max_end_va.0;
        // Guard page
        user_stack_bottom += PAGE_SIZE;
        let user_stack_top = user_stack_bottom + USER_STACK_SIZE;
        memory_set.push(
            MapArea::new(
                user_stack_bottom.into(),
                user_stack_top.into(),
                MapType::Framed,
                MapPermission::R | MapPermission::W | MapPermission::U,
            ),
            None,
        );
        // Map thread exit trampoline
        memory_set.map_thread_exit_trampoline();

        (
            memory_set,
            user_stack_top,
            elf.header.pt2.entry_point() as usize,
        )
    }

    /// Clone address space using Copy-On-Write (COW) for fork.
    ///
    /// Instead of deep-copying all physical frames, shared frames are mapped
    /// read-only in both parent and child. Writable pages are marked with the
    /// COW flag (RSW bit 0). On write, a page fault triggers COW resolution
    /// which allocates a new frame and copies the data.
    pub fn from_existing(user_space: &mut MemorySet) -> MemorySet {
        let mut memory_set = Self::new_bare();
        // First, map kernel space
        memory_set.map_kernel_space();

        // Collect COW page info: (vpn, ppn, was_writable)
        struct CowPageInfo {
            vpn: VirtPageNum,
            ppn: PhysPageNum,
            writable: bool,
        }
        let mut cow_pages: Vec<CowPageInfo> = Vec::new();
        // Track seen VPNs to avoid double-counting overlapping ELF segments
        let mut seen_vpns: BTreeMap<VirtPageNum, PhysPageNum> = BTreeMap::new();

        // Step 1: Modify parent's MapAreas — move frames from data_frames
        // to cow_frames and register/increment refcounts.
        for area in user_space.areas.iter_mut() {
            if area.map_type == MapType::Identical {
                continue;
            }
            let writable = area.map_perm.contains(MapPermission::W);
            for vpn in area.vpn_range.clone() {
                let ppn_opt = user_space
                    .page_table
                    .translate(vpn)
                    .and_then(|pte| if pte.is_valid() { Some(pte.ppn()) } else { None });
                if let Some(ppn) = ppn_opt {
                    if seen_vpns.contains_key(&vpn) {
                        // Already processed this VPN in a previous area
                        // (overlapping ELF segments at page boundary).
                        // Do NOT add cow_frames — the first area owns the refcount.
                        continue;
                    }
                    seen_vpns.insert(vpn, ppn);

                    if let Some(frame) = area.data_frames.remove(&vpn) {
                        // First time sharing: move from exclusive to COW
                        area.cow_frames.insert(vpn, ppn);
                        frame_refcount_register(ppn); // parent = 1
                        frame_refcount_inc(ppn);       // child  = 2
                        core::mem::forget(frame);      // prevent FrameTracker drop
                    } else if area.cow_frames.contains_key(&vpn) {
                        // Already COW (e.g. double fork): just increment
                        frame_refcount_inc(ppn);
                    }
                    cow_pages.push(CowPageInfo { vpn, ppn, writable });
                }
            }
        }

        // Step 2: Mark writable parent PTEs as read-only + COW
        for info in &cow_pages {
            if info.writable {
                if let Some(pte) = user_space.page_table.translate_mut(info.vpn) {
                    pte.clear_writable();
                    pte.set_cow();
                }
            }
        }

        // Step 3: Create child's MapAreas and page table entries
        // Track which VPNs have already been mapped in the child to avoid
        // double cow_frames entries (which would cause double refcount_dec on drop).
        let mut child_seen: BTreeMap<VirtPageNum, PhysPageNum> = BTreeMap::new();
        for area in user_space.areas.iter() {
            if area.map_type == MapType::Identical {
                continue;
            }
            let mut new_area = MapArea::new(
                area.vpn_range.start.into(),
                area.vpn_range.end.into(),
                area.map_type,
                area.map_perm,
            );
            for info in cow_pages
                .iter()
                .filter(|i| area.vpn_range.contains(&i.vpn))
            {
                if child_seen.contains_key(&info.vpn) {
                    // Already mapped in a previous area — do NOT add cow_frames
                    // to avoid double refcount_dec on Drop.
                    continue;
                }
                child_seen.insert(info.vpn, info.ppn);

                if info.writable {
                    // Map read-only + COW in child
                    let pte_flags = PTEFlags::from_bits(area.map_perm.bits()).unwrap();
                    let cow_flags = (pte_flags - PTEFlags::W) | PTEFlags::V;
                    memory_set.page_table.map(info.vpn, info.ppn, cow_flags);
                    if let Some(pte) = memory_set.page_table.translate_mut(info.vpn) {
                        pte.set_cow();
                    }
                } else {
                    // Read-only page: share as-is (no COW flag needed)
                    let pte_flags = PTEFlags::from_bits(area.map_perm.bits()).unwrap();
                    memory_set
                        .page_table
                        .map(info.vpn, info.ppn, pte_flags | PTEFlags::V);
                }
                new_area.cow_frames.insert(info.vpn, info.ppn);
            }
            memory_set.areas.push(new_area);
        }

        // Flush TLB for parent's modified PTEs
        unsafe {
            asm!("sfence.vma");
        }
        memory_set
    }

    /// Handle a COW page fault at the given VPN.
    ///
    /// Returns `true` if the fault was resolved (retry the instruction),
    /// `false` if it was a genuine page fault (should kill the process).
    pub fn handle_cow_fault(&mut self, vpn: VirtPageNum) -> bool {
        // Check PTE state
        let ppn = match self.page_table.translate(vpn) {
            Some(pte) if pte.is_cow() => pte.ppn(),
            Some(pte) if pte.writable() => {
                // Another thread already resolved COW for this page.
                // Just flush stale TLB entry and retry.
                unsafe { asm!("sfence.vma"); }
                return true;
            }
            _ => return false,
        };

        // Find the area containing this VPN
        let area_idx = match self.areas.iter().position(|a| a.vpn_range.contains(&vpn)) {
            Some(idx) => idx,
            None => return false,
        };

        let refcount = frame_refcount_get(ppn);

        if refcount > 1 {
            // Multiple references: allocate new frame and copy
            let new_frame = match frame_alloc() {
                Some(f) => f,
                None => return false,
            };
            let new_ppn = new_frame.ppn;

            // Copy data from old frame to new frame
            new_ppn
                .get_bytes_array()
                .copy_from_slice(ppn.get_bytes_array());

            // Decrement refcount of old frame
            frame_refcount_dec(ppn);

            // Update area: remove from cow_frames, add to data_frames
            self.areas[area_idx].cow_frames.remove(&vpn);
            self.areas[area_idx].data_frames.insert(vpn, new_frame);

            // Update PTE: set new PPN, restore W, clear COW
            if let Some(pte) = self.page_table.translate_mut(vpn) {
                pte.set_ppn(new_ppn);
                pte.set_writable();
                pte.clear_cow();
            }
        } else {
            // Single reference: take exclusive ownership
            frame_refcount_take(ppn);

            // Update area: remove from cow_frames, add to data_frames
            self.areas[area_idx].cow_frames.remove(&vpn);
            self.areas[area_idx]
                .data_frames
                .insert(vpn, FrameTracker::from_existing_ppn(ppn));

            // Update PTE: restore W, clear COW
            if let Some(pte) = self.page_table.translate_mut(vpn) {
                pte.set_writable();
                pte.clear_cow();
            }
        }

        // Flush TLB for this page
        unsafe {
            asm!("sfence.vma");
        }
        true
    }

    /// Allocate a new user stack of USER_STACK_SIZE bytes ending at `stack_top`.
    /// Maps the pages with R+W+U permissions. Returns `stack_top`.
    pub fn alloc_user_stack(&mut self, stack_top: usize) -> usize {
        let stack_bottom = stack_top - USER_STACK_SIZE;
        self.push(
            MapArea::new(
                stack_bottom.into(),
                stack_top.into(),
                MapType::Framed,
                MapPermission::R | MapPermission::W | MapPermission::U,
            ),
            None,
        );
        stack_top
    }

    /// Map the thread-exit trampoline page (R+X+U) at a fixed VA.
    /// When a thread function returns, ra lands here and the thread exits cleanly.
    fn map_thread_exit_trampoline(&mut self) {
        let area = MapArea::new(
            (THREAD_EXIT_TRAMPOLINE as usize).into(),
            ((THREAD_EXIT_TRAMPOLINE as usize) + PAGE_SIZE).into(),
            MapType::Framed,
            MapPermission::R | MapPermission::X | MapPermission::U,
        );
        self.push(area, None);
        // Write trampoline: li a7,93; li a0,0; ecall
        let page_bytes = self
            .page_table
            .translate(VirtPageNum(THREAD_EXIT_TRAMPOLINE / PAGE_SIZE))
            .unwrap()
            .ppn()
            .get_bytes_array();
        // li a7, 93   (addi x17, x0, 93) = 0x05d00893
        page_bytes[0..4].copy_from_slice(&[0x93, 0x08, 0xd0, 0x05]);
        // li a0, 0    (addi x10, x0, 0)  = 0x00000513
        page_bytes[4..8].copy_from_slice(&[0x13, 0x05, 0x00, 0x00]);
        // ecall                            = 0x00000073
        page_bytes[8..12].copy_from_slice(&[0x73, 0x00, 0x00, 0x00]);
    }

    pub fn activate(&self) {
        let satp = self.token();
        unsafe {
            asm!("csrw satp, {}", in(reg) satp);
            asm!("sfence.vma");
        }
    }

    #[allow(dead_code)]
    pub fn translate(&self, vpn: VirtPageNum) -> Option<PageTableEntry> {
        self.page_table.translate(vpn)
    }
}
