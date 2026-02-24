//! TGP 硬件数学单元。
//!
//! TGP 提供若干专用硬件加速器，i960 通过 I/O 寄存器访问：
//!
//! | 地址         | 功能            | MAME 函数名        |
//! |------------|-----------------|-------------------|
//! | 0x00020–23 | Sin/Cos 查表    | copro_sincos_r/w  |
//! | 0x00024–27 | Atan 反正切     | copro_atan_r/w    |
//! | 0x00028–29 | 1/x 倒数        | copro_inv_r/w     |
//! | 0x0002A–2B | 1/√x 逆平方根  | copro_isqrt_r/w   |
//!
//! # 数值格式
//!
//! TGP 内部使用 **16.16 定点数**（高16位整数部分，低16位小数部分）。
//! 角度用整圈 = 0x10000（即 65536 = 2π），1° ≈ 182.04。
//!
//! 转换公式：
//! - `f32 → fixed16`: `(val * 65536.0) as i32`
//! - `fixed16 → f32`: `(val as f32) / 65536.0`

/// 16.16 定点数类型别名（有符号）。
pub type Fixed16 = i32;
/// 无符号 16.16 定点数（用于幅度）。
pub type UFixed16 = u32;

// ---------------------------------------------------------------------------
// 格式转换
// ---------------------------------------------------------------------------

/// f32 → 16.16 定点（截断）。
#[inline]
pub fn f32_to_fixed(v: f32) -> Fixed16 {
    (v * 65536.0) as i32
}

/// 16.16 定点 → f32。
#[inline]
pub fn fixed_to_f32(v: Fixed16) -> f32 {
    v as f32 / 65536.0
}

/// u32 原始位 → f32（把 TGP 的固定格式字解释为浮点）。
/// TGP 实际上直接使用 IEEE 754 单精度，这里只是方便调用。
#[inline]
pub fn raw_to_f32(v: u32) -> f32 {
    f32::from_bits(v)
}

#[inline]
pub fn f32_to_raw(v: f32) -> u32 {
    v.to_bits()
}

// ---------------------------------------------------------------------------
// 硬件数学单元（高级仿真：直接调用 Rust 内置函数）
// ---------------------------------------------------------------------------

/// Sin/Cos 单元状态机。
///
/// 写入角度，读出 sin（第一次）和 cos（第二次）。
#[derive(Debug, Default)]
pub struct SinCosUnit {
    /// 输入角度（16.16 定点，整圈 = 0x10000）。
    angle: u32,
    /// 预计算的 sin（u32 bits = f32）。
    sin_val: u32,
    /// 预计算的 cos（u32 bits = f32）。
    cos_val: u32,
    /// 读取计数（0 → 返回 sin，1 → 返回 cos）。
    read_idx: u8,
}

impl SinCosUnit {
    /// 写入角度（触发计算）。
    pub fn write(&mut self, angle: u32) {
        self.angle = angle;
        // 把 TGP 角度（整圈=0x10000）转为弧度
        let rad = (angle as f32 / 65536.0) * std::f32::consts::TAU;
        self.sin_val = rad.sin().to_bits();
        self.cos_val = rad.cos().to_bits();
        self.read_idx = 0;
    }

    /// 读取（第一次→sin，第二次→cos）。
    pub fn read(&mut self) -> u32 {
        let val = if self.read_idx == 0 {
            self.sin_val
        } else {
            self.cos_val
        };
        self.read_idx = (self.read_idx + 1) & 1;
        val
    }
}

/// 1/x 倒数单元。
#[derive(Debug, Default)]
pub struct InvUnit {
    result: u32,
}

impl InvUnit {
    pub fn write(&mut self, val: u32) {
        let f = f32::from_bits(val);
        let r = if f == 0.0 { f32::MAX } else { 1.0 / f };
        self.result = r.to_bits();
    }

    pub fn read(&self) -> u32 {
        self.result
    }
}

/// 1/√x 逆平方根单元。
#[derive(Debug, Default)]
pub struct ISqrtUnit {
    result: u32,
}

impl ISqrtUnit {
    pub fn write(&mut self, val: u32) {
        let f = f32::from_bits(val);
        let r = if f <= 0.0 { f32::MAX } else { 1.0 / f.sqrt() };
        self.result = r.to_bits();
    }

    pub fn read(&self) -> u32 {
        self.result
    }
}

/// Atan 反正切单元。
///
/// 写入 y，再写入 x，触发 atan2(y, x) 计算。
/// 结果为 TGP 角度格式（整圈 = 0x10000）。
#[derive(Debug, Default)]
pub struct AtanUnit {
    y: f32,
    result: u32,
    phase: u8, // 0=等待y, 1=等待x
}

impl AtanUnit {
    pub fn write(&mut self, val: u32) {
        let f = f32::from_bits(val);
        match self.phase {
            0 => {
                self.y = f;
                self.phase = 1;
            }
            _ => {
                let x = f;
                let angle_rad = self.y.atan2(x);
                // 转换为 TGP 角度格式：弧度 / (2π) * 0x10000
                let angle_tgp = ((angle_rad / std::f32::consts::TAU) * 65536.0) as i32 as u32;
                self.result = angle_tgp;
                self.phase = 0;
            }
        }
    }

    pub fn read(&self) -> u32 {
        self.result
    }
}

// ---------------------------------------------------------------------------
// 统一数学单元集合
// ---------------------------------------------------------------------------

/// TGP 所有硬件数学单元。
#[derive(Debug, Default)]
pub struct MathUnits {
    pub sincos: SinCosUnit,
    pub inv: InvUnit,
    pub isqrt: ISqrtUnit,
    pub atan: AtanUnit,
}

#[cfg(test)]
mod tests {
    use std::f32;

    use super::*;

    fn approx_eq(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn sincos_zero() {
        let mut sc = SinCosUnit::default();
        sc.write(0); // angle = 0
        let sin_bits = sc.read();
        let cos_bits = sc.read();
        let s = f32::from_bits(sin_bits);
        let c = f32::from_bits(cos_bits);
        assert!(approx_eq(s, 0.0), "sin(0)={}", s);
        assert!(approx_eq(c, 1.0), "cos(0)={}", c);
    }

    #[test]
    fn sincos_quarter_turn() {
        // 0x4000 = 1/4 圈 = 90°
        let mut sc = SinCosUnit::default();
        sc.write(0x4000);
        let sin_bits = sc.read();
        let cos_bits = sc.read();
        let s = f32::from_bits(sin_bits);
        let c = f32::from_bits(cos_bits);
        assert!(approx_eq(s, 1.0), "sin(90°)={}", s);
        assert!(approx_eq(c, 0.0), "cos(90°)={}", c);
    }

    #[test]
    fn inv_unit() {
        let mut u = InvUnit::default();
        u.write(2.0f32.to_bits());
        let r = f32::from_bits(u.read());
        assert!(approx_eq(r, 0.5), "1/2={}", r);
    }

    #[test]
    fn isqrt_unit() {
        let mut u = ISqrtUnit::default();
        u.write(4.0f32.to_bits());
        let r = f32::from_bits(u.read());
        assert!(approx_eq(r, 0.5), "1/sqrt(4)={}", r);
    }

    #[test]
    fn atan_unit() {
        let mut u = AtanUnit::default();
        // atan2(1, 0) = π/2 → TGP = 0x4000
        u.write(1.0f32.to_bits()); // y
        u.write(0.0f32.to_bits()); // x
        let result = u.read();
        // 允许 ±2 的误差
        let diff = (result as i32 - 0x4000i32).abs();
        assert!(
            diff <= 2,
            "atan2(1,0) TGP angle = {:#x}, expected ~0x4000",
            result
        );
    }

    #[test]
    fn fixed_roundtrip() {
        let v = f32::consts::PI;
        let fixed = f32_to_fixed(v);
        let back = fixed_to_f32(fixed);
        assert!((v - back).abs() < 0.0001, "roundtrip: {} → {}", v, back);
    }
}
