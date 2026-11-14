//! The units Zeno knows, for quantities such as `500 ms` and `3.3 V`.
//!
//! A unit has to be on this list to be used. That is what turns a misspelt
//! unit (`500 mss`) into an error instead of a new, different unit.

/// Each unit, with what it measures, and how many of the smallest unit of
/// that kind it holds. A second is 1 000 000 000 nanoseconds.
///
/// Sizes are whole numbers on purpose. Going from any unit to a finer one of
/// the same kind is then a multiplication by a whole number, which loses
/// nothing.
///
/// `degC` and `K` are kept apart: they differ by an offset, not a scale, so
/// adding them means nothing. `deg` and `rad` are kept apart because the
/// factor between them is not an exact number.
pub const UNITS: &[(&str, &str, f64)] = &[
    ("ns", "time", 1.0),
    ("us", "time", 1e3),
    ("ms", "time", 1e6),
    ("s", "time", 1e9),
    ("min", "time", 60e9),
    ("h", "time", 3600e9),
    ("Hz", "frequency", 1.0),
    ("kHz", "frequency", 1e3),
    ("MHz", "frequency", 1e6),
    ("uV", "voltage", 1.0),
    ("mV", "voltage", 1e3),
    ("V", "voltage", 1e6),
    ("kV", "voltage", 1e9),
    ("uA", "current", 1.0),
    ("mA", "current", 1e3),
    ("A", "current", 1e6),
    ("ohm", "resistance", 1.0),
    ("kohm", "resistance", 1e3),
    ("Mohm", "resistance", 1e6),
    ("mW", "power", 1.0),
    ("W", "power", 1e3),
    ("kW", "power", 1e6),
    ("J", "energy", 1.0),
    ("Wh", "energy", 3600.0),
    ("kWh", "energy", 3600e3),
    ("degC", "temperature in Celsius", 1.0),
    ("K", "temperature in kelvin", 1.0),
    ("mm", "length", 1.0),
    ("cm", "length", 10.0),
    ("m", "length", 1e3),
    ("km", "length", 1e6),
    ("mg", "mass", 1.0),
    ("g", "mass", 1e3),
    ("kg", "mass", 1e6),
    ("deg", "angle in degrees", 1.0),
    ("rad", "angle in radians", 1.0),
    ("rpm", "rotation", 1.0),
];

pub fn is_unit(name: &str) -> bool {
    UNITS.iter().any(|(unit, _, _)| *unit == name)
}

/// The message for a unit that is not on the list.
pub fn unknown(name: &str) -> String {
    format!("`{name}` is not a unit Zeno knows. Some that it does: s, ms, us, Hz, V, mV, A, mA, W, degC")
}

/// How to turn a number in one unit into a number in another: multiply by
/// `times`, or divide by `over`. One of the two is always 1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scale {
    pub times: f64,
    pub over: f64,
}

impl Scale {
    pub fn apply(self, x: f64) -> f64 {
        x * self.times / self.over
    }
}

fn find(name: &str) -> Option<(&'static str, &'static str, f64)> {
    UNITS.iter().copied().find(|(unit, _, _)| *unit == name)
}

/// How to turn a quantity in `from` into one in `to`, as `(3 s).ms` asks.
/// `None` if the two measure different things.
pub fn convert(from: &str, to: &str) -> Option<Scale> {
    let ((_, from_kind, from_size), (_, to_kind, to_size)) = (find(from)?, find(to)?);
    if from_kind != to_kind {
        return None;
    }
    Some(if from_size >= to_size {
        // To a finer unit: a whole number of times bigger.
        Scale { times: from_size / to_size, over: 1.0 }
    } else {
        Scale { times: 1.0, over: to_size / from_size }
    })
}

/// When two quantities meet in `+`, `-` or a comparison, the unit they are
/// both turned into: the finer of the two, so that neither loses anything.
/// `1 s + 500 ms` is `1500 ms`. Also returns how to scale each side.
pub fn common(a: &str, b: &str) -> Option<(&'static str, Scale, Scale)> {
    let ((a_name, _, a_size), (b_name, _, b_size)) = (find(a)?, find(b)?);
    let unit = if a_size <= b_size { a_name } else { b_name };
    Some((unit, convert(a, unit)?, convert(b, unit)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn units_of_one_kind_convert() {
        assert_eq!(convert("s", "ms").map(|scale| scale.apply(3.0)), Some(3000.0));
        assert_eq!(convert("ms", "s").map(|scale| scale.apply(1500.0)), Some(1.5));
        assert_eq!(convert("h", "min").map(|scale| scale.apply(2.0)), Some(120.0));
        assert_eq!(convert("kWh", "J").map(|scale| scale.apply(1.0)), Some(3_600_000.0));
        assert_eq!(convert("s", "V"), None);
        assert_eq!(convert("degC", "K"), None);
    }

    #[test]
    fn the_finer_unit_wins() {
        let (unit, a, b) = common("s", "ms").unwrap();
        assert_eq!((unit, a.apply(1.0), b.apply(500.0)), ("ms", 1000.0, 500.0));
        assert_eq!(common("mV", "V").map(|(unit, _, _)| unit), Some("mV"));
        assert_eq!(common("ms", "ms").map(|(unit, a, _)| (unit, a.apply(7.0))), Some(("ms", 7.0)));
        assert!(common("ms", "mV").is_none());
    }

    #[test]
    fn every_size_is_a_whole_multiple_of_the_finer_ones() {
        for (a, a_kind, a_size) in UNITS {
            for (b, b_kind, b_size) in UNITS {
                if a_kind == b_kind && a_size >= b_size {
                    assert_eq!((a_size / b_size).fract(), 0.0, "{a} in {b}");
                }
            }
        }
    }
}
