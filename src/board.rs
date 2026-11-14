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

/// The level of every pin. All of them start low.
pub struct Board {
    levels: [bool; PINS as usize],
}

impl Board {
    pub fn new() -> Board {
        Board { levels: [false; PINS as usize] }
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

/// What a pin is called in the lines the board prints.
pub fn name(pin: i64) -> String {
    if pin == LED { "led".to_string() } else { format!("pin {pin}") }
}

/// A clock that only goes forward, in milliseconds. Only the difference
/// between two readings means anything.
pub fn now() -> f64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed().as_secs_f64() * 1000.0
}
