//! 时钟与调度框架（轻量骨架）。
//!
//! 第一阶段只定义接口和数据结构，不做真正的多 CPU 协调。
//! 第三阶段会在此基础上扩展。
//!
//! # Model 2 各处理器时钟
//!
//! | 处理器       | 频率   |
//! |------------|--------|
//! | i960 主 CPU | 25 MHz |
//! | 68000 音频  | 10 MHz |
//! | YM3834      |  8 MHz |

/// 时钟频率（Hz）。
pub type Hz = u64;

/// 单个时钟域：跟踪某个设备已执行的周期数和目标周期数。
#[derive(Debug, Clone)]
pub struct Clock {
    /// 时钟名称（用于调试）。
    pub name: &'static str,
    /// 时钟频率（Hz）。
    pub freq: Hz,
    /// 已执行的总周期数。
    pub cycles: u64,
}

impl Clock {
    pub const fn new(name: &'static str, freq: Hz) -> Self {
        Self {
            name,
            freq,
            cycles: 0,
        }
    }

    /// 将已执行周期数转换为纳秒。
    pub fn elapsed_ns(&self) -> u64 {
        self.cycles * 1_000_000_000 / self.freq
    }

    /// 给定目标时间（纳秒），还需要运行多少个周期。
    pub fn cycles_until_ns(&self, target_ns: u64) -> u64 {
        let elapsed = self.elapsed_ns();
        if target_ns <= elapsed {
            return 0;
        }
        (target_ns - elapsed) * self.freq / 1_000_000_000
    }

    /// 记录已执行 `n` 个周期。
    #[inline]
    pub fn advance(&mut self, n: u64) {
        self.cycles += n;
    }
}

// ---------------------------------------------------------------------------
// Model 2 预定义时钟
// ---------------------------------------------------------------------------

pub const CLOCK_I960: Clock = Clock::new("i960", 25_000_000);
pub const CLOCK_M68K: Clock = Clock::new("m68k", 10_000_000);
pub const CLOCK_YM3834: Clock = Clock::new("ym3834", 8_000_000);

// ---------------------------------------------------------------------------
// 简单调度器骨架
// ---------------------------------------------------------------------------

/// 执行令牌：调度器发给某个处理器，告知它可以运行多少个周期。
#[derive(Debug, Clone, Copy)]
pub struct RunToken {
    /// 允许运行的最大周期数。
    pub cycles: u64,
}

/// 处理器必须实现的调度接口（第二阶段起各 CPU crate 实现此 trait）。
pub trait Tickable {
    /// 运行最多 `token.cycles` 个周期，返回实际运行的周期数。
    fn tick(&mut self, token: RunToken) -> u64;
}

/// 轻量调度器：按时间片轮流驱动各处理器（第三阶段完善）。
///
/// 当前是骨架，只持有时钟状态，不真正调度。
pub struct Scheduler {
    pub i960: Clock,
    pub m68k: Clock,
    pub ym3834: Clock,
    /// 当前全局时间（纳秒）。
    pub now_ns: u64,
}

impl Scheduler {
    pub fn new() -> Self {
        Self {
            i960: CLOCK_I960,
            m68k: CLOCK_M68K,
            ym3834: CLOCK_YM3834,
            now_ns: 0,
        }
    }

    /// 推进全局时间 `delta_ns` 纳秒，返回各处理器应运行的周期数。
    ///
    /// 第三阶段会把这里改成真正驱动 `Tickable` 的循环。
    pub fn advance_ns(&mut self, delta_ns: u64) -> (u64, u64, u64) {
        let i960_cycles = self.i960.cycles_until_ns(self.now_ns + delta_ns);
        let m68k_cycles = self.m68k.cycles_until_ns(self.now_ns + delta_ns);
        let ym_cycles = self.ym3834.cycles_until_ns(self.now_ns + delta_ns);
        self.now_ns += delta_ns;
        (i960_cycles, m68k_cycles, ym_cycles)
    }
}

impl Default for Scheduler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_elapsed() {
        let mut c = Clock::new("test", 1_000_000); // 1 MHz
        c.advance(1_000_000); // 1 秒
        assert_eq!(c.elapsed_ns(), 1_000_000_000);
    }

    #[test]
    fn cycles_until() {
        let c = Clock::new("test", 1_000); // 1 KHz
        // 目标 1 秒后 = 1_000_000_000 ns → 需要 1000 cycles
        assert_eq!(c.cycles_until_ns(1_000_000_000), 1_000);
    }

    #[test]
    fn scheduler_advance() {
        let mut s = Scheduler::new();
        let (i, m, y) = s.advance_ns(1_000_000); // 1 ms
        // i960 25 MHz → 25000 cycles/ms
        assert_eq!(i, 25_000);
        // m68k 10 MHz → 10000 cycles/ms
        assert_eq!(m, 10_000);
        // ym3834 8 MHz → 8000 cycles/ms
        assert_eq!(y, 8_000);
    }
}
