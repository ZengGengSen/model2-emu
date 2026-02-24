//! FIFO 队列：连接 i960 主 CPU 与 TGP 协处理器。
//!
//! # 硬件结构
//!
//! Model 2 有两条 FIFO：
//!
//! - **Input FIFO**（`copro_fifo_in`）：i960 写入，TGP 读取。
//!   i960 通过写 `0x00884000` 把几何命令和数据压入此队列。
//!
//! - **Output FIFO**（`copro_fifo_out`）：TGP 写入，i960 读取。
//!   TGP 把变换结果（顶点、法线等）写入此队列，i960 从 `0x00884000` 读取。
//!
//! 两条 FIFO 在地址上共享同一端口（读 → output，写 → input）。
//!
//! # 容量
//!
//! MAME 实现使用无界 FIFO（`GENERIC_FIFO_U32`），这里用 `VecDeque` 模拟，
//! 设置一个软上限用于调试警告。

use log::warn;
use std::collections::VecDeque;

/// FIFO 软上限（超过时发出警告，不阻塞）。
const FIFO_WARN_DEPTH: usize = 4096;

/// 单方向 32 位 FIFO。
#[derive(Debug, Default)]
pub struct Fifo {
    queue: VecDeque<u32>,
    name: &'static str,
}

impl Fifo {
    pub fn new(name: &'static str) -> Self {
        Self {
            queue: VecDeque::new(),
            name,
        }
    }

    /// 压入一个字。
    pub fn push(&mut self, val: u32) {
        if self.queue.len() >= FIFO_WARN_DEPTH {
            warn!(
                "FIFO '{}' depth {} exceeds warning threshold",
                self.name,
                self.queue.len()
            );
        }
        self.queue.push_back(val);
    }

    /// 弹出一个字；队列空时返回 `None`。
    pub fn pop(&mut self) -> Option<u32> {
        self.queue.pop_front()
    }

    /// 查看队首（不弹出）。
    pub fn peek(&self) -> Option<u32> {
        self.queue.front().copied()
    }

    /// 当前深度。
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// 清空。
    pub fn clear(&mut self) {
        self.queue.clear();
    }
}

/// i960 ↔ TGP 的双向 FIFO 对。
#[derive(Debug)]
pub struct FifoPair {
    /// i960 → TGP（i960 写入几何命令）。
    pub input: Fifo,
    /// TGP → i960（TGP 写入变换结果）。
    pub output: Fifo,
}

impl FifoPair {
    pub fn new() -> Self {
        Self {
            input: Fifo::new("copro_fifo_in"),
            output: Fifo::new("copro_fifo_out"),
        }
    }

    // -----------------------------------------------------------------------
    // i960 侧接口（对应总线地址 0x00884000）
    // -----------------------------------------------------------------------

    /// i960 写：把数据压入 input FIFO（发给 TGP）。
    pub fn cpu_write(&mut self, val: u32) {
        self.input.push(val);
    }

    /// i960 读：从 output FIFO 取出 TGP 的返回数据。
    pub fn cpu_read(&mut self) -> u32 {
        match self.output.pop() {
            Some(v) => v,
            None => {
                warn!("i960 read from empty output FIFO");
                0xFFFF_FFFF
            }
        }
    }

    // -----------------------------------------------------------------------
    // TGP 侧接口
    // -----------------------------------------------------------------------

    /// TGP 读：从 input FIFO 取出 i960 发来的命令/数据。
    pub fn tgp_read(&mut self) -> Option<u32> {
        self.input.pop()
    }

    /// TGP 写：把结果压入 output FIFO（返回给 i960）。
    pub fn tgp_write(&mut self, val: u32) {
        self.output.push(val);
    }
}

impl Default for FifoPair {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_flow() {
        let mut pair = FifoPair::new();

        // i960 发两个字给 TGP
        pair.cpu_write(0x0102_0304);
        pair.cpu_write(0xDEAD_BEEF);

        assert_eq!(pair.tgp_read(), Some(0x0102_0304));
        assert_eq!(pair.tgp_read(), Some(0xDEAD_BEEF));
        assert_eq!(pair.tgp_read(), None);

        // TGP 返回一个结果给 i960
        pair.tgp_write(0x1234_5678);
        assert_eq!(pair.cpu_read(), 0x1234_5678);
    }

    #[test]
    fn empty_read_returns_ff() {
        let mut pair = FifoPair::new();
        assert_eq!(pair.cpu_read(), 0xFFFF_FFFF);
    }
}
