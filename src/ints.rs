//! Whole numbers with a size: `u8`, `u16`, `u32`, `i8`, `i16`, `i32`.
//!
//! An `int` is 64 bits everywhere. These say that a value fits in fewer,
//! which matters where it is stored or sent: a field of an object, a
//! byte on a wire, a register. See docs/sized-ints.md.
//!
//! This file is the one place that knows what each size holds. The
//! checker, the interpreter and both compilers ask it.

/// One of the six sizes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Width {
    /// As it is written in a program: `u8`.
    pub name: &'static str,
    /// The name of its wrapping conversion: `u8.wrap`.
    pub wrap: &'static str,
    pub bits: u32,
    pub signed: bool,
}

pub const WIDTHS: [Width; 6] = [
    Width { name: "u8", wrap: "u8.wrap", bits: 8, signed: false },
    Width { name: "u16", wrap: "u16.wrap", bits: 16, signed: false },
    Width { name: "u32", wrap: "u32.wrap", bits: 32, signed: false },
    Width { name: "i8", wrap: "i8.wrap", bits: 8, signed: true },
    Width { name: "i16", wrap: "i16.wrap", bits: 16, signed: true },
    Width { name: "i32", wrap: "i32.wrap", bits: 32, signed: true },
];

/// The size with this name, if it is one: `u8`.
pub fn width(name: &str) -> Option<Width> {
    WIDTHS.iter().copied().find(|width| width.name == name)
}

/// The size whose wrapping conversion has this name: `u8.wrap`.
pub fn wrapping(name: &str) -> Option<Width> {
    WIDTHS.iter().copied().find(|width| width.wrap == name)
}

impl Width {
    /// The smallest number it holds.
    pub fn min(self) -> i64 {
        if self.signed { -(1 << (self.bits - 1)) } else { 0 }
    }

    /// The largest number it holds.
    pub fn max(self) -> i64 {
        if self.signed { (1 << (self.bits - 1)) - 1 } else { (1 << self.bits) - 1 }
    }

    pub fn holds(self, n: i64) -> bool {
        (self.min()..=self.max()).contains(&n)
    }

    /// What is said when a number does not fit.
    pub fn does_not_fit(self, n: impl std::fmt::Display) -> String {
        // "a u8" but "an i8", as they are said aloud.
        let article = if self.signed { "an" } else { "a" };
        format!("{n} does not fit in {article} {} ({} to {})", self.name, self.min(), self.max())
    }

    /// Keep the low bits of `n` and read them as this size. Nothing is
    /// checked: 300 as a `u8` is 44, and 200 as an `i8` is -56.
    pub fn wrap(self, n: i64) -> i64 {
        let low = n & ((1 << self.bits) - 1);
        // In a signed size the top bit counts as minus.
        if self.signed && low > self.max() { low - (1 << self.bits) } else { low }
    }

    /// The C type that holds it in an object.
    pub fn c_type(self) -> String {
        format!("{}int{}_t", if self.signed { "" } else { "u" }, self.bits)
    }

    /// The LLVM IR type that holds it in an object.
    pub fn ir_type(self) -> &'static str {
        match self.bits {
            8 => "i8",
            16 => "i16",
            _ => "i32",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_size_knows_what_it_holds() {
        let ranges: Vec<(i64, i64)> = WIDTHS.iter().map(|width| (width.min(), width.max())).collect();
        assert_eq!(
            ranges,
            [(0, 255), (0, 65535), (0, 4294967295), (-128, 127), (-32768, 32767), (-2147483648, 2147483647)]
        );
        let u8 = width("u8").unwrap();
        assert!(u8.holds(0) && u8.holds(255) && !u8.holds(256) && !u8.holds(-1));
        assert_eq!(u8.does_not_fit(300), "300 does not fit in a u8 (0 to 255)");
        assert_eq!(width("i8").unwrap().does_not_fit(-129), "-129 does not fit in an i8 (-128 to 127)");
        assert!(width("u64").is_none());
    }

    #[test]
    fn wrapping_keeps_the_low_bits() {
        let (u8, i8, i16, u32) = (width("u8").unwrap(), width("i8").unwrap(), width("i16").unwrap(), width("u32").unwrap());
        assert_eq!([u8.wrap(300), u8.wrap(-1), u8.wrap(255), u8.wrap(256)], [44, 255, 255, 0]);
        assert_eq!([i8.wrap(200), i8.wrap(127), i8.wrap(128), i8.wrap(-129)], [-56, 127, -128, 127]);
        assert_eq!([i16.wrap(0xFFFF), i16.wrap(0x8000), u32.wrap(-1)], [-1, -32768, 4294967295]);
        assert_eq!(wrapping("i16.wrap"), Some(i16));
        // A number that fits comes back as it went in.
        for width in WIDTHS {
            for n in [width.min(), -1, 0, 1, width.max()] {
                if width.holds(n) {
                    assert_eq!(width.wrap(n), n);
                }
            }
        }
    }
}
