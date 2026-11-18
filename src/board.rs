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

/// How many serial ports the simulated board has. Every one of them is
/// the terminal (decision 46.6).
pub const SERIALS: i64 = 4;

/// How many bits the converter of the simulated board counts with, and
/// the voltage its largest count stands for (decision 47.3).
pub const ANALOG_BITS: u32 = 12;
pub const ANALOG_REFERENCE: f64 = 3.3;

/// The largest count of the converter.
pub const fn analog_max() -> i64 {
    (1 << ANALOG_BITS) - 1
}

/// The pin the LED is on. 13 is where an Arduino Uno has its LED, which
/// makes it the number most people have already met.
pub const LED: i64 = 13;

/// The level of every pin. All of them start low. And what was last
/// written to each register, by its address. A register nobody has
/// written to reads as 0.
pub struct Board {
    /// For each pin that makes a PWM signal: its frequency in hertz and
    /// the share of the time it is on. Both 0 for every other pin.
    pwm: [(f64, f64); PINS as usize],
    levels: [bool; PINS as usize],
    registers: std::collections::HashMap<i64, i64>,
}

impl Board {
    pub fn new() -> Board {
        Board { pwm: [(0.0, 0.0); PINS as usize], levels: [false; PINS as usize], registers: std::collections::HashMap::new() }
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

    /// The frequency the pin's PWM signal was made with, in hertz.
    pub fn pwm_hertz(&self, pin: i64) -> f64 {
        self.pwm[pin as usize].0
    }

    /// Make the pin's PWM signal this: so many hertz, on for this share
    /// of the time. A pin that was driven high goes low first, since the
    /// timer has it now: that line is given back, if there is one. And
    /// whether the signal changed, which is when the board prints it. A
    /// signal that is off all the time is the same at every frequency.
    pub fn pwm(&mut self, pin: i64, hertz: f64, duty: f64) -> (Option<String>, bool) {
        let low = self.write(pin, false);
        let (old_hertz, old_duty) = self.pwm[pin as usize];
        self.pwm[pin as usize] = (hertz, duty);
        (low, duty != old_duty || (duty > 0.0 && hertz != old_hertz))
    }

    /// What the converter counts on the pin. Nothing is wired to the
    /// simulated board, so a pin that is high reads the whole reference
    /// and one that is low reads nothing (decision 47.4).
    pub fn analog_read(&self, pin: i64) -> i64 {
        if self.levels[pin as usize] { analog_max() } else { 0 }
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
