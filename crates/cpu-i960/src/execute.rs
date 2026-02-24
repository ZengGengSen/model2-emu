//! i960 指令执行。
//!
//! 每条 [`Insn`] 对应一个执行函数，修改 [`Regs`] 并通过 [`Bus`] 访问内存。
//! 返回本条指令消耗的时钟周期数（近似值，暂时全部为 1）。

use log::{trace, warn};

use model2_core::Bus;

use crate::decode::Insn;
use crate::regs::{CondCode, Regs};

/// 执行一条已解码的指令。
///
/// # 参数
/// - `insn`: 解码结果
/// - `insn_addr`: 该指令的地址（用于计算相对跳转目标）
/// - `regs`: 寄存器堆（in/out）
/// - `bus`: 系统总线（内存访问）
///
/// # 返回值
/// 消耗的时钟周期数（近似，未来从手册精确表格填入）。
pub fn execute(insn: &Insn, insn_addr: u32, regs: &mut Regs, bus: &mut Bus) -> u32 {
    match *insn {
        // ---------------------------------------------------------------
        // CTRL：跳转 / 调用
        // ---------------------------------------------------------------
        Insn::B { disp } => {
            // IP 已经在 fetch 时推进了 4，所以目标 = insn_addr + disp
            let target = insn_addr.wrapping_add(disp as u32);
            trace!("B {:#010x}", target);
            regs.ip = target;
            3
        }

        Insn::Bal { disp } => {
            let target = insn_addr.wrapping_add(disp as u32);
            // 保存返回地址到 g14（ABI 约定）
            regs.w(14, regs.ip); // regs.ip 此时已是 insn_addr+4
            trace!("BAL {:#010x} (g14={:#010x})", target, regs.ip);
            regs.ip = target;
            3
        }

        Insn::Call { disp } => {
            let target = insn_addr.wrapping_add(disp as u32);
            // 简化：不做真正的寄存器窗口，只把返回地址存 g14
            // 真正实现需要在栈上保存局部寄存器帧
            regs.w(14, regs.ip);
            trace!("CALL {:#010x}", target);
            regs.ip = target;
            4
        }

        Insn::Ret => {
            // 从 g14 取回返回地址（简化）
            let ret_addr = regs.r(14);
            trace!("RET → {:#010x}", ret_addr);
            regs.ip = ret_addr;
            10
        }

        // ---------------------------------------------------------------
        // CTRL：条件分支（按 AC 条件码）
        // ---------------------------------------------------------------
        Insn::Bno => {
            /* never branch */
            1
        }

        Insn::Bg { disp } => {
            if regs.cc().g {
                regs.ip = insn_addr.wrapping_add(disp as u32);
            }
            1
        }
        Insn::Be { disp } => {
            if regs.cc().e {
                regs.ip = insn_addr.wrapping_add(disp as u32);
            }
            1
        }
        Insn::Bge { disp } => {
            let cc = regs.cc();
            if cc.g || cc.e {
                regs.ip = insn_addr.wrapping_add(disp as u32);
            }
            1
        }
        Insn::Bl { disp } => {
            let cc = regs.cc();
            if !cc.g && !cc.e {
                regs.ip = insn_addr.wrapping_add(disp as u32);
            }
            1
        }
        Insn::Bne { disp } => {
            if !regs.cc().e {
                regs.ip = insn_addr.wrapping_add(disp as u32);
            }
            1
        }
        Insn::Ble { disp } => {
            let cc = regs.cc();
            if !cc.g {
                regs.ip = insn_addr.wrapping_add(disp as u32);
            }
            1
        }
        Insn::Bo { disp } => {
            // "ordered"——对整数运算总为真
            regs.ip = insn_addr.wrapping_add(disp as u32);
            1
        }

        // ---------------------------------------------------------------
        // COBR：比较 + 条件跳转
        // ---------------------------------------------------------------
        Insn::Cmpob {
            src1,
            src2,
            disp,
            mask,
        } => {
            let a = regs.r(src1);
            let b = regs.r(src2);
            let cc = cmp_ord(a, b);
            regs.set_cc(cc);
            let cc_bits = cc.to_bits();
            if cc_bits & mask != 0 {
                regs.ip = insn_addr.wrapping_add(disp as u32);
            }
            2
        }

        Insn::Bbc { bit_pos, src, disp } => {
            let pos = regs.r(bit_pos) & 31;
            let val = regs.r(src);
            if (val >> pos) & 1 == 0 {
                regs.ip = insn_addr.wrapping_add(disp as u32);
            }
            2
        }

        Insn::Bbs { bit_pos, src, disp } => {
            let pos = regs.r(bit_pos) & 31;
            let val = regs.r(src);
            if (val >> pos) & 1 == 1 {
                regs.ip = insn_addr.wrapping_add(disp as u32);
            }
            2
        }

        // ---------------------------------------------------------------
        // REG：数据移动
        // ---------------------------------------------------------------
        Insn::Mov { dst, src } => {
            let v = regs.r(src);
            regs.w(dst, v);
            1
        }

        Insn::LdLit { dst, lit } => {
            regs.w(dst, lit);
            1
        }

        // ---------------------------------------------------------------
        // REG：整数运算
        // ---------------------------------------------------------------
        Insn::Add { dst, src1, src2 } | Insn::Addo { dst, src1, src2 } => {
            let v = regs.r(src1).wrapping_add(regs.r(src2));
            regs.w(dst, v);
            1
        }

        Insn::Sub { dst, src1, src2 } | Insn::Subo { dst, src1, src2 } => {
            // i960: dst = src1 - src2（注意操作数顺序）
            let v = regs.r(src1).wrapping_sub(regs.r(src2));
            regs.w(dst, v);
            1
        }

        Insn::Mul { dst, src1, src2 } => {
            let v = regs.r(src1).wrapping_mul(regs.r(src2));
            regs.w(dst, v);
            10
        }

        Insn::Divo { dst, src1, src2 } => {
            let divisor = regs.r(src2);
            if divisor == 0 {
                warn!("divo: division by zero @ {:#010x}", insn_addr);
                regs.w(dst, 0);
            } else {
                regs.w(dst, regs.r(src1) / divisor);
            }
            20
        }

        Insn::Remo { dst, src1, src2 } => {
            let divisor = regs.r(src2);
            if divisor == 0 {
                warn!("remo: division by zero @ {:#010x}", insn_addr);
                regs.w(dst, 0);
            } else {
                regs.w(dst, regs.r(src1) % divisor);
            }
            20
        }

        // ---------------------------------------------------------------
        // REG：位运算
        // ---------------------------------------------------------------
        Insn::And { dst, src1, src2 } => {
            regs.w(dst, regs.r(src1) & regs.r(src2));
            1
        }
        Insn::Or { dst, src1, src2 } => {
            regs.w(dst, regs.r(src1) | regs.r(src2));
            1
        }
        Insn::Xor { dst, src1, src2 } => {
            regs.w(dst, regs.r(src1) ^ regs.r(src2));
            1
        }
        Insn::Nand { dst, src1, src2 } => {
            regs.w(dst, !(regs.r(src1) & regs.r(src2)));
            1
        }
        Insn::Nor { dst, src1, src2 } => {
            regs.w(dst, !(regs.r(src1) | regs.r(src2)));
            1
        }
        Insn::Xnor { dst, src1, src2 } => {
            regs.w(dst, !(regs.r(src1) ^ regs.r(src2)));
            1
        }
        Insn::Not { dst, src } => {
            regs.w(dst, !regs.r(src));
            1
        }

        // ---------------------------------------------------------------
        // REG：移位
        // ---------------------------------------------------------------
        Insn::Shlo { dst, src, cnt } => {
            let shift = regs.r(cnt) & 31;
            regs.w(dst, regs.r(src) << shift);
            1
        }
        Insn::Shro { dst, src, cnt } => {
            let shift = regs.r(cnt) & 31;
            regs.w(dst, regs.r(src) >> shift);
            1
        }
        Insn::Shli { dst, src, cnt } => {
            let shift = regs.r(cnt) & 31;
            regs.w(dst, ((regs.r(src) as i32) << shift) as u32);
            1
        }
        Insn::Shri { dst, src, cnt } => {
            let shift = regs.r(cnt) & 31;
            regs.w(dst, ((regs.r(src) as i32) >> shift) as u32);
            1
        }

        // ---------------------------------------------------------------
        // REG：比较（更新 AC 条件码）
        // ---------------------------------------------------------------
        Insn::Cmpo { src1, src2 } => {
            let cc = cmp_ord(regs.r(src1), regs.r(src2));
            regs.set_cc(cc);
            1
        }

        Insn::Cmpi { src1, src2 } => {
            let cc = cmp_int(regs.r(src1) as i32, regs.r(src2) as i32);
            regs.set_cc(cc);
            1
        }

        // ---------------------------------------------------------------
        // MEM：加载
        // ---------------------------------------------------------------
        Insn::Ld { dst, abase, offset } => {
            let addr = regs.r(abase).wrapping_add(offset);
            let v = bus.read_u32(addr);
            regs.w(dst, v);
            trace!("LD   g{}={:#010x} ← [{:#010x}]", dst, v, addr);
            3
        }

        Insn::Ldob { dst, abase, offset } => {
            let addr = regs.r(abase).wrapping_add(offset);
            regs.w(dst, bus.read_u8(addr) as u32);
            3
        }

        Insn::Ldos { dst, abase, offset } => {
            let addr = regs.r(abase).wrapping_add(offset);
            regs.w(dst, bus.read_u16(addr) as u32);
            3
        }

        Insn::Ldib { dst, abase, offset } => {
            let addr = regs.r(abase).wrapping_add(offset);
            regs.w(dst, bus.read_u8(addr) as i8 as i32 as u32);
            3
        }

        Insn::Ldis { dst, abase, offset } => {
            let addr = regs.r(abase).wrapping_add(offset);
            regs.w(dst, bus.read_u16(addr) as i16 as i32 as u32);
            3
        }

        // ---------------------------------------------------------------
        // MEM：存储
        // ---------------------------------------------------------------
        Insn::St { src, abase, offset } => {
            let addr = regs.r(abase).wrapping_add(offset);
            let v = regs.r(src);
            trace!("ST   [{:#010x}] ← {:#010x}", addr, v);
            bus.write_u32(addr, v);
            3
        }

        Insn::Stob { src, abase, offset } => {
            let addr = regs.r(abase).wrapping_add(offset);
            bus.write_u8(addr, regs.r(src) as u8);
            3
        }

        Insn::Stos { src, abase, offset } => {
            let addr = regs.r(abase).wrapping_add(offset);
            bus.write_u16(addr, regs.r(src) as u16);
            3
        }

        // ---------------------------------------------------------------
        // 未实现
        // ---------------------------------------------------------------
        Insn::Unimplemented { raw } => {
            warn!("unimplemented insn {:#010x} @ {:#010x}", raw, insn_addr);
            1
        }
    }
}

// ---------------------------------------------------------------------------
// 条件码辅助
// ---------------------------------------------------------------------------

/// 无符号比较，返回条件码。
fn cmp_ord(a: u32, b: u32) -> CondCode {
    CondCode {
        n: false,
        e: a == b,
        g: a > b,
    }
}

/// 有符号比较，返回条件码。
fn cmp_int(a: i32, b: i32) -> CondCode {
    CondCode {
        n: a < b,
        e: a == b,
        g: a > b,
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
    fn add_basic() {
        let (mut r, mut bus) = setup();
        r.w(1, 10);
        r.w(2, 32);
        let insn = Insn::Add {
            dst: 0,
            src1: 1,
            src2: 2,
        };
        execute(&insn, 0x800000, &mut r, &mut bus);
        assert_eq!(r.r(0), 42);
    }

    #[test]
    fn sub_basic() {
        let (mut r, mut bus) = setup();
        r.w(1, 100);
        r.w(2, 58);
        let insn = Insn::Sub {
            dst: 3,
            src1: 1,
            src2: 2,
        };
        execute(&insn, 0x800000, &mut r, &mut bus);
        assert_eq!(r.r(3), 42);
    }

    #[test]
    fn cmpo_sets_cc() {
        let (mut r, mut bus) = setup();
        r.w(0, 5);
        r.w(1, 5);
        execute(&Insn::Cmpo { src1: 0, src2: 1 }, 0, &mut r, &mut bus);
        assert!(r.cc().e);

        r.w(1, 3);
        execute(&Insn::Cmpo { src1: 0, src2: 1 }, 0, &mut r, &mut bus);
        assert!(r.cc().g);
    }

    #[test]
    fn branch_taken() {
        let (mut r, mut bus) = setup();
        r.ip = 0x800004; // 已经推进了4
        execute(&Insn::B { disp: 16 }, 0x800000, &mut r, &mut bus);
        assert_eq!(r.ip, 0x800010);
    }

    #[test]
    fn load_store_roundtrip() {
        let (mut r, mut bus) = setup();
        r.w(1, 0); // abase = 0
        execute(
            &Insn::St {
                src: 0,
                abase: 1,
                offset: 0x100,
            },
            0,
            &mut r,
            &mut bus,
        );
        // r[0] = 0 → 存 0 到 0x100

        r.w(0, 0xDEAD_BEEF);
        execute(
            &Insn::St {
                src: 0,
                abase: 1,
                offset: 0x200,
            },
            0,
            &mut r,
            &mut bus,
        );

        execute(
            &Insn::Ld {
                dst: 2,
                abase: 1,
                offset: 0x200,
            },
            0,
            &mut r,
            &mut bus,
        );
        assert_eq!(r.r(2), 0xDEAD_BEEF);
    }

    #[test]
    fn shlo_basic() {
        let (mut r, mut bus) = setup();
        r.w(1, 1);
        r.w(2, 8);
        execute(
            &Insn::Shlo {
                dst: 0,
                src: 1,
                cnt: 2,
            },
            0,
            &mut r,
            &mut bus,
        );
        assert_eq!(r.r(0), 256);
    }
}
