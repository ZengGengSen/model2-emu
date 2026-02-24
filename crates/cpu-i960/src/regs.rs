//! i960 寄存器堆。
//!
//! # i960 寄存器架构概览
//!
//! i960 Kx/Cx 有 **32 个全局整数寄存器**（g0–g15、fp 等别名）和一组
//! **局部寄存器**（r0–r15，随调用帧切换）。
//!
//! ## 全局寄存器（g0–g15）
//!
//! | 名称       | 编号   | 用途                          |
//! |----------|--------|------------------------------|
//! | g0–g7    | 0–7    | 通用，过程参数 / 返回值        |
//! | g8–g11   | 8–11   | 通用                          |
//! | g12      | 12     | 通用（有时用作帧指针 fp）      |
//! | g13      | 13     | 通用                          |
//! | g14      | 14     | 通用（return address 约定）    |
//! | g15 / fp | 15     | 帧指针（Frame Pointer）        |
//!
//! ## 局部寄存器（r0–r15）
//!
//! r0–r15（编号 16–31）是当前调用帧的本地寄存器，调用时由硬件自动保存/恢复。
//! 简单解释器里我们用一个扁平数组模拟（不做真正的寄存器窗口）。
//!
//! ## 特殊寄存器
//!
//! | 名称   | 用途                                  |
//! |------|--------------------------------------|
//! | IP   | 指令指针（Program Counter）           |
//! | AC   | Arithmetic Controls（条件码等）       |
//! | PC   | Process Controls                      |
//! | TC   | Trace Controls                        |

/// 条件码位（AC 寄存器 bit[2:0]）。
#[derive(Debug, Clone, Copy, Default)]
pub struct CondCode {
    pub n: bool, // negative（有符号比较用）
    pub e: bool, // equal / zero
    pub g: bool, // greater
}

impl CondCode {
    /// 编码为 AC[2:0] 的 3 位值。
    pub fn to_bits(self) -> u8 {
        (self.n as u8)       // bit 0
        | ((self.e as u8) << 1) // bit 1
        | ((self.g as u8) << 2) // bit 2
    }

    pub fn from_bits(v: u8) -> Self {
        Self {
            n: v & 1 != 0,
            e: v & 2 != 0,
            g: v & 4 != 0,
        }
    }
}

/// i960 寄存器堆。
#[derive(Debug, Clone)]
pub struct Regs {
    /// 全局寄存器 g0–g15（下标 0–15）
    /// + 局部寄存器 r0–r15（下标 16–31，简化：不做真正的寄存器窗口）
    pub gpr: [u32; 32],

    /// 指令指针（Program Counter）。
    pub ip: u32,

    /// Arithmetic Controls（只存条件码部分，其余位暂留 0）。
    pub ac: u32,

    /// Process Controls（特权模式等，暂时只读）。
    pub pc: u32,

    /// Trace Controls（单步等，暂时只读）。
    pub tc: u32,
}

impl Regs {
    pub fn new() -> Self {
        Self {
            gpr: [0u32; 32],
            ip: 0,
            ac: 0,
            pc: 0,
            tc: 0,
        }
    }

    // -----------------------------------------------------------------------
    // 寄存器访问
    // -----------------------------------------------------------------------

    /// 读取寄存器（0–31）。文字编号 31 为 r15（g15/fp）。
    #[inline]
    pub fn r(&self, idx: usize) -> u32 {
        debug_assert!(idx < 32, "register index out of range: {}", idx);
        self.gpr[idx]
    }

    /// 写入寄存器（0–31）。写 g0 是 no-op（i960 g0 = 常量 0，不可写）。
    /// 注意：实际 i960 中 g0 可写，只有字面量 0 是特殊寄存器，这里先简化。
    #[inline]
    pub fn w(&mut self, idx: usize, val: u32) {
        debug_assert!(idx < 32, "register index out of range: {}", idx);
        self.gpr[idx] = val;
    }

    // -----------------------------------------------------------------------
    // 条件码
    // -----------------------------------------------------------------------

    pub fn cc(&self) -> CondCode {
        CondCode::from_bits((self.ac & 0x7) as u8)
    }

    pub fn set_cc(&mut self, cc: CondCode) {
        self.ac = (self.ac & !0x7) | cc.to_bits() as u32;
    }

    // -----------------------------------------------------------------------
    // 调试
    // -----------------------------------------------------------------------

    /// 格式化打印所有寄存器（调试用）。
    pub fn dump(&self) {
        println!("  IP = {:#010x}  AC = {:#010x}", self.ip, self.ac);
        for row in 0..4 {
            let base = row * 8;
            let names = [
                "g0 ", "g1 ", "g2 ", "g3 ", "g4 ", "g5 ", "g6 ", "g7 ", "g8 ", "g9 ", "g10", "g11",
                "g12", "g13", "g14", "fp ", "r0 ", "r1 ", "r2 ", "r3 ", "r4 ", "r5 ", "r6 ", "r7 ",
                "r8 ", "r9 ", "r10", "r11", "r12", "r13", "r14", "r15",
            ];
            let line: String = (0..8)
                .map(|i| format!("  {}={:#010x}", names[base + i], self.gpr[base + i]))
                .collect::<Vec<_>>()
                .join("");
            println!("{}", line);
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
    fn cc_roundtrip() {
        let cc = CondCode {
            n: true,
            e: false,
            g: true,
        };
        let restored = CondCode::from_bits(cc.to_bits());
        assert_eq!(restored.n, cc.n);
        assert_eq!(restored.e, cc.e);
        assert_eq!(restored.g, cc.g);
    }

    #[test]
    fn reg_rw() {
        let mut r = Regs::new();
        r.w(5, 0xDEAD_BEEF);
        assert_eq!(r.r(5), 0xDEAD_BEEF);
    }
}
