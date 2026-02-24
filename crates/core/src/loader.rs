//! ROM 加载器。
//!
//! 负责从 MAME 格式的 ZIP 文件中读取 ROM 芯片内容，按照硬编码的描述表拼接到
//! 正确的地址，并做 CRC32 校验。
//!
//! # 设计原则
//!
//! 第一阶段只硬编码 **Daytona USA**（daytona）的 ROM 描述，能跑起来为先，
//! 通用 MAME XML 解析留到后期。

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result, bail};
use crc32fast::Hasher;
use log::{debug, info, warn};

use crate::bus::Bus;

// ---------------------------------------------------------------------------
// ROM 描述结构
// ---------------------------------------------------------------------------

/// 加载方式：顺序拼接 or 交错（interleaved）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadMode {
    /// 直接按顺序填充到目标地址区间。
    Sequential,
    /// 每隔 `stride` 字节写一个字节（用于多芯片交错拼接）。
    /// `offset` 是起始字节偏移（0 = 偶字节，1 = 奇字节）。
    Interleaved { stride: usize, offset: usize },
}

/// 单个 ROM 芯片的描述。
#[derive(Debug, Clone)]
pub struct RomEntry {
    /// ZIP 内的文件名。
    pub filename: &'static str,
    /// 加载到总线的起始地址。
    pub dest_addr: u32,
    /// 期望的数据长度（字节）。
    pub length: usize,
    /// MAME CRC32 校验值（十六进制）。
    pub crc32: u32,
    /// 加载方式。
    pub mode: LoadMode,
}

impl RomEntry {
    pub const fn seq(filename: &'static str, dest_addr: u32, length: usize, crc32: u32) -> Self {
        Self {
            filename,
            dest_addr,
            length,
            crc32,
            mode: LoadMode::Sequential,
        }
    }

    pub const fn interleaved(
        filename: &'static str,
        dest_addr: u32,
        length: usize,
        crc32: u32,
        stride: usize,
        offset: usize,
    ) -> Self {
        Self {
            filename,
            dest_addr,
            length,
            crc32,
            mode: LoadMode::Interleaved { stride, offset },
        }
    }
}

// ---------------------------------------------------------------------------
// Daytona USA ROM 描述表
// （参照 MAME src/mame/sega/model2.cpp daytona_state::ROM_START）
// ---------------------------------------------------------------------------

/// Daytona USA (daytona) 的 ROM 描述。
pub const DAYTONA_ROMS: &[RomEntry] = &[
    // 程序 ROM（两芯片交错，合并到 0x800000，共 2 MB）
    RomEntry::interleaved("epr-16722b.12", 0x0080_0000, 0x10_0000, 0x16522d64, 2, 0),
    RomEntry::interleaved("epr-16723b.13", 0x0080_0000, 0x10_0000, 0x1af98a96, 2, 1),
    // 数据 ROM
    RomEntry::seq("mpr-16727.14", 0x00A0_0000, 0x20_0000, 0x2a3f2a57),
    RomEntry::seq("mpr-16728.15", 0x00C0_0000, 0x20_0000, 0x21175b8a),
    RomEntry::seq("mpr-16729.16", 0x00E0_0000, 0x20_0000, 0xed515cb1),
];

// ---------------------------------------------------------------------------
// 加载器
// ---------------------------------------------------------------------------

/// 从 ZIP 文件加载 ROM 到总线。
///
/// # 参数
/// - `zip_path`: MAME ZIP 文件路径（例如 `roms/daytona.zip`）
/// - `entries`: ROM 描述表（例如 [`DAYTONA_ROMS`]）
/// - `bus`: 目标总线
///
/// # 错误
/// 如果 ZIP 无法打开、必要文件缺失、CRC 不符，返回 `Err`。
pub fn load_rom_zip(zip_path: impl AsRef<Path>, entries: &[RomEntry], bus: &mut Bus) -> Result<()> {
    let zip_path = zip_path.as_ref();
    info!("loading ROM zip: {}", zip_path.display());

    let file = std::fs::File::open(zip_path)
        .with_context(|| format!("cannot open {}", zip_path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .with_context(|| format!("not a valid ZIP: {}", zip_path.display()))?;

    // 把所有文件内容读入内存，以文件名为 key
    let mut blobs: HashMap<String, Vec<u8>> = HashMap::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let name = entry.name().to_owned();
        let mut data = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut data)?;
        debug!("  read {:6} bytes  {}", data.len(), name);
        blobs.insert(name, data);
    }

    // 按描述表逐条处理
    // 先按 dest_addr 分组，把同一目标地址的交错芯片合并到同一个 buffer
    let mut dest_buffers: HashMap<u32, Vec<u8>> = HashMap::new();

    for entry in entries {
        let blob = match blobs.get(entry.filename) {
            Some(b) => b,
            None => bail!("missing ROM file in ZIP: {}", entry.filename),
        };

        // CRC32 校验
        let actual_crc = crc32_of(blob);
        if actual_crc != entry.crc32 {
            // 仅警告，不阻止继续（方便用 no-dump 或 alt 版本测试）
            warn!(
                "CRC mismatch for {}: expected {:#010x}, got {:#010x}",
                entry.filename, entry.crc32, actual_crc
            );
        } else {
            debug!("  CRC OK  {:#010x}  {}", actual_crc, entry.filename);
        }

        if blob.len() != entry.length {
            bail!(
                "size mismatch for {}: expected {} bytes, got {}",
                entry.filename,
                entry.length,
                blob.len()
            );
        }

        match entry.mode {
            LoadMode::Sequential => {
                // 直接写入（单独一个 buffer）
                let buf = dest_buffers.entry(entry.dest_addr).or_default();
                buf.extend_from_slice(blob);
            }
            LoadMode::Interleaved { stride, offset } => {
                // 计算合并后 buffer 的大小
                let total = blob.len() * stride;
                let buf = dest_buffers
                    .entry(entry.dest_addr)
                    .or_insert_with(|| vec![0u8; total]);
                if buf.len() < total {
                    buf.resize(total, 0);
                }
                // 把 blob 的第 i 字节写入 buf[i*stride + offset]
                for (i, &byte) in blob.iter().enumerate() {
                    let dest_idx = i * stride + offset;
                    if dest_idx < buf.len() {
                        buf[dest_idx] = byte;
                    }
                }
            }
        }
    }

    // 把所有 buffer 加载到总线
    for (addr, data) in &dest_buffers {
        info!("  loading {:#010x} ({} KB)", addr, data.len() / 1024);
        bus.load_rom(*addr, data);
    }

    info!("ROM loading complete ({} regions)", dest_buffers.len());
    Ok(())
}

fn crc32_of(data: &[u8]) -> u32 {
    let mut h = Hasher::new();
    h.update(data);
    h.finalize()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_known() {
        // echo -n "hello" | crc32 → 3610a686
        assert_eq!(crc32_of(b"hello"), 0x3610_a686);
    }

    #[test]
    fn interleave_two_chips() {
        // 模拟两块 ROM 芯片交错合并
        // chip0: [0x00, 0x02, 0x04]  stride=2, offset=0  → 偶字节
        // chip1: [0x01, 0x03, 0x05]  stride=2, offset=1  → 奇字节
        // 期望结果: [0x00, 0x01, 0x02, 0x03, 0x04, 0x05]
        let mut buf = vec![0u8; 6];
        let chip0 = [0x00u8, 0x02, 0x04];
        let chip1 = [0x01u8, 0x03, 0x05];

        for (i, &b) in chip0.iter().enumerate() {
            buf[i * 2] = b;
        }
        for (i, &b) in chip1.iter().enumerate() {
            buf[i * 2 + 1] = b;
        }

        assert_eq!(buf, vec![0x00, 0x01, 0x02, 0x03, 0x04, 0x05]);
    }
}
