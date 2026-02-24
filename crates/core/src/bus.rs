//! 系统总线。
//!
//! [`Bus`] 是对 [`Memory`] 的封装，负责按 Model 2 的地址映射初始化内存子系统，
//! 并提供统一的读写入口给各 CPU 和外设使用。
//!
//! 第一阶段只做最基础的 RAM/ROM 映射，I/O 回调留作桩，后续各模块自己注册。

use crate::mem::{Memory, PAGE_SIZE, ReadFn, WriteFn};
use log::info;

/// Model 2 主系统总线。
pub struct Bus {
    pub mem: Memory,
}

impl Bus {
    /// 创建总线并初始化 RAM 区域（ROM 由 [`crate::loader`] 另行加载）。
    pub fn new() -> Self {
        let mut mem = Memory::new();

        // 主 RAM：0x0000_0000 - 0x001F_FFFF（2 MB）
        mem.map_ram(0x0000_0000, 0x0020_0000);
        info!("mapped MainRam    0x00000000 - 0x001FFFFF (2 MB)");

        // 图形 RAM：0x0020_0000 - 0x003F_FFFF（2 MB）
        mem.map_ram(0x0020_0000, 0x0020_0000);
        info!("mapped GraphicsRam 0x00200000 - 0x003FFFFF (2 MB)");

        Self { mem }
    }

    // -----------------------------------------------------------------------
    // 转发给 Memory 的读写接口（CPU 通过这里访问总线）
    // -----------------------------------------------------------------------

    #[inline]
    pub fn read_u8(&self, addr: u32) -> u8 {
        self.mem.read_u8(addr)
    }
    #[inline]
    pub fn read_u16(&self, addr: u32) -> u16 {
        self.mem.read_u16(addr)
    }
    #[inline]
    pub fn read_u32(&self, addr: u32) -> u32 {
        self.mem.read_u32(addr)
    }
    #[inline]
    pub fn write_u8(&mut self, addr: u32, v: u8) {
        self.mem.write_u8(addr, v)
    }
    #[inline]
    pub fn write_u16(&mut self, addr: u32, v: u16) {
        self.mem.write_u16(addr, v)
    }
    #[inline]
    pub fn write_u32(&mut self, addr: u32, v: u32) {
        self.mem.write_u32(addr, v)
    }

    // -----------------------------------------------------------------------
    // 动态注册接口（供后续模块使用）
    // -----------------------------------------------------------------------

    /// 将一段 ROM 数据映射到地址空间。
    pub fn load_rom(&mut self, start: u32, data: &[u8]) {
        // map_rom 要求大小是 PAGE_SIZE 的整数倍，不足时补零
        let aligned_size = align_up(data.len(), PAGE_SIZE);
        if aligned_size != data.len() {
            let mut padded = vec![0u8; aligned_size];
            padded[..data.len()].copy_from_slice(data);
            self.mem.map_rom(start, &padded);
        } else {
            self.mem.map_rom(start, data);
        }
        info!(
            "mapped ROM        {:#010x} - {:#010x} ({} KB)",
            start,
            start + aligned_size as u32 - 1,
            aligned_size / 1024,
        );
    }

    /// 注册一段 I/O 回调。
    pub fn register_io(
        &mut self,
        start: u32,
        end: u32,
        read: Option<ReadFn>,
        write: Option<WriteFn>,
    ) {
        self.mem.map_io(start, end, read, write);
        info!("mapped IO         {:#010x} - {:#010x}", start, end);
    }

    /// 调试用：hexdump 一段内存。
    pub fn hexdump(&self, addr: u32, len: usize) {
        self.mem.hexdump(addr, len);
    }
}

impl Default for Bus {
    fn default() -> Self {
        Self::new()
    }
}

fn align_up(x: usize, align: usize) -> usize {
    (x + align - 1) & !(align - 1)
}
