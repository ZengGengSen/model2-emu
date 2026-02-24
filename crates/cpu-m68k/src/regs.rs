//! 68000 寄存器堆。
//!
//! # 68000 寄存器概览
//!
//! ## 数据寄存器 D0–D7
//! 32 位通用数据寄存器，支持 .b/.w/.l 三种宽度操作。
//! 窄宽度操作只影响低位，高位保留。
//!
//! ## 地址寄存器 A0–A7
//! 32 位地址寄存器，A7 是用户栈指针（USP），
//! 特权模式下 A7 指向超级用户栈（SSP）。
//! Model 2 音频子系统运行在用户模式，只需 USP。
//!
//! ## 特殊寄存器
//!
//! | 名称 | 宽度 | 用途                               |
//! |------|------|------------------------------------|
//! | PC   | 32位 | 程序计数器                          |
//! | SR   | 16位 | 状态寄存器（高字节为系统字节，低为CCR）|
//! | CCR  |  8位 | 条件码寄存器（SR低字节）              |
//!
//! ## CCR 位域（SR[4:0]）
//!
//! | 位  | 名称 | 含义           |
//! |-----|------|----------------|
//! |  4  |  X   | 扩展（Extend）  |
//! |  3  |  N   | 负数（Negative）|
//! |  2  |  Z   | 零（Zero）      |
//! |  1  |  V   | 溢出（Overflow）|
//! |  0  |  C   | 进位（Carry）   |

/// 操作数宽度。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Size {
    Byte, // .b  8位
    Word, // .w 16位
    Long, // .l 32位
}

impl Size {
    pub fn bytes(self) -> u32 {
        match self {
            Size::Byte => 1,
            Size::Word => 2,
            Size::Long => 4,
        }
    }

    pub fn suffix(self) -> &'static str {
        match self {
            Size::Byte => "b",
            Size::Word => "w",
            Size::Long => "l",
        }
    }
}

/// 条件码寄存器（CCR / SR[4:0]）。
#[derive(Debug, Clone, Copy, Default)]
pub struct Ccr {
    pub c: bool, // carry
    pub v: bool, // overflow
    pub z: bool, // zero
    pub n: bool, // negative
    pub x: bool, // extend
}

impl Ccr {
    pub fn to_byte(self) -> u8 {
        (self.c as u8)
            | ((self.v as u8) << 1)
            | ((self.z as u8) << 2)
            | ((self.n as u8) << 3)
            | ((self.x as u8) << 4)
    }

    pub fn from_byte(v: u8) -> Self {
        Self {
            c: v & 0x01 != 0,
            v: v & 0x02 != 0,
            z: v & 0x04 != 0,
            n: v & 0x08 != 0,
            x: v & 0x10 != 0,
        }
    }

    /// 根据结果值（32位，只看有效宽度）更新 N 和 Z 标志。
    pub fn update_nz(&mut self, result: u32, size: Size) {
        let (n, z) = match size {
            Size::Byte => {
                let v = result as u8;
                (v & 0x80 != 0, v == 0)
            }
            Size::Word => {
                let v = result as u16;
                (v & 0x8000 != 0, v == 0)
            }
            Size::Long => (result & 0x8000_0000 != 0, result == 0),
        };
        self.n = n;
        self.z = z;
    }
}

/// 68000 寄存器堆。
#[derive(Debug, Clone)]
pub struct Regs {
    /// 数据寄存器 D0–D7（全32位存储）。
    pub d: [u32; 8],
    /// 地址寄存器 A0–A7（A7 = USP/SSP）。
    pub a: [u32; 8],
    /// 程序计数器。
    pub pc: u32,
    /// 状态寄存器（SR）。CCR 是低字节。
    pub sr: u16,
}

impl Regs {
    pub fn new() -> Self {
        Self {
            d: [0u32; 8],
            a: [0u32; 8],
            pc: 0,
            sr: 0x2700, // 特权模式，中断级别7（复位默认值）
        }
    }

    // -----------------------------------------------------------------------
    // 数据寄存器访问（尊重操作宽度）
    // -----------------------------------------------------------------------

    /// 读数据寄存器，按 `size` 宽度零扩展。
    #[inline]
    pub fn read_d(&self, n: usize, size: Size) -> u32 {
        match size {
            Size::Byte => self.d[n] & 0xFF,
            Size::Word => self.d[n] & 0xFFFF,
            Size::Long => self.d[n],
        }
    }

    /// 写数据寄存器，按 `size` 只修改低位，高位保留。
    #[inline]
    pub fn write_d(&mut self, n: usize, val: u32, size: Size) {
        match size {
            Size::Byte => self.d[n] = (self.d[n] & 0xFFFF_FF00) | (val & 0xFF),
            Size::Word => self.d[n] = (self.d[n] & 0xFFFF_0000) | (val & 0xFFFF),
            Size::Long => self.d[n] = val,
        }
    }

    /// 读地址寄存器（总是32位）。
    #[inline]
    pub fn read_a(&self, n: usize) -> u32 {
        self.a[n]
    }

    /// 写地址寄存器。
    #[inline]
    pub fn write_a(&mut self, n: usize, val: u32) {
        self.a[n] = val;
    }

    // -----------------------------------------------------------------------
    // CCR / SR
    // -----------------------------------------------------------------------

    pub fn ccr(&self) -> Ccr {
        Ccr::from_byte((self.sr & 0xFF) as u8)
    }

    pub fn set_ccr(&mut self, ccr: Ccr) {
        self.sr = (self.sr & 0xFF00) | ccr.to_byte() as u16;
    }

    pub fn update_ccr<F: FnOnce(&mut Ccr)>(&mut self, f: F) {
        let mut ccr = self.ccr();
        f(&mut ccr);
        self.set_ccr(ccr);
    }

    /// 是否处于超级用户模式（SR[13] = S bit）。
    pub fn supervisor(&self) -> bool {
        self.sr & 0x2000 != 0
    }

    /// 栈指针（A7）。
    pub fn sp(&self) -> u32 {
        self.a[7]
    }
    pub fn set_sp(&mut self, v: u32) {
        self.a[7] = v;
    }

    // -----------------------------------------------------------------------
    // 调试
    // -----------------------------------------------------------------------

    pub fn dump(&self) {
        let ccr = self.ccr();
        println!(
            "  PC={:#010x}  SR={:#06x}  CCR: X={} N={} Z={} V={} C={}",
            self.pc, self.sr, ccr.x as u8, ccr.n as u8, ccr.z as u8, ccr.v as u8, ccr.c as u8
        );
        for i in 0..8 {
            println!("  D{i}={:#010x}  A{i}={:#010x}", self.d[i], self.a[i]);
        }
    }
}

impl Default for Regs {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_d_byte_preserves_high() {
        let mut r = Regs::new();
        r.d[0] = 0xDEAD_BEEF;
        r.write_d(0, 0xAB, Size::Byte);
        assert_eq!(r.d[0], 0xDEAD_BEAB);
    }

    #[test]
    fn write_d_word_preserves_high() {
        let mut r = Regs::new();
        r.d[0] = 0xDEAD_BEEF;
        r.write_d(0, 0x1234, Size::Word);
        assert_eq!(r.d[0], 0xDEAD_1234);
    }

    #[test]
    fn ccr_roundtrip() {
        let mut r = Regs::new();
        let ccr = Ccr {
            c: true,
            v: false,
            z: true,
            n: false,
            x: true,
        };
        r.set_ccr(ccr);
        let out = r.ccr();
        assert!(out.c);
        assert!(out.z);
        assert!(out.x);
        assert!(!out.v);
    }

    #[test]
    fn update_nz_byte() {
        let mut ccr = Ccr::default();
        ccr.update_nz(0xFF, Size::Byte); // -1 as i8
        assert!(ccr.n);
        assert!(!ccr.z);

        ccr.update_nz(0, Size::Byte);
        assert!(!ccr.n);
        assert!(ccr.z);
    }
}
