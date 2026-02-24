//! 68000 指令执行。
//!
//! # EA 求值
//!
//! 每条内存访问指令都需要先把 EA 解析成实际地址，才能读写总线。
//! [`ea_read`] 和 [`ea_write`] 封装了这个过程，并处理自增/自减副作用。

use log::{trace, warn};

use model2_core::Bus;

use crate::decode::{BitSrc, Cond, EA, Insn, ShiftCnt};
use crate::regs::{Ccr, Regs, Size};

// ---------------------------------------------------------------------------
// EA 求值（计算有效地址，不读写内存）
// ---------------------------------------------------------------------------

/// 把 EA 解析成物理地址（仅对内存类 EA 有意义）。
/// 对于 PreDec，先减后返回地址；对于 PostInc，返回当前地址（调用方负责事后自增）。
pub fn ea_addr(ea: &EA, regs: &mut Regs, size: Size) -> u32 {
    match *ea {
        EA::Indirect(an) => regs.read_a(an),
        EA::PostInc(an) => regs.read_a(an), // 调用方事后自增
        EA::PreDec(an) => {
            let new = regs.read_a(an).wrapping_sub(size.bytes());
            regs.write_a(an, new);
            new
        }
        EA::Disp16(an, d) => regs.read_a(an).wrapping_add(d as i32 as u32),
        EA::Index(an, xn, d) => regs
            .read_a(an)
            .wrapping_add(regs.d[xn])
            .wrapping_add(d as i32 as u32),
        EA::AbsShort(addr) => addr,
        EA::AbsLong(addr) => addr,
        EA::PcDisp16(d) => regs.pc.wrapping_add(d as i32 as u32),
        _ => 0, // DataReg/AddrReg/Immediate 不是内存地址
    }
}

/// 从 EA 读取值（零扩展到 u32）。
pub fn ea_read(ea: &EA, regs: &mut Regs, bus: &Bus, size: Size) -> u32 {
    match *ea {
        EA::DataReg(dn) => regs.read_d(dn, size),
        EA::AddrReg(an) => regs.read_a(an),
        EA::Immediate(imm) => imm,
        EA::PostInc(an) => {
            let addr = regs.read_a(an);
            let val = mem_read(bus, addr, size);
            regs.write_a(an, addr.wrapping_add(size.bytes()));
            val
        }
        _ => {
            let addr = ea_addr(ea, regs, size);
            mem_read(bus, addr, size)
        }
    }
}

/// 向 EA 写入值。
pub fn ea_write(ea: &EA, regs: &mut Regs, bus: &mut Bus, size: Size, val: u32) {
    match *ea {
        EA::DataReg(dn) => regs.write_d(dn, val, size),
        EA::AddrReg(an) => regs.write_a(an, val),
        EA::PostInc(an) => {
            let addr = regs.read_a(an);
            mem_write(bus, addr, size, val);
            regs.write_a(an, addr.wrapping_add(size.bytes()));
        }
        _ => {
            let addr = ea_addr(ea, regs, size);
            mem_write(bus, addr, size, val);
        }
    }
}

fn mem_read(bus: &Bus, addr: u32, size: Size) -> u32 {
    match size {
        Size::Byte => bus.read_u8(addr) as u32,
        Size::Word => bus.read_u16(addr) as u32,
        Size::Long => bus.read_u32(addr),
    }
}

fn mem_write(bus: &mut Bus, addr: u32, size: Size, val: u32) {
    match size {
        Size::Byte => bus.write_u8(addr, val as u8),
        Size::Word => bus.write_u16(addr, val as u16),
        Size::Long => bus.write_u32(addr, val),
    }
}

// ---------------------------------------------------------------------------
// 条件码求值
// ---------------------------------------------------------------------------

fn eval_cond(cond: Cond, ccr: &Ccr) -> bool {
    match cond {
        Cond::T => true,
        Cond::F => false,
        Cond::Hi => !ccr.c && !ccr.z,
        Cond::Ls => ccr.c || ccr.z,
        Cond::Cc => !ccr.c,
        Cond::Cs => ccr.c,
        Cond::Ne => !ccr.z,
        Cond::Eq => ccr.z,
        Cond::Vc => !ccr.v,
        Cond::Vs => ccr.v,
        Cond::Pl => !ccr.n,
        Cond::Mi => ccr.n,
        Cond::Ge => ccr.n == ccr.v,
        Cond::Lt => ccr.n != ccr.v,
        Cond::Gt => !ccr.z && (ccr.n == ccr.v),
        Cond::Le => ccr.z || (ccr.n != ccr.v),
    }
}

// ---------------------------------------------------------------------------
// 栈操作辅助
// ---------------------------------------------------------------------------

fn push_u32(regs: &mut Regs, bus: &mut Bus, val: u32) {
    let sp = regs.sp().wrapping_sub(4);
    regs.set_sp(sp);
    bus.write_u32(sp, val);
}

fn pop_u32(regs: &mut Regs, bus: &Bus) -> u32 {
    let sp = regs.sp();
    let val = bus.read_u32(sp);
    regs.set_sp(sp.wrapping_add(4));
    val
}

// ---------------------------------------------------------------------------
// 主执行函数
// ---------------------------------------------------------------------------

/// 执行一条已解码的 68000 指令。
///
/// 返回消耗的时钟周期数（近似值）。
pub fn execute(insn: &Insn, insn_pc: u32, regs: &mut Regs, bus: &mut Bus) -> u32 {
    match insn {
        // ---------------------------------------------------------------
        // 数据移动
        // ---------------------------------------------------------------
        Insn::Move { size, src, dst } => {
            let val = ea_read(src, regs, bus, *size);
            regs.update_ccr(|ccr| {
                ccr.update_nz(val, *size);
                ccr.v = false;
                ccr.c = false;
            });
            ea_write(dst, regs, bus, *size, val);
            trace!("MOVE.{} → {:#x}", size.suffix(), val);
            4
        }

        Insn::Movea { size, src, an } => {
            let raw = ea_read(src, regs, bus, *size);
            // MOVEA 符号扩展到 32 位，不修改 CCR
            let val = match size {
                Size::Word => raw as i16 as i32 as u32,
                _ => raw,
            };
            regs.write_a(*an, val);
            4
        }

        Insn::Moveq { dn, imm } => {
            let val = *imm as i32 as u32;
            regs.write_d(*dn, val, Size::Long);
            regs.update_ccr(|ccr| {
                ccr.update_nz(val, Size::Long);
                ccr.v = false;
                ccr.c = false;
            });
            4
        }

        Insn::Lea { src, an } => {
            let addr = ea_addr(src, regs, Size::Long);
            regs.write_a(*an, addr);
            4
        }

        Insn::Pea { src } => {
            let addr = ea_addr(src, regs, Size::Long);
            push_u32(regs, bus, addr);
            6
        }

        Insn::Exg { rx, ry, .. } => {
            regs.d.swap(*rx, *ry);
            6
        }

        // ---------------------------------------------------------------
        // 整数运算
        // ---------------------------------------------------------------
        Insn::Add { size, src, dst } => {
            let a = ea_read(src, regs, bus, *size);
            let b = ea_read(dst, regs, bus, *size);
            let res = add_with_flags(a, b, *size, regs);
            ea_write(dst, regs, bus, *size, res);
            4
        }

        Insn::Adda { size, src, an } => {
            let val = ea_read(src, regs, bus, *size);
            let ext = match size {
                Size::Word => val as i16 as i32 as u32,
                _ => val,
            };
            let new = regs.read_a(*an).wrapping_add(ext);
            regs.write_a(*an, new);
            // ADDA 不修改 CCR
            6
        }

        Insn::Addi { size, imm, dst } => {
            let b = ea_read(dst, regs, bus, *size);
            let res = add_with_flags(*imm, b, *size, regs);
            ea_write(dst, regs, bus, *size, res);
            8
        }

        Insn::Addq { size, imm, dst } => {
            let b = ea_read(dst, regs, bus, *size);
            let res = add_with_flags(*imm as u32, b, *size, regs);
            ea_write(dst, regs, bus, *size, res);
            4
        }

        Insn::Sub { size, src, dst } => {
            let a = ea_read(dst, regs, bus, *size); // dst 是被减数
            let b = ea_read(src, regs, bus, *size);
            let res = sub_with_flags(a, b, *size, regs);
            ea_write(dst, regs, bus, *size, res);
            4
        }

        Insn::Suba { size, src, an } => {
            let val = ea_read(src, regs, bus, *size);
            let ext = match size {
                Size::Word => val as i16 as i32 as u32,
                _ => val,
            };
            let new = regs.read_a(*an).wrapping_sub(ext);
            regs.write_a(*an, new);
            6
        }

        Insn::Subi { size, imm, dst } => {
            let a = ea_read(dst, regs, bus, *size);
            let res = sub_with_flags(a, *imm, *size, regs);
            ea_write(dst, regs, bus, *size, res);
            8
        }

        Insn::Subq { size, imm, dst } => {
            let a = ea_read(dst, regs, bus, *size);
            let res = sub_with_flags(a, *imm as u32, *size, regs);
            ea_write(dst, regs, bus, *size, res);
            4
        }

        Insn::Neg { size, dst } => {
            let val = ea_read(dst, regs, bus, *size);
            let res = sub_with_flags(0, val, *size, regs);
            ea_write(dst, regs, bus, *size, res);
            4
        }

        Insn::Cmp { size, src, dn } => {
            let a = regs.read_d(*dn, *size);
            let b = ea_read(src, regs, bus, *size);
            sub_with_flags(a, b, *size, regs);
            4
        }

        Insn::Cmpa { size, src, an } => {
            let a = regs.read_a(*an);
            let raw = ea_read(src, regs, bus, *size);
            let b = match size {
                Size::Word => raw as i16 as i32 as u32,
                _ => raw,
            };
            sub_with_flags(a, b, Size::Long, regs);
            6
        }

        Insn::Cmpi { size, imm, dst } => {
            let a = ea_read(dst, regs, bus, *size);
            sub_with_flags(a, *imm, *size, regs);
            8
        }

        Insn::Mulu { src, dn } => {
            let a = regs.read_d(*dn, Size::Word);
            let b = ea_read(src, regs, bus, Size::Word);
            let res = a * b;
            regs.write_d(*dn, res, Size::Long);
            regs.update_ccr(|ccr| {
                ccr.update_nz(res, Size::Long);
                ccr.v = false;
                ccr.c = false;
            });
            54 // 68000 MULU 最多 70 周期，近似取中
        }

        Insn::Muls { src, dn } => {
            let a = regs.read_d(*dn, Size::Word) as i16 as i32;
            let b = ea_read(src, regs, bus, Size::Word) as i16 as i32;
            let res = (a * b) as u32;
            regs.write_d(*dn, res, Size::Long);
            regs.update_ccr(|ccr| {
                ccr.update_nz(res, Size::Long);
                ccr.v = false;
                ccr.c = false;
            });
            54
        }

        Insn::Divu { src, dn } => {
            let divisor = ea_read(src, regs, bus, Size::Word) as u16;
            if divisor == 0 {
                warn!("DIVU by zero @ {:#010x}", insn_pc);
                // TODO: division by zero trap
            } else {
                let dividend = regs.read_d(*dn, Size::Long);
                let quot = dividend / divisor as u32;
                let rem = dividend % divisor as u32;
                if quot > 0xFFFF {
                    regs.update_ccr(|ccr| {
                        ccr.v = true;
                    });
                } else {
                    regs.write_d(*dn, (rem << 16) | (quot & 0xFFFF), Size::Long);
                    regs.update_ccr(|ccr| {
                        ccr.update_nz(quot, Size::Word);
                        ccr.v = false;
                        ccr.c = false;
                    });
                }
            }
            140
        }

        Insn::Divs { src, dn } => {
            let divisor = ea_read(src, regs, bus, Size::Word) as i16;
            if divisor == 0 {
                warn!("DIVS by zero @ {:#010x}", insn_pc);
            } else {
                let dividend = regs.read_d(*dn, Size::Long) as i32;
                let quot = dividend / divisor as i32;
                let rem = dividend % divisor as i32;
                if !(-32768..=32767).contains(&quot) {
                    regs.update_ccr(|ccr| {
                        ccr.v = true;
                    });
                } else {
                    regs.write_d(*dn, ((rem as u32) << 16) | (quot as u16 as u32), Size::Long);
                    regs.update_ccr(|ccr| {
                        ccr.update_nz(quot as u32, Size::Word);
                        ccr.v = false;
                        ccr.c = false;
                    });
                }
            }
            158
        }

        // ---------------------------------------------------------------
        // 逻辑运算
        // ---------------------------------------------------------------
        Insn::And { size, src, dst } => {
            let a = ea_read(src, regs, bus, *size);
            let b = ea_read(dst, regs, bus, *size);
            let r = a & b;
            logic_flags(r, *size, regs);
            ea_write(dst, regs, bus, *size, r);
            4
        }
        Insn::Andi { size, imm, dst } => {
            let b = ea_read(dst, regs, bus, *size);
            let r = imm & b;
            logic_flags(r, *size, regs);
            ea_write(dst, regs, bus, *size, r);
            8
        }
        Insn::Or { size, src, dst } => {
            let a = ea_read(src, regs, bus, *size);
            let b = ea_read(dst, regs, bus, *size);
            let r = a | b;
            logic_flags(r, *size, regs);
            ea_write(dst, regs, bus, *size, r);
            4
        }
        Insn::Ori { size, imm, dst } => {
            let b = ea_read(dst, regs, bus, *size);
            let r = imm | b;
            logic_flags(r, *size, regs);
            ea_write(dst, regs, bus, *size, r);
            8
        }
        Insn::Eor { size, dn, dst } => {
            let a = regs.read_d(*dn, *size);
            let b = ea_read(dst, regs, bus, *size);
            let r = a ^ b;
            logic_flags(r, *size, regs);
            ea_write(dst, regs, bus, *size, r);
            4
        }
        Insn::Eori { size, imm, dst } => {
            let b = ea_read(dst, regs, bus, *size);
            let r = imm ^ b;
            logic_flags(r, *size, regs);
            ea_write(dst, regs, bus, *size, r);
            8
        }
        Insn::Not { size, dst } => {
            let v = ea_read(dst, regs, bus, *size);
            let r = !v & size_mask(*size);
            logic_flags(r, *size, regs);
            ea_write(dst, regs, bus, *size, r);
            4
        }

        // ---------------------------------------------------------------
        // 移位 / 旋转
        // ---------------------------------------------------------------
        Insn::Lsl { size, cnt, dst } | Insn::Asl { size, cnt, dst } => {
            let shift = shift_count(cnt, regs);
            let val = ea_read(dst, regs, bus, *size);
            let (res, last_bit) = shift_left(val, shift, *size);
            shift_flags(res, last_bit, *size, false, regs);
            ea_write(dst, regs, bus, *size, res);
            4 + 2 * shift as u32
        }
        Insn::Lsr { size, cnt, dst } => {
            let shift = shift_count(cnt, regs);
            let val = ea_read(dst, regs, bus, *size);
            let (res, last_bit) = shift_right_logical(val, shift, *size);
            shift_flags(res, last_bit, *size, false, regs);
            ea_write(dst, regs, bus, *size, res);
            4 + 2 * shift as u32
        }
        Insn::Asr { size, cnt, dst } => {
            let shift = shift_count(cnt, regs);
            let val = ea_read(dst, regs, bus, *size);
            let (res, last_bit) = shift_right_arith(val, shift, *size);
            shift_flags(res, last_bit, *size, false, regs);
            ea_write(dst, regs, bus, *size, res);
            4 + 2 * shift as u32
        }
        Insn::Rol { size, cnt, dst } => {
            let shift = shift_count(cnt, regs);
            let val = ea_read(dst, regs, bus, *size);
            let (res, carry) = rotate_left(val, shift, *size);
            shift_flags(res, carry, *size, false, regs);
            ea_write(dst, regs, bus, *size, res);
            4 + 2 * shift as u32
        }
        Insn::Ror { size, cnt, dst } => {
            let shift = shift_count(cnt, regs);
            let val = ea_read(dst, regs, bus, *size);
            let (res, carry) = rotate_right(val, shift, *size);
            shift_flags(res, carry, *size, false, regs);
            ea_write(dst, regs, bus, *size, res);
            4 + 2 * shift as u32
        }

        // ---------------------------------------------------------------
        // 位操作
        // ---------------------------------------------------------------
        Insn::Btst { bit, dst } => {
            let n = bit_num(bit, regs, dst);
            let v = ea_read(dst, regs, bus, bit_ea_size(dst));
            regs.update_ccr(|ccr| ccr.z = (v >> n) & 1 == 0);
            4
        }
        Insn::Bset { bit, dst } => {
            let n = bit_num(bit, regs, dst);
            let v = ea_read(dst, regs, bus, bit_ea_size(dst));
            regs.update_ccr(|ccr| ccr.z = (v >> n) & 1 == 0);
            ea_write(dst, regs, bus, bit_ea_size(dst), v | (1 << n));
            8
        }
        Insn::Bclr { bit, dst } => {
            let n = bit_num(bit, regs, dst);
            let v = ea_read(dst, regs, bus, bit_ea_size(dst));
            regs.update_ccr(|ccr| ccr.z = (v >> n) & 1 == 0);
            ea_write(dst, regs, bus, bit_ea_size(dst), v & !(1 << n));
            10
        }
        Insn::Bchg { bit, dst } => {
            let n = bit_num(bit, regs, dst);
            let v = ea_read(dst, regs, bus, bit_ea_size(dst));
            regs.update_ccr(|ccr| ccr.z = (v >> n) & 1 == 0);
            ea_write(dst, regs, bus, bit_ea_size(dst), v ^ (1 << n));
            8
        }

        // ---------------------------------------------------------------
        // 分支 / 跳转
        // ---------------------------------------------------------------
        Insn::Bra { disp } => {
            // PC 在 fetch 后已推进，目标 = insn_pc + 2 + disp
            regs.pc = insn_pc.wrapping_add(2).wrapping_add(*disp as i32 as u32);
            10
        }

        Insn::Bsr { disp } => {
            push_u32(regs, bus, regs.pc); // 返回地址（已推进后的 PC）
            regs.pc = insn_pc.wrapping_add(2).wrapping_add(*disp as i32 as u32);
            18
        }

        Insn::Bcc { cond, disp } => {
            let ccr = regs.ccr();
            if eval_cond(*cond, &ccr) {
                regs.pc = insn_pc.wrapping_add(2).wrapping_add(*disp as i32 as u32);
                10
            } else {
                8
            }
        }

        Insn::Dbcc { cond, dn, disp } => {
            let ccr = regs.ccr();
            if !eval_cond(*cond, &ccr) {
                let counter = (regs.read_d(*dn, Size::Word) as i16).wrapping_sub(1);
                regs.write_d(*dn, counter as u32, Size::Word);
                if counter != -1 {
                    regs.pc = insn_pc.wrapping_add(2).wrapping_add(*disp as i32 as u32);
                    return 10;
                }
            }
            14
        }

        Insn::Jmp { dst } => {
            let addr = ea_addr(dst, regs, Size::Long);
            regs.pc = addr;
            4
        }

        Insn::Jsr { dst } => {
            push_u32(regs, bus, regs.pc);
            let addr = ea_addr(dst, regs, Size::Long);
            regs.pc = addr;
            16
        }

        Insn::Rts => {
            regs.pc = pop_u32(regs, bus);
            16
        }

        Insn::Rte => {
            // 简化：只弹 PC，不恢复完整 SR（特权模式处理留后）
            let sr = pop_u32(regs, bus);
            let pc = pop_u32(regs, bus);
            regs.sr = sr as u16;
            regs.pc = pc;
            20
        }

        // ---------------------------------------------------------------
        // MOVEM
        // ---------------------------------------------------------------
        Insn::Movem {
            size,
            reg_mask,
            ea,
            to_mem,
        } => {
            if *to_mem {
                // 寄存器 → 内存（预减模式下 mask 位序相反）
                let mut addr = ea_addr(ea, regs, *size);
                let is_predec = matches!(ea, EA::PreDec(_));
                for i in 0..16u32 {
                    let bit = if is_predec { 15 - i } else { i };
                    if *reg_mask & (1 << bit) != 0 {
                        let val = if i < 8 {
                            regs.d[i as usize]
                        } else {
                            regs.a[(i - 8) as usize]
                        };
                        if is_predec {
                            addr = addr.wrapping_sub(size.bytes());
                        }
                        mem_write(bus, addr, *size, val);
                        if !is_predec {
                            addr = addr.wrapping_add(size.bytes());
                        }
                    }
                }
                if let EA::PreDec(an) = ea {
                    regs.write_a(*an, addr);
                }
            } else {
                // 内存 → 寄存器
                let mut addr = ea_addr(ea, regs, *size);
                for i in 0..16u32 {
                    if *reg_mask & (1 << i) != 0 {
                        let raw = mem_read(bus, addr, *size);
                        let val = match size {
                            Size::Word => raw as i16 as i32 as u32,
                            _ => raw,
                        };
                        if i < 8 {
                            regs.d[i as usize] = val;
                        } else {
                            regs.a[(i - 8) as usize] = val;
                        }
                        addr = addr.wrapping_add(size.bytes());
                    }
                }
                if let EA::PostInc(an) = ea {
                    regs.write_a(*an, addr);
                }
            }
            12
        }

        // ---------------------------------------------------------------
        // 杂项
        // ---------------------------------------------------------------
        Insn::Nop => 4,

        Insn::Clr { size, dst } => {
            ea_write(dst, regs, bus, *size, 0);
            regs.update_ccr(|ccr| {
                ccr.n = false;
                ccr.z = true;
                ccr.v = false;
                ccr.c = false;
            });
            4
        }

        Insn::Tst { size, src } => {
            let v = ea_read(src, regs, bus, *size);
            regs.update_ccr(|ccr| {
                ccr.update_nz(v, *size);
                ccr.v = false;
                ccr.c = false;
            });
            4
        }

        Insn::Ext { size, dn } => {
            let val = match size {
                Size::Word => {
                    let v = regs.read_d(*dn, Size::Byte) as i8 as i16 as u32;
                    regs.write_d(*dn, v, Size::Word);
                    v
                }
                Size::Long => {
                    let v = regs.read_d(*dn, Size::Word) as i16 as i32 as u32;
                    regs.write_d(*dn, v, Size::Long);
                    v
                }
                _ => 0,
            };
            regs.update_ccr(|ccr| {
                ccr.update_nz(val, *size);
                ccr.v = false;
                ccr.c = false;
            });
            4
        }

        Insn::Swap { dn } => {
            let v = regs.d[*dn];
            let r = v.rotate_right(16);
            regs.d[*dn] = r;
            regs.update_ccr(|ccr| {
                ccr.update_nz(r, Size::Long);
                ccr.v = false;
                ccr.c = false;
            });
            4
        }

        Insn::Trap { vector } => {
            warn!("TRAP #{} @ {:#010x} (unhandled)", vector, insn_pc);
            // TODO: 跳转到 trap vector
            34
        }

        Insn::Unimplemented { opcode } => {
            warn!("unimplemented 68k insn {:#06x} @ {:#010x}", opcode, insn_pc);
            4
        }
    }
}

// ---------------------------------------------------------------------------
// 算术辅助
// ---------------------------------------------------------------------------

fn size_mask(size: Size) -> u32 {
    match size {
        Size::Byte => 0xFF,
        Size::Word => 0xFFFF,
        Size::Long => 0xFFFF_FFFF,
    }
}

fn sign_bit(size: Size) -> u32 {
    match size {
        Size::Byte => 0x80,
        Size::Word => 0x8000,
        Size::Long => 0x8000_0000,
    }
}

/// 有符号加法，更新全部 CCR 标志。返回结果（截断至 size）。
fn add_with_flags(a: u32, b: u32, size: Size, regs: &mut Regs) -> u32 {
    let mask = size_mask(size);
    let sign = sign_bit(size);
    let res64 = (a as u64).wrapping_add(b as u64);
    let res = (res64 as u32) & mask;
    let carry = res64 > mask as u64;
    let overflow = (!(a ^ b) & (a ^ res)) & sign != 0;
    regs.update_ccr(|ccr| {
        ccr.update_nz(res, size);
        ccr.c = carry;
        ccr.v = overflow;
        ccr.x = carry;
    });
    res
}

/// 有符号减法 (a - b)，更新 CCR。返回结果。
fn sub_with_flags(a: u32, b: u32, size: Size, regs: &mut Regs) -> u32 {
    let mask = size_mask(size);
    let sign = sign_bit(size);
    let res64 = (a as u64).wrapping_add((!b as u64 & mask as u64) + 1);
    let res = (res64 as u32) & mask;
    let borrow = (a & mask) < (b & mask);
    let overflow = ((a ^ b) & (a ^ res)) & sign != 0;
    regs.update_ccr(|ccr| {
        ccr.update_nz(res, size);
        ccr.c = borrow;
        ccr.v = overflow;
        ccr.x = borrow;
    });
    res
}

fn logic_flags(res: u32, size: Size, regs: &mut Regs) {
    regs.update_ccr(|ccr| {
        ccr.update_nz(res, size);
        ccr.v = false;
        ccr.c = false;
    });
}

// ---------------------------------------------------------------------------
// 移位辅助
// ---------------------------------------------------------------------------

fn shift_count(cnt: &ShiftCnt, regs: &Regs) -> u8 {
    match *cnt {
        ShiftCnt::Imm(n) => n,
        ShiftCnt::Reg(dn) => (regs.read_d(dn, Size::Byte) & 63) as u8,
    }
}

fn shift_left(val: u32, n: u8, size: Size) -> (u32, bool) {
    let bits = size.bytes() * 8;
    if n == 0 {
        return (val & size_mask(size), false);
    }
    let n = n as u32;
    let last = if n <= bits {
        (val >> (bits - n)) & 1 != 0
    } else {
        false
    };
    let res = if n >= bits {
        0
    } else {
        (val << n) & size_mask(size)
    };
    (res, last)
}

fn shift_right_logical(val: u32, n: u8, size: Size) -> (u32, bool) {
    if n == 0 {
        return (val & size_mask(size), false);
    }
    let n = n as u32;
    let last = if n <= 32 {
        (val >> (n - 1)) & 1 != 0
    } else {
        false
    };
    let res = if n >= 32 {
        0
    } else {
        (val >> n) & size_mask(size)
    };
    (res, last)
}

fn shift_right_arith(val: u32, n: u8, size: Size) -> (u32, bool) {
    let sign = sign_bit(size);
    let is_neg = val & sign != 0;
    if n == 0 {
        return (val & size_mask(size), false);
    }
    let n = n as u32;
    let last = (val >> (n - 1).min(31)) & 1 != 0;
    let shifted = val >> n.min(31);
    let res = if is_neg {
        // 填充符号位
        let fill =
            size_mask(size) << (size.bytes() * 8 - n.min(size.bytes() * 8)) & size_mask(size);
        (shifted | fill) & size_mask(size)
    } else {
        shifted & size_mask(size)
    };
    (res, last)
}

fn rotate_left(val: u32, n: u8, size: Size) -> (u32, bool) {
    let bits = size.bytes() * 8;
    let n = (n as u32) % bits;
    if n == 0 {
        return (val & size_mask(size), val & 1 != 0);
    }
    let res = ((val << n) | (val >> (bits - n))) & size_mask(size);
    (res, res & 1 != 0)
}

fn rotate_right(val: u32, n: u8, size: Size) -> (u32, bool) {
    let bits = size.bytes() * 8;
    let n = (n as u32) % bits;
    if n == 0 {
        return (val & size_mask(size), val & sign_bit(size) != 0);
    }
    let res = ((val >> n) | (val << (bits - n))) & size_mask(size);
    (res, res & sign_bit(size) != 0)
}

fn shift_flags(res: u32, last_out: bool, size: Size, overflow: bool, regs: &mut Regs) {
    regs.update_ccr(|ccr| {
        ccr.update_nz(res, size);
        ccr.c = last_out;
        ccr.x = last_out;
        ccr.v = overflow;
    });
}

// ---------------------------------------------------------------------------
// 位操作辅助
// ---------------------------------------------------------------------------

fn bit_num(bit: &BitSrc, regs: &Regs, ea: &EA) -> u32 {
    let raw = match *bit {
        BitSrc::Imm(n) => n as u32,
        BitSrc::Reg(dn) => regs.read_d(dn, Size::Byte),
    };
    // 数据寄存器 mod 32，内存操作 mod 8
    if matches!(ea, EA::DataReg(_)) {
        raw & 31
    } else {
        raw & 7
    }
}

fn bit_ea_size(ea: &EA) -> Size {
    if matches!(ea, EA::DataReg(_)) {
        Size::Long
    } else {
        Size::Byte
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use model2_core::Bus;

    fn setup() -> (Regs, Bus) {
        (Regs::new(), Bus::new())
    }

    #[test]
    fn moveq_sets_reg_and_flags() {
        let (mut r, mut bus) = setup();
        execute(&Insn::Moveq { dn: 0, imm: -1 }, 0, &mut r, &mut bus);
        assert_eq!(r.d[0], 0xFFFF_FFFF);
        assert!(r.ccr().n);
        assert!(!r.ccr().z);
    }

    #[test]
    fn add_basic() {
        let (mut r, mut bus) = setup();
        r.d[0] = 10;
        r.d[1] = 32;
        execute(
            &Insn::Add {
                size: Size::Long,
                src: EA::DataReg(1),
                dst: EA::DataReg(0),
            },
            0,
            &mut r,
            &mut bus,
        );
        assert_eq!(r.d[0], 42);
    }

    #[test]
    fn sub_basic() {
        let (mut r, mut bus) = setup();
        r.d[2] = 100;
        r.d[3] = 58;
        execute(
            &Insn::Sub {
                size: Size::Long,
                src: EA::DataReg(3),
                dst: EA::DataReg(2),
            },
            0,
            &mut r,
            &mut bus,
        );
        assert_eq!(r.d[2], 42);
    }

    #[test]
    fn bra_taken() {
        let (mut r, mut bus) = setup();
        r.pc = 0x1002; // 已推进
        execute(&Insn::Bra { disp: 16 }, 0x1000, &mut r, &mut bus);
        // 目标 = 0x1000 + 2 + 16 = 0x1012
        assert_eq!(r.pc, 0x1012);
    }

    #[test]
    fn bcc_not_taken() {
        let (mut r, mut bus) = setup();
        r.pc = 0x1002;
        // BEQ，但 Z=0 → 不跳
        execute(
            &Insn::Bcc {
                cond: Cond::Eq,
                disp: 20,
            },
            0x1000,
            &mut r,
            &mut bus,
        );
        assert_eq!(r.pc, 0x1002); // 未修改
    }

    #[test]
    fn lsl_byte() {
        let (mut r, mut bus) = setup();
        r.d[0] = 0x01;
        execute(
            &Insn::Lsl {
                size: Size::Byte,
                cnt: ShiftCnt::Imm(3),
                dst: EA::DataReg(0),
            },
            0,
            &mut r,
            &mut bus,
        );
        assert_eq!(r.read_d(0, Size::Byte), 0x08);
    }

    #[test]
    fn jsr_rts_roundtrip() {
        let (mut r, mut bus) = setup();
        r.a[7] = 0x0010_0000; // 栈顶（在 MainRam 内）
        r.pc = 0x0010_0010; // 已推进后的 PC
        execute(
            &Insn::Jsr {
                dst: EA::AbsLong(0x0080_0000),
            },
            0x0010_000E,
            &mut r,
            &mut bus,
        );
        assert_eq!(r.pc, 0x0080_0000);
        execute(&Insn::Rts, 0x0080_0000, &mut r, &mut bus);
        assert_eq!(r.pc, 0x0010_0010);
    }
}
