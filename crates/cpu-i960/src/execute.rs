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

    // -----------------------------------------------------------------------
    // 新增综合测试
    // -----------------------------------------------------------------------

    /// addo（无符号加）与 add 产生相同结果（整数加法无区别）。
    #[test]
    fn addo_same_as_add() {
        let (mut r1, mut bus1) = setup();
        let (mut r2, mut bus2) = setup();
        r1.w(1, 0xFFFF_0000); r1.w(2, 0x0000_FFFF);
        r2.w(1, 0xFFFF_0000); r2.w(2, 0x0000_FFFF);
        execute(&Insn::Add  { dst: 0, src1: 1, src2: 2 }, 0, &mut r1, &mut bus1);
        execute(&Insn::Addo { dst: 0, src1: 1, src2: 2 }, 0, &mut r2, &mut bus2);
        assert_eq!(r1.r(0), r2.r(0));
    }

    /// 加法溢出回绕。
    #[test]
    fn add_wrapping_overflow() {
        let (mut r, mut bus) = setup();
        r.w(1, 0xFFFF_FFFF);
        r.w(2, 1);
        execute(&Insn::Add { dst: 0, src1: 1, src2: 2 }, 0, &mut r, &mut bus);
        assert_eq!(r.r(0), 0);
    }

    /// sub：减法回绕（无符号下溢）。
    #[test]
    fn sub_wrapping_underflow() {
        let (mut r, mut bus) = setup();
        r.w(1, 0);
        r.w(2, 1);
        execute(&Insn::Sub { dst: 0, src1: 1, src2: 2 }, 0, &mut r, &mut bus);
        assert_eq!(r.r(0), 0xFFFF_FFFF);
    }

    /// mul 乘法。
    #[test]
    fn mul_basic() {
        let (mut r, mut bus) = setup();
        r.w(1, 7);
        r.w(2, 6);
        execute(&Insn::Mul { dst: 0, src1: 1, src2: 2 }, 0, &mut r, &mut bus);
        assert_eq!(r.r(0), 42);
    }

    /// mul 溢出回绕。
    #[test]
    fn mul_wrapping() {
        let (mut r, mut bus) = setup();
        r.w(1, 0x8000_0000);
        r.w(2, 2);
        execute(&Insn::Mul { dst: 0, src1: 1, src2: 2 }, 0, &mut r, &mut bus);
        assert_eq!(r.r(0), 0); // 0x1_0000_0000 截断
    }

    /// divo 无符号除法。
    #[test]
    fn divo_basic() {
        let (mut r, mut bus) = setup();
        r.w(1, 42);
        r.w(2, 6);
        execute(&Insn::Divo { dst: 0, src1: 1, src2: 2 }, 0, &mut r, &mut bus);
        assert_eq!(r.r(0), 7);
    }

    /// divo 除零不 panic（结果置 0）。
    #[test]
    fn divo_by_zero() {
        let (mut r, mut bus) = setup();
        r.w(1, 100);
        r.w(2, 0); // divisor = 0
        execute(&Insn::Divo { dst: 0, src1: 1, src2: 2 }, 0, &mut r, &mut bus);
        assert_eq!(r.r(0), 0);
    }

    /// remo 无符号取余。
    #[test]
    fn remo_basic() {
        let (mut r, mut bus) = setup();
        r.w(1, 17);
        r.w(2, 5);
        execute(&Insn::Remo { dst: 0, src1: 1, src2: 2 }, 0, &mut r, &mut bus);
        assert_eq!(r.r(0), 2);
    }

    /// remo 除零不 panic。
    #[test]
    fn remo_by_zero() {
        let (mut r, mut bus) = setup();
        r.w(1, 99);
        r.w(2, 0);
        execute(&Insn::Remo { dst: 0, src1: 1, src2: 2 }, 0, &mut r, &mut bus);
        assert_eq!(r.r(0), 0);
    }

    // -----------------------------------------------------------------------
    // 位运算
    // -----------------------------------------------------------------------

    #[test]
    fn and_or_xor() {
        let (mut r, mut bus) = setup();
        r.w(1, 0xF0F0_F0F0);
        r.w(2, 0x0F0F_0F0F);

        execute(&Insn::And  { dst: 0, src1: 1, src2: 2 }, 0, &mut r, &mut bus);
        assert_eq!(r.r(0), 0x0000_0000);

        execute(&Insn::Or   { dst: 0, src1: 1, src2: 2 }, 0, &mut r, &mut bus);
        assert_eq!(r.r(0), 0xFFFF_FFFF);

        execute(&Insn::Xor  { dst: 0, src1: 1, src2: 2 }, 0, &mut r, &mut bus);
        assert_eq!(r.r(0), 0xFFFF_FFFF);
    }

    #[test]
    fn nand_nor_xnor() {
        let (mut r, mut bus) = setup();
        r.w(1, 0xFFFF_FFFF);
        r.w(2, 0xFFFF_FFFF);

        execute(&Insn::Nand { dst: 0, src1: 1, src2: 2 }, 0, &mut r, &mut bus);
        assert_eq!(r.r(0), 0); // ~(1&1) = 0

        execute(&Insn::Nor  { dst: 0, src1: 1, src2: 2 }, 0, &mut r, &mut bus);
        assert_eq!(r.r(0), 0); // ~(1|1) = 0

        execute(&Insn::Xnor { dst: 0, src1: 1, src2: 2 }, 0, &mut r, &mut bus);
        assert_eq!(r.r(0), 0xFFFF_FFFF); // ~(1^1) = ~0 = !0
    }

    #[test]
    fn not_basic() {
        let (mut r, mut bus) = setup();
        r.w(1, 0xAAAA_AAAA);
        execute(&Insn::Not { dst: 0, src: 1 }, 0, &mut r, &mut bus);
        assert_eq!(r.r(0), 0x5555_5555);
    }

    // -----------------------------------------------------------------------
    // 移位
    // -----------------------------------------------------------------------

    #[test]
    fn shro_logical_right_shift() {
        let (mut r, mut bus) = setup();
        r.w(1, 0x8000_0000); // 最高位置 1
        r.w(2, 1);            // 移 1 位
        execute(&Insn::Shro { dst: 0, src: 1, cnt: 2 }, 0, &mut r, &mut bus);
        assert_eq!(r.r(0), 0x4000_0000); // 逻辑右移，不补符号
    }

    #[test]
    fn shri_arithmetic_right_shift() {
        let (mut r, mut bus) = setup();
        r.w(1, 0x8000_0000u32); // 负数（符号位=1）
        r.w(2, 4);
        execute(&Insn::Shri { dst: 0, src: 1, cnt: 2 }, 0, &mut r, &mut bus);
        assert_eq!(r.r(0), 0xF800_0000); // 算术右移，高位补 1
    }

    #[test]
    fn shli_left_shift_integer() {
        let (mut r, mut bus) = setup();
        r.w(1, 1);
        r.w(2, 16);
        execute(&Insn::Shli { dst: 0, src: 1, cnt: 2 }, 0, &mut r, &mut bus);
        assert_eq!(r.r(0), 0x0001_0000);
    }

    // -----------------------------------------------------------------------
    // 比较
    // -----------------------------------------------------------------------

    /// cmpi：有符号比较（负数 < 正数）。
    #[test]
    fn cmpi_signed() {
        let (mut r, mut bus) = setup();
        r.w(1, 0xFFFF_FFFFu32); // -1 as i32
        r.w(2, 1);              //  1
        execute(&Insn::Cmpi { src1: 1, src2: 2 }, 0, &mut r, &mut bus);
        let cc = r.cc();
        assert!(cc.n, "expected n (less-than) flag");
        assert!(!cc.e, "expected !e");
        assert!(!cc.g, "expected !g");
    }

    #[test]
    fn cmpi_equal() {
        let (mut r, mut bus) = setup();
        r.w(1, 0x8000_0000u32); // same value
        r.w(2, 0x8000_0000u32);
        execute(&Insn::Cmpi { src1: 1, src2: 2 }, 0, &mut r, &mut bus);
        assert!(r.cc().e);
        assert!(!r.cc().g);
        assert!(!r.cc().n);
    }

    /// cmpo：无符号比较（大值 > 小值）。
    #[test]
    fn cmpo_unsigned_greater() {
        let (mut r, mut bus) = setup();
        r.w(1, 0xFFFF_FFFFu32); // large unsigned
        r.w(2, 1);
        execute(&Insn::Cmpo { src1: 1, src2: 2 }, 0, &mut r, &mut bus);
        assert!(r.cc().g);
        assert!(!r.cc().e);
    }

    // -----------------------------------------------------------------------
    // 条件分支
    // -----------------------------------------------------------------------

    fn do_branch(insn: Insn, cc: CondCode, insn_addr: u32) -> u32 {
        let (mut r, mut bus) = setup();
        r.set_cc(cc);
        r.ip = insn_addr + 4;
        execute(&insn, insn_addr, &mut r, &mut bus);
        r.ip
    }

    #[test]
    fn conditional_branches_taken_and_not_taken() {
        let base = 0x100u32;
        let cc_eq = CondCode { n: false, e: true,  g: false };
        let cc_gt = CondCode { n: false, e: false, g: true  };
        let cc_lt = CondCode { n: true,  e: false, g: false };

        // Bg (branch if greater)
        assert_eq!(do_branch(Insn::Bg  { disp: 8 }, cc_gt, base), base + 8);
        assert_eq!(do_branch(Insn::Bg  { disp: 8 }, cc_eq, base), base + 4); // not taken

        // Be (branch if equal)
        assert_eq!(do_branch(Insn::Be  { disp: 8 }, cc_eq, base), base + 8);
        assert_eq!(do_branch(Insn::Be  { disp: 8 }, cc_gt, base), base + 4);

        // Bge (branch if ≥)
        assert_eq!(do_branch(Insn::Bge { disp: 8 }, cc_gt, base), base + 8);
        assert_eq!(do_branch(Insn::Bge { disp: 8 }, cc_eq, base), base + 8);
        assert_eq!(do_branch(Insn::Bge { disp: 8 }, cc_lt, base), base + 4);

        // Bl (branch if less)
        assert_eq!(do_branch(Insn::Bl  { disp: 8 }, cc_lt, base), base + 8);
        assert_eq!(do_branch(Insn::Bl  { disp: 8 }, cc_eq, base), base + 4);

        // Bne (branch if ≠)
        assert_eq!(do_branch(Insn::Bne { disp: 8 }, cc_gt, base), base + 8);
        assert_eq!(do_branch(Insn::Bne { disp: 8 }, cc_eq, base), base + 4);

        // Ble (branch if ≤)
        assert_eq!(do_branch(Insn::Ble { disp: 8 }, cc_eq, base), base + 8);
        assert_eq!(do_branch(Insn::Ble { disp: 8 }, cc_lt, base), base + 8);
        assert_eq!(do_branch(Insn::Ble { disp: 8 }, cc_gt, base), base + 4);

        // Bo（always branch for integers）
        assert_eq!(do_branch(Insn::Bo  { disp: 8 }, cc_eq, base), base + 8);

        // Bno（never branch）
        assert_eq!(do_branch(Insn::Bno,             cc_eq, base), base + 4);
    }

    // -----------------------------------------------------------------------
    // COBR
    // -----------------------------------------------------------------------

    #[test]
    fn cmpob_branch_taken_and_not_taken() {
        let (mut r, mut bus) = setup();
        r.w(1, 5);
        r.w(2, 5);
        r.ip = 0x104;
        // cmpobe (mask=0b010, e=true)：5==5 → taken
        execute(
            &Insn::Cmpob { src1: 1, src2: 2, disp: 8, mask: 0b010 },
            0x100,
            &mut r,
            &mut bus,
        );
        assert_eq!(r.ip, 0x108);

        // 修改 src2，使不等，则不跳转
        r.w(2, 6);
        r.ip = 0x104;
        execute(
            &Insn::Cmpob { src1: 1, src2: 2, disp: 8, mask: 0b010 },
            0x100,
            &mut r,
            &mut bus,
        );
        assert_eq!(r.ip, 0x104); // not taken
    }

    #[test]
    fn bbc_bbs_branch() {
        let (mut r, mut bus) = setup();
        r.w(1, 0); // bit_pos = 0
        r.w(2, 0b0101); // src value

        r.ip = 0x104;
        // bit 0 of 0b0101 is set → bbc (branch if clear) not taken
        execute(&Insn::Bbc { bit_pos: 1, src: 2, disp: 8 }, 0x100, &mut r, &mut bus);
        assert_eq!(r.ip, 0x104); // not taken (bit 0 of r1=0 → pos=0, r2[0]=1)

        // bbs (branch if set): bit 0 is set → taken
        r.ip = 0x104;
        execute(&Insn::Bbs { bit_pos: 1, src: 2, disp: 8 }, 0x100, &mut r, &mut bus);
        assert_eq!(r.ip, 0x108); // taken
    }

    // -----------------------------------------------------------------------
    // 数据移动
    // -----------------------------------------------------------------------

    #[test]
    fn mov_copies_register() {
        let (mut r, mut bus) = setup();
        r.w(3, 0x1234_5678);
        execute(&Insn::Mov { dst: 5, src: 3 }, 0, &mut r, &mut bus);
        assert_eq!(r.r(5), 0x1234_5678);
        assert_eq!(r.r(3), 0x1234_5678); // src unchanged
    }

    #[test]
    fn ldlit_stores_literal() {
        let (mut r, mut bus) = setup();
        execute(&Insn::LdLit { dst: 7, lit: 0x1F }, 0, &mut r, &mut bus);
        assert_eq!(r.r(7), 0x1F);
    }

    // -----------------------------------------------------------------------
    // 跳转 / 调用
    // -----------------------------------------------------------------------

    #[test]
    fn bal_saves_return_address_in_g14() {
        let (mut r, mut bus) = setup();
        r.ip = 0x800004; // 已推进
        execute(&Insn::Bal { disp: 16 }, 0x800000, &mut r, &mut bus);
        assert_eq!(r.ip, 0x800010, "branch target");
        assert_eq!(r.r(14), 0x800004, "g14 = return addr");
    }

    #[test]
    fn call_and_ret_roundtrip() {
        let (mut r, mut bus) = setup();
        r.ip = 0x800004;
        execute(&Insn::Call { disp: 32 }, 0x800000, &mut r, &mut bus);
        assert_eq!(r.ip, 0x800020, "call target");
        let ret_addr = r.r(14);

        execute(&Insn::Ret, 0x800020, &mut r, &mut bus);
        assert_eq!(r.ip, ret_addr, "ret returns to saved addr");
    }

    // -----------------------------------------------------------------------
    // MEM：字节 / 半字访问
    // -----------------------------------------------------------------------

    #[test]
    fn ldob_ldos_unsigned_load() {
        let (mut r, mut bus) = setup();
        r.w(1, 0); // abase
        // stob先写入
        bus.write_u8(0x100, 0xAB);
        execute(&Insn::Ldob { dst: 0, abase: 1, offset: 0x100 }, 0, &mut r, &mut bus);
        assert_eq!(r.r(0), 0xAB); // zero-extended

        bus.write_u16(0x200, 0xCDEF);
        execute(&Insn::Ldos { dst: 0, abase: 1, offset: 0x200 }, 0, &mut r, &mut bus);
        assert_eq!(r.r(0), 0xCDEF); // zero-extended
    }

    #[test]
    fn ldib_ldis_signed_load() {
        let (mut r, mut bus) = setup();
        r.w(1, 0);
        bus.write_u8(0x100, 0xFF); // -1 as i8
        execute(&Insn::Ldib { dst: 0, abase: 1, offset: 0x100 }, 0, &mut r, &mut bus);
        assert_eq!(r.r(0) as i32, -1); // sign-extended

        bus.write_u16(0x200, 0x8000); // -32768 as i16
        execute(&Insn::Ldis { dst: 0, abase: 1, offset: 0x200 }, 0, &mut r, &mut bus);
        assert_eq!(r.r(0) as i32, -32768);
    }

    #[test]
    fn stob_stos_byte_short_store() {
        let (mut r, mut bus) = setup();
        r.w(1, 0);
        r.w(0, 0xDEAD_BEEF);
        execute(&Insn::Stob { src: 0, abase: 1, offset: 0x100 }, 0, &mut r, &mut bus);
        assert_eq!(bus.read_u8(0x100), 0xEF); // low byte

        execute(&Insn::Stos { src: 0, abase: 1, offset: 0x200 }, 0, &mut r, &mut bus);
        assert_eq!(bus.read_u16(0x200), 0xBEEF); // low 2 bytes
    }

    /// Unimplemented insn 仍返回 1 周期（不 panic）。
    #[test]
    fn unimplemented_returns_one_cycle() {
        let (mut r, mut bus) = setup();
        let cycles = execute(&Insn::Unimplemented { raw: 0xDEAD }, 0, &mut r, &mut bus);
        assert_eq!(cycles, 1);
    }
}
