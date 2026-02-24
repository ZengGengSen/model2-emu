//! 4×4 矩阵和三维向量，用于 TGP 几何变换。
//!
//! TGP 内部维护一个矩阵栈，模型/视图/投影变换通过矩阵乘法完成。
//! 所有数值使用 f32（TGP 硬件为 IEEE 754 单精度）。

use std::ops::{Mul, MulAssign};

// ---------------------------------------------------------------------------
// Vec3 / Vec4
// ---------------------------------------------------------------------------

/// 三维向量（齐次坐标用 Vec4）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const ZERO: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };

    pub fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    pub fn dot(self, rhs: Self) -> f32 {
        self.x * rhs.x + self.y * rhs.y + self.z * rhs.z
    }

    pub fn cross(self, rhs: Self) -> Self {
        Self {
            x: self.y * rhs.z - self.z * rhs.y,
            y: self.z * rhs.x - self.x * rhs.z,
            z: self.x * rhs.y - self.y * rhs.x,
        }
    }

    pub fn length(self) -> f32 {
        self.dot(self).sqrt()
    }

    pub fn normalize(self) -> Self {
        let len = self.length();
        if len < 1e-10 {
            return Self::ZERO;
        }
        Self::new(self.x / len, self.y / len, self.z / len)
    }

    pub fn scale(self, s: f32) -> Self {
        Self::new(self.x * s, self.y * s, self.z * s)
    }

    /// 转为齐次坐标 (x, y, z, 1)。
    pub fn to_vec4(self) -> Vec4 {
        Vec4 {
            x: self.x,
            y: self.y,
            z: self.z,
            w: 1.0,
        }
    }
}

impl std::ops::Add for Vec3 {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        Self::new(self.x + rhs.x, self.y + rhs.y, self.z + rhs.z)
    }
}

impl std::ops::Sub for Vec3 {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self {
        Self::new(self.x - rhs.x, self.y - rhs.y, self.z - rhs.z)
    }
}

/// 四维向量（齐次坐标）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vec4 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Vec4 {
    pub fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        Self { x, y, z, w }
    }

    /// 齐次除法 → Vec3。
    pub fn perspective_divide(self) -> Vec3 {
        if self.w.abs() < 1e-10 {
            Vec3::new(self.x, self.y, self.z)
        } else {
            Vec3::new(self.x / self.w, self.y / self.w, self.z / self.w)
        }
    }

    pub fn to_vec3(self) -> Vec3 {
        Vec3::new(self.x, self.y, self.z)
    }
}

// ---------------------------------------------------------------------------
// Mat4
// ---------------------------------------------------------------------------

/// 4×4 列主序矩阵（与 OpenGL/TGP 约定一致）。
///
/// `m[col][row]` 存储方式，即 `m[0]` 是第一列。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mat4 {
    /// 按列存储：`cols[c][r]`。
    pub cols: [[f32; 4]; 4],
}

impl Mat4 {
    /// 单位矩阵。
    pub const IDENTITY: Self = Self {
        cols: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    };

    /// 从行主序 16 元素数组构造（方便用 TGP 下发的数据初始化）。
    pub fn from_row_major(data: &[f32; 16]) -> Self {
        let mut m = Self::IDENTITY;
        for r in 0..4 {
            for c in 0..4 {
                m.cols[c][r] = data[r * 4 + c];
            }
        }
        m
    }

    /// 从 u32 数组（IEEE 754 bits）按行主序构造。
    pub fn from_u32_row_major(data: &[u32; 16]) -> Self {
        let floats: [f32; 16] = std::array::from_fn(|i| f32::from_bits(data[i]));
        Self::from_row_major(&floats)
    }

    /// 变换一个 Vec4。
    pub fn transform_vec4(self, v: Vec4) -> Vec4 {
        let mut result = [0.0f32; 4];
        let vv = [v.x, v.y, v.z, v.w];
        for (r, res) in result.iter_mut().enumerate() {
            for (c, v) in vv.iter().enumerate() {
                *res += self.cols[c][r] * v;
            }
        }
        Vec4::new(result[0], result[1], result[2], result[3])
    }

    /// 变换一个点（Vec3，w=1）。
    pub fn transform_point(self, v: Vec3) -> Vec3 {
        self.transform_vec4(v.to_vec4()).perspective_divide()
    }

    /// 变换一个方向向量（Vec3，w=0，不做透视除法）。
    pub fn transform_dir(self, v: Vec3) -> Vec3 {
        let v4 = Vec4 {
            x: v.x,
            y: v.y,
            z: v.z,
            w: 0.0,
        };
        self.transform_vec4(v4).to_vec3()
    }

    /// 转置。
    pub fn transpose(self) -> Self {
        let mut m = Self::IDENTITY;
        for r in 0..4 {
            for c in 0..4 {
                m.cols[r][c] = self.cols[c][r];
            }
        }
        m
    }

    /// 平移矩阵。
    pub fn translate(tx: f32, ty: f32, tz: f32) -> Self {
        let mut m = Self::IDENTITY;
        m.cols[3][0] = tx;
        m.cols[3][1] = ty;
        m.cols[3][2] = tz;
        m
    }

    /// 缩放矩阵。
    pub fn scale(sx: f32, sy: f32, sz: f32) -> Self {
        let mut m = Self::IDENTITY;
        m.cols[0][0] = sx;
        m.cols[1][1] = sy;
        m.cols[2][2] = sz;
        m
    }

    /// 绕 X 轴旋转矩阵（弧度）。
    pub fn rotate_x(angle: f32) -> Self {
        let (s, c) = angle.sin_cos();
        let mut m = Self::IDENTITY;
        m.cols[1][1] = c;
        m.cols[1][2] = s;
        m.cols[2][1] = -s;
        m.cols[2][2] = c;
        m
    }

    /// 绕 Y 轴旋转矩阵（弧度）。
    pub fn rotate_y(angle: f32) -> Self {
        let (s, c) = angle.sin_cos();
        let mut m = Self::IDENTITY;
        m.cols[0][0] = c;
        m.cols[0][2] = -s;
        m.cols[2][0] = s;
        m.cols[2][2] = c;
        m
    }

    /// 绕 Z 轴旋转矩阵（弧度）。
    pub fn rotate_z(angle: f32) -> Self {
        let (s, c) = angle.sin_cos();
        let mut m = Self::IDENTITY;
        m.cols[0][0] = c;
        m.cols[0][1] = s;
        m.cols[1][0] = -s;
        m.cols[1][1] = c;
        m
    }
}

impl Mul for Mat4 {
    type Output = Self;

    fn mul(self, rhs: Self) -> Self {
        let mut result = Self {
            cols: [[0.0; 4]; 4],
        };
        for c in 0..4 {
            for r in 0..4 {
                let mut sum = 0.0f32;
                for k in 0..4 {
                    sum += self.cols[k][r] * rhs.cols[c][k];
                }
                result.cols[c][r] = sum;
            }
        }
        result
    }
}

impl MulAssign for Mat4 {
    fn mul_assign(&mut self, rhs: Self) {
        *self = *self * rhs;
    }
}

// ---------------------------------------------------------------------------
// 矩阵栈
// ---------------------------------------------------------------------------

/// TGP 矩阵栈（最大深度 16，与硬件约定一致）。
pub struct MatrixStack {
    stack: Vec<Mat4>,
}

impl MatrixStack {
    pub fn new() -> Self {
        Self {
            stack: vec![Mat4::IDENTITY],
        }
    }

    /// 当前顶部矩阵（只读）。
    pub fn top(&self) -> Mat4 {
        *self.stack.last().unwrap()
    }

    /// 当前顶部矩阵（可变引用）。
    pub fn top_mut(&mut self) -> &mut Mat4 {
        self.stack.last_mut().unwrap()
    }

    /// 压栈（复制当前顶部）。
    pub fn push(&mut self) {
        let top = self.top();
        self.stack.push(top);
    }

    /// 弹栈。返回 false 表示已经是栈底。
    pub fn pop(&mut self) -> bool {
        if self.stack.len() <= 1 {
            log::warn!("MatrixStack underflow");
            return false;
        }
        self.stack.pop();
        true
    }

    /// 用新矩阵替换当前顶部。
    pub fn load(&mut self, m: Mat4) {
        *self.top_mut() = m;
    }

    /// 将新矩阵左乘到当前顶部（current = m * current）。
    pub fn mul_left(&mut self, m: Mat4) {
        let top = self.top();
        *self.top_mut() = m * top;
    }

    /// 将新矩阵右乘到当前顶部（current = current * m）。
    pub fn mul_right(&mut self, m: Mat4) {
        let top = self.top();
        *self.top_mut() = top * m;
    }

    pub fn depth(&self) -> usize {
        self.stack.len()
    }
}

impl Default for MatrixStack {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx_eq_f32(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    fn approx_eq_vec3(a: Vec3, b: Vec3) -> bool {
        approx_eq_f32(a.x, b.x) && approx_eq_f32(a.y, b.y) && approx_eq_f32(a.z, b.z)
    }

    #[test]
    fn identity_transform() {
        let v = Vec3::new(1.0, 2.0, 3.0);
        let r = Mat4::IDENTITY.transform_point(v);
        assert!(approx_eq_vec3(r, v));
    }

    #[test]
    fn translate() {
        let m = Mat4::translate(1.0, 2.0, 3.0);
        let v = Vec3::new(0.0, 0.0, 0.0);
        let r = m.transform_point(v);
        assert!(approx_eq_vec3(r, Vec3::new(1.0, 2.0, 3.0)));
    }

    #[test]
    fn scale_then_translate() {
        // translate * scale：先缩放后平移
        let s = Mat4::scale(2.0, 2.0, 2.0);
        let t = Mat4::translate(1.0, 0.0, 0.0);
        let m = t * s;
        let r = m.transform_point(Vec3::new(1.0, 0.0, 0.0));
        // scale(1,0,0) = (2,0,0); translate(2,0,0) = (3,0,0)
        assert!(approx_eq_f32(r.x, 3.0));
    }

    #[test]
    fn rotate_x_90() {
        let m = Mat4::rotate_x(std::f32::consts::FRAC_PI_2);
        let v = Vec3::new(0.0, 1.0, 0.0);
        let r = m.transform_point(v);
        // 绕 X 旋转 90°: (0,1,0) → (0,0,1)
        assert!(approx_eq_f32(r.x, 0.0));
        assert!(approx_eq_f32(r.y, 0.0));
        assert!(approx_eq_f32(r.z, 1.0));
    }

    #[test]
    fn mat_mul_identity() {
        let m = Mat4::translate(1.0, 2.0, 3.0);
        let r = m * Mat4::IDENTITY;
        assert_eq!(r, m);
    }

    #[test]
    fn matrix_stack_push_pop() {
        let mut stack = MatrixStack::new();
        stack.load(Mat4::translate(1.0, 0.0, 0.0));
        stack.push();
        stack.load(Mat4::translate(2.0, 0.0, 0.0));
        assert!(approx_eq_f32(stack.top().cols[3][0], 2.0));
        stack.pop();
        assert!(approx_eq_f32(stack.top().cols[3][0], 1.0));
    }

    #[test]
    fn vec3_dot_cross() {
        let a = Vec3::new(1.0, 0.0, 0.0);
        let b = Vec3::new(0.0, 1.0, 0.0);
        assert!(approx_eq_f32(a.dot(b), 0.0));
        let c = a.cross(b);
        assert!(approx_eq_vec3(c, Vec3::new(0.0, 0.0, 1.0)));
    }
}
