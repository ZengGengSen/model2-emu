//! 地址区间描述符。
//!
//! 每一段已知的地址空间都用一个 [`MemRegion`] 描述，包含起止地址和区域类型。

/// Model 2 地址空间中各已知区域的枚举。
///
/// 顺序与物理地址升序一致，便于对照 MAME model2.cpp 中的 ADDRESS_MAP。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionKind {
    /// 主工作 RAM（可读写）
    MainRam,
    /// 图形 RAM，存放多边形 / 纹理数据
    GraphicsRam,
    /// 程序 ROM（只读）
    ProgramRom,
    /// 数据 ROM（只读）
    DataRom,
    /// I/O 映射寄存器（通过回调访问）
    IoRegisters,
    /// 其他未映射区域
    Unmapped,
}

/// 一段连续地址区间的描述。
#[derive(Debug, Clone, Copy)]
pub struct MemRegion {
    pub start: u32,
    pub end: u32, // inclusive
    pub kind: RegionKind,
}

impl MemRegion {
    pub const fn new(start: u32, end: u32, kind: RegionKind) -> Self {
        assert!(end >= start, "region end must be >= start");
        Self { start, end, kind }
    }

    #[inline]
    pub fn contains(self, addr: u32) -> bool {
        addr >= self.start && addr <= self.end
    }

    #[inline]
    pub fn len(self) -> usize {
        (self.end - self.start + 1) as usize
    }

    pub fn is_empty(self) -> bool {
        self.len() == 0
    }
}

/// Model 2A/2B 的静态地址映射表（参照 MAME model2.cpp）。
///
/// 这里只列出第一阶段需要用到的区域，后续可以继续补充。
pub const MODEL2_REGIONS: &[MemRegion] = &[
    MemRegion::new(0x0000_0000, 0x001F_FFFF, RegionKind::MainRam),
    MemRegion::new(0x0020_0000, 0x003F_FFFF, RegionKind::GraphicsRam),
    MemRegion::new(0x0080_0000, 0x009F_FFFF, RegionKind::ProgramRom),
    MemRegion::new(0x00A0_0000, 0x00BF_FFFF, RegionKind::DataRom),
    MemRegion::new(0x0100_0000, 0x0100_FFFF, RegionKind::IoRegisters),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_contains() {
        let r = MemRegion::new(0x100, 0x1FF, RegionKind::MainRam);
        assert!(r.contains(0x100));
        assert!(r.contains(0x1FF));
        assert!(!r.contains(0x200));
        assert!(!r.contains(0x0FF));
    }

    #[test]
    fn region_len() {
        let r = MemRegion::new(0x000, 0x1FF, RegionKind::MainRam);
        assert_eq!(r.len(), 0x200);
    }
}
