//! 68000 反汇编器。
//!
//! 格式尽量接近 GNU binutils m68k 输出，便于与 objdump 交叉对照。

use crate::decode::{BitSrc, EA, Insn, ShiftCnt};
use crate::regs::Size;
use model2_core::Bus;

// ---------------------------------------------------------------------------
// EA 格式化
// ---------------------------------------------------------------------------

fn fmt_ea(ea: &EA, size: Size) -> String {
    match *ea {
        EA::DataReg(dn) => format!("%d{}", dn),
        EA::AddrReg(an) => format!("%a{}", an),
        EA::Indirect(an) => format!("%a{}@", an),
        EA::PostInc(an) => format!("%a{}@+", an),
        EA::PreDec(an) => format!("%a{}@-", an),
        EA::Disp16(an, d) => format!("%a{}@({})", an, d),
        EA::Index(an, xn, d) => format!("%a{}@({},%d{})", an, d, xn),
        EA::AbsShort(addr) => format!("{:#x}:w", addr),
        EA::AbsLong(addr) => format!("{:#010x}", addr),
        EA::PcDisp16(d) => format!("pc@({})", d),
        EA::Immediate(imm) => match size {
            Size::Byte => format!("#0x{:02x}", imm & 0xFF),
            Size::Word => format!("#0x{:04x}", imm & 0xFFFF),
            Size::Long => format!("#0x{:08x}", imm),
        },
    }
}

fn fmt_shift_cnt(cnt: &ShiftCnt) -> String {
    match *cnt {
        ShiftCnt::Imm(n) => format!("#{}", n),
        ShiftCnt::Reg(dn) => format!("%d{}", dn),
    }
}

fn fmt_bit_src(bit: &BitSrc) -> String {
    match *bit {
        BitSrc::Imm(n) => format!("#{}", n),
        BitSrc::Reg(dn) => format!("%d{}", dn),
    }
}

fn fmt_regmask(mask: u16) -> String {
    let mut parts = Vec::new();
    for i in 0..8u16 {
        if mask & (1 << i) != 0 {
            parts.push(format!("d{}", i));
        }
    }
    for i in 0..8u16 {
        if mask & (1 << (i + 8)) != 0 {
            parts.push(format!("a{}", i));
        }
    }
    parts.join("/")
}

// ---------------------------------------------------------------------------
// 主格式化函数
// ---------------------------------------------------------------------------

/// 把一条 [`Insn`] 格式化为汇编文本。
pub fn format_insn(insn: &Insn, insn_pc: u32) -> String {
    use Insn::*;

    match insn {
        // 数据移动
        Move { size, src, dst } => format!(
            "move.{}\t{},{}",
            size.suffix(),
            fmt_ea(src, *size),
            fmt_ea(dst, *size)
        ),
        Movea { size, src, an } => {
            format!("movea.{}\t{},%a{}", size.suffix(), fmt_ea(src, *size), an)
        }
        Moveq { dn, imm } => format!("moveq\t#{},%d{}", imm, dn),
        Lea { src, an } => format!("lea\t{},%a{}", fmt_ea(src, Size::Long), an),
        Pea { src } => format!("pea\t{}", fmt_ea(src, Size::Long)),
        Exg { rx, ry, .. } => format!("exg\t%d{},%d{}", rx, ry),

        // 整数
        Add { size, src, dst } => format!(
            "add.{}\t{},{}",
            size.suffix(),
            fmt_ea(src, *size),
            fmt_ea(dst, *size)
        ),
        Adda { size, src, an } => {
            format!("adda.{}\t{},%a{}", size.suffix(), fmt_ea(src, *size), an)
        }
        Addi { size, imm, dst } => {
            format!("addi.{}\t#{}*{}", size.suffix(), imm, fmt_ea(dst, *size))
        }
        Addq { size, imm, dst } => {
            format!("addq.{}\t#{}*{}", size.suffix(), imm, fmt_ea(dst, *size))
        }
        Sub { size, src, dst } => format!(
            "sub.{}\t{},{}",
            size.suffix(),
            fmt_ea(src, *size),
            fmt_ea(dst, *size)
        ),
        Suba { size, src, an } => {
            format!("suba.{}\t{},%a{}", size.suffix(), fmt_ea(src, *size), an)
        }
        Subi { size, imm, dst } => {
            format!("subi.{}\t#{},{}", size.suffix(), imm, fmt_ea(dst, *size))
        }
        Subq { size, imm, dst } => {
            format!("subq.{}\t#{},{}", size.suffix(), imm, fmt_ea(dst, *size))
        }
        Neg { size, dst } => format!("neg.{}\t{}", size.suffix(), fmt_ea(dst, *size)),
        Cmp { size, src, dn } => format!("cmp.{}\t{},%d{}", size.suffix(), fmt_ea(src, *size), dn),
        Cmpa { size, src, an } => {
            format!("cmpa.{}\t{},%a{}", size.suffix(), fmt_ea(src, *size), an)
        }
        Cmpi { size, imm, dst } => {
            format!("cmpi.{}\t#{},{}", size.suffix(), imm, fmt_ea(dst, *size))
        }
        Mulu { src, dn } => format!("mulu.w\t{},%d{}", fmt_ea(src, Size::Word), dn),
        Muls { src, dn } => format!("muls.w\t{},%d{}", fmt_ea(src, Size::Word), dn),
        Divu { src, dn } => format!("divu.w\t{},%d{}", fmt_ea(src, Size::Word), dn),
        Divs { src, dn } => format!("divs.w\t{},%d{}", fmt_ea(src, Size::Word), dn),

        // 逻辑
        And { size, src, dst } => format!(
            "and.{}\t{},{}",
            size.suffix(),
            fmt_ea(src, *size),
            fmt_ea(dst, *size)
        ),
        Andi { size, imm, dst } => {
            format!("andi.{}\t#{},{}", size.suffix(), imm, fmt_ea(dst, *size))
        }
        Or { size, src, dst } => format!(
            "or.{}\t{},{}",
            size.suffix(),
            fmt_ea(src, *size),
            fmt_ea(dst, *size)
        ),
        Ori { size, imm, dst } => format!("ori.{}\t#{},{}", size.suffix(), imm, fmt_ea(dst, *size)),
        Eor { size, dn, dst } => format!("eor.{}\t%d{},{}", size.suffix(), dn, fmt_ea(dst, *size)),
        Eori { size, imm, dst } => {
            format!("eori.{}\t#{},{}", size.suffix(), imm, fmt_ea(dst, *size))
        }
        Not { size, dst } => format!("not.{}\t{}", size.suffix(), fmt_ea(dst, *size)),

        // 移位
        Lsl { size, cnt, dst } => format!(
            "lsl.{}\t{},{}",
            size.suffix(),
            fmt_shift_cnt(cnt),
            fmt_ea(dst, *size)
        ),
        Lsr { size, cnt, dst } => format!(
            "lsr.{}\t{},{}",
            size.suffix(),
            fmt_shift_cnt(cnt),
            fmt_ea(dst, *size)
        ),
        Asl { size, cnt, dst } => format!(
            "asl.{}\t{},{}",
            size.suffix(),
            fmt_shift_cnt(cnt),
            fmt_ea(dst, *size)
        ),
        Asr { size, cnt, dst } => format!(
            "asr.{}\t{},{}",
            size.suffix(),
            fmt_shift_cnt(cnt),
            fmt_ea(dst, *size)
        ),
        Rol { size, cnt, dst } => format!(
            "rol.{}\t{},{}",
            size.suffix(),
            fmt_shift_cnt(cnt),
            fmt_ea(dst, *size)
        ),
        Ror { size, cnt, dst } => format!(
            "ror.{}\t{},{}",
            size.suffix(),
            fmt_shift_cnt(cnt),
            fmt_ea(dst, *size)
        ),

        // 位操作
        Btst { bit, dst } => format!("btst\t{},{}", fmt_bit_src(bit), fmt_ea(dst, Size::Byte)),
        Bset { bit, dst } => format!("bset\t{},{}", fmt_bit_src(bit), fmt_ea(dst, Size::Byte)),
        Bclr { bit, dst } => format!("bclr\t{},{}", fmt_bit_src(bit), fmt_ea(dst, Size::Byte)),
        Bchg { bit, dst } => format!("bchg\t{},{}", fmt_bit_src(bit), fmt_ea(dst, Size::Byte)),

        // 分支
        Bra { disp } => format!(
            "bra\t{:#010x}",
            insn_pc.wrapping_add(2).wrapping_add(*disp as i32 as u32)
        ),
        Bsr { disp } => format!(
            "bsr\t{:#010x}",
            insn_pc.wrapping_add(2).wrapping_add(*disp as i32 as u32)
        ),
        Bcc { cond, disp } => format!(
            "b{}\t{:#010x}",
            cond.mnemonic(),
            insn_pc.wrapping_add(2).wrapping_add(*disp as i32 as u32)
        ),
        Dbcc { cond, dn, disp } => format!(
            "db{}\t%d{},{:#010x}",
            cond.mnemonic(),
            dn,
            insn_pc.wrapping_add(2).wrapping_add(*disp as i32 as u32)
        ),
        Jmp { dst } => format!("jmp\t{}", fmt_ea(dst, Size::Long)),
        Jsr { dst } => format!("jsr\t{}", fmt_ea(dst, Size::Long)),
        Rts => "rts".to_owned(),
        Rte => "rte".to_owned(),

        // MOVEM
        Movem {
            size,
            reg_mask,
            ea,
            to_mem,
        } => {
            if *to_mem {
                format!(
                    "movem.{}\t{},{}",
                    size.suffix(),
                    fmt_regmask(*reg_mask),
                    fmt_ea(ea, *size)
                )
            } else {
                format!(
                    "movem.{}\t{},{}",
                    size.suffix(),
                    fmt_ea(ea, *size),
                    fmt_regmask(*reg_mask)
                )
            }
        }

        // 杂项
        Nop => "nop".to_owned(),
        Clr { size, dst } => format!("clr.{}\t{}", size.suffix(), fmt_ea(dst, *size)),
        Tst { size, src } => format!("tst.{}\t{}", size.suffix(), fmt_ea(src, *size)),
        Ext { size, dn } => format!("ext.{}\t%d{}", size.suffix(), dn),
        Swap { dn } => format!("swap\t%d{}", dn),
        Trap { vector } => format!("trap\t#{}", vector),
        Unimplemented { opcode } => format!(".word\t{:#06x}\t; UNIMPLEMENTED", opcode),
    }
}

// ---------------------------------------------------------------------------
// 公开入口
// ---------------------------------------------------------------------------

/// 从总线读取并反汇编一条 68000 指令（大端读取）。
///
/// 返回 `(汇编文本, 指令字节长度)`。
pub fn disasm_one(addr: u32, bus: &Bus) -> (String, u32) {
    // 预读最多 12 字节（6个 u16），足以覆盖最长指令
    let words: Vec<u16> = (0..6)
        .map(|i| {
            let hi = bus.read_u8(addr + i * 2) as u16;
            let lo = bus.read_u8(addr + i * 2 + 1) as u16;
            (hi << 8) | lo
        })
        .collect();

    let (insn, len) = crate::decode::decode(&words);
    let text = format_insn(&insn, addr);
    (text, len)
}

/// 反汇编从 `start` 开始的 `count` 条指令，打印到 stdout。
pub fn disasm_range(start: u32, count: usize, bus: &Bus) {
    let mut addr = start;
    for _ in 0..count {
        let words: Vec<u16> = (0..6)
            .map(|i| {
                let hi = bus.read_u8(addr + i * 2) as u16;
                let lo = bus.read_u8(addr + i * 2 + 1) as u16;
                (hi << 8) | lo
            })
            .collect();

        let (insn, len) = crate::decode::decode(&words);
        let text = format_insn(&insn, addr);

        // 打印原始字节（最多 len 字节）
        let raw_hex: String = (0..len)
            .map(|i| format!("{:02x}", bus.read_u8(addr + i)))
            .collect::<Vec<_>>()
            .join(" ");
        println!("{:#010x}  {:<20}  {}", addr, raw_hex, text);

        addr += len;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_nop() {
        let insn = Insn::Nop;
        assert_eq!(format_insn(&insn, 0), "nop");
    }

    #[test]
    fn format_moveq() {
        let insn = Insn::Moveq { dn: 3, imm: -5 };
        let s = format_insn(&insn, 0);
        assert!(s.contains("moveq") && s.contains("%d3"));
    }

    #[test]
    fn format_bra() {
        let insn = Insn::Bra { disp: 10 };
        let s = format_insn(&insn, 0x1000);
        // target = 0x1000 + 2 + 10 = 0x100c
        assert!(s.contains("0x0000100c"), "got: {}", s);
    }
}
