//! Keyboard / gamepad input state.
//!
//! Maps physical keys to the Model 2 cabinet controls.
//! A full gamepad implementation can be layered on top later.

use winit::keyboard::KeyCode;

/// Bit-flags for the emulated cabinet buttons
#[derive(Debug, Clone, Copy, Default)]
pub struct Buttons(pub u32);

impl Buttons {
    pub const START: u32 = 1 << 0;
    pub const COIN: u32 = 1 << 1;
    pub const BTN1: u32 = 1 << 2;
    pub const BTN2: u32 = 1 << 3;
    pub const BTN3: u32 = 1 << 4;
    pub const BTN4: u32 = 1 << 5;
    pub const UP: u32 = 1 << 6;
    pub const DOWN: u32 = 1 << 7;
    pub const LEFT: u32 = 1 << 8;
    pub const RIGHT: u32 = 1 << 9;
    pub const SERVICE: u32 = 1 << 10;
    pub const TEST: u32 = 1 << 11;

    pub fn is_set(self, flag: u32) -> bool {
        self.0 & flag != 0
    }

    fn set(&mut self, flag: u32) {
        self.0 |= flag;
    }
    fn clear(&mut self, flag: u32) {
        self.0 &= !flag;
    }
}

/// Full input state passed to the emulator tick
#[derive(Debug, Clone, Default)]
pub struct InputState {
    pub p1: Buttons,
    pub p2: Buttons,
}

impl InputState {
    /// Update state from a winit keyboard event
    pub fn on_key(&mut self, key: KeyCode, pressed: bool) {
        let (player, flag) = match key {
            // ── Player 1 ──────────────────────────────────────���──────────
            KeyCode::Enter => (&mut self.p1, Buttons::START),
            KeyCode::Digit5 => (&mut self.p1, Buttons::COIN),
            KeyCode::KeyZ => (&mut self.p1, Buttons::BTN1),
            KeyCode::KeyX => (&mut self.p1, Buttons::BTN2),
            KeyCode::KeyC => (&mut self.p1, Buttons::BTN3),
            KeyCode::KeyV => (&mut self.p1, Buttons::BTN4),
            KeyCode::ArrowUp => (&mut self.p1, Buttons::UP),
            KeyCode::ArrowDown => (&mut self.p1, Buttons::DOWN),
            KeyCode::ArrowLeft => (&mut self.p1, Buttons::LEFT),
            KeyCode::ArrowRight => (&mut self.p1, Buttons::RIGHT),
            KeyCode::F2 => (&mut self.p1, Buttons::SERVICE),
            KeyCode::F3 => (&mut self.p1, Buttons::TEST),

            // ── Player 2 ─────────────────────────────────────────────────
            KeyCode::KeyA => (&mut self.p2, Buttons::BTN1),
            KeyCode::KeyS => (&mut self.p2, Buttons::BTN2),
            KeyCode::KeyD => (&mut self.p2, Buttons::BTN3),
            KeyCode::KeyF => (&mut self.p2, Buttons::BTN4),
            KeyCode::KeyW => (&mut self.p2, Buttons::UP),
            KeyCode::KeyR => (&mut self.p2, Buttons::DOWN),
            KeyCode::KeyQ => (&mut self.p2, Buttons::LEFT),
            KeyCode::KeyE => (&mut self.p2, Buttons::RIGHT),

            _ => return,
        };

        if pressed {
            player.set(flag);
        } else {
            player.clear(flag);
        }
    }
}
