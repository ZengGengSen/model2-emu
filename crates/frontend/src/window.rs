//! Main window and event loop.
//!
//! Creates the OS window via `winit`, drives the `pixels` framebuffer,
//! ticks the emulator core, and forwards input events.

use std::sync::Arc;

use pixels::{Pixels, SurfaceTexture};
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::{KeyEvent, WindowEvent},
    event_loop::ActiveEventLoop,
    keyboard::{KeyCode, PhysicalKey},
    window::{Window, WindowId},
};

use crate::{DISPLAY_HEIGHT, DISPLAY_WIDTH, emulator::Emulator, input::InputState, renderer};

// ── State ─────────────────────────────────────────────────────────────────────

/// Holds all resources that require a valid window to be created first.
///
/// # 为什么用 `Arc<Window>`
///
/// `pixels::SurfaceTexture` 需要持有对 `Window` 的引用（作为 wgpu surface 的 target），
/// 而 `Window` 本身也必须保活以响应事件。若把两者放进同一个结构体，
/// 就会形成自引用（`pixels` 借用 `window`，但两者同属 `WindowState`），
/// Rust 的借用检查器不接受这种布局——即使添加生命周期参数 `'a` 也无法绕开，
/// 因为 `'a` 无法表达"我借用的是自己字段"这种关系。
///
/// 解法：用 `Arc<Window>` 让 `SurfaceTexture` 和 `WindowState` 共享所有权，
/// 双方都持有引用计数指针，Rust 可以安全地允许两者同时存在于同一结构体中。
pub struct WindowState<'a> {
    window: Arc<Window>,
    pixels: Pixels<'a>,
    input: InputState,
    emulator: Emulator,
}

impl<'a> WindowState<'a> {
    fn new(event_loop: &ActiveEventLoop) -> anyhow::Result<Self> {
        let size = LogicalSize::new(DISPLAY_WIDTH * 2, DISPLAY_HEIGHT * 2);

        let attrs = Window::default_attributes()
            .with_title("Sega Model 2 Emulator")
            .with_inner_size(size)
            .with_resizable(false);

        // create_window 返回 Window（拥有所有权），包入 Arc 以便共享。
        let window = Arc::new(event_loop.create_window(attrs)?);

        let inner = window.inner_size();
        // SurfaceTexture 现在持有的是 Arc<Window> 的克隆，不借用 WindowState 本身。
        let surface = SurfaceTexture::new(inner.width, inner.height, Arc::clone(&window));
        let pixels = Pixels::new(DISPLAY_WIDTH, DISPLAY_HEIGHT, surface)?;

        // ── 模拟器初始化 ──────────────────────────────────────────────────
        let mut emulator = Emulator::new();
        // 尝试从环境变量 MODEL2_ROM 加载 ROM；找不到时以 no-ROM 模式继续
        emulator.try_load_rom_from_env();

        Ok(Self {
            window,
            pixels,
            input: InputState::default(),
            emulator,
        })
    }

    /// 驱动一帧：tick 模拟器 → 渲染帧缓存 → 上屏。
    fn redraw(&mut self, event_loop: &ActiveEventLoop) {
        // 1. 模拟器推进一帧
        let still_running = self.emulator.tick_frame();

        // 2. 渲染到 pixels 帧缓存
        let fb = self.pixels.frame_mut();
        renderer::clear(fb);

        // 状态指示条（底部彩虹条，随帧号滚动；ROM 停机时变灰）
        if still_running {
            renderer::draw_status_bar(
                fb,
                DISPLAY_WIDTH,
                DISPLAY_HEIGHT,
                self.emulator.stats.frame_count,
            );
        } else {
            // 所有 CPU 停机：底部画灰条提示
            let y0 = DISPLAY_HEIGHT.saturating_sub(4);
            for y in y0..DISPLAY_HEIGHT {
                for x in 0..DISPLAY_WIDTH {
                    renderer::put_pixel_xrgb(fb, DISPLAY_WIDTH, x, y, 0x44_44_44);
                }
            }
        }

        // 更新窗口标题（每 60 帧一次）
        if self.emulator.stats.frame_count.is_multiple_of(60) {
            let s = &self.emulator.stats;
            let title = format!(
                "Sega Model 2 | frame {} | last {:.1} ms | ROM: {}",
                s.frame_count,
                s.frame_time_us as f64 / 1000.0,
                if self.emulator.rom_loaded {
                    "loaded"
                } else {
                    "no-ROM"
                },
            );
            self.window.set_title(&title);
        }

        // 3. 通知驱动即将呈现（减少撕裂）
        self.window.pre_present_notify();

        // 4. 上屏
        if let Err(e) = self.pixels.render() {
            log::error!("pixels render error: {e}");
            event_loop.exit();
            return;
        }

        // 5. 帧率限制（让出多余 CPU 时间）
        self.emulator.wait_for_frame_end();

        // 6. 请求下一帧
        self.window.request_redraw();
    }
}

// ── Application ───────────────────────────────────────────────────────────────

/// Top-level application state handed to the `winit` event loop.
///
/// 原来的 `Application<'a>` 生命周期参数来自 `WindowState<'a>`，
/// 改用 `Arc<Window>` 后两者都不再需要生命周期参数。
#[derive(Default)]
pub struct Application<'a> {
    state: Option<WindowState<'a>>,
}

impl<'a> ApplicationHandler for Application<'a> {
    // Called when the event loop is ready (or the app is resumed on mobile).
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return; // already initialised
        }

        match WindowState::new(event_loop) {
            Ok(s) => self.state = Some(s),
            Err(e) => {
                log::error!("Failed to create window: {e}");
                event_loop.exit();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(state) = self.state.as_mut() else {
            return;
        };

        match event {
            // ── Close ─────────────────────────────────────────────────────
            WindowEvent::CloseRequested => {
                log::info!("Close requested; stopping");
                log::info!("Final stats: {:?}", state.emulator.stats);
                event_loop.exit();
            }

            // ── Keyboard ──────────────────────────────────────────────────
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        physical_key,
                        state: key_state,
                        ..
                    },
                ..
            } => {
                let pressed = key_state == winit::event::ElementState::Pressed;

                if let PhysicalKey::Code(key) = physical_key {
                    match key {
                        // Escape → 退出
                        KeyCode::Escape if pressed => {
                            event_loop.exit();
                        }
                        // F12 → 打印当前状态到日志（调试）
                        KeyCode::F12 if pressed => {
                            state.emulator.dump_state();
                        }
                        // F5 → 重置模拟器（不重新加载 ROM）
                        // KeyCode::F5 if pressed => {
                        //     log::info!("Soft reset");
                        //     state.emulator.i960.regs.ip = 0x0080_0000;
                        //     state.emulator.i960.resume();
                        //     state.emulator.m68k.regs.pc = 0x0000_0000;
                        //     state.emulator.m68k.resume();
                        //     state.emulator.tgp.reset();
                        //     state.emulator.tgp.copro_ctl_write(0);
                        // }
                        _ => {
                            state.input.on_key(key, pressed);
                        }
                    }
                }
            }

            WindowEvent::Resized(new_size) => {
                if let Err(e) = state.pixels.resize_surface(new_size.width, new_size.height) {
                    log::error!("pixels resize error: {e}");
                    event_loop.exit();
                }
            }

            WindowEvent::RedrawRequested => {
                state.redraw(event_loop);
            }

            _ => {}
        }
    }
}
