//! TGP 顶层结构体。
//!
//! [`Tgp`] 是对外接口，整合了：
//! - FIFO 对（i960 ↔ TGP）
//! - 硬件数学单元（sin/cos、inv、isqrt、atan）
//! - 几何引擎（矩阵变换、多边形处理）
//! - 控制寄存器（copro_ctl、geo_ctl）
//! - buffer RAM（变换后多边形数据暂存区）
//!
//! # i960 访问接口
//!
//! i960 通过以下地址访问 TGP（参照 MAME model2.cpp addr map）：
//!
//! | 地址范围               | 方向     | 说明                         |
//! |----------------------|----------|------------------------------|
//! | 0x00880000–0x00883FFF | i960写   | Function port（命令上传）     |
//! | 0x00884000–0x00887FFF | 读写     | FIFO 端口                     |
//! | 0x00980000–0x00980003 | 读写     | copro_ctl1 控制寄存器         |
//! | 0x00980008–0x0098000B | 写       | geo_ctl1 控制寄存器           |
//! | 0x00020–0x00023       | 读写     | sin/cos 单元                  |
//! | 0x00024–0x00027       | 读写     | atan 单元                     |
//! | 0x00028–0x00029       | 读写     | inv 单元                      |
//! | 0x0002A–0x0002B       | 读写     | isqrt 单元                    |

use log::{debug, warn};

use crate::fifo::FifoPair;
use crate::geo::GeoEngine;
use crate::math::MathUnits;

// ---------------------------------------------------------------------------
// Buffer RAM
// ---------------------------------------------------------------------------

/// TGP / GEO buffer RAM 大小（参照 MAME：0x400000 字节 = 4 MB）。
const BUFFER_RAM_SIZE: usize = 0x40_0000;

// ---------------------------------------------------------------------------
// 控制寄存器
// ---------------------------------------------------------------------------

/// copro_ctl1 寄存器（0x00980000）。
///
/// bit 0：TGP 复位（写1复位，写0开始运行）。
#[derive(Debug, Default, Clone, Copy)]
pub struct CoproCtl {
    pub reset: bool,
    pub raw: u32,
}

/// geo_ctl1 寄存器（0x00980008）。
#[derive(Debug, Default, Clone, Copy)]
pub struct GeoCtl {
    pub raw: u32,
}

// ---------------------------------------------------------------------------
// TGP 主结构体
// ---------------------------------------------------------------------------

/// Sega Model 2 TGP（Triangle Generation Processor）。
pub struct Tgp {
    /// FIFO 对。
    pub fifo: FifoPair,
    /// 硬件数学单元。
    pub math: MathUnits,
    /// 几何引擎。
    pub geo: GeoEngine,
    /// Buffer RAM（几何数据暂存）。
    pub buffer_ram: Vec<u8>,
    /// Banking 寄存器（控制 buffer RAM 分页）。
    pub bank_reg: u32,
    /// copro_ctl1。
    pub copro_ctl: CoproCtl,
    /// geo_ctl1。
    pub geo_ctl: GeoCtl,
    /// TGP 是否已启动（未复位）。
    pub running: bool,
}

impl Tgp {
    pub fn new() -> Self {
        Self {
            fifo: FifoPair::new(),
            math: MathUnits::default(),
            geo: GeoEngine::new(),
            buffer_ram: vec![0u8; BUFFER_RAM_SIZE],
            bank_reg: 0,
            copro_ctl: CoproCtl::default(),
            geo_ctl: GeoCtl::default(),
            running: false,
        }
    }

    // -----------------------------------------------------------------------
    // 每帧驱动
    // -----------------------------------------------------------------------

    /// 处理一个时间片：消耗 input FIFO 中所有命令，更新几何引擎。
    ///
    /// 调度器每帧调用一次（或按需调用）。
    pub fn tick(&mut self) {
        if !self.running {
            return;
        }
        self.geo.process(&mut self.fifo);
    }

    /// 帧开始：清空上一帧多边形列表。
    pub fn begin_frame(&mut self) {
        self.geo.begin_frame();
    }

    // -----------------------------------------------------------------------
    // i960 写接口
    // -----------------------------------------------------------------------

    /// **Function port 写**（0x00880000–0x00883FFF）。
    ///
    /// 地址的 `[23:16]` 位编码 function code，数据的低 23 位是参数。
    pub fn function_port_write(&mut self, addr: u32, data: u32) {
        // function code 从地址中提取（bits [30:23]）
        let func = ((addr >> 2) & 0xFF) as u8;
        // 把完整的命令字（含 func 字段）压入 input FIFO
        let cmd = ((func as u32) << 23) | (data & 0x7F_FFFF);
        debug!("function_port_write func={:#04x} data={:#010x}", func, data);
        self.fifo.cpu_write(cmd);
        // 立即处理（function port 是同步接口）
        if self.running {
            self.geo.process(&mut self.fifo);
        }
    }

    /// **FIFO 端口写**（0x00884000）：i960 把数据推入 input FIFO。
    pub fn fifo_write(&mut self, val: u32) {
        self.fifo.cpu_write(val);
    }

    /// **FIFO 端口读**（0x00884000）：i960 从 output FIFO 读取 TGP 返回值。
    pub fn fifo_read(&mut self) -> u32 {
        self.fifo.cpu_read()
    }

    /// **copro_ctl1 写**（0x00980000）。
    pub fn copro_ctl_write(&mut self, val: u32) {
        self.copro_ctl.raw = val;
        self.copro_ctl.reset = val & 1 != 0;
        if self.copro_ctl.reset {
            self.reset();
        } else {
            self.running = true;
            debug!("TGP: started");
        }
    }

    /// **copro_ctl1 读**（0x00980000）。
    pub fn copro_ctl_read(&self) -> u32 {
        // bit 0 返回 ready 状态（FIFO output 非空时为 1）
        if self.fifo.output.is_empty() { 0 } else { 1 }
    }

    /// **geo_ctl1 写**（0x00980008）。
    pub fn geo_ctl_write(&mut self, val: u32) {
        self.geo_ctl.raw = val;
        debug!("TGP: geo_ctl1 = {:#010x}", val);
    }

    // -----------------------------------------------------------------------
    // 数学单元 I/O 寄存器接口
    // -----------------------------------------------------------------------

    /// 数学单元写（地址相对于 I/O 基地址）。
    pub fn math_write(&mut self, offset: u32, val: u32) {
        match offset {
            0x20..=0x23 => self.math.sincos.write(val),
            0x24..=0x27 => self.math.atan.write(val),
            0x28..=0x29 => self.math.inv.write(val),
            0x2A..=0x2B => self.math.isqrt.write(val),
            _ => warn!("TGP math write: unknown offset {:#x}", offset),
        }
    }

    /// 数学单元读。
    pub fn math_read(&mut self, offset: u32) -> u32 {
        match offset {
            0x20..=0x23 => self.math.sincos.read(),
            0x24..=0x27 => self.math.atan.read(),
            0x28..=0x29 => self.math.inv.read(),
            0x2A..=0x2B => self.math.isqrt.read(),
            _ => {
                warn!("TGP math read: unknown offset {:#x}", offset);
                0
            }
        }
    }

    // -----------------------------------------------------------------------
    // Buffer RAM 接口
    // -----------------------------------------------------------------------

    /// 读取 buffer RAM（银行寻址）。
    pub fn buffer_ram_read(&self, addr: u32) -> u32 {
        let phy = self.bank_addr(addr);
        if phy + 3 >= BUFFER_RAM_SIZE {
            warn!("buffer_ram_read OOB {:#x}", phy);
            return 0;
        }
        u32::from_le_bytes([
            self.buffer_ram[phy],
            self.buffer_ram[phy + 1],
            self.buffer_ram[phy + 2],
            self.buffer_ram[phy + 3],
        ])
    }

    /// 写入 buffer RAM。
    pub fn buffer_ram_write(&mut self, addr: u32, val: u32) {
        let phy = self.bank_addr(addr);
        if phy + 3 >= BUFFER_RAM_SIZE {
            warn!("buffer_ram_write OOB {:#x}", phy);
            return;
        }
        let bytes = val.to_le_bytes();
        self.buffer_ram[phy..phy + 4].copy_from_slice(&bytes);
    }

    /// bank_reg 映射（参照 MAME copro_tgp_memory_r/w）。
    fn bank_addr(&self, addr: u32) -> usize {
        let bank = (self.bank_reg & 0xF) as usize;
        ((bank * 0x10000) + (addr as usize & 0xFFFF)) * 4
    }

    // -----------------------------------------------------------------------
    // 复位
    // -----------------------------------------------------------------------

    pub fn reset(&mut self) {
        self.fifo.input.clear();
        self.fifo.output.clear();
        self.geo = GeoEngine::new();
        self.running = false;
        debug!("TGP: reset");
    }

    // -----------------------------------------------------------------------
    // 调试
    // -----------------------------------------------------------------------

    /// 打印当前状态摘要。
    pub fn dump_status(&self) {
        println!("TGP status:");
        println!("  running:     {}", self.running);
        println!("  input FIFO:  {} words", self.fifo.input.len());
        println!("  output FIFO: {} words", self.fifo.output.len());
        println!("  matrix stack depth: {}", self.geo.model_stack.depth());
        println!("  polys this frame:   {}", self.geo.output_polys.len());
        println!("  bank_reg:    {:#010x}", self.bank_reg);
    }
}

impl Default for Tgp {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_clears_fifo() {
        let mut tgp = Tgp::new();
        tgp.fifo.cpu_write(0xDEAD_BEEF);
        tgp.reset();
        assert!(tgp.fifo.input.is_empty());
        assert!(!tgp.running);
    }

    #[test]
    fn start_stop_via_ctl() {
        let mut tgp = Tgp::new();
        tgp.copro_ctl_write(0); // start
        assert!(tgp.running);
        tgp.copro_ctl_write(1); // reset
        assert!(!tgp.running);
    }

    #[test]
    fn math_sincos_via_io() {
        let mut tgp = Tgp::new();
        tgp.copro_ctl_write(0); // start

        // 写入角度 0 → sin=0, cos=1
        tgp.math_write(0x20, 0);
        let sin_bits = tgp.math_read(0x20);
        let cos_bits = tgp.math_read(0x20);
        let s = f32::from_bits(sin_bits);
        let c = f32::from_bits(cos_bits);
        assert!((s - 0.0).abs() < 1e-4, "sin(0)={}", s);
        assert!((c - 1.0).abs() < 1e-4, "cos(0)={}", c);
    }

    #[test]
    fn function_port_nop() {
        let mut tgp = Tgp::new();
        tgp.copro_ctl_write(0);
        // func=0x00 (NOP)
        tgp.function_port_write(0x00880000, 0);
        assert!(tgp.fifo.input.is_empty()); // NOP 被立即消耗
    }

    #[test]
    fn buffer_ram_rw() {
        let mut tgp = Tgp::new();
        tgp.buffer_ram_write(0, 0xCAFE_BABE);
        assert_eq!(tgp.buffer_ram_read(0), 0xCAFE_BABE);
    }
}
