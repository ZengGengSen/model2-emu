//! 基于页表的内存子系统。
//!
//! # 设计
//!
//! 地址空间被切分成固定大小的页（[`PAGE_SIZE`] = 4 KB）。每个页表项是一个
//! [`PageEntry`]，区分三种情况：
//!
//! - **直接指针**：RAM / ROM，读写直接走裸指针，零函数调用开销。
//! - **I/O 回调**：寄存器区域，走 `Box<dyn Fn>` 回调，慢一点但少见。
//! - **未映射**：访问时记录警告并返回开放总线值 `0xFF`。
//!
//! # 字节序
//!
//! i960 是小端，Model 2 硬件也是小端，内存里按小端存储，16/32 位访问直接
//! `from_le_bytes` / `to_le_bytes`。

use log::warn;

/// 页大小：4 KB。
pub const PAGE_SIZE: usize = 0x1000;
/// 总页数（32 位地址空间 / 4 KB）。
const PAGE_COUNT: usize = 0x10_0000; // 1 M pages

/// I/O 读回调签名：接受地址，返回 u32。
pub type ReadFn = Box<dyn Fn(u32) -> u32 + Send + Sync>;
/// I/O 写回调签名：接受地址和数据。
pub type WriteFn = Box<dyn Fn(u32, u32) + Send + Sync>;

enum PageEntry {
    /// 直接内存，存储 backing buffer 的基地址指针和区间起始地址。
    /// 实际访问：`ptr.add((addr - base) as usize)`
    Direct {
        /// 指向 backing Vec 中该页第一个字节的裸指针。
        ptr: *mut u8,
        /// 该段映射的物理起始地址（用于计算页内偏移）。
        base: u32,
        /// 是否允许写入（ROM 为 false）。
        writable: bool,
    },
    /// I/O 寄存器回调。
    Io {
        read: Option<ReadFn>,
        write: Option<WriteFn>,
    },
    /// 未映射。
    Unmapped,
}

// 裸指针手动标记 Send/Sync——我们保证单线程访问或内部加锁。
unsafe impl Send for PageEntry {}
unsafe impl Sync for PageEntry {}

/// 内存子系统。
///
/// 持有所有 backing buffer（RAM、ROM），并通过页表将地址空间切片映射到这些
/// buffer 或 I/O 回调。
pub struct Memory {
    /// 页表，长度固定为 [`PAGE_COUNT`]。
    pages: Vec<PageEntry>,
    /// 所有 backing buffer（RAM / ROM）的所有权在这里。
    /// 页表里的裸指针指向这些 Vec 的数据区，Vec 不能 realloc，所以用 Box。
    _buffers: Vec<Box<[u8]>>,
}

impl Memory {
    /// 创建空内存，所有页均为 `Unmapped`。
    pub fn new() -> Self {
        // 用 Vec::from_fn 的等价写法避免 PAGE_COUNT 次 clone
        let mut pages = Vec::with_capacity(PAGE_COUNT);
        for _ in 0..PAGE_COUNT {
            pages.push(PageEntry::Unmapped);
        }
        Self {
            pages,
            _buffers: Vec::new(),
        }
    }

    // -----------------------------------------------------------------------
    // 映射接口
    // -----------------------------------------------------------------------

    /// 映射一段 RAM（可读写）到指定地址区间。
    ///
    /// `size` 必须是 [`PAGE_SIZE`] 的整数倍，`start` 也必须页对齐。
    pub fn map_ram(&mut self, start: u32, size: usize) {
        assert!(
            (start as usize).is_multiple_of(PAGE_SIZE),
            "start must be page-aligned"
        );
        assert!(
            size.is_multiple_of(PAGE_SIZE),
            "size must be a multiple of PAGE_SIZE"
        );

        let mut buf: Box<[u8]> = vec![0u8; size].into_boxed_slice();
        let base_ptr = buf.as_mut_ptr();
        self._buffers.push(buf);

        let page_start = (start as usize) / PAGE_SIZE;
        let page_count = size / PAGE_SIZE;
        for i in 0..page_count {
            let ptr = unsafe { base_ptr.add(i * PAGE_SIZE) };
            self.pages[page_start + i] = PageEntry::Direct {
                ptr,
                base: start + (i * PAGE_SIZE) as u32,
                writable: true,
            };
        }
    }

    /// 映射一段 ROM（只读）到指定地址区间，数据从 `data` 复制。
    ///
    /// `data.len()` 必须是 [`PAGE_SIZE`] 的整数倍，`start` 必须页对齐。
    pub fn map_rom(&mut self, start: u32, data: &[u8]) {
        assert!(
            (start as usize).is_multiple_of(PAGE_SIZE),
            "start must be page-aligned"
        );
        assert!(
            data.len().is_multiple_of(PAGE_SIZE),
            "ROM size must be a multiple of PAGE_SIZE"
        );

        let mut buf: Box<[u8]> = data.to_vec().into_boxed_slice();
        let base_ptr = buf.as_mut_ptr();
        self._buffers.push(buf);

        let page_start = (start as usize) / PAGE_SIZE;
        let page_count = data.len() / PAGE_SIZE;
        for i in 0..page_count {
            let ptr = unsafe { base_ptr.add(i * PAGE_SIZE) };
            self.pages[page_start + i] = PageEntry::Direct {
                ptr,
                base: start + (i * PAGE_SIZE) as u32,
                writable: false,
            };
        }
    }

    /// 映射一段 I/O 区间（回调访问）。
    ///
    /// `start`/`end` 不需要页对齐，但会以页为粒度映射（整页都走回调）。
    pub fn map_io(&mut self, start: u32, end: u32, read: Option<ReadFn>, write: Option<WriteFn>) {
        // 因为回调不能 Clone，我们把同一对回调放进 Arc 共享给多个页
        use std::sync::Arc;
        let read = read.map(Arc::new);
        let write = write.map(Arc::new);

        let page_start = (start as usize) / PAGE_SIZE;
        let page_end = (end as usize) / PAGE_SIZE;
        for p in page_start..=page_end {
            self.pages[p] = PageEntry::Io {
                read: read.as_ref().map(|f| {
                    let f = Arc::clone(f);
                    Box::new(move |a| f(a)) as ReadFn
                }),
                write: write.as_ref().map(|f| {
                    let f = Arc::clone(f);
                    Box::new(move |a, d| f(a, d)) as WriteFn
                }),
            };
        }
    }

    // -----------------------------------------------------------------------
    // 读接口
    // -----------------------------------------------------------------------

    #[inline]
    pub fn read_u8(&self, addr: u32) -> u8 {
        match self.page(addr) {
            PageEntry::Direct { ptr, base, .. } => unsafe { *ptr.add((addr - base) as usize) },
            PageEntry::Io { read: Some(f), .. } => f(addr) as u8,
            PageEntry::Io { read: None, .. } => {
                warn!("read_u8 from write-only IO @ {:#010x}", addr);
                0xFF
            }
            PageEntry::Unmapped => {
                warn!("read_u8 from unmapped addr {:#010x}", addr);
                0xFF
            }
        }
    }

    #[inline]
    pub fn read_u16(&self, addr: u32) -> u16 {
        // 对齐访问快路径
        if addr & 1 == 0
            && let PageEntry::Direct { ptr, base, .. } = self.page(addr)
        {
            let offset = (addr - base) as usize;
            let bytes = unsafe { *(ptr.add(offset) as *const [u8; 2]) };
            return u16::from_le_bytes(bytes);
        }
        // 慢路径（非对齐或 IO）
        u16::from_le_bytes([self.read_u8(addr), self.read_u8(addr + 1)])
    }

    #[inline]
    pub fn read_u32(&self, addr: u32) -> u32 {
        if addr & 3 == 0
            && let PageEntry::Direct { ptr, base, .. } = self.page(addr)
        {
            let offset = (addr - base) as usize;
            let bytes = unsafe { *(ptr.add(offset) as *const [u8; 4]) };
            return u32::from_le_bytes(bytes);
        }
        u32::from_le_bytes([
            self.read_u8(addr),
            self.read_u8(addr + 1),
            self.read_u8(addr + 2),
            self.read_u8(addr + 3),
        ])
    }

    // -----------------------------------------------------------------------
    // 写接口
    // -----------------------------------------------------------------------

    #[inline]
    pub fn write_u8(&mut self, addr: u32, val: u8) {
        match self.page_mut(addr) {
            PageEntry::Direct {
                ptr,
                base,
                writable,
            } => {
                if *writable {
                    unsafe {
                        *ptr.add((addr - *base) as usize) = val;
                    }
                } else {
                    warn!("write_u8 to ROM @ {:#010x} ignored", addr);
                }
            }
            PageEntry::Io { write: Some(f), .. } => f(addr, val as u32),
            PageEntry::Io { write: None, .. } => {
                warn!("write_u8 to read-only IO @ {:#010x}", addr);
            }
            PageEntry::Unmapped => {
                warn!("write_u8 to unmapped addr {:#010x} = {:#04x}", addr, val);
            }
        }
    }

    #[inline]
    pub fn write_u16(&mut self, addr: u32, val: u16) {
        let bytes = val.to_le_bytes();
        if addr & 1 == 0
            && let PageEntry::Direct {
                ptr,
                base,
                writable,
            } = self.page_mut(addr)
            && *writable
        {
            let offset = (addr - *base) as usize;
            unsafe {
                *(ptr.add(offset) as *mut [u8; 2]) = bytes;
            }
            return;
        }
        self.write_u8(addr, bytes[0]);
        self.write_u8(addr + 1, bytes[1]);
    }

    #[inline]
    pub fn write_u32(&mut self, addr: u32, val: u32) {
        let bytes = val.to_le_bytes();
        if addr & 3 == 0
            && let PageEntry::Direct {
                ptr,
                base,
                writable,
            } = self.page_mut(addr)
            && *writable
        {
            let offset = (addr - *base) as usize;
            unsafe {
                *(ptr.add(offset) as *mut [u8; 4]) = bytes;
            }
            return;
        }
        self.write_u8(addr, bytes[0]);
        self.write_u8(addr + 1, bytes[1]);
        self.write_u8(addr + 2, bytes[2]);
        self.write_u8(addr + 3, bytes[3]);
    }

    // -----------------------------------------------------------------------
    // 调试工具
    // -----------------------------------------------------------------------

    /// 以十六进制 dump 一段内存，每行 16 字节。
    pub fn hexdump(&self, start: u32, len: usize) {
        let mut addr = start;
        let mut remaining = len;
        while remaining > 0 {
            let row = remaining.min(16);
            let bytes: Vec<u8> = (0..row as u32).map(|i| self.read_u8(addr + i)).collect();
            let hex: String = bytes.iter().map(|b| format!("{:02X} ", b)).collect();
            let ascii: String = bytes
                .iter()
                .map(|&b| {
                    if b.is_ascii_graphic() || b == b' ' {
                        b as char
                    } else {
                        '.'
                    }
                })
                .collect();
            println!("{:#010x}  {:<48} |{}|", addr, hex, ascii);
            addr += row as u32;
            remaining -= row;
        }
    }

    /// 在 `[start, start+len)` 范围内搜索 u32 值（小端）。
    pub fn find_u32(&self, start: u32, len: usize, needle: u32) -> Vec<u32> {
        let mut results = Vec::new();
        let needle_bytes = needle.to_le_bytes();
        for offset in (0..len as u32).step_by(4) {
            let addr = start + offset;
            let b = [
                self.read_u8(addr),
                self.read_u8(addr + 1),
                self.read_u8(addr + 2),
                self.read_u8(addr + 3),
            ];
            if b == needle_bytes {
                results.push(addr);
            }
        }
        results
    }

    // -----------------------------------------------------------------------
    // 内部辅助
    // -----------------------------------------------------------------------

    #[inline]
    fn page(&self, addr: u32) -> &PageEntry {
        &self.pages[(addr >> 12) as usize]
    }

    #[inline]
    fn page_mut(&mut self, addr: u32) -> &mut PageEntry {
        &mut self.pages[(addr >> 12) as usize]
    }
}

impl Default for Memory {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_mem() -> Memory {
        let mut m = Memory::new();
        m.map_ram(0x0000_0000, 0x2000); // 8 KB RAM
        m
    }

    #[test]
    fn ram_rw_u8() {
        let mut m = make_mem();
        m.write_u8(0x100, 0xAB);
        assert_eq!(m.read_u8(0x100), 0xAB);
    }

    #[test]
    fn ram_rw_u32_aligned() {
        let mut m = make_mem();
        m.write_u32(0x200, 0xDEAD_BEEF);
        assert_eq!(m.read_u32(0x200), 0xDEAD_BEEF);
    }

    #[test]
    fn ram_rw_u32_unaligned() {
        let mut m = make_mem();
        m.write_u32(0x101, 0x1234_5678);
        assert_eq!(m.read_u32(0x101), 0x1234_5678);
    }

    #[test]
    fn rom_read_only() {
        let mut m = Memory::new();
        let data = vec![0u8; PAGE_SIZE]; // 全零
        m.map_rom(0x0000_0000, &data);
        m.write_u8(0x00, 0xFF); // 应该被忽略（有 warn）
        assert_eq!(m.read_u8(0x00), 0x00);
    }

    #[test]
    fn unmapped_returns_ff() {
        let m = Memory::new();
        assert_eq!(m.read_u8(0xFFFF_FFFF), 0xFF);
    }

    #[test]
    fn hexdump_smoke() {
        let mut m = make_mem();
        for i in 0u32..32 {
            m.write_u8(i, i as u8);
        }
        m.hexdump(0, 32); // 不 panic 就算通过
    }
}
