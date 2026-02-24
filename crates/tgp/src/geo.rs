//! 几何引擎（GEO）。
//!
//! 接收 i960 通过 FIFO 发来的多边形命令，做顶点变换，把结果写回 output FIFO。
//!
//! # 命令格式
//!
//! 每条 GEO 命令的第一个字包含：
//! - `[31]`：符号位（保留）
//! - `[30:23]`：8 位功能码（function code）
//! - `[22:0]`：命令参数（长度字段等）
//!
//! Function port 地址 `0x00880000` 的写入也遵循同样格式（地址中嵌入 function code）。
//!
//! # 已知功能码（参照 MAME model2_v.cpp）
//!
//! | 功能码 | 名称               | 说明                             |
//! |--------|-------------------|----------------------------------|
//! | 0x00   | NOP               |                                  |
//! | 0x01   | ModelMatrix       | 上传 4×4 模型矩阵（16个f32字）   |
//! | 0x02   | ViewMatrix        | 上传视图矩阵                      |
//! | 0x03   | ProjMatrix        | 上传投影矩阵                      |
//! | 0x04   | Polygon           | 上传多边形（顶点列表）            |
//! | 0x05   | LightDir          | 设置光照方向向量                  |
//! | 0x06   | TranslateMatrix   | 矩阵乘法平移                      |
//! | 0x07   | ScaleMatrix       | 矩阵乘法缩放                      |
//! | 0x08   | PushMatrix        | 压矩阵栈                          |
//! | 0x09   | PopMatrix         | 弹矩阵栈                          |

use log::{debug, trace, warn};

use crate::fifo::FifoPair;
use crate::matrix::{Mat4, MatrixStack, Vec3};

// ---------------------------------------------------------------------------
// 输出顶点（变换后）
// ---------------------------------------------------------------------------

/// 变换后的顶点（传递给光栅化器）。
#[derive(Debug, Clone, Copy)]
pub struct TransformedVertex {
    /// 裁剪空间坐标。
    pub pos: Vec3,
    /// 纹理坐标（u, v）。
    pub uv: (f32, f32),
    /// 法线（光照用）。
    pub normal: Vec3,
}

/// 变换后的多边形（四边形，Model 2 使用四边形而非三角形）。
#[derive(Debug, Clone)]
pub struct TransformedPoly {
    pub verts: Vec<TransformedVertex>,
    /// 纹理参数（原始字，供光栅化器解析）。
    pub tex_param: u32,
    /// 颜色/属性字。
    pub attr: u32,
}

// ---------------------------------------------------------------------------
// 几何引擎状态
// ---------------------------------------------------------------------------

/// 几何引擎。
pub struct GeoEngine {
    /// 模型矩阵栈。
    pub model_stack: MatrixStack,
    /// 视图矩阵（固定，不入栈）。
    pub view: Mat4,
    /// 投影矩阵。
    pub proj: Mat4,
    /// 光照方向向量（世界空间，归一化）。
    pub light_dir: Vec3,
    /// 本帧已处理的多边形列表（供调试 / 后续光栅化使用）。
    pub output_polys: Vec<TransformedPoly>,
    /// 待发往 i960 的结果字（通过 output FIFO）。
    pending_output: Vec<u32>,
}

impl GeoEngine {
    pub fn new() -> Self {
        Self {
            model_stack: MatrixStack::new(),
            view: Mat4::IDENTITY,
            proj: Mat4::IDENTITY,
            light_dir: Vec3::new(0.0, 0.0, -1.0),
            output_polys: Vec::new(),
            pending_output: Vec::new(),
        }
    }

    /// 清空帧缓存（每帧开始时调用）。
    pub fn begin_frame(&mut self) {
        self.output_polys.clear();
    }

    // -----------------------------------------------------------------------
    // 主处理循环：消耗 input FIFO 中所有可用命令
    // -----------------------------------------------------------------------

    /// 处理 FIFO 中所有待处理命令，把结果写入 output FIFO。
    ///
    /// 设计为非阻塞：FIFO 中数据不足时立即返回（等下次调用）。
    pub fn process(&mut self, fifo: &mut FifoPair) {
        // 把所有 pending output 推入 FIFO
        for v in self.pending_output.drain(..) {
            fifo.tgp_write(v);
        }

        // 处理命令
        while let Some(header) = fifo.input.peek() {
            let func = ((header >> 23) & 0xFF) as u8;
            let param = header & 0x7F_FFFF;

            // 预估这条命令需要多少个字
            let words_needed = self.words_needed(func, param);
            if fifo.input.len() < words_needed {
                break; // 数据不足，等下次
            }

            // 消费 header
            fifo.tgp_read();

            // 读取剩余数据字
            let mut data = Vec::with_capacity(words_needed.saturating_sub(1));
            for _ in 1..words_needed {
                data.push(fifo.tgp_read().unwrap_or(0));
            }

            self.execute_command(func, param, &data, fifo);
        }
    }

    /// 预估命令所需的总字数（含 header）。
    fn words_needed(&self, func: u8, param: u32) -> usize {
        match func {
            0x00 => 1,             // NOP
            0x01..=0x03 => 1 + 16, // 矩阵上传（header + 16个f32）
            0x04 => {
                // 多边形：header 后接 attr + 顶点数 * 5 (x,y,z,u,v) + tex_param
                let vert_count = (param & 0xF) as usize;
                1 + 1 + vert_count * 5 + 1
            }
            0x05 => 1 + 3,    // 光照方向：3个f32
            0x06 => 1 + 3,    // 平移：3个f32
            0x07 => 1 + 3,    // 缩放：3个f32
            0x08 | 0x09 => 1, // push/pop
            _ => 1,           // 未知，只消耗 header
        }
    }

    /// 执行单条命令。
    fn execute_command(&mut self, func: u8, param: u32, data: &[u32], fifo: &mut FifoPair) {
        trace!(
            "GEO cmd {:#04x} param={:#x} data_len={}",
            func,
            param,
            data.len()
        );

        match func {
            0x00 => { /* NOP */ }

            // 矩阵上传
            0x01 => {
                if data.len() >= 16 {
                    let m = Mat4::from_u32_row_major(data[..16].try_into().unwrap());
                    self.model_stack.load(m);
                    debug!("GEO: load model matrix");
                }
            }
            0x02 => {
                if data.len() >= 16 {
                    self.view = Mat4::from_u32_row_major(data[..16].try_into().unwrap());
                    debug!("GEO: load view matrix");
                }
            }
            0x03 => {
                if data.len() >= 16 {
                    self.proj = Mat4::from_u32_row_major(data[..16].try_into().unwrap());
                    debug!("GEO: load proj matrix");
                }
            }

            // 多边形
            0x04 => {
                self.process_polygon(param, data, fifo);
            }

            // 光照方向
            0x05 => {
                if data.len() >= 3 {
                    let dir = Vec3::new(
                        f32::from_bits(data[0]),
                        f32::from_bits(data[1]),
                        f32::from_bits(data[2]),
                    );
                    self.light_dir = dir.normalize();
                }
            }

            // 平移（在当前矩阵上叠加平移）
            0x06 => {
                if data.len() >= 3 {
                    let tx = f32::from_bits(data[0]);
                    let ty = f32::from_bits(data[1]);
                    let tz = f32::from_bits(data[2]);
                    self.model_stack.mul_right(Mat4::translate(tx, ty, tz));
                }
            }

            // 缩放
            0x07 => {
                if data.len() >= 3 {
                    let sx = f32::from_bits(data[0]);
                    let sy = f32::from_bits(data[1]);
                    let sz = f32::from_bits(data[2]);
                    self.model_stack.mul_right(Mat4::scale(sx, sy, sz));
                }
            }

            0x08 => {
                self.model_stack.push();
            }
            0x09 => {
                self.model_stack.pop();
            }

            _ => {
                warn!("GEO: unknown function {:#04x} param={:#x}", func, param);
            }
        }
    }

    /// 处理多边形命令，变换顶点，把结果存入 output_polys 并写 output FIFO。
    fn process_polygon(&mut self, param: u32, data: &[u32], fifo: &mut FifoPair) {
        let vert_count = (param & 0xF) as usize;
        let attr = data.first().copied().unwrap_or(0);

        // MVP 矩阵
        let mvp = self.proj * self.view * self.model_stack.top();

        let mut transformed = Vec::with_capacity(vert_count);
        let base = 1usize; // data[0] 是 attr

        for i in 0..vert_count {
            let offset = base + i * 5;
            if offset + 4 >= data.len() {
                break;
            }

            let x = f32::from_bits(data[offset]);
            let y = f32::from_bits(data[offset + 1]);
            let z = f32::from_bits(data[offset + 2]);
            let u = f32::from_bits(data[offset + 3]);
            let v = f32::from_bits(data[offset + 4]);

            // let world_pos = self.model_stack.top().transform_point(Vec3::new(x, y, z));
            let clip_pos = mvp.transform_point(Vec3::new(x, y, z));

            // 简单漫反射光照：dot(normal, light_dir)，法线暂时用Z轴
            let normal = Vec3::new(0.0, 0.0, 1.0);
            let world_normal = self.model_stack.top().transform_dir(normal).normalize();

            transformed.push(TransformedVertex {
                pos: clip_pos,
                uv: (u, v),
                normal: world_normal,
            });

            trace!(
                "  vert[{}]: ({:.3},{:.3},{:.3}) → clip({:.3},{:.3},{:.3})",
                i, x, y, z, clip_pos.x, clip_pos.y, clip_pos.z
            );
        }

        let tex_param = data.last().copied().unwrap_or(0);
        let poly = TransformedPoly {
            verts: transformed,
            tex_param,
            attr,
        };

        // 把变换后的顶点写入 output FIFO（i960 读取这些结果）
        for vert in &poly.verts {
            fifo.tgp_write(vert.pos.x.to_bits());
            fifo.tgp_write(vert.pos.y.to_bits());
            fifo.tgp_write(vert.pos.z.to_bits());
        }

        self.output_polys.push(poly);
    }
}

impl Default for GeoEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fifo::FifoPair;

    // fn push_f32(fifo: &mut FifoPair, v: f32) {
    //     fifo.cpu_write(v.to_bits());
    // }

    fn make_matrix_cmd(func: u8, m: Mat4) -> Vec<u32> {
        // header：func << 23
        let mut cmd = vec![(func as u32) << 23];
        for c in 0..4 {
            for r in 0..4 {
                cmd.push(m.cols[c][r].to_bits());
            }
        }
        cmd
    }

    #[test]
    fn nop_command() {
        let mut fifo = FifoPair::new();
        let mut geo = GeoEngine::new();
        fifo.cpu_write(0x0000_0000); // NOP
        geo.process(&mut fifo);
        assert!(fifo.input.is_empty());
    }

    #[test]
    fn load_model_matrix() {
        let mut fifo = FifoPair::new();
        let mut geo = GeoEngine::new();
        let target = Mat4::translate(5.0, 0.0, 0.0);
        for w in make_matrix_cmd(0x01, target) {
            fifo.cpu_write(w);
        }
        geo.process(&mut fifo);
        let top = geo.model_stack.top();
        assert!((top.cols[3][0] - 5.0).abs() < 1e-5);
    }

    #[test]
    fn push_pop_stack() {
        let mut fifo = FifoPair::new();
        let mut geo = GeoEngine::new();

        // 加载一个矩阵
        for w in make_matrix_cmd(0x01, Mat4::translate(1.0, 0.0, 0.0)) {
            fifo.cpu_write(w);
        }
        // push
        fifo.cpu_write(0x08 << 23);
        // 加载另一个矩阵
        for w in make_matrix_cmd(0x01, Mat4::translate(2.0, 0.0, 0.0)) {
            fifo.cpu_write(w);
        }
        geo.process(&mut fifo);
        assert!((geo.model_stack.top().cols[3][0] - 2.0).abs() < 1e-5);

        // pop
        fifo.cpu_write(0x09 << 23);
        geo.process(&mut fifo);
        assert!((geo.model_stack.top().cols[3][0] - 1.0).abs() < 1e-5);
    }
}
