//! 68000 CPU 主结构体。
//!
//! # 与 i960 的差异
//!
//! - **大端字节序**：68000 是大端，指令字从总线读取时需要 swap 字节序。
//!   总线（Bus）内部按小端存储，所以读 u16 时要从两个 u8 拼成大端 u16。
//! - **变长指令**：fetch 需要先读主操作码字，解码时可能继续读扩展字。
//!   CPU 内部用一个"预读窗口"来支持这一点。
//! - **没有寄存器窗口**：比 i960 简单，CALL/RET 换成 JSR/RTS，用栈保存返回地址。

use log::{debug, trace, warn};

use model2_core::Bus;
use model2_core::clock::RunToken;

use crate::decode::decode;
use crate::disasm::format_insn;
use crate::execute::execute;
use crate::regs::Regs;

// ---------------------------------------------------------------------------
// 断点 / 状态
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuState {
    Running,
    Paused,
    Halted,
}

#[derive(Debug, Clone, Copy)]
pub struct Breakpoint {
    pub addr: u32,
    pub enabled: bool,
}

// ---------------------------------------------------------------------------
// CPU 主结构体
// ---------------------------------------------------------------------------

/// Motorola 68000 解释器。
pub struct Cpu {
    pub regs: Regs,
    pub state: CpuState,
    pub breakpoints: Vec<Breakpoint>,
    /// 跟踪模式：每条指令打印反汇编。
    pub trace: bool,
    /// 累计执行指令数。
    pub insn_count: u64,
}

impl Cpu {
    /// 创建 CPU，PC 设为 `entry`。
    ///
    /// 68000 复位时从地址 0x000000 读 SSP（4字节），再读 PC（4字节）。
    /// 在 Model 2 音频子系统中，复位向量由主 CPU 写入，这里先直接设 PC。
    pub fn new(entry: u32) -> Self {
        let mut regs = Regs::new();
        regs.pc = entry;
        Self {
            regs,
            state: CpuState::Running,
            breakpoints: Vec::new(),
            trace: false,
            insn_count: 0,
        }
    }

    // -----------------------------------------------------------------------
    // 调试接口
    // -----------------------------------------------------------------------

    pub fn add_breakpoint(&mut self, addr: u32) {
        self.breakpoints.push(Breakpoint {
            addr,
            enabled: true,
        });
        debug!("m68k breakpoint @ {:#010x}", addr);
    }

    pub fn remove_breakpoint(&mut self, addr: u32) {
        self.breakpoints.retain(|bp| bp.addr != addr);
    }

    pub fn resume(&mut self) {
        if self.state == CpuState::Paused {
            self.state = CpuState::Running;
        }
    }

    pub fn dump_regs(&self) {
        self.regs.dump();
    }

    // -----------------------------------------------------------------------
    // Fetch（大端 u16 读取）
    // -----------------------------------------------------------------------

    /// 从总线按大端读取一个 u16（68000 指令字节序）。
    fn fetch_u16(bus: &Bus, addr: u32) -> u16 {
        let hi = bus.read_u8(addr) as u16;
        let lo = bus.read_u8(addr + 1) as u16;
        (hi << 8) | lo
    }

    /// 预读最多 12 字节（6个 u16），构成解码窗口。
    fn prefetch(pc: u32, bus: &Bus) -> [u16; 6] {
        [
            Self::fetch_u16(bus, pc),
            Self::fetch_u16(bus, pc + 2),
            Self::fetch_u16(bus, pc + 4),
            Self::fetch_u16(bus, pc + 6),
            Self::fetch_u16(bus, pc + 8),
            Self::fetch_u16(bus, pc + 10),
        ]
    }

    // -----------------------------------------------------------------------
    // 单步 / 运行
    // -----------------------------------------------------------------------

    /// 执行单条指令，返回消耗的周期数。
    pub fn step(&mut self, bus: &mut Bus) -> u32 {
        if self.state != CpuState::Running {
            return 0;
        }

        let pc = self.regs.pc;

        // 检查断点
        if let Some(bp) = self
            .breakpoints
            .iter()
            .find(|bp| bp.enabled && bp.addr == pc)
        {
            debug!("m68k breakpoint hit @ {:#010x}", bp.addr);
            self.state = CpuState::Paused;
            return 0;
        }

        // Fetch & Decode
        let window = Self::prefetch(pc, bus);
        let (insn, len) = decode(&window);

        if self.trace {
            let text = format_insn(&insn, pc);
            trace!("{:#010x}  {}", pc, text);
        }

        // 推进 PC（在执行前推进，68000 分支目标基于当前指令地址+2，与 i960 约定不同）
        self.regs.pc = pc.wrapping_add(len);

        // Execute
        let cycles = execute(&insn, pc, &mut self.regs, bus);

        // 未实现指令停机
        if let crate::decode::Insn::Unimplemented { opcode } = insn {
            warn!("m68k halting: unimplemented {:#06x} @ {:#010x}", opcode, pc);
            self.state = CpuState::Halted;
        }

        self.insn_count += 1;
        cycles
    }

    /// 运行最多 `max_cycles` 个周期。
    pub fn run(&mut self, bus: &mut Bus, max_cycles: u64) -> u64 {
        let mut total = 0u64;
        while total < max_cycles && self.state == CpuState::Running {
            let c = self.step(bus) as u64;
            if c == 0 {
                break;
            }
            total += c;
        }
        total
    }

    /// 供调度器调用的 tick 函数。
    pub fn tick_with_bus(&mut self, bus: &mut Bus, token: RunToken) -> u64 {
        self.run(bus, token.cycles)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 把 u16 大端数组写入 RAM 用于测试。
    fn make_bus_with_program(words: &[u16]) -> Bus {
        let mut bus = Bus::new();
        // 68000 代码放在 MainRam 起始处（测试用）
        for (i, &w) in words.iter().enumerate() {
            let addr = (i * 2) as u32;
            bus.write_u8(addr, (w >> 8) as u8);
            bus.write_u8(addr + 1, (w & 0xFF) as u8);
        }
        bus
    }

    #[test]
    fn step_nop() {
        let mut bus = make_bus_with_program(&[0x4E71]); // NOP
        let mut cpu = Cpu::new(0);
        let cycles = cpu.step(&mut bus);
        assert!(cycles > 0);
        assert_eq!(cpu.regs.pc, 2);
        assert_eq!(cpu.state, CpuState::Running);
    }

    #[test]
    fn step_moveq() {
        // MOVEQ #42, D0 = 0x703A
        let mut bus = make_bus_with_program(&[0x703A]);
        let mut cpu = Cpu::new(0);
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.d[0], 42);
    }

    #[test]
    fn breakpoint_pauses() {
        // NOP NOP NOP
        let mut bus = make_bus_with_program(&[0x4E71, 0x4E71, 0x4E71]);
        let mut cpu = Cpu::new(0);
        cpu.add_breakpoint(2);

        cpu.step(&mut bus); // PC=0 → 正常执行
        assert_eq!(cpu.state, CpuState::Running);

        cpu.step(&mut bus); // PC=2 → 命中断点
        assert_eq!(cpu.state, CpuState::Paused);

        cpu.resume();
        assert_eq!(cpu.state, CpuState::Running);
    }

    #[test]
    fn run_cycle_limit() {
        // BRA -2（自身无限循环）= 0x60FE
        let mut bus = make_bus_with_program(&[0x60FE]);
        let mut cpu = Cpu::new(0);
        let spent = cpu.run(&mut bus, 200);
        assert!(spent <= 200);
        assert_eq!(cpu.state, CpuState::Running);
    }

    #[test]
    fn rts_returns() {
        // JSR 0x0010（绝对长地址 = 0x4EB9 0x0000 0x0010）
        // 在 0x10 放 RTS（0x4E75）
        let mut bus = Bus::new();
        // JSR abs.l 0x00000010
        let jsr: &[u8] = &[0x4E, 0xB9, 0x00, 0x00, 0x00, 0x10];
        for (i, &b) in jsr.iter().enumerate() {
            bus.write_u8(i as u32, b);
        }
        // RTS at 0x10
        bus.write_u8(0x10, 0x4E);
        bus.write_u8(0x11, 0x75);
        // 初始栈
        let mut cpu = Cpu::new(0);
        cpu.regs.a[7] = 0x0010_0000;

        cpu.step(&mut bus); // JSR → 跳到 0x10，压入返回地址 0x6
        assert_eq!(cpu.regs.pc, 0x10);

        cpu.step(&mut bus); // RTS → 弹出返回地址
        assert_eq!(cpu.regs.pc, 6);
    }
}
