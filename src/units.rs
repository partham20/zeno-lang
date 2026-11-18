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
/// factor between them is not an exact number: they never meet in `+` or
/// a comparison by themselves, but `.rad` and `.deg` change one into the
/// other when the program asks (see `convert`).
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
    // The speed of a serial line: `serial(1, 115200 baud)` (decision 46.2).
    ("baud", "speed of a serial line", 1.0),
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
    // An angle is the one thing with two units that are not a whole
    // number apart. Asked for, it is changed: half a turn is `180 deg`
    // and `pi rad`.
    match (from, to) {
        ("deg", "rad") => return Some(Scale { times: std::f64::consts::PI / 180.0, over: 1.0 }),
        ("rad", "deg") => return Some(Scale { times: 180.0 / std::f64::consts::PI, over: 1.0 }),
        _ => {}
    }
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
    let ((a_name, a_kind, a_size), (b_name, b_kind, b_size)) = (find(a)?, find(b)?);
    // Degrees and radians convert when asked, but neither is the finer
    // of the two, so they do not meet by themselves.
    if a_kind != b_kind {
        return None;
    }
    let unit = if a_size <= b_size { a_name } else { b_name };
    Some((unit, convert(a, unit)?, convert(b, unit)?))
}

/// What `*` and `/` make of two quantities (decision 43). Each row is
/// the operator, what the left side measures (`""` for a plain number),
/// what the right side measures, and what the answer measures (`""` for a
/// plain number). The list is closed, as the list of units is: these are
/// the sums a firmware program does every day.
pub const MADE: &[(&str, &str, &str, &str)] = &[
    ("*", "voltage", "current", "power"),
    ("*", "current", "voltage", "power"),
    ("*", "current", "resistance", "voltage"),
    ("*", "resistance", "current", "voltage"),
    ("*", "power", "time", "energy"),
    ("*", "time", "power", "energy"),
    ("*", "time", "frequency", ""),
    ("*", "frequency", "time", ""),
    ("/", "power", "voltage", "current"),
    ("/", "power", "current", "voltage"),
    ("/", "voltage", "current", "resistance"),
    ("/", "voltage", "resistance", "current"),
    ("/", "energy", "time", "power"),
    ("/", "energy", "power", "time"),
    ("/", "", "time", "frequency"),
    ("/", "", "frequency", "time"),
];

/// The unit of each kind with no prefix, which a made quantity is in:
/// `3.3 V * 20 mA` is `0.066 W`, and `.mW` makes it `66 mW`.
pub const PLAIN: &[(&str, &str)] = &[
    ("time", "s"),
    ("frequency", "Hz"),
    ("voltage", "V"),
    ("current", "A"),
    ("resistance", "ohm"),
    ("power", "W"),
    ("energy", "J"),
];

/// What an answer made by `*` or `/` is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Made {
    /// A quantity in this unit.
    Unit(&'static str),
    /// A plain number, as `10 ms * 1 kHz` is 10 cycles.
    Number,
}

/// What `left op right` makes, where `left` is `None` for a plain
/// number. `None` when the two make nothing on the list.
pub fn made(op: &str, left: Option<&str>, right: &str) -> Option<Made> {
    let left_kind = match left {
        Some(unit) => find(unit)?.1,
        None => "",
    };
    let right_kind = find(right)?.1;
    let (_, _, _, kind) = MADE.iter().find(|(o, l, r, _)| *o == op && *l == left_kind && *r == right_kind)?;
    if kind.is_empty() {
        return Some(Made::Number);
    }
    PLAIN.iter().find(|(k, _)| k == kind).map(|(_, unit)| Made::Unit(unit))
}

/// How to turn a quantity in `unit` into one in the unit of its kind with
/// no prefix, before it is multiplied or divided.
pub fn to_plain(unit: &str) -> Option<Scale> {
    let kind = find(unit)?.1;
    let (_, plain) = PLAIN.iter().find(|(k, _)| *k == kind)?;
    convert(unit, plain)
}

/// For a message: the sums `made` knows, as `V * A is W`.
pub fn made_list() -> &'static str {
    "V * A is W, A * ohm is V, W * s is J, V / A is ohm, V / ohm is A, W / V is A, J / s is W, 1 / s is Hz"
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
    fn an_angle_converts_when_asked_and_never_by_itself() {
        use std::f64::consts::PI;
        assert_eq!(convert("deg", "rad").map(|scale| scale.apply(180.0)), Some(PI));
        assert_eq!(convert("deg", "rad").map(|scale| scale.apply(90.0)), Some(PI / 2.0));
        assert_eq!(convert("rad", "deg").map(|scale| scale.apply(PI)), Some(180.0));
        assert_eq!(convert("rad", "rad").map(|scale| scale.apply(2.0)), Some(2.0));
        assert!(common("deg", "rad").is_none());
        assert!(common("rad", "deg").is_none());
        assert_eq!(common("deg", "deg").map(|(unit, _, _)| unit), Some("deg"));
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
    fn units_make_units() {
        assert_eq!(made("*", Some("V"), "mA"), Some(Made::Unit("W")));
        assert_eq!(made("*", Some("mA"), "kohm"), Some(Made::Unit("V")));
        assert_eq!(made("/", Some("V"), "mA"), Some(Made::Unit("ohm")));
        assert_eq!(made("/", Some("kWh"), "h"), Some(Made::Unit("W")));
        assert_eq!(made("/", None, "ms"), Some(Made::Unit("Hz")));
        assert_eq!(made("/", None, "kHz"), Some(Made::Unit("s")));
        assert_eq!(made("*", Some("ms"), "kHz"), Some(Made::Number));
        assert_eq!(made("*", Some("m"), "m"), None);
        assert_eq!(made("/", None, "V"), None);
        assert_eq!(to_plain("mA").map(|scale| scale.apply(20.0)), Some(0.02));
        assert_eq!(to_plain("kohm").map(|scale| scale.apply(4.7)), Some(4700.0));
        // Every kind that makes or is made has a unit with no prefix.
        for (_, left, right, answer) in MADE {
            for kind in [left, right, answer] {
                assert!(kind.is_empty() || PLAIN.iter().any(|(k, _)| k == kind), "{kind}");
            }
        }
        for (kind, unit) in PLAIN {
            assert_eq!(find(unit).map(|(_, k, _)| k), Some(*kind));
        }
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
