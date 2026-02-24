//! i960 CPU 主结构体。
//!
//! [`Cpu`] 把寄存器堆、fetch-decode-execute 循环、调试钩子整合在一起，
//! 并实现 `model2_core::clock::Tickable` 接口供调度器驱动。

use log::{debug, trace, warn};

use model2_core::Bus;
use model2_core::clock::RunToken;

use crate::decode::decode;
use crate::disasm::format_insn;
use crate::execute::execute;
use crate::regs::Regs;

// ---------------------------------------------------------------------------
// 断点
// ---------------------------------------------------------------------------

/// 断点类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakpointKind {
    /// 执行到该地址时中断。
    Execute,
    /// 读取该地址时中断。
    Read,
    /// 写入该地址时中断。
    Write,
}

#[derive(Debug, Clone, Copy)]
pub struct Breakpoint {
    pub addr: u32,
    pub kind: BreakpointKind,
    pub enabled: bool,
}

// ---------------------------------------------------------------------------
// CPU 运行状态
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuState {
    /// 正常运行。
    Running,
    /// 遇到断点，已暂停。
    Paused,
    /// 执行了未实现指令，已停机。
    Halted,
}

// ---------------------------------------------------------------------------
// CPU 主结构体
// ---------------------------------------------------------------------------

/// i960 Kx 解释器。
pub struct Cpu {
    /// 寄存器堆。
    pub regs: Regs,
    /// 运行状态。
    pub state: CpuState,
    /// 断点列表。
    pub breakpoints: Vec<Breakpoint>,
    /// 跟踪模式：每条指令都打印反汇编。
    pub trace: bool,
    /// 累计执行的指令数（用于调试）。
    pub insn_count: u64,
}

impl Cpu {
    /// 创建 CPU，IP 设置为 `entry`（i960 复位入口通常在 0x00000000，
    /// Model 2 会在启动时从 ROM 加载真正的入口地址）。
    pub fn new(entry: u32) -> Self {
        let mut regs = Regs::new();
        regs.ip = entry;
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

    /// 添加执行断点。
    pub fn add_breakpoint(&mut self, addr: u32) {
        self.breakpoints.push(Breakpoint {
            addr,
            kind: BreakpointKind::Execute,
            enabled: true,
        });
        debug!("breakpoint set @ {:#010x}", addr);
    }

    /// 删除断点。
    pub fn remove_breakpoint(&mut self, addr: u32) {
        self.breakpoints.retain(|bp| bp.addr != addr);
    }

    /// 列出所有断点。
    pub fn list_breakpoints(&self) {
        for (i, bp) in self.breakpoints.iter().enumerate() {
            println!(
                "  #{}: {:#010x} {:?} {}",
                i,
                bp.addr,
                bp.kind,
                if bp.enabled { "enabled" } else { "disabled" }
            );
        }
    }

    /// 打印当前寄存器。
    pub fn dump_regs(&self) {
        self.regs.dump();
    }

    /// 恢复运行（从 Paused 状态）。
    pub fn resume(&mut self) {
        if self.state == CpuState::Paused {
            self.state = CpuState::Running;
        }
    }

    // -----------------------------------------------------------------------
    // 单步 / 运行
    // -----------------------------------------------------------------------

    /// 执行单条指令，返回消耗的周期数。
    ///
    /// 如果 CPU 不在 Running 状态，返回 0。
    pub fn step(&mut self, bus: &mut Bus) -> u32 {
        if self.state != CpuState::Running {
            return 0;
        }

        let pc = self.regs.ip;

        // 检查执行断点
        if let Some(bp) = self
            .breakpoints
            .iter()
            .find(|bp| bp.enabled && bp.kind == BreakpointKind::Execute && bp.addr == pc)
        {
            debug!("hit breakpoint @ {:#010x}", bp.addr);
            self.state = CpuState::Paused;
            return 0;
        }

        // Fetch
        let word = bus.read_u32(pc);

        // Decode
        let insn = decode(word);

        // 跟踪输出
        if self.trace {
            let text = format_insn(&insn, pc);
            trace!("{:#010x}  {:08x}  {}", pc, word, text);
        }

        // 推进 IP（先推进，方便跳转指令直接覆写）
        self.regs.ip = pc.wrapping_add(4);

        // Execute
        let cycles = execute(&insn, pc, &mut self.regs, bus);

        // 遇到未实现指令时停机
        if let crate::decode::Insn::Unimplemented { raw } = insn {
            warn!("halting: unimplemented insn {:#010x} @ {:#010x}", raw, pc);
            self.state = CpuState::Halted;
        }

        self.insn_count += 1;
        cycles
    }

    /// 运行最多 `max_cycles` 个周期，返回实际执行的周期数。
    ///
    /// 遇到断点或停机时提前退出。
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
}

// ---------------------------------------------------------------------------
// 实现 Tickable（供调度器使用）
// ---------------------------------------------------------------------------

/// i960 CPU 需要通过一个包装类型持有 Bus 引用才能实现 Tickable，
/// 或者在调度层手动调用 `run`。这里提供一个独立的 tick 函数供调度层使用。
///
/// 真正的 Tickable 实现在调度层完成（因为 Bus 不属于 CPU 所有）。
impl Cpu {
    /// 供调度器调用的 tick 函数：运行最多 `token.cycles` 个周期。
    pub fn tick_with_bus(&mut self, bus: &mut Bus, token: RunToken) -> u64 {
        self.run(bus, token.cycles)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use model2_core::mem::PAGE_SIZE;

    fn make_bus_with_program(words: &[u32]) -> Bus {
        let mut bus = Bus::new();
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        let mut padded = vec![0u8; bytes.len().div_ceil(PAGE_SIZE) * PAGE_SIZE];
        padded[..bytes.len()].copy_from_slice(&bytes);
        bus.load_rom(0x0080_0000, &padded);
        bus
    }

    #[test]
    fn single_step_add() {
        // addo g1, g2, g0（g1=10, g2=32 → g0=42）
        let word: u32 = 0x59u32 << 24 | 2 << 14 | 1;
        let mut bus = make_bus_with_program(&[word]);
        let mut cpu = Cpu::new(0x0080_0000);
        cpu.regs.w(1, 10);
        cpu.regs.w(2, 32);

        let cycles = cpu.step(&mut bus);
        assert!(cycles > 0);
        assert_eq!(cpu.regs.r(0), 42);
        assert_eq!(cpu.regs.ip, 0x0080_0004);
    }

    #[test]
    fn breakpoint_pauses() {
        let word: u32 = 0x59u32 << 24; // 任意合法指令
        let mut bus = make_bus_with_program(&[word, word, word]);
        let mut cpu = Cpu::new(0x0080_0000);
        cpu.add_breakpoint(0x0080_0004); // 第二条指令处断点

        // 第一步：正常执行
        cpu.step(&mut bus);
        assert_eq!(cpu.state, CpuState::Running);

        // 第二步：命中断点
        cpu.step(&mut bus);
        assert_eq!(cpu.state, CpuState::Paused);

        // 恢复后可继续
        cpu.resume();
        assert_eq!(cpu.state, CpuState::Running);
    }

    #[test]
    fn run_cycles_limit() {
        // 无限循环：B 0（跳回自身）
        let b_self: u32 = 0x0800_0000; // B disp=0
        let mut bus = make_bus_with_program(&[b_self]);
        let mut cpu = Cpu::new(0x0080_0000);

        // 设置最大周期，不能跑死
        let spent = cpu.run(&mut bus, 100);
        assert!(spent <= 100);
        assert_eq!(cpu.state, CpuState::Running);
    }
}
