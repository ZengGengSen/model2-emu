//! i960 指令解码。
//!
//! # i960 指令格式（4种，均为 32 位定长）
//!
//! ```text
//! CTRL  [31:24]=opcode  [23:2]=displacement(22-bit signed)  [1]=预测位  [0]=0
//! COBR  [31:24]=opcode  [23:19]=src1  [18:14]=src2  [13:2]=displacement  [1]=M1  [0]=0
//! REG   [31:24]=opcode  [23:19]=src/dst1  [18:14]=src2  [13]=0  [12:10]=opcode_ext  [9:7]=mode  [6:5]=0  [4:0]=src3/dst
//! MEM   [31:24]=opcode  [23:19]=src/dst  [18:14]=abase  [13:12]=mode  ... (MEM_A or MEM_B)
//! ```
//!
//! opcode 高 4 位（[31:28]）决定格式：
//! - 0x0..=0x1 → CTRL
//! - 0x2..=0x3 → COBR  
//! - 0x5..=0x7 → REG（opcode 高 4 位 0x5x/0x6x/0x7x）
//! - 0x8..=0xF → MEM

/// 解码后的指令（AST节点）。
#[derive(Debug, Clone)]
pub enum Insn {
    // -----------------------------------------------------------------------
    // CTRL 格式：无条件跳转 / 调用
    // -----------------------------------------------------------------------
    B {
        disp: i32,
    }, // branch (unconditional)
    Call {
        disp: i32,
    }, // call (push frame, jump)
    Ret, // return（从帧返回，CTRL op=0x0A）
    Bal {
        disp: i32,
    }, // branch and link

    // -----------------------------------------------------------------------
    // COBR 格式：比较+条件跳转
    // -----------------------------------------------------------------------
    /// 比较两个整数寄存器，按 cc_mask 决定是否跳转。
    Cmpob {
        src1: usize,
        src2: usize,
        disp: i32,
        mask: u8,
    },
    /// 测试位并按条件跳转（bit test and branch）。
    Bbc {
        bit_pos: usize,
        src: usize,
        disp: i32,
    },
    Bbs {
        bit_pos: usize,
        src: usize,
        disp: i32,
    },

    // -----------------------------------------------------------------------
    // REG 格式：寄存器运算
    // -----------------------------------------------------------------------
    // 数据移动
    Mov {
        dst: usize,
        src: usize,
    },
    /// Load literal：把立即数（5 bit，0–31）放入寄存器。
    LdLit {
        dst: usize,
        lit: u32,
    },

    // 整数运算
    Add {
        dst: usize,
        src1: usize,
        src2: usize,
    },
    Addo {
        dst: usize,
        src1: usize,
        src2: usize,
    }, // add ordinal (same as add, unsigned)
    Sub {
        dst: usize,
        src1: usize,
        src2: usize,
    },
    Subo {
        dst: usize,
        src1: usize,
        src2: usize,
    },
    Mul {
        dst: usize,
        src1: usize,
        src2: usize,
    },
    Divo {
        dst: usize,
        src1: usize,
        src2: usize,
    }, // divide ordinal (unsigned)
    Remo {
        dst: usize,
        src1: usize,
        src2: usize,
    }, // remainder ordinal

    // 位运算
    And {
        dst: usize,
        src1: usize,
        src2: usize,
    },
    Or {
        dst: usize,
        src1: usize,
        src2: usize,
    },
    Xor {
        dst: usize,
        src1: usize,
        src2: usize,
    },
    Not {
        dst: usize,
        src: usize,
    },
    Nand {
        dst: usize,
        src1: usize,
        src2: usize,
    },
    Nor {
        dst: usize,
        src1: usize,
        src2: usize,
    },
    Xnor {
        dst: usize,
        src1: usize,
        src2: usize,
    },

    // 移位
    Shlo {
        dst: usize,
        src: usize,
        cnt: usize,
    }, // shift left ordinal
    Shro {
        dst: usize,
        src: usize,
        cnt: usize,
    }, // shift right ordinal
    Shli {
        dst: usize,
        src: usize,
        cnt: usize,
    }, // shift left integer
    Shri {
        dst: usize,
        src: usize,
        cnt: usize,
    }, // shift right integer (arithmetic)

    // 比较
    Cmpo {
        src1: usize,
        src2: usize,
    }, // compare ordinal
    Cmpi {
        src1: usize,
        src2: usize,
    }, // compare integer (signed)

    // 条件跳转（REG格式，根据 AC 条件码）
    Bno, // branch if no condition (NOP等价)
    Bg {
        disp: i32,
    },
    Be {
        disp: i32,
    },
    Bge {
        disp: i32,
    },
    Bl {
        disp: i32,
    },
    Bne {
        disp: i32,
    },
    Ble {
        disp: i32,
    },
    Bo {
        disp: i32,
    }, // branch if ordered

    // -----------------------------------------------------------------------
    // MEM 格式：内存访问
    // -----------------------------------------------------------------------
    Ld {
        dst: usize,
        abase: usize,
        offset: u32,
    }, // load word (32-bit)
    Ldob {
        dst: usize,
        abase: usize,
        offset: u32,
    }, // load byte unsigned
    Ldos {
        dst: usize,
        abase: usize,
        offset: u32,
    }, // load short unsigned
    Ldib {
        dst: usize,
        abase: usize,
        offset: u32,
    }, // load byte signed
    Ldis {
        dst: usize,
        abase: usize,
        offset: u32,
    }, // load short signed
    St {
        src: usize,
        abase: usize,
        offset: u32,
    }, // store word
    Stob {
        src: usize,
        abase: usize,
        offset: u32,
    }, // store byte
    Stos {
        src: usize,
        abase: usize,
        offset: u32,
    }, // store short

    /// 未实现的指令（存原始编码，方便调试）。
    Unimplemented {
        raw: u32,
    },
}

/// 解码一条 32 位 i960 指令字。
pub fn decode(word: u32) -> Insn {
    let opcode = (word >> 24) as u8;

    match opcode >> 4 {
        0x0 | 0x1 => decode_ctrl(opcode, word),
        0x2 | 0x3 => decode_cobr(opcode, word),
        0x5..=0x7 => decode_reg(opcode, word),
        0x8..=0xF => decode_mem(opcode, word),
        _ => Insn::Unimplemented { raw: word },
    }
}

// ---------------------------------------------------------------------------
// CTRL 解码
// ---------------------------------------------------------------------------

fn decode_ctrl(opcode: u8, word: u32) -> Insn {
    // displacement: bits [23:2]，符号扩展到 32 位，低 2 位补 0
    let raw_disp = (word & 0x00FF_FFFC) as i32;
    // 符号扩展（22位有效位）
    let disp = (raw_disp << 8) >> 8;

    match opcode {
        0x08 => Insn::B { disp },
        0x09 => Insn::Call { disp },
        0x0A => Insn::Ret,
        0x0B => Insn::Bal { disp },

        // 条件分支（0x10–0x1F）
        0x10 => Insn::Bno,
        0x11 => Insn::Bg { disp },
        0x12 => Insn::Be { disp },
        0x13 => Insn::Bge { disp },
        0x14 => Insn::Bl { disp },
        0x15 => Insn::Bne { disp },
        0x16 => Insn::Ble { disp },
        0x17 => Insn::Bo { disp },

        _ => Insn::Unimplemented { raw: word },
    }
}

// ---------------------------------------------------------------------------
// COBR 解码
// ---------------------------------------------------------------------------

#[rustfmt::skip]
fn decode_cobr(opcode: u8, word: u32) -> Insn {
    let src1 = ((word >> 19) & 0x1F) as usize;
    let src2 = ((word >> 14) & 0x1F) as usize;
    let raw_disp = (word & 0x0000_3FFC) as i32;
    // 13位符号扩展
    let disp = (raw_disp << 18) >> 18;

    match opcode {
        0x30 => Insn::Cmpob { src1, src2, disp, mask: 0b010 }, // cmpobe (equal)
        0x31 => Insn::Cmpob { src1, src2, disp, mask: 0b100 }, // cmpobg
        0x32 => Insn::Cmpob { src1, src2, disp, mask: 0b110 }, // cmpobge
        0x33 => Insn::Cmpob { src1, src2, disp, mask: 0b001 }, // cmpobl
        0x34 => Insn::Cmpob { src1, src2, disp, mask: 0b011 }, // cmpoble
        0x35 => Insn::Cmpob { src1, src2, disp, mask: 0b101 }, // cmpobne

        0x3A => Insn::Bbc   { bit_pos: src1, src: src2, disp },
        0x3B => Insn::Bbs   { bit_pos: src1, src: src2, disp },

        _    => Insn::Unimplemented { raw: word },
    }
}

// ---------------------------------------------------------------------------
// REG 解码
// ---------------------------------------------------------------------------
// TODO: Unused?
#[allow(dead_code)]
fn reg_src(word: u32, shift: u32, m_bit: bool) -> RegOperand {
    let idx = ((word >> shift) & 0x1F) as usize;
    if m_bit {
        RegOperand::Literal(idx as u32)
    } else {
        RegOperand::Reg(idx)
    }
}

/// REG 格式操作数：寄存器或立即数（0–31）。
#[derive(Debug, Clone, Copy)]
pub enum RegOperand {
    Reg(usize),
    Literal(u32),
}

impl RegOperand {
    /// 把立即数或寄存器编号都折叠成 usize（用于 `Insn` 字段）。
    pub fn as_reg_or_lit_idx(self) -> usize {
        match self {
            RegOperand::Reg(r) => r,
            RegOperand::Literal(l) => l as usize, // lit 0–31 存成伪寄存器编号
        }
    }
}

#[rustfmt::skip]
fn decode_reg(opcode: u8, word: u32) -> Insn {
    let dst = ((word >> 19) & 0x1F) as usize; // src/dst1
    let src2 = ((word >> 14) & 0x1F) as usize;
    let ext = ((word >> 10) & 0x0F) as u8; // opcode extension (4 bits, bits [13:10])
    // let m1 = (word >> 13) & 1 != 0; // src1 is literal
    let src1_raw = (word & 0x1F) as usize; // src3 / src1 in some encodings
    // M1 flag：当 m1=true，src2 字段是立即数
    let _src2_op = if (word >> 12) & 1 != 0 {
        RegOperand::Literal(src2 as u32)
    } else {
        RegOperand::Reg(src2)
    };

    // i960 REG 指令的完整 opcode 是 [31:24] + ext[12:10]
    // 常用编码（参照 Intel i960 Kx 手册 Table 8-3）
    let full = ((opcode as u16) << 4) | ext as u16;

    match full {
        // --- 数据移动 ---
        0x5CC => Insn::Mov { dst, src: src2 },

        // --- 整数运算 ---
        0x590 => Insn::Addo { dst, src1: src1_raw, src2 },
        0x591 => Insn::Add  { dst, src1: src1_raw, src2 },
        0x592 => Insn::Subo { dst, src1: src1_raw, src2 },
        0x593 => Insn::Sub  { dst, src1: src1_raw, src2 },
        0x598 => Insn::Mul  { dst, src1: src1_raw, src2 },
        0x59B => Insn::Divo { dst, src1: src1_raw, src2 },
        0x59A => Insn::Remo { dst, src1: src1_raw, src2 },

        // --- 位运算 ---
        0x580 => Insn::Nand { dst, src1: src1_raw, src2 },
        0x581 => Insn::And  { dst, src1: src1_raw, src2 },
        0x582 => Insn::Xnor { dst, src1: src1_raw, src2 },
        0x584 => Insn::Nor  { dst, src1: src1_raw, src2 },
        0x585 => Insn::Xor  { dst, src1: src1_raw, src2 },
        0x587 => Insn::Or   { dst, src1: src1_raw, src2 },
        0x58C => Insn::Not  { dst, src:  src2 },

        // --- 移位 ---
        0x59C => Insn::Shro { dst, src: src1_raw, cnt: src2 },
        0x59D => Insn::Shri { dst, src: src1_raw, cnt: src2 },
        0x59E => Insn::Shlo { dst, src: src1_raw, cnt: src2 },
        0x59F => Insn::Shli { dst, src: src1_raw, cnt: src2 },

        // --- 比较 ---
        0x5A0 => Insn::Cmpo { src1: src1_raw, src2 },
        0x5A1 => Insn::Cmpi { src1: src1_raw, src2 },

        // 立即数 load（opcode高字节 = 0x8C 为 ldconst，实际在 MEM 格式里）
        _ => Insn::Unimplemented { raw: word }
    }
}

// ---------------------------------------------------------------------------
// MEM 解码
// ---------------------------------------------------------------------------
#[rustfmt::skip]
fn decode_mem(opcode: u8, word: u32) -> Insn {
    let src_dst = ((word >> 19) & 0x1F) as usize;
    let abase = ((word >> 14) & 0x1F) as usize;
    let mode = (word >> 10) & 0xF; // bits [13:10]

    // MEM_A: offset 在 word[11:0]（12位无符号）
    // MEM_B: 紧跟的第二个 word 是 32 位位移（这里先只实现 MEM_A）
    let offset = if mode & 0x4 == 0 {
        // MEM_A：12 位立即偏移
        word & 0x0FFF
    } else {
        // MEM_B：暂时返回 0，execute 里需要再取一个 word
        0
    };

    match opcode {
        // LOAD
        0x80 => Insn::Ldob { dst: src_dst, abase, offset },
        0x82 => Insn::Ldos { dst: src_dst, abase, offset },
        0x84 => Insn::Ldib { dst: src_dst, abase, offset },
        0x86 => Insn::Ldis { dst: src_dst, abase, offset },
        0x88 => Insn::Ld   { dst: src_dst, abase, offset },

        // STORE
        0x8C => Insn::Stob { src: src_dst, abase, offset },
        0x8E => Insn::Stos { src: src_dst, abase, offset },
        0x92 => Insn::St   { src: src_dst, abase, offset },

        _    => Insn::Unimplemented { raw: word },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造 REG 格式指令字。
    /// opcode=[31:24], dst=[23:19], src2=[18:14], ext=[13:10], src1=[4:0]
    fn reg_word(opcode: u8, dst: usize, src2: usize, ext: u8, src1: usize) -> u32 {
        (opcode as u32) << 24
            | (dst as u32) << 19
            | (src2 as u32) << 14
            | (ext as u32) << 10
            | src1 as u32
    }

    /// 构造 COBR 格式指令字。
    /// opcode=[31:24], src1=[23:19], src2=[18:14], disp=[13:2]
    fn cobr_word(opcode: u8, src1: usize, src2: usize, disp: i32) -> u32 {
        (opcode as u32) << 24 | (src1 as u32) << 19 | (src2 as u32) << 14 | ((disp as u32) & 0x3FFC)
    }

    #[test]
    fn decode_b() {
        // B 指令：opcode=0x08，disp=+4（bits[23:2]=1 → disp=4）
        // word = 0x08000004
        let word: u32 = 0x0800_0004;
        match decode(word) {
            Insn::B { disp } => assert_eq!(disp, 4),
            other => panic!("expected B, got {:?}", other),
        }
    }

    #[test]
    fn decode_add() {
        // addo dst=g0(0), src1=g1(1), src2=g2(2)
        // opcode=0x59, ext=0x0 → full=0x590
        // word[31:24]=0x59  [23:19]=0(dst)  [18:14]=2(src2)  [13:10]=0(ext)  [4:0]=1(src1)
        let word: u32 = 0x59u32 << 24  // dst = g0
            | 2 << 14  // ext = 0
            | 1; // src1 = g1
        match decode(word) {
            Insn::Addo { dst, src1, src2 } => {
                assert_eq!(dst, 0);
                assert_eq!(src1, 1);
                assert_eq!(src2, 2);
            }
            other => panic!("expected Addo, got {:?}", other),
        }
    }

    // -----------------------------------------------------------------------
    // CTRL 格式全覆盖
    // -----------------------------------------------------------------------

    #[test]
    fn decode_ctrl_call_ret_bal() {
        // Call disp=8
        match decode(0x0900_0008) {
            Insn::Call { disp } => assert_eq!(disp, 8),
            other => panic!("expected Call, got {:?}", other),
        }
        // Ret（低位随意）
        match decode(0x0A00_0000) {
            Insn::Ret => {}
            other => panic!("expected Ret, got {:?}", other),
        }
        // Bal disp=16
        match decode(0x0B00_0010) {
            Insn::Bal { disp } => assert_eq!(disp, 16),
            other => panic!("expected Bal, got {:?}", other),
        }
    }

    #[test]
    fn decode_ctrl_conditional_branches() {
        struct Case {
            word: u32,
            check: fn(Insn) -> bool,
        }

        impl Case {
            fn new(word: u32, check: fn(Insn) -> bool) -> Self {
                Self { word, check }
            }
        }

        #[rustfmt::skip]
        let cases = &[
            Case::new(0x1000_0000, |i| matches!(i, Insn::Bno)),
            Case::new(0x1100_0008, |i| matches!(i, Insn::Bg { disp: 8 })),
            Case::new(0x1200_0008, |i| matches!(i, Insn::Be { disp: 8 })),
            Case::new(0x1300_0008, |i| matches!(i, Insn::Bge { disp: 8 })),
            Case::new(0x1400_0008, |i| matches!(i, Insn::Bl { disp: 8 })),
            Case::new(0x1500_0008, |i| matches!(i, Insn::Bne { disp: 8 })),
            Case::new(0x1600_0008, |i| matches!(i, Insn::Ble { disp: 8 })),
            Case::new(0x1700_0008, |i| matches!(i, Insn::Bo { disp: 8 })),
        ];
        for case in cases {
            let insn = decode(case.word);
            assert!((case.check)(insn), "word={:#010x}", case.word);
        }
    }

    /// B disp=0：跳回自身（无限循环）。
    #[test]
    fn decode_b_disp_zero() {
        match decode(0x0800_0000) {
            Insn::B { disp } => assert_eq!(disp, 0),
            other => panic!("expected B, got {:?}", other),
        }
    }

    // -----------------------------------------------------------------------
    // COBR 格式全覆盖
    // -----------------------------------------------------------------------

    #[test]
    fn decode_cobr_cmpob_all_masks() {
        // (opcode, expected_mask)
        let cases: &[(u8, u8)] = &[
            (0x30, 0b010), // cmpobe
            (0x31, 0b100), // cmpobg
            (0x32, 0b110), // cmpobge
            (0x33, 0b001), // cmpobl
            (0x34, 0b011), // cmpoble
            (0x35, 0b101), // cmpobne
        ];
        for &(opcode, expected_mask) in cases {
            let word = cobr_word(opcode, 1, 2, 8);
            match decode(word) {
                Insn::Cmpob {
                    src1,
                    src2,
                    disp,
                    mask,
                } => {
                    assert_eq!(src1, 1, "opcode={:#04x}", opcode);
                    assert_eq!(src2, 2, "opcode={:#04x}", opcode);
                    assert_eq!(disp, 8, "opcode={:#04x}", opcode);
                    assert_eq!(mask, expected_mask, "opcode={:#04x}", opcode);
                }
                other => panic!("opcode={:#04x}: expected Cmpob, got {:?}", opcode, other),
            }
        }
    }

    #[test]
    fn decode_cobr_bbc_bbs() {
        let word_bbc = cobr_word(0x3A, 3, 4, 12);
        match decode(word_bbc) {
            Insn::Bbc { bit_pos, src, disp } => {
                assert_eq!(bit_pos, 3);
                assert_eq!(src, 4);
                assert_eq!(disp, 12);
            }
            other => panic!("expected Bbc, got {:?}", other),
        }
        let word_bbs = cobr_word(0x3B, 5, 6, 8);
        match decode(word_bbs) {
            Insn::Bbs { bit_pos, src, disp } => {
                assert_eq!(bit_pos, 5);
                assert_eq!(src, 6);
                assert_eq!(disp, 8);
            }
            other => panic!("expected Bbs, got {:?}", other),
        }
    }

    // -----------------------------------------------------------------------
    // REG 格式全覆盖
    // -----------------------------------------------------------------------

    #[test]
    fn decode_reg_arithmetic() {
        // (ext, expected variant check)
        macro_rules! check_reg {
            ($opcode:expr, $ext:expr, $pat:pat) => {{
                let w = reg_word($opcode, 0, 2, $ext, 1);
                assert!(
                    matches!(decode(w), $pat),
                    "opcode={:#04x} ext={}: got {:?}",
                    $opcode,
                    $ext,
                    decode(w)
                );
            }};
        }
        check_reg!(0x59, 0, Insn::Addo { .. });
        check_reg!(0x59, 1, Insn::Add { .. });
        check_reg!(0x59, 2, Insn::Subo { .. });
        check_reg!(0x59, 3, Insn::Sub { .. });
        check_reg!(0x59, 8, Insn::Mul { .. });
        check_reg!(0x59, 0xA, Insn::Remo { .. });
        check_reg!(0x59, 0xB, Insn::Divo { .. });
    }

    #[test]
    fn decode_reg_bitops() {
        macro_rules! check_reg {
            ($opcode:expr, $ext:expr, $pat:pat) => {{
                let w = reg_word($opcode, 0, 2, $ext, 1);
                assert!(
                    matches!(decode(w), $pat),
                    "opcode={:#04x} ext={}",
                    $opcode,
                    $ext
                );
            }};
        }
        check_reg!(0x58, 0, Insn::Nand { .. });
        check_reg!(0x58, 1, Insn::And { .. });
        check_reg!(0x58, 2, Insn::Xnor { .. });
        check_reg!(0x58, 4, Insn::Nor { .. });
        check_reg!(0x58, 5, Insn::Xor { .. });
        check_reg!(0x58, 7, Insn::Or { .. });
        check_reg!(0x58, 0xC, Insn::Not { .. });
    }

    #[test]
    fn decode_reg_shifts() {
        macro_rules! check_reg {
            ($ext:expr, $pat:pat) => {{
                let w = reg_word(0x59, 0, 2, $ext, 1);
                assert!(matches!(decode(w), $pat), "ext={}", $ext);
            }};
        }
        check_reg!(0xC, Insn::Shro { .. });
        check_reg!(0xD, Insn::Shri { .. });
        check_reg!(0xE, Insn::Shlo { .. });
        check_reg!(0xF, Insn::Shli { .. });
    }

    #[test]
    fn decode_reg_compare_and_mov() {
        assert!(matches!(
            decode(reg_word(0x5A, 0, 2, 0, 1)),
            Insn::Cmpo { .. }
        ));
        assert!(matches!(
            decode(reg_word(0x5A, 0, 2, 1, 1)),
            Insn::Cmpi { .. }
        ));
        assert!(matches!(
            decode(reg_word(0x5C, 0, 2, 0xC, 0)),
            Insn::Mov { .. }
        ));
    }

    /// REG 指令各字段（dst/src1/src2）解码正确。
    #[test]
    fn decode_reg_field_values() {
        // addi dst=g3, src2=g4, src1=g5
        let w = reg_word(0x59, 3, 4, 1, 5);
        match decode(w) {
            Insn::Add { dst, src1, src2 } => {
                assert_eq!(dst, 3);
                assert_eq!(src1, 5);
                assert_eq!(src2, 4);
            }
            other => panic!("expected Add, got {:?}", other),
        }
    }

    // -----------------------------------------------------------------------
    // MEM 格式全覆盖
    // -----------------------------------------------------------------------

    /// 构造 MEM_A 格式指令字（mode[13:12]=0b00，12位偏移）。
    fn mem_word(opcode: u8, src_dst: usize, abase: usize, offset: u32) -> u32 {
        (opcode as u32) << 24 | (src_dst as u32) << 19 | (abase as u32) << 14 | (offset & 0x0FFF)
    }

    #[test]
    fn decode_mem_loads() {
        struct Case {
            opcode: u8,
            check: fn(Insn) -> bool,
        }

        impl Case {
            fn new(opcode: u8, check: fn(Insn) -> bool) -> Self {
                Self { opcode, check }
            }
        }

        #[rustfmt::skip]
        let cases = &[
            Case::new(0x80, |i| matches!(i, Insn::Ldob { dst: 1, abase: 2, offset: 0x10 })),
            Case::new(0x82, |i| matches!(i, Insn::Ldos { dst: 1, abase: 2, offset: 0x10 })),
            Case::new(0x84, |i| matches!(i, Insn::Ldib { dst: 1, abase: 2, offset: 0x10 })),
            Case::new(0x86, |i| matches!(i, Insn::Ldis { dst: 1, abase: 2, offset: 0x10 })),
            Case::new(0x88, |i| matches!(i, Insn::Ld { dst: 1, abase: 2, offset: 0x10 })),
        ];
        for case in cases {
            let w = mem_word(case.opcode, 1, 2, 0x10);
            assert!((case.check)(decode(w)), "opcode={:#04x}", case.opcode);
        }
    }

    #[test]
    fn decode_mem_stores() {
        struct Case {
            opcode: u8,
            check: fn(Insn) -> bool,
        }

        impl Case {
            fn new(opcode: u8, check: fn(Insn) -> bool) -> Self {
                Self { opcode, check }
            }
        }

        #[rustfmt::skip]
        let cases = &[
            Case::new(0x8C, |i| matches!(i, Insn::Stob { src: 1, abase: 2, offset: 0x20 })),
            Case::new(0x8E, |i| matches!(i, Insn::Stos { src: 1, abase: 2, offset: 0x20 })),
            Case::new(0x92, |i| matches!(i, Insn::St { src: 1, abase: 2, offset: 0x20 })),
        ];
        for case in cases {
            let w = mem_word(case.opcode, 1, 2, 0x20);
            assert!((case.check)(decode(w)), "opcode={:#04x}", case.opcode);
        }
    }

    // -----------------------------------------------------------------------
    // Unimplemented 兜底
    // -----------------------------------------------------------------------

    #[test]
    fn decode_unimplemented_opcode() {
        // opcode 0x4x 不在任何已知格式中
        let word: u32 = 0x4000_0000;
        assert!(matches!(decode(word), Insn::Unimplemented { .. }));
    }
}
