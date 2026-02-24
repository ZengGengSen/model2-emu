//! Sega Model 2 模拟器前端入口点。
//!
//! # 使用方法
//!
//! ```
//! # 加载 ROM（推荐）
//! MODEL2_ROM=roms/daytona.zip cargo run --release -p frontend
//!
//! # 无 ROM 模式（窗口正常打开，CPU 很快停机）
//! cargo run --release -p frontend
//! ```
//!
//! # 调试快捷键
//!
//! | 键      | 功能                              |
//! |--------|----------------------------------|
//! | Escape | 退出                              |
//! | F5     | 软复位（重置 CPU，不重载 ROM）     |
//! | F12    | 打印当前寄存器/统计到日志           |
//!
//! # 环境变量
//!
//! | 变量        | 说明                          |
//! |-----------|------------------------------|
//! | MODEL2_ROM | Daytona USA ZIP 路径         |
//! | RUST_LOG   | 日志级别（info/debug/trace）  |

pub mod emulator;
pub mod input;
pub mod renderer;
pub mod window;

/// Model 2 硬件原生显示分辨率。
pub const DISPLAY_WIDTH: u32 = 496;
pub const DISPLAY_HEIGHT: u32 = 384;

fn main() -> anyhow::Result<()> {
    // 初始化日志（遵循 RUST_LOG 环境变量；默认 info 级别）
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .parse_default_env()
        .init();

    log::info!("Sega Model 2 Emulator starting");
    log::info!("  Set MODEL2_ROM=/path/to/daytona.zip to load a game");
    log::info!("  Set RUST_LOG=debug for verbose output");

    let event_loop = winit::event_loop::EventLoop::new()?;

    // Poll 模式：以最快速度请求重绘，帧率由 emulator::wait_for_frame_end() 控制。
    event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);

    let mut app = window::Application::default();
    event_loop.run_app(&mut app)?;

    log::info!("Emulator stopped");
    Ok(())
}
