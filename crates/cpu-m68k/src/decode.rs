//! 68000 指令解码。
//!
//! # 68000 指令格式特点
//!
//! 与 i960 的 32 位定长不同，68000 是**变长指令**：
//! - 主操作码字：16 位
//! - 后面可跟 0–5 个 16 位扩展字（立即数、位移、绝对地址等）
//!
//! 解码必须从总线连续读取，并追踪 PC 偏移。
//!
//! # 寻址模式（Effective Address, EA）
//!
//! 68000 的强大之处在于正交的 EA 系统，几乎所有指令都可以用任意 EA 组合。
//!
//! | mode | reg | 含义                          |
//! |------|-----|-------------------------------|
//! | 000  | Dn  | 数据寄存器直接                 |
//! | 001  | An  | 地址寄存器直接                 |
//! | 010  | An  | 地址寄存器间接 (An)            |
//! | 011  | An  | 地址寄存器间接后自增 (An)+     |
//! | 100  | An  | 地址寄存器间接预自减 -(An)     |
//! | 101  | An  | 地址寄存器间接+位移 d16(An)   |
//! | 110  | An  | 地址寄存器间接+变址            |
//! | 111  | 000 | 绝对短地址 xxx.W              |
//! | 111  | 001 | 绝对长地址 xxx.L              |
//! | 111  | 010 | PC相对+位移 d16(PC)           |
//! | 111  | 011 | PC相对+变址                   |
//! | 111  | 100 | 立即数 #imm                   |

use crate::regs::Size;

/// 有效地址（EA）。
#[derive(Debug, Clone, Copy)]
pub enum EA {
    /// 数据寄存器 Dn（直接）。
    DataReg(usize),
    /// 地址寄存器 An（直接，只用于地址操作）。
    AddrReg(usize),
    /// (An) — 地址寄存器间接。
    Indirect(usize),
    /// (An)+ — 后自增。
    PostInc(usize),
    /// -(An) — 预自减。
    PreDec(usize),
    /// d16(An) — 16位有符号位移。
    Disp16(usize, i16),
    /// d8(An, Xn) — 变址，暂简化（不含完整Brief Extension Word）。
    Index(usize, usize, i8),
    /// xxx.W — 绝对短地址（16位零扩展到32位）。
    AbsShort(u32),
    /// xxx.L — 绝对长地址。
    AbsLong(u32),
    /// d16(PC)。
    PcDisp16(i16),
    /// #imm — 立即数。
    Immediate(u32),
}

/// 条件码枚举（68000 Bcc/Scc/DBcc 用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cond {
    T,  // always true
    F,  // always false
    Hi, // higher (unsigned >)
    Ls, // lower or same
    Cc, // carry clear (C=0)
    Cs, // carry set (C=1)
    Ne, // not equal
    Eq, // equal
    Vc, // overflow clear
    Vs, // overflow set
    Pl, // plus (N=0)
    Mi, // minus (N=1)
    Ge, // greater or equal (signed)
    Lt, // less than (signed)
    Gt, // greater than (signed)
    Le, // less or equal (signed)
}

impl Cond {
    pub fn from_bits(v: u8) -> Self {
        match v & 0xF {
            0x0 => Cond::T,
            0x1 => Cond::F,
            0x2 => Cond::Hi,
            0x3 => Cond::Ls,
            0x4 => Cond::Cc,
            0x5 => Cond::Cs,
            0x6 => Cond::Ne,
            0x7 => Cond::Eq,
            0x8 => Cond::Vc,
            0x9 => Cond::Vs,
            0xA => Cond::Pl,
            0xB => Cond::Mi,
            0xC => Cond::Ge,
            0xD => Cond::Lt,
            0xE => Cond::Gt,
            _ => Cond::Le,
        }
    }

    pub fn mnemonic(self) -> &'static str {
        match self {
            Cond::T => "t",
            Cond::F => "f",
            Cond::Hi => "hi",
            Cond::Ls => "ls",
            Cond::Cc => "cc",
            Cond::Cs => "cs",
            Cond::Ne => "ne",
            Cond::Eq => "eq",
            Cond::Vc => "vc",
            Cond::Vs => "vs",
            Cond::Pl => "pl",
            Cond::Mi => "mi",
            Cond::Ge => "ge",
            Cond::Lt => "lt",
            Cond::Gt => "gt",
            Cond::Le => "le",
        }
    }
}

// ---------------------------------------------------------------------------
// 解码上下文（管理 PC 推进和扩展字读取）
// ---------------------------------------------------------------------------

/// 解码时从内存顺序读取指令字的辅助结构。
pub struct Decoder<'a> {
    pub data: &'a [u16], // 从当前 PC 开始的指令流（预读）
    pub pos: usize,      // 当前偏移（单位：u16）
}

impl<'a> Decoder<'a> {
    pub fn new(data: &'a [u16]) -> Self {
        Self { data, pos: 0 }
    }

    /// 读取下一个 16 位字并推进。
    pub fn next_word(&mut self) -> u16 {
        let w = self.data[self.pos];
        self.pos += 1;
        w
    }

    /// 读取下一个 32 位双字（两个连续 16 位字，大端）。
    pub fn next_long(&mut self) -> u32 {
        let hi = self.next_word() as u32;
        let lo = self.next_word() as u32;
        (hi << 16) | lo
    }

    /// 读取 EA 扩展字（如果需要）。
    pub fn read_ea(&mut self, mode: u8, reg: usize, size: Size) -> EA {
        match mode {
            0b000 => EA::DataReg(reg),
            0b001 => EA::AddrReg(reg),
            0b010 => EA::Indirect(reg),
            0b011 => EA::PostInc(reg),
            0b100 => EA::PreDec(reg),
            0b101 => {
                let d = self.next_word() as i16;
                EA::Disp16(reg, d)
            }
            0b110 => {
                // Brief Extension Word
                let ext = self.next_word();
                let xn = ((ext >> 12) & 7) as usize;
                let d8 = ext as i8;
                EA::Index(reg, xn, d8)
            }
            0b111 => match reg {
                0 => {
                    let addr = self.next_word() as i16 as i32 as u32;
                    EA::AbsShort(addr)
                }
                1 => {
                    let addr = self.next_long();
                    EA::AbsLong(addr)
                }
                2 => {
                    let d = self.next_word() as i16;
                    EA::PcDisp16(d)
                }
                4 => {
                    let imm = match size {
                        Size::Byte => self.next_word() as u32 & 0xFF,
                        Size::Word => self.next_word() as u32,
                        Size::Long => self.next_long(),
                    };
                    EA::Immediate(imm)
                }
                _ => EA::AbsLong(0), // 其他模式暂不支持
            },
            _ => EA::Immediate(0),
        }
    }

    /// 已消耗的字节数。
    pub fn bytes_consumed(&self) -> u32 {
        (self.pos * 2) as u32
    }
}

// ---------------------------------------------------------------------------
// 解码后的指令
// ---------------------------------------------------------------------------

/// 解码后的 68000 指令。
#[derive(Debug, Clone)]
pub enum Insn {
    // -----------------------------------------------------------------------
    // 数据移动
    // -----------------------------------------------------------------------
    /// MOVE src, dst
    Move {
        size: Size,
        src: EA,
        dst: EA,
    },
    /// MOVEA src, An（地址寄存器赋值，总是符号扩展到32位）
    Movea {
        size: Size,
        src: EA,
        an: usize,
    },
    /// MOVEQ #imm8, Dn（快速8位立即数 → 32位）
    Moveq {
        dn: usize,
        imm: i8,
    },
    /// LEA ea, An
    Lea {
        src: EA,
        an: usize,
    },
    /// PEA ea（push effective address）
    Pea {
        src: EA,
    },
    /// EXG Rx, Ry
    Exg {
        rx: usize,
        ry: usize,
        mode: u8,
    },

    // -----------------------------------------------------------------------
    // 整数运算
    // -----------------------------------------------------------------------
    Add {
        size: Size,
        src: EA,
        dst: EA,
    },
    Adda {
        size: Size,
        src: EA,
        an: usize,
    },
    Addi {
        size: Size,
        imm: u32,
        dst: EA,
    },
    Addq {
        size: Size,
        imm: u8,
        dst: EA,
    }, // imm: 1–8
    Sub {
        size: Size,
        src: EA,
        dst: EA,
    },
    Suba {
        size: Size,
        src: EA,
        an: usize,
    },
    Subi {
        size: Size,
        imm: u32,
        dst: EA,
    },
    Subq {
        size: Size,
        imm: u8,
        dst: EA,
    },
    Neg {
        size: Size,
        dst: EA,
    },
    Cmp {
        size: Size,
        src: EA,
        dn: usize,
    },
    Cmpa {
        size: Size,
        src: EA,
        an: usize,
    },
    Cmpi {
        size: Size,
        imm: u32,
        dst: EA,
    },
    Mulu {
        src: EA,
        dn: usize,
    }, // multiply unsigned (.w)
    Muls {
        src: EA,
        dn: usize,
    }, // multiply signed (.w)
    Divu {
        src: EA,
        dn: usize,
    },
    Divs {
        src: EA,
        dn: usize,
    },

    // -----------------------------------------------------------------------
    // 逻辑运算
    // -----------------------------------------------------------------------
    And {
        size: Size,
        src: EA,
        dst: EA,
    },
    Andi {
        size: Size,
        imm: u32,
        dst: EA,
    },
    Or {
        size: Size,
        src: EA,
        dst: EA,
    },
    Ori {
        size: Size,
        imm: u32,
        dst: EA,
    },
    Eor {
        size: Size,
        dn: usize,
        dst: EA,
    },
    Eori {
        size: Size,
        imm: u32,
        dst: EA,
    },
    Not {
        size: Size,
        dst: EA,
    },

    // -----------------------------------------------------------------------
    // 移位 / 旋转
    // -----------------------------------------------------------------------
    Lsl {
        size: Size,
        cnt: ShiftCnt,
        dst: EA,
    },
    Lsr {
        size: Size,
        cnt: ShiftCnt,
        dst: EA,
    },
    Asl {
        size: Size,
        cnt: ShiftCnt,
        dst: EA,
    },
    Asr {
        size: Size,
        cnt: ShiftCnt,
        dst: EA,
    },
    Rol {
        size: Size,
        cnt: ShiftCnt,
        dst: EA,
    },
    Ror {
        size: Size,
        cnt: ShiftCnt,
        dst: EA,
    },

    // -----------------------------------------------------------------------
    // 位操作
    // -----------------------------------------------------------------------
    Btst {
        bit: BitSrc,
        dst: EA,
    },
    Bset {
        bit: BitSrc,
        dst: EA,
    },
    Bclr {
        bit: BitSrc,
        dst: EA,
    },
    Bchg {
        bit: BitSrc,
        dst: EA,
    },

    // -----------------------------------------------------------------------
    // 分支 / 跳转
    // -----------------------------------------------------------------------
    Bra {
        disp: i16,
    },
    Bsr {
        disp: i16,
    },
    Bcc {
        cond: Cond,
        disp: i16,
    },
    Dbcc {
        cond: Cond,
        dn: usize,
        disp: i16,
    },
    Jmp {
        dst: EA,
    },
    Jsr {
        dst: EA,
    },
    Rts,
    Rte,

    // -----------------------------------------------------------------------
    // 栈操作
    // -----------------------------------------------------------------------
    /// MOVEM regs, ea  or  MOVEM ea, regs
    Movem {
        size: Size,
        reg_mask: u16,
        ea: EA,
        to_mem: bool,
    },

    // -----------------------------------------------------------------------
    // 杂项
    // -----------------------------------------------------------------------
    Nop,
    Clr {
        size: Size,
        dst: EA,
    },
    Tst {
        size: Size,
        src: EA,
    },
    Ext {
        size: Size,
        dn: usize,
    }, // 符号扩展
    Swap {
        dn: usize,
    },
    Trap {
        vector: u8,
    },
    /// 未实现（存原始操作码）。
    Unimplemented {
        opcode: u16,
    },
}

/// 移位计数来源：立即数（1–8）或数据寄存器。
#[derive(Debug, Clone, Copy)]
pub enum ShiftCnt {
    Imm(u8),    // 1–8
    Reg(usize), // Dn
}

/// 位操作的位号来源。
#[derive(Debug, Clone, Copy)]
pub enum BitSrc {
    Imm(u8),
    Reg(usize),
}

// ---------------------------------------------------------------------------
// 顶层解码函数
// ---------------------------------------------------------------------------

/// 解码从 `words` 开始的一条 68000 指令。
///
/// 返回 `(Insn, 指令字节长度)`。
pub fn decode(words: &[u16]) -> (Insn, u32) {
    let mut dec = Decoder::new(words);
    let opcode = dec.next_word();
    let insn = decode_opcode(opcode, &mut dec);
    (insn, dec.bytes_consumed())
}

fn decode_opcode(op: u16, dec: &mut Decoder) -> Insn {
    // 68000 指令由操作码最高 4 位（[15:12]）决定大类
    match op >> 12 {
        0x0 => decode_bit_movep_imm(op, dec),
        0x1 => decode_move(op, dec, Size::Byte),
        0x2 => decode_move(op, dec, Size::Long),
        0x3 => decode_move(op, dec, Size::Word),
        0x4 => decode_misc(op, dec),
        0x5 => decode_addq_subq_scc_dbcc(op, dec),
        0x6 => decode_bcc_bsr_bra(op, dec),
        0x7 => {
            // MOVEQ
            let dn = ((op >> 9) & 7) as usize;
            let imm = op as i8;
            Insn::Moveq { dn, imm }
        }
        0x8 => decode_or_div(op, dec),
        0x9 => decode_sub_subx(op, dec),
        0xB => decode_cmp_eor(op, dec),
        0xC => decode_and_mul_exg(op, dec),
        0xD => decode_add_addx(op, dec),
        0xE => decode_shift(op, dec),
        _ => Insn::Unimplemented { opcode: op },
    }
}

// ---------------------------------------------------------------------------
// 大小编码（bits [7:6]）
// ---------------------------------------------------------------------------

fn size_from_bits(bits: u8) -> Option<Size> {
    match bits {
        0b00 => Some(Size::Byte),
        0b01 => Some(Size::Word),
        0b10 => Some(Size::Long),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Bit/MOVEP/Immediate  (group 0x0)
// ---------------------------------------------------------------------------

fn decode_bit_movep_imm(op: u16, dec: &mut Decoder) -> Insn {
    let dn_or_type = (op >> 9) & 7;
    let size_bits = ((op >> 6) & 3) as u8;
    let mode = ((op >> 3) & 7) as u8;
    let reg = (op & 7) as usize;

    // 位操作（动态：bit号来自Dn）
    if op & 0x0100 != 0 {
        let bit = BitSrc::Reg(dn_or_type as usize);
        let ea = dec.read_ea(mode, reg, Size::Byte);
        return match (op >> 6) & 3 {
            0 => Insn::Btst { bit, dst: ea },
            1 => Insn::Bchg { bit, dst: ea },
            2 => Insn::Bclr { bit, dst: ea },
            _ => Insn::Bset { bit, dst: ea },
        };
    }

    // 立即数操作
    let ea_mode = mode;
    let ea_reg = reg;

    match dn_or_type {
        0b000 => {
            // ORI
            if let Some(size) = size_from_bits(size_bits) {
                let imm = read_imm(dec, size);
                let dst = dec.read_ea(ea_mode, ea_reg, size);
                Insn::Ori { size, imm, dst }
            } else {
                Insn::Unimplemented { opcode: op }
            }
        }
        0b001 => {
            // ANDI
            if let Some(size) = size_from_bits(size_bits) {
                let imm = read_imm(dec, size);
                let dst = dec.read_ea(ea_mode, ea_reg, size);
                Insn::Andi { size, imm, dst }
            } else {
                Insn::Unimplemented { opcode: op }
            }
        }
        0b010 => {
            // SUBI
            if let Some(size) = size_from_bits(size_bits) {
                let imm = read_imm(dec, size);
                let dst = dec.read_ea(ea_mode, ea_reg, size);
                Insn::Subi { size, imm, dst }
            } else {
                Insn::Unimplemented { opcode: op }
            }
        }
        0b011 => {
            // ADDI
            if let Some(size) = size_from_bits(size_bits) {
                let imm = read_imm(dec, size);
                let dst = dec.read_ea(ea_mode, ea_reg, size);
                Insn::Addi { size, imm, dst }
            } else {
                Insn::Unimplemented { opcode: op }
            }
        }
        0b100 => {
            // BTST/BCHG/BCLR/BSET（静态，立即数位号）
            let bit_n = (dec.next_word() & 0x1F) as u8;
            let bit = BitSrc::Imm(bit_n);
            let dst = dec.read_ea(ea_mode, ea_reg, Size::Byte);
            match size_bits {
                0 => Insn::Btst { bit, dst },
                1 => Insn::Bchg { bit, dst },
                2 => Insn::Bclr { bit, dst },
                _ => Insn::Bset { bit, dst },
            }
        }
        0b101 => {
            // EORI
            if let Some(size) = size_from_bits(size_bits) {
                let imm = read_imm(dec, size);
                let dst = dec.read_ea(ea_mode, ea_reg, size);
                Insn::Eori { size, imm, dst }
            } else {
                Insn::Unimplemented { opcode: op }
            }
        }
        0b110 => {
            // CMPI
            if let Some(size) = size_from_bits(size_bits) {
                let imm = read_imm(dec, size);
                let dst = dec.read_ea(ea_mode, ea_reg, size);
                Insn::Cmpi { size, imm, dst }
            } else {
                Insn::Unimplemented { opcode: op }
            }
        }
        _ => Insn::Unimplemented { opcode: op },
    }
}

/// 从解码流读取立即数（按宽度）。
fn read_imm(dec: &mut Decoder, size: Size) -> u32 {
    match size {
        Size::Byte => dec.next_word() as u32 & 0xFF,
        Size::Word => dec.next_word() as u32,
        Size::Long => dec.next_long(),
    }
}

// ---------------------------------------------------------------------------
// MOVE  (groups 0x1/0x2/0x3)
// ---------------------------------------------------------------------------

fn decode_move(op: u16, dec: &mut Decoder, size: Size) -> Insn {
    // dst EA: bits[11:6]（注意：mode 和 reg 顺序与 src 相反）
    let dst_reg = ((op >> 9) & 7) as usize;
    let dst_mode = ((op >> 6) & 7) as u8;
    let src_mode = ((op >> 3) & 7) as u8;
    let src_reg = (op & 7) as usize;

    let src = dec.read_ea(src_mode, src_reg, size);

    // MOVEA：dst_mode == 001
    if dst_mode == 0b001 {
        return Insn::Movea {
            size,
            src,
            an: dst_reg,
        };
    }

    let dst = dec.read_ea(dst_mode, dst_reg, size);
    Insn::Move { size, src, dst }
}

// ---------------------------------------------------------------------------
// Misc  (group 0x4)
// ---------------------------------------------------------------------------

fn decode_misc(op: u16, dec: &mut Decoder) -> Insn {
    let mode = ((op >> 3) & 7) as u8;
    let reg = (op & 7) as usize;
    let size_bits = ((op >> 6) & 3) as u8;

    match (op >> 8) & 0xFF {
        0x40..=0x47 => {
            // NEGX / NEG / NOT / CLR
            if let Some(size) = size_from_bits(size_bits) {
                let dst = dec.read_ea(mode, reg, size);
                match (op >> 9) & 3 {
                    1 => Insn::Clr { size, dst },
                    2 => Insn::Neg { size, dst },
                    3 => Insn::Not { size, dst },
                    _ => Insn::Unimplemented { opcode: op },
                }
            } else {
                Insn::Unimplemented { opcode: op }
            }
        }

        0x4A => {
            // TST
            if let Some(size) = size_from_bits(size_bits) {
                let src = dec.read_ea(mode, reg, size);
                Insn::Tst { size, src }
            } else {
                Insn::Unimplemented { opcode: op }
            }
        }

        0x48 => {
            // EXT / MOVEM（reg→mem）/ PEA / SWAP
            match op & 0xFF {
                0x80..=0x87 => Insn::Swap { dn: reg },
                0x40 | 0x48 | 0xC0 => {
                    // EXT Dn
                    let size = if op & 0x40 != 0 {
                        Size::Long
                    } else {
                        Size::Word
                    };
                    Insn::Ext { size, dn: reg }
                }
                _ => {
                    // MOVEM reg-list → ea
                    if op & 0x0040 != 0 {
                        let mask = dec.next_word();
                        let ea = dec.read_ea(mode, reg, Size::Word);
                        Insn::Movem {
                            size: Size::Word,
                            reg_mask: mask,
                            ea,
                            to_mem: true,
                        }
                    } else {
                        Insn::Unimplemented { opcode: op }
                    }
                }
            }
        }

        0x49 => {
            // MOVEM.L reg→ea  or EXT.L
            if op & 0x0080 != 0 {
                let mask = dec.next_word();
                let ea = dec.read_ea(mode, reg, Size::Long);
                Insn::Movem {
                    size: Size::Long,
                    reg_mask: mask,
                    ea,
                    to_mem: true,
                }
            } else {
                Insn::Ext {
                    size: Size::Long,
                    dn: reg,
                }
            }
        }

        0x4C => {
            // MOVEM ea→reg
            let size_bit = (op >> 6) & 1;
            let size = if size_bit == 0 {
                Size::Word
            } else {
                Size::Long
            };
            let mask = dec.next_word();
            let ea = dec.read_ea(mode, reg, size);
            Insn::Movem {
                size,
                reg_mask: mask,
                ea,
                to_mem: false,
            }
        }

        0x4E => {
            match op & 0xFF {
                0x71 => Insn::Nop,
                0x73 => Insn::Rte,
                0x75 => Insn::Rts,
                0x76 => Insn::Trap {
                    vector: (op & 0xF) as u8,
                },
                _ if op & 0xF0 == 0x40 => Insn::Trap {
                    vector: (op & 0xF) as u8,
                },
                _ if op & 0xC0 == 0x80 => {
                    // JSR
                    let ea = dec.read_ea(mode, reg, Size::Long);
                    Insn::Jsr { dst: ea }
                }
                _ if op & 0xC0 == 0xC0 => {
                    // JMP
                    let ea = dec.read_ea(mode, reg, Size::Long);
                    Insn::Jmp { dst: ea }
                }
                _ => Insn::Unimplemented { opcode: op },
            }
        }

        _ if (op >> 6) & 3 == 3 => {
            // LEA
            let an = ((op >> 9) & 7) as usize;
            let src = dec.read_ea(mode, reg, Size::Long);
            Insn::Lea { src, an }
        }

        _ => Insn::Unimplemented { opcode: op },
    }
}

// ---------------------------------------------------------------------------
// ADDQ/SUBQ/Scc/DBcc  (group 0x5)
// ---------------------------------------------------------------------------

fn decode_addq_subq_scc_dbcc(op: u16, dec: &mut Decoder) -> Insn {
    let imm_raw = ((op >> 9) & 7) as u8;
    let imm = if imm_raw == 0 { 8 } else { imm_raw }; // 0 编码为8
    let size_bits = ((op >> 6) & 3) as u8;
    let mode = ((op >> 3) & 7) as u8;
    let reg = (op & 7) as usize;

    if size_bits == 0b11 {
        // Scc / DBcc
        let cond = Cond::from_bits(((op >> 8) & 0xF) as u8);
        if mode == 0b001 {
            // DBcc Dn, d16
            let disp = dec.next_word() as i16;
            Insn::Dbcc {
                cond,
                dn: reg,
                disp,
            }
        } else {
            Insn::Unimplemented { opcode: op } // Scc（暂不实现）
        }
    } else if let Some(size) = size_from_bits(size_bits) {
        let dst = dec.read_ea(mode, reg, size);
        if op & 0x0100 != 0 {
            Insn::Subq { size, imm, dst }
        } else {
            Insn::Addq { size, imm, dst }
        }
    } else {
        Insn::Unimplemented { opcode: op }
    }
}

// ---------------------------------------------------------------------------
// Bcc/BSR/BRA  (group 0x6)
// ---------------------------------------------------------------------------

fn decode_bcc_bsr_bra(op: u16, dec: &mut Decoder) -> Insn {
    let cond_bits = ((op >> 8) & 0xF) as u8;
    let disp8 = (op & 0xFF) as i8;

    // 8位位移为0时，紧跟一个16位位移字
    let disp = if disp8 == 0 {
        dec.next_word() as i16
    } else {
        disp8 as i16
    };

    match cond_bits {
        0x0 => Insn::Bra { disp },
        0x1 => Insn::Bsr { disp },
        _ => Insn::Bcc {
            cond: Cond::from_bits(cond_bits),
            disp,
        },
    }
}

// ---------------------------------------------------------------------------
// OR/DIV  (group 0x8)
// ---------------------------------------------------------------------------

fn decode_or_div(op: u16, dec: &mut Decoder) -> Insn {
    let dn = ((op >> 9) & 7) as usize;
    let size_bits = ((op >> 6) & 3) as u8;
    let mode = ((op >> 3) & 7) as u8;
    let reg = (op & 7) as usize;

    match size_bits {
        0b11 => {
            let src = dec.read_ea(mode, reg, Size::Word);
            if op & 0x0100 != 0 {
                Insn::Divs { src, dn }
            } else {
                Insn::Divu { src, dn }
            }
        }
        _ => {
            if let Some(size) = size_from_bits(size_bits) {
                let ea = dec.read_ea(mode, reg, size);
                // 方向位：bit 8
                let (src, dst) = if op & 0x0100 != 0 {
                    (EA::DataReg(dn), ea)
                } else {
                    (ea, EA::DataReg(dn))
                };
                Insn::Or { size, src, dst }
            } else {
                Insn::Unimplemented { opcode: op }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// SUB/SUBX  (group 0x9)
// ---------------------------------------------------------------------------

fn decode_sub_subx(op: u16, dec: &mut Decoder) -> Insn {
    let dn = ((op >> 9) & 7) as usize;
    let size_bits = ((op >> 6) & 3) as u8;
    let mode = ((op >> 3) & 7) as u8;
    let reg = (op & 7) as usize;

    if size_bits == 0b11 {
        let size = if op & 0x0100 != 0 {
            Size::Long
        } else {
            Size::Word
        };
        let src = dec.read_ea(mode, reg, size);
        return Insn::Suba { size, src, an: dn };
    }

    if let Some(size) = size_from_bits(size_bits) {
        let ea = dec.read_ea(mode, reg, size);
        if op & 0x0100 != 0 {
            Insn::Sub {
                size,
                src: EA::DataReg(dn),
                dst: ea,
            }
        } else {
            Insn::Sub {
                size,
                src: ea,
                dst: EA::DataReg(dn),
            }
        }
    } else {
        Insn::Unimplemented { opcode: op }
    }
}

// ---------------------------------------------------------------------------
// CMP/EOR  (group 0xB)
// ---------------------------------------------------------------------------

fn decode_cmp_eor(op: u16, dec: &mut Decoder) -> Insn {
    let dn = ((op >> 9) & 7) as usize;
    let opmode = ((op >> 6) & 7) as u8;
    let mode = ((op >> 3) & 7) as u8;
    let reg = (op & 7) as usize;

    match opmode {
        0b000 => {
            let src = dec.read_ea(mode, reg, Size::Byte);
            Insn::Cmp {
                size: Size::Byte,
                src,
                dn,
            }
        }
        0b001 => {
            let src = dec.read_ea(mode, reg, Size::Word);
            Insn::Cmp {
                size: Size::Word,
                src,
                dn,
            }
        }
        0b010 => {
            let src = dec.read_ea(mode, reg, Size::Long);
            Insn::Cmp {
                size: Size::Long,
                src,
                dn,
            }
        }
        0b011 => {
            let src = dec.read_ea(mode, reg, Size::Word);
            Insn::Cmpa {
                size: Size::Word,
                src,
                an: dn,
            }
        }
        0b111 => {
            let src = dec.read_ea(mode, reg, Size::Long);
            Insn::Cmpa {
                size: Size::Long,
                src,
                an: dn,
            }
        }
        // EOR Dn, dst
        0b100 => {
            let dst = dec.read_ea(mode, reg, Size::Byte);
            Insn::Eor {
                size: Size::Byte,
                dn,
                dst,
            }
        }
        0b101 => {
            let dst = dec.read_ea(mode, reg, Size::Word);
            Insn::Eor {
                size: Size::Word,
                dn,
                dst,
            }
        }
        0b110 => {
            let dst = dec.read_ea(mode, reg, Size::Long);
            Insn::Eor {
                size: Size::Long,
                dn,
                dst,
            }
        }
        _ => Insn::Unimplemented { opcode: op },
    }
}

// ---------------------------------------------------------------------------
// AND/MUL/EXG  (group 0xC)
// ---------------------------------------------------------------------------

fn decode_and_mul_exg(op: u16, dec: &mut Decoder) -> Insn {
    let dn = ((op >> 9) & 7) as usize;
    let size_bits = ((op >> 6) & 3) as u8;
    let mode = ((op >> 3) & 7) as u8;
    let reg = (op & 7) as usize;

    match size_bits {
        0b11 => {
            // MULU/MULS
            let src = dec.read_ea(mode, reg, Size::Word);
            if op & 0x0100 != 0 {
                Insn::Muls { src, dn }
            } else {
                Insn::Mulu { src, dn }
            }
        }
        _ if op & 0x0100 != 0 && size_bits == 0b01 => {
            // EXG
            Insn::Exg {
                rx: dn,
                ry: reg,
                mode: ((op >> 3) & 0x1F) as u8,
            }
        }
        _ => {
            if let Some(size) = size_from_bits(size_bits) {
                let ea = dec.read_ea(mode, reg, size);
                if op & 0x0100 != 0 {
                    Insn::And {
                        size,
                        src: EA::DataReg(dn),
                        dst: ea,
                    }
                } else {
                    Insn::And {
                        size,
                        src: ea,
                        dst: EA::DataReg(dn),
                    }
                }
            } else {
                Insn::Unimplemented { opcode: op }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// ADD/ADDX  (group 0xD)
// ---------------------------------------------------------------------------

fn decode_add_addx(op: u16, dec: &mut Decoder) -> Insn {
    let dn = ((op >> 9) & 7) as usize;
    let size_bits = ((op >> 6) & 3) as u8;
    let mode = ((op >> 3) & 7) as u8;
    let reg = (op & 7) as usize;

    if size_bits == 0b11 {
        let size = if op & 0x0100 != 0 {
            Size::Long
        } else {
            Size::Word
        };
        let src = dec.read_ea(mode, reg, size);
        return Insn::Adda { size, src, an: dn };
    }

    if let Some(size) = size_from_bits(size_bits) {
        let ea = dec.read_ea(mode, reg, size);
        if op & 0x0100 != 0 {
            Insn::Add {
                size,
                src: EA::DataReg(dn),
                dst: ea,
            }
        } else {
            Insn::Add {
                size,
                src: ea,
                dst: EA::DataReg(dn),
            }
        }
    } else {
        Insn::Unimplemented { opcode: op }
    }
}

// ---------------------------------------------------------------------------
// 移位 / 旋转  (group 0xE)
// ---------------------------------------------------------------------------

fn decode_shift(op: u16, dec: &mut Decoder) -> Insn {
    let cnt_or_dn = ((op >> 9) & 7) as usize;
    let size_bits = ((op >> 6) & 3) as u8;
    let ir = (op >> 5) & 1 != 0; // 0=立即数, 1=寄存器
    let kind = ((op >> 3) & 3) as u8;
    let reg = (op & 7) as usize;

    let cnt = if ir {
        ShiftCnt::Reg(cnt_or_dn)
    } else {
        let n = if cnt_or_dn == 0 { 8 } else { cnt_or_dn as u8 };
        ShiftCnt::Imm(n)
    };

    if let Some(size) = size_from_bits(size_bits) {
        let dst = EA::DataReg(reg);
        let dir_left = (op >> 8) & 1 != 0;
        match kind {
            0 => {
                if dir_left {
                    Insn::Asl { size, cnt, dst }
                } else {
                    Insn::Asr { size, cnt, dst }
                }
            }
            1 => {
                if dir_left {
                    Insn::Lsl { size, cnt, dst }
                } else {
                    Insn::Lsr { size, cnt, dst }
                }
            }
            3 => {
                if dir_left {
                    Insn::Rol { size, cnt, dst }
                } else {
                    Insn::Ror { size, cnt, dst }
                }
            }
            _ => Insn::Unimplemented { opcode: op },
        }
    } else {
        // 内存移位（size_bits=11，cnt=1，单独编码）
        let mode = ((op >> 3) & 7) as u8;
        let ea = dec.read_ea(mode, reg, Size::Word);
        let dir_left = (op >> 8) & 1 != 0;
        match (op >> 9) & 3 {
            0 => {
                if dir_left {
                    Insn::Asl {
                        size: Size::Word,
                        cnt: ShiftCnt::Imm(1),
                        dst: ea,
                    }
                } else {
                    Insn::Asr {
                        size: Size::Word,
                        cnt: ShiftCnt::Imm(1),
                        dst: ea,
                    }
                }
            }
            1 => {
                if dir_left {
                    Insn::Lsl {
                        size: Size::Word,
                        cnt: ShiftCnt::Imm(1),
                        dst: ea,
                    }
                } else {
                    Insn::Lsr {
                        size: Size::Word,
                        cnt: ShiftCnt::Imm(1),
                        dst: ea,
                    }
                }
            }
            _ => Insn::Unimplemented { opcode: op },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_nop() {
        let words = [0x4E71u16];
        let (insn, len) = decode(&words);
        assert!(matches!(insn, Insn::Nop));
        assert_eq!(len, 2);
    }

    #[test]
    fn decode_moveq() {
        // MOVEQ #42, D0  → 0x703A
        let words = [0x703Au16];
        let (insn, len) = decode(&words);
        match insn {
            Insn::Moveq { dn, imm } => {
                assert_eq!(dn, 0);
                assert_eq!(imm, 42);
            }
            other => panic!("expected Moveq, got {:?}", other),
        }
        assert_eq!(len, 2);
    }

    #[test]
    fn decode_bra() {
        // BRA +10: 0x6000 0x000A
        let words = [0x6000u16, 0x000A];
        let (insn, len) = decode(&words);
        match insn {
            Insn::Bra { disp } => assert_eq!(disp, 10),
            other => panic!("expected Bra, got {:?}", other),
        }
        assert_eq!(len, 4);
    }

    #[test]
    fn decode_move_l_d0_d1() {
        // MOVE.L D0, D1 → 0x2200
        let words = [0x2200u16];
        let (insn, len) = decode(&words);
        match insn {
            Insn::Move {
                size: Size::Long, ..
            } => {}
            other => panic!("expected Move.L, got {:?}", other),
        }
        assert_eq!(len, 2);
    }

    #[test]
    fn decode_rts() {
        let words = [0x4E75u16];
        let (insn, _) = decode(&words);
        assert!(matches!(insn, Insn::Rts));
    }
}
