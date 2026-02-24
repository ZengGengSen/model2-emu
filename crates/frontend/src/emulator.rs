//! 模拟器核心：持有所有子系统并按帧驱动它们。
//!
//! # 帧驱动模型
//!
//! Model 2 以 60 Hz 运行（帧周期 ≈ 16.667 ms）。
//! 每次 `tick_frame()` 调用对应一个视频帧，内部把时间切片分发给各 CPU：
//!
//! ```text
//! tick_frame()
//!   ├─ i960  运行 25_000_000 / 60 ≈ 416_666 cycles
//!   ├─ m68k  运行 10_000_000 / 60 ≈ 166_666 cycles
//!   └─ TGP   处理所有待命令（非周期驱动，按需消耗）
//! ```
//!
//! # ROM 加载
//!
//! ROM 路径由调用方传入（通常来自命令行参数或环境变量）。
//! 找不到 ROM 时不崩溃——模拟器以 "no-ROM" 模式启动，CPU 从全零内存执行
//! （会很快命中 `Unimplemented` 停机，但窗口正常显示）。

use std::path::Path;
use std::time::{Duration, Instant};

use log::{info, warn};

use model2_core::bus::Bus;
use model2_core::clock::RunToken;
use model2_core::loader::{self, DAYTONA_ROMS};
use model2_cpu_i960::cpu::{Cpu as I960, CpuState as I960State};
use model2_cpu_m68k::cpu::{Cpu as M68k, CpuState as M68kState};
use model2_tgp::Tgp;

// ---------------------------------------------------------------------------
// 时钟常数
// ---------------------------------------------------------------------------

/// 目标帧率（Hz）。
const TARGET_FPS: u64 = 60;
/// 每帧时长（纳秒）。
pub const FRAME_NS: u64 = 1_000_000_000 / TARGET_FPS;

/// i960 每帧应执行的时钟周期数。
const I960_CYCLES_PER_FRAME: u64 = 25_000_000 / TARGET_FPS;
/// m68k 每帧应执行的时钟周期数。
const M68K_CYCLES_PER_FRAME: u64 = 10_000_000 / TARGET_FPS;

// ---------------------------------------------------------------------------
// i960 复位入口
// ---------------------------------------------------------------------------

/// Daytona USA 程序 ROM 起始地址（i960 从这里开始执行）。
const I960_ENTRY: u32 = 0x0080_0000;
/// m68k 音频 CPU 复位入口（实际由主 CPU 写入，这里先硬编码一个安全地址）。
const M68K_ENTRY: u32 = 0x0000_0000;

// ---------------------------------------------------------------------------
// Emulator
// ---------------------------------------------------------------------------

/// 帧统计信息（调试 / 标题栏显示用）。
#[derive(Debug, Default, Clone, Copy)]
pub struct FrameStats {
    pub frame_count: u64,
    pub i960_cycles: u64,
    pub m68k_cycles: u64,
    pub frame_time_us: u64, // 上一帧实际耗时（微秒）
}

/// 模拟器顶层结构，持有所有子系统。
pub struct Emulator {
    /// 系统总线（内存、ROM、I/O）。
    pub bus: Bus,
    /// i960 主 CPU（25 MHz）。
    pub i960: I960,
    /// 68000 音频 CPU（10 MHz）。
    pub m68k: M68k,
    /// TGP 几何处理器。
    pub tgp: Tgp,
    /// 是否已加载 ROM。
    pub rom_loaded: bool,
    /// 帧统计。
    pub stats: FrameStats,
    /// 上一帧开始时间（用于帧时间测量）。
    frame_start: Instant,
}

impl Emulator {
    // -----------------------------------------------------------------------
    // 构造
    // -----------------------------------------------------------------------

    /// 创建模拟器，不加载 ROM（CPU 从全零内存起跑）。
    pub fn new() -> Self {
        let bus = Bus::new();
        let i960 = I960::new(I960_ENTRY);
        let m68k = M68k::new(M68K_ENTRY);
        let tgp = Tgp::new();

        Self {
            bus,
            i960,
            m68k,
            tgp,
            rom_loaded: false,
            stats: FrameStats::default(),
            frame_start: Instant::now(),
        }
    }

    /// 尝试从 ZIP 文件加载 Daytona USA ROM。
    ///
    /// 失败时记录警告并继续（no-ROM 模式）。
    pub fn load_daytona(&mut self, zip_path: &Path) {
        match loader::load_rom_zip(zip_path, DAYTONA_ROMS, &mut self.bus) {
            Ok(()) => {
                info!("ROM loaded successfully: {}", zip_path.display());
                self.rom_loaded = true;
                // TGP 启动（写 copro_ctl = 0 = 不复位）
                self.tgp.copro_ctl_write(0);
            }
            Err(e) => {
                warn!("ROM load failed ({}): running in no-ROM mode", e);
            }
        }
    }

    /// 尝试从环境变量 `MODEL2_ROM` 指定的路径加载 ROM。
    /// 变量未设置或文件不存在时静默跳过。
    pub fn try_load_rom_from_env(&mut self) {
        if let Ok(path) = std::env::var("MODEL2_ROM") {
            self.load_daytona(Path::new(&path));
        } else {
            warn!("MODEL2_ROM not set; running in no-ROM mode");
            warn!("  Set MODEL2_ROM=/path/to/daytona.zip to load a game");
        }
    }

    // -----------------------------------------------------------------------
    // 每帧驱动（由窗口事件循环调用）
    // -----------------------------------------------------------------------

    /// 驱动一帧：按比例给各 CPU 分配时间片，处理 TGP 命令。
    ///
    /// 返回本帧是否正常完成（false = 所有 CPU 均已停机）。
    pub fn tick_frame(&mut self) -> bool {
        let t0 = Instant::now();

        // ── i960 ──────────────────────────────────────────────────────────
        let i960_spent = if self.i960.state == I960State::Running {
            self.i960.tick_with_bus(
                &mut self.bus,
                RunToken {
                    cycles: I960_CYCLES_PER_FRAME,
                },
            )
        } else {
            0
        };

        // ── m68k ──────────────────────────────────────────────────────────
        let m68k_spent = if self.m68k.state == M68kState::Running {
            self.m68k.tick_with_bus(
                &mut self.bus,
                RunToken {
                    cycles: M68K_CYCLES_PER_FRAME,
                },
            )
        } else {
            0
        };

        // ── TGP ───────────────────────────────────────────────────────────
        self.tgp.tick();

        // ── 统计 ──────────────────────────────────────────────────────────
        self.stats.frame_count += 1;
        self.stats.i960_cycles += i960_spent;
        self.stats.m68k_cycles += m68k_spent;
        self.stats.frame_time_us = t0.elapsed().as_micros() as u64;

        // 只要有一个 CPU 还在跑就认为帧有效
        self.i960.state == I960State::Running || self.m68k.state == M68kState::Running
    }

    // -----------------------------------------------------------------------
    // 帧速率限制（可选）
    // -----------------------------------------------------------------------

    /// 如果本帧执行太快，睡眠至帧周期结束。
    ///
    /// 在 `ControlFlow::Poll` 模式下调用此函数可以把 CPU 占用率压到合理水平。
    /// 如果模拟器本身跑不够快（帧时间 > 16.67ms），直接返回不等待。
    pub fn wait_for_frame_end(&mut self) {
        let elapsed = self.frame_start.elapsed();
        let target = Duration::from_nanos(FRAME_NS);
        if elapsed < target {
            std::thread::sleep(target - elapsed);
        }
        self.frame_start = Instant::now();
    }

    // -----------------------------------------------------------------------
    // 调试
    // -----------------------------------------------------------------------

    /// 打印所有 CPU 的当前寄存器（调试用）。
    pub fn dump_state(&self) {
        println!("=== i960 ===");
        self.i960.dump_regs();
        println!("=== m68k ===");
        self.m68k.dump_regs();
        println!("=== TGP ===");
        self.tgp.dump_status();
        println!("=== Stats ===");
        println!("  frame #{}", self.stats.frame_count);
        println!("  last frame: {} µs", self.stats.frame_time_us);
    }
}

impl Default for Emulator {
    fn default() -> Self {
        Self::new()
    }
}
