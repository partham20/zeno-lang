//! The board a Zeno program runs on when it runs on a computer: a
//! simulated one. It has 64 pins, and an LED on pin 13.
//!
//! A real board switches a pin by writing to a register. This one keeps
//! each pin's level in an array and prints a line when a level changes,
//! which is what a logic analyser clipped to the pin would show:
//!
//! ```text
//! led on
//! led off
//! pin 7 on
//! ```
//!
//! `zeno run` uses this file. A compiled program uses the same board
//! written in C, at the end of `runtime.c`, and there it can be swapped
//! for a real one (see `docs/board.md`).

/// How many pins the simulated board has. They are numbered from 0.
pub const PINS: i64 = 64;

/// The pin the LED is on. 13 is where an Arduino Uno has its LED, which
/// makes it the number most people have already met.
pub const LED: i64 = 13;

/// The level of every pin. All of them start low. And what was last
/// written to each register, by its address. A register nobody has
/// written to reads as 0.
pub struct Board {
    levels: [bool; PINS as usize],
    registers: std::collections::HashMap<i64, i64>,
}

impl Board {
    pub fn new() -> Board {
        Board { levels: [false; PINS as usize], registers: std::collections::HashMap::new() }
    }

    /// What the register at `address` holds.
    pub fn register_read(&self, address: i64) -> i64 {
        self.registers.get(&address).copied().unwrap_or(0)
    }

    /// Write to the register at `address`. Gives back the line to print.
    /// `digits` is how many hex digits the register is wide.
    ///
    /// Every write prints, also one that changes nothing. A pin that is
    /// driven high twice is still high, but on real hardware a write to
    /// a register is an event: the same byte written twice to a serial
    /// port is two bytes sent.
    pub fn register_write(&mut self, address: i64, value: i64, digits: usize) -> String {
        self.registers.insert(address, value);
        format!("register 0x{address:08x} = 0x{value:0digits$x}")
    }

    /// Make the pin an input. It is not driven, so what it reads is what
    /// holds it: a pull-up reads high, a pull-down low, and a pin nothing
    /// pulls keeps the level it had. Nothing is printed: the lines are
    /// what the program drives, and a pull is not driven (decision 45.5).
    pub fn input(&mut self, pin: i64, pull: Pull) {
        match pull {
            Pull::Up => self.levels[pin as usize] = true,
            Pull::Down => self.levels[pin as usize] = false,
            Pull::Nothing => {}
        }
    }

    /// Is the pin high?
    pub fn read(&self, pin: i64) -> bool {
        self.levels[pin as usize]
    }

    /// Drive the pin high or low. Gives back the line to print when the
    /// level changed, and nothing when the pin was already there.
    pub fn write(&mut self, pin: i64, level: bool) -> Option<String> {
        if self.levels[pin as usize] == level {
            return None;
        }
        self.levels[pin as usize] = level;
        Some(format!("{} {}", name(pin), if level { "on" } else { "off" }))
    }
}

/// What holds an input pin when nothing drives it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Pull {
    Nothing,
    Up,
    Down,
}

/// What a pin is called in the lines the board prints.
pub fn name(pin: i64) -> String {
    if pin == LED { "led".to_string() } else { format!("pin {pin}") }
}

/// A clock that only goes forward, in milliseconds. Only the difference
/// between two readings means anything.
#[cfg(not(target_arch = "wasm32"))]
pub fn now() -> f64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed().as_secs_f64() * 1000.0
}

/// In a browser the clock is the page's.
#[cfg(target_arch = "wasm32")]
pub fn now() -> f64 {
    crate::web_host::now()
}
