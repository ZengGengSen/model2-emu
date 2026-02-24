//! i960 反汇编器。
//!
//! 把解码后的 [`Insn`] 转为人类可读的汇编文本，格式尽量与 GNU binutils
//! 的 i960 输出接近，方便对照参考资料排查问题。
//!
//! 入口：[`disasm_one`]（单条）和 [`disasm_range`]（一段内存）。

use crate::decode::{Insn, decode};
use model2_core::Bus;

/// 寄存器名称（g0–g15 / r0–r15 / fp = g15 别名）。
fn reg_name(idx: usize) -> &'static str {
    const NAMES: [&str; 32] = [
        "g0", "g1", "g2", "g3", "g4", "g5", "g6", "g7", "g8", "g9", "g10", "g11", "g12", "g13",
        "g14", "fp", "r0", "r1", "r2", "r3", "r4", "r5", "r6", "r7", "r8", "r9", "r10", "r11",
        "r12", "r13", "r14", "r15",
    ];
    if idx < 32 { NAMES[idx] } else { "??" }
}

/// 把一条 [`Insn`] 格式化为汇编字符串。
///
/// `insn_addr` 用于把相对位移转换为绝对跳转目标地址。
pub fn format_insn(insn: &Insn, insn_addr: u32) -> String {
    use Insn::*;
    match *insn {
        // ---------------------------------------------------------------
        // CTRL
        // ---------------------------------------------------------------
        B { disp } => format!("b       {:#010x}", insn_addr.wrapping_add(disp as u32)),
        Bal { disp } => format!("bal     {:#010x}", insn_addr.wrapping_add(disp as u32)),
        Call { disp } => format!("call    {:#010x}", insn_addr.wrapping_add(disp as u32)),
        Ret => "ret".to_owned(),
        Bno => "bno".to_owned(),
        Bg { disp } => format!("bg      {:#010x}", insn_addr.wrapping_add(disp as u32)),
        Be { disp } => format!("be      {:#010x}", insn_addr.wrapping_add(disp as u32)),
        Bge { disp } => format!("bge     {:#010x}", insn_addr.wrapping_add(disp as u32)),
        Bl { disp } => format!("bl      {:#010x}", insn_addr.wrapping_add(disp as u32)),
        Bne { disp } => format!("bne     {:#010x}", insn_addr.wrapping_add(disp as u32)),
        Ble { disp } => format!("ble     {:#010x}", insn_addr.wrapping_add(disp as u32)),
        Bo { disp } => format!("bo      {:#010x}", insn_addr.wrapping_add(disp as u32)),

        // ---------------------------------------------------------------
        // COBR
        // ---------------------------------------------------------------
        Cmpob {
            src1,
            src2,
            disp,
            mask,
        } => {
            let mnemonic = match mask {
                0b010 => "cmpobe",
                0b100 => "cmpobg",
                0b110 => "cmpobge",
                0b001 => "cmpobl",
                0b011 => "cmpoble",
                0b101 => "cmpobne",
                _ => "cmpob?",
            };
            format!(
                "{:<8}{}, {}, {:#010x}",
                mnemonic,
                reg_name(src1),
                reg_name(src2),
                insn_addr.wrapping_add(disp as u32)
            )
        }
        Bbc { bit_pos, src, disp } => format!(
            "bbc     {}, {}, {:#010x}",
            reg_name(bit_pos),
            reg_name(src),
            insn_addr.wrapping_add(disp as u32)
        ),
        Bbs { bit_pos, src, disp } => format!(
            "bbs     {}, {}, {:#010x}",
            reg_name(bit_pos),
            reg_name(src),
            insn_addr.wrapping_add(disp as u32)
        ),

        // ---------------------------------------------------------------
        // REG：数据移动
        // ---------------------------------------------------------------
        Mov { dst, src } => format!("mov     {}, {}", reg_name(src), reg_name(dst)),
        LdLit { dst, lit } => format!("ldconst {:#x}, {}", lit, reg_name(dst)),

        // ---------------------------------------------------------------
        // REG：整数运算
        // ---------------------------------------------------------------
        Add { dst, src1, src2 } => fmt3("addi", src1, src2, dst),
        Addo { dst, src1, src2 } => fmt3("addo", src1, src2, dst),
        Sub { dst, src1, src2 } => fmt3("subi", src1, src2, dst),
        Subo { dst, src1, src2 } => fmt3("subo", src1, src2, dst),
        Mul { dst, src1, src2 } => fmt3("muli", src1, src2, dst),
        Divo { dst, src1, src2 } => fmt3("divo", src1, src2, dst),
        Remo { dst, src1, src2 } => fmt3("remo", src1, src2, dst),

        // ---------------------------------------------------------------
        // REG：位运算
        // ---------------------------------------------------------------
        And { dst, src1, src2 } => fmt3("and", src1, src2, dst),
        Or { dst, src1, src2 } => fmt3("or", src1, src2, dst),
        Xor { dst, src1, src2 } => fmt3("xor", src1, src2, dst),
        Nand { dst, src1, src2 } => fmt3("nand", src1, src2, dst),
        Nor { dst, src1, src2 } => fmt3("nor", src1, src2, dst),
        Xnor { dst, src1, src2 } => fmt3("xnor", src1, src2, dst),
        Not { dst, src } => format!("not     {}, {}", reg_name(src), reg_name(dst)),

        // ---------------------------------------------------------------
        // REG：移位
        // ---------------------------------------------------------------
        Shlo { dst, src, cnt } => fmt3("shlo", src, cnt, dst),
        Shro { dst, src, cnt } => fmt3("shro", src, cnt, dst),
        Shli { dst, src, cnt } => fmt3("shli", src, cnt, dst),
        Shri { dst, src, cnt } => fmt3("shri", src, cnt, dst),

        // ---------------------------------------------------------------
        // REG：比较
        // ---------------------------------------------------------------
        Cmpo { src1, src2 } => format!("cmpo    {}, {}", reg_name(src1), reg_name(src2)),
        Cmpi { src1, src2 } => format!("cmpi    {}, {}", reg_name(src1), reg_name(src2)),

        // ---------------------------------------------------------------
        // MEM：加载
        // ---------------------------------------------------------------
        Ld { dst, abase, offset } => fmt_mem("ld", abase, offset, dst, false),
        Ldob { dst, abase, offset } => fmt_mem("ldob", abase, offset, dst, false),
        Ldos { dst, abase, offset } => fmt_mem("ldos", abase, offset, dst, false),
        Ldib { dst, abase, offset } => fmt_mem("ldib", abase, offset, dst, false),
        Ldis { dst, abase, offset } => fmt_mem("ldis", abase, offset, dst, false),

        // ---------------------------------------------------------------
        // MEM：存储（src/dst 顺序相反）
        // ---------------------------------------------------------------
        St { src, abase, offset } => fmt_mem("st", abase, offset, src, true),
        Stob { src, abase, offset } => fmt_mem("stob", abase, offset, src, true),
        Stos { src, abase, offset } => fmt_mem("stos", abase, offset, src, true),

        // ---------------------------------------------------------------
        // 未知
        // ---------------------------------------------------------------
        Unimplemented { raw } => format!(".word   {:#010x}    ; UNIMPLEMENTED", raw),
    }
}

// ---------------------------------------------------------------------------
// 格式化辅助
// ---------------------------------------------------------------------------

/// 三操作数指令：`mnemonic src1, src2, dst`
fn fmt3(mnemonic: &str, src1: usize, src2: usize, dst: usize) -> String {
    format!(
        "{:<8}{}, {}, {}",
        mnemonic,
        reg_name(src1),
        reg_name(src2),
        reg_name(dst)
    )
}

/// 内存访问格式：`mnemonic offset(abase), reg` 或 `mnemonic reg, offset(abase)`（store）
fn fmt_mem(mnemonic: &str, abase: usize, offset: u32, reg: usize, is_store: bool) -> String {
    let mem = if offset == 0 {
        format!("({})", reg_name(abase))
    } else {
        format!("{:#x}({})", offset, reg_name(abase))
    };
    if is_store {
        format!("{:<8}{}, {}", mnemonic, reg_name(reg), mem)
    } else {
        format!("{:<8}{}, {}", mnemonic, mem, reg_name(reg))
    }
}

// ---------------------------------------------------------------------------
// 公开入口
// ---------------------------------------------------------------------------

/// 从总线读取并反汇编一条指令。
///
/// 返回 `(汇编文本, 指令长度字节数)`。
/// i960 基础指令均为 4 字节，MEM_B 格式为 8 字节（暂时不处理）。
pub fn disasm_one(addr: u32, bus: &Bus) -> (String, u32) {
    let word = bus.read_u32(addr);
    let insn = decode(word);
    let text = format_insn(&insn, addr);
    (text, 4)
}

/// 反汇编从 `start` 开始的 `count` 条指令，打印到 stdout。
pub fn disasm_range(start: u32, count: usize, bus: &Bus) {
    let mut addr = start;
    for _ in 0..count {
        let raw = bus.read_u32(addr);
        let insn = decode(raw);
        let text = format_insn(&insn, addr);
        println!("{:#010x}  {:08x}  {}", addr, raw, text);
        addr += 4;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use model2_core::mem::PAGE_SIZE;

    fn make_bus_with_rom(words: &[u32]) -> Bus {
        let mut bus = Bus::new();
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        // 补齐到页大小
        let mut padded = vec![0u8; PAGE_SIZE];
        let copy_len = bytes.len().min(PAGE_SIZE);
        padded[..copy_len].copy_from_slice(&bytes[..copy_len]);
        bus.load_rom(0x0080_0000, &padded);
        bus
    }

    #[test]
    fn disasm_b() {
        // B +8: word = 0x08000008
        let bus = make_bus_with_rom(&[0x0800_0008]);
        let (text, len) = disasm_one(0x0080_0000, &bus);
        assert_eq!(len, 4);
        assert!(text.contains("b"), "got: {}", text);
        assert!(text.contains("0x00800008"), "got: {}", text);
    }

    #[test]
    fn disasm_add() {
        // addo g1, g2, g0
        let word: u32 = 0x59u32 << 24 | 2 << 14 | 1;
        let bus = make_bus_with_rom(&[word]);
        let (text, _) = disasm_one(0x0080_0000, &bus);
        assert!(text.contains("addo"), "got: {}", text);
    }

    #[test]
    fn disasm_range_no_panic() {
        let bus = make_bus_with_rom(&[0x0800_0004, 0x5900_0001, 0x9200_0100]);
        disasm_range(0x0080_0000, 3, &bus);
    }

    // -----------------------------------------------------------------------
    // 新增综合测试
    // -----------------------------------------------------------------------

    /// format_insn 直接测试（绕过总线）。
    fn fmt(insn: Insn, addr: u32) -> String {
        format_insn(&insn, addr)
    }

    #[test]
    fn disasm_ctrl_mnemonics() {
        assert!(fmt(Insn::Ret, 0).contains("ret"));
        assert!(fmt(Insn::Bno, 0).contains("bno"));
        assert!(fmt(Insn::B   { disp: 8 }, 0x100).contains("0x00000108"));
        assert!(fmt(Insn::Call { disp: 0 }, 0x200).contains("call"));
        assert!(fmt(Insn::Bal  { disp: 4 }, 0x100).contains("bal"));
        assert!(fmt(Insn::Bg   { disp: 8 }, 0x100).starts_with("bg"));
        assert!(fmt(Insn::Be   { disp: 8 }, 0x100).starts_with("be"));
        assert!(fmt(Insn::Bge  { disp: 8 }, 0x100).starts_with("bge"));
        assert!(fmt(Insn::Bl   { disp: 8 }, 0x100).starts_with("bl"));
        assert!(fmt(Insn::Bne  { disp: 8 }, 0x100).starts_with("bne"));
        assert!(fmt(Insn::Ble  { disp: 8 }, 0x100).starts_with("ble"));
        assert!(fmt(Insn::Bo   { disp: 8 }, 0x100).starts_with("bo"));
    }

    #[test]
    fn disasm_cobr_mnemonics() {
        let masks: &[(&str, u8)] = &[
            ("cmpobe",  0b010),
            ("cmpobg",  0b100),
            ("cmpobge", 0b110),
            ("cmpobl",  0b001),
            ("cmpoble", 0b011),
            ("cmpobne", 0b101),
        ];
        for &(mnemonic, mask) in masks {
            let text = fmt(
                Insn::Cmpob { src1: 0, src2: 1, disp: 8, mask },
                0x100,
            );
            assert!(text.contains(mnemonic), "mask={:#05b}: got '{}'", mask, text);
        }
        // Bbc / Bbs
        let bbc = fmt(Insn::Bbc { bit_pos: 2, src: 3, disp: 8 }, 0x100);
        assert!(bbc.contains("bbc"), "got: {}", bbc);
        let bbs = fmt(Insn::Bbs { bit_pos: 2, src: 3, disp: 8 }, 0x100);
        assert!(bbs.contains("bbs"), "got: {}", bbs);
    }

    #[test]
    fn disasm_reg_mnemonics() {
        // 数据移动
        assert!(fmt(Insn::Mov   { dst: 0, src: 1 }, 0).contains("mov"));
        assert!(fmt(Insn::LdLit { dst: 0, lit: 5 }, 0).contains("ldconst"));

        // 整数运算
        assert!(fmt(Insn::Add  { dst: 0, src1: 1, src2: 2 }, 0).contains("addi"));
        assert!(fmt(Insn::Addo { dst: 0, src1: 1, src2: 2 }, 0).contains("addo"));
        assert!(fmt(Insn::Sub  { dst: 0, src1: 1, src2: 2 }, 0).contains("subi"));
        assert!(fmt(Insn::Subo { dst: 0, src1: 1, src2: 2 }, 0).contains("subo"));
        assert!(fmt(Insn::Mul  { dst: 0, src1: 1, src2: 2 }, 0).contains("muli"));
        assert!(fmt(Insn::Divo { dst: 0, src1: 1, src2: 2 }, 0).contains("divo"));
        assert!(fmt(Insn::Remo { dst: 0, src1: 1, src2: 2 }, 0).contains("remo"));

        // 位运算
        assert!(fmt(Insn::And  { dst: 0, src1: 1, src2: 2 }, 0).contains("and"));
        assert!(fmt(Insn::Or   { dst: 0, src1: 1, src2: 2 }, 0).contains("or"));
        assert!(fmt(Insn::Xor  { dst: 0, src1: 1, src2: 2 }, 0).contains("xor"));
        assert!(fmt(Insn::Nand { dst: 0, src1: 1, src2: 2 }, 0).contains("nand"));
        assert!(fmt(Insn::Nor  { dst: 0, src1: 1, src2: 2 }, 0).contains("nor"));
        assert!(fmt(Insn::Xnor { dst: 0, src1: 1, src2: 2 }, 0).contains("xnor"));
        assert!(fmt(Insn::Not  { dst: 0, src: 1 }, 0).contains("not"));

        // 移位
        assert!(fmt(Insn::Shlo { dst: 0, src: 1, cnt: 2 }, 0).contains("shlo"));
        assert!(fmt(Insn::Shro { dst: 0, src: 1, cnt: 2 }, 0).contains("shro"));
        assert!(fmt(Insn::Shli { dst: 0, src: 1, cnt: 2 }, 0).contains("shli"));
        assert!(fmt(Insn::Shri { dst: 0, src: 1, cnt: 2 }, 0).contains("shri"));

        // 比较
        assert!(fmt(Insn::Cmpo { src1: 1, src2: 2 }, 0).contains("cmpo"));
        assert!(fmt(Insn::Cmpi { src1: 1, src2: 2 }, 0).contains("cmpi"));
    }

    #[test]
    fn disasm_mem_mnemonics() {
        // load
        assert!(fmt(Insn::Ld   { dst: 0, abase: 1, offset: 0x10 }, 0).contains("ld"));
        assert!(fmt(Insn::Ldob { dst: 0, abase: 1, offset: 0 },    0).contains("ldob"));
        assert!(fmt(Insn::Ldos { dst: 0, abase: 1, offset: 0 },    0).contains("ldos"));
        assert!(fmt(Insn::Ldib { dst: 0, abase: 1, offset: 0 },    0).contains("ldib"));
        assert!(fmt(Insn::Ldis { dst: 0, abase: 1, offset: 0 },    0).contains("ldis"));

        // store（操作数顺序相反：寄存器先，内存后）
        let st = fmt(Insn::St   { src: 2, abase: 1, offset: 0x40 }, 0);
        assert!(st.contains("st"), "got: {}", st);
        assert!(st.contains("g2"), "got: {}", st);

        assert!(fmt(Insn::Stob { src: 2, abase: 1, offset: 0 }, 0).contains("stob"));
        assert!(fmt(Insn::Stos { src: 2, abase: 1, offset: 0 }, 0).contains("stos"));
    }

    /// MEM 格式：offset=0 时只打印 (abase)，否则打印 offset(abase)。
    #[test]
    fn disasm_mem_offset_format() {
        let no_offset = fmt(Insn::Ld { dst: 0, abase: 1, offset: 0 }, 0);
        assert!(no_offset.contains("(g1)"), "got: {}", no_offset);
        assert!(!no_offset.contains("0x0("), "got: {}", no_offset);

        let with_offset = fmt(Insn::Ld { dst: 0, abase: 1, offset: 0x10 }, 0);
        assert!(with_offset.contains("0x10(g1)"), "got: {}", with_offset);
    }

    /// 未实现指令输出 .word 标记。
    #[test]
    fn disasm_unimplemented() {
        let text = fmt(Insn::Unimplemented { raw: 0xDEAD_BEEF }, 0);
        assert!(text.contains(".word"), "got: {}", text);
        assert!(text.to_uppercase().contains("UNIMPLEMENTED"), "got: {}", text);
    }

    /// 反汇编结果长度始终为 4 字节。
    #[test]
    fn disasm_one_always_four_bytes() {
        let words = [0x0800_0000u32, 0x5900_0001, 0x9200_0100];
        let bus = make_bus_with_rom(&words);
        for (i, _) in words.iter().enumerate() {
            let addr = 0x0080_0000 + (i as u32) * 4;
            let (_, len) = disasm_one(addr, &bus);
            assert_eq!(len, 4, "addr={:#010x}", addr);
        }
    }
}
