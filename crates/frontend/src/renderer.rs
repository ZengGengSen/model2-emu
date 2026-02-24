//! Framebuffer 辅助函数。
//!
//! `pixels` crate 提供 RGBA8 格式的裸 `&mut [u8]` 切片。
//! 这个模块把模拟器原生的 RGB565 / XRGB8888 像素格式写入其中。

/// 把一个 XRGB8888 字写入 `pixels` 帧缓存的 (x, y) 位置。
#[inline]
pub fn put_pixel_xrgb(fb: &mut [u8], width: u32, x: u32, y: u32, xrgb: u32) {
    let offset = ((y * width + x) * 4) as usize;
    if offset + 3 >= fb.len() {
        return;
    }
    fb[offset] = ((xrgb >> 16) & 0xFF) as u8; // R
    fb[offset + 1] = ((xrgb >> 8) & 0xFF) as u8; // G
    fb[offset + 2] = (xrgb & 0xFF) as u8; // B
    fb[offset + 3] = 0xFF; // A（不透明）
}

/// 把一个 RGB565 半字写入 (x, y) 位置。
#[inline]
pub fn put_pixel_rgb565(fb: &mut [u8], width: u32, x: u32, y: u32, rgb565: u16) {
    let r = ((rgb565 >> 11) & 0x1F) as u32;
    let g = ((rgb565 >> 5) & 0x3F) as u32;
    let b = (rgb565 & 0x1F) as u32;
    let xrgb = ((r * 255 / 31) << 16) | ((g * 255 / 63) << 8) | (b * 255 / 31);
    put_pixel_xrgb(fb, width, x, y, xrgb);
}

/// 把帧缓存清成不透明黑色。
pub fn clear(fb: &mut [u8]) {
    for chunk in fb.chunks_exact_mut(4) {
        chunk[0] = 0x00;
        chunk[1] = 0x00;
        chunk[2] = 0x00;
        chunk[3] = 0xFF;
    }
}

/// 在帧缓存上叠加一个简单的状态文字（用纯色像素画字母，无需字体库）。
/// 目前只画一行彩色横条作为"系统活跃"指示器，真正的 OSD 留到后期。
pub fn draw_status_bar(fb: &mut [u8], width: u32, height: u32, frame: u64) {
    // 底部 4 像素高的彩虹条，颜色随帧号滚动
    let y_base = height.saturating_sub(4);
    let hue_offset = (frame % width as u64) as u32;
    for y in y_base..height {
        for x in 0..width {
            let hue = ((x + hue_offset) % width) * 360 / width;
            let xrgb = hue_to_xrgb(hue);
            put_pixel_xrgb(fb, width, x, y, xrgb);
        }
    }
}

/// 把色相（0–359）转换为 XRGB8888（饱和度=1，亮度=1）。
fn hue_to_xrgb(h: u32) -> u32 {
    let sector = h / 60;
    let frac = (h % 60) * 255 / 60;
    let (r, g, b) = match sector {
        0 => (255, frac, 0),
        1 => (255 - frac, 255, 0),
        2 => (0, 255, frac),
        3 => (0, 255 - frac, 255),
        4 => (frac, 0, 255),
        _ => (255, 0, 255 - frac),
    };
    (r << 16) | (g << 8) | b
}
