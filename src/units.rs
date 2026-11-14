//! The units Zeno knows, for quantities such as `500 ms` and `3.3 V`.
//!
//! A unit has to be on this list to be used. That is what turns a misspelt
//! unit (`500 mss`) into an error instead of a new, different unit.

/// Each unit, with what it measures.
pub const UNITS: &[(&str, &str)] = &[
    ("ns", "time"),
    ("us", "time"),
    ("ms", "time"),
    ("s", "time"),
    ("min", "time"),
    ("h", "time"),
    ("Hz", "frequency"),
    ("kHz", "frequency"),
    ("MHz", "frequency"),
    ("uV", "voltage"),
    ("mV", "voltage"),
    ("V", "voltage"),
    ("kV", "voltage"),
    ("uA", "current"),
    ("mA", "current"),
    ("A", "current"),
    ("ohm", "resistance"),
    ("kohm", "resistance"),
    ("Mohm", "resistance"),
    ("mW", "power"),
    ("W", "power"),
    ("kW", "power"),
    ("J", "energy"),
    ("Wh", "energy"),
    ("kWh", "energy"),
    ("degC", "temperature"),
    ("K", "temperature"),
    ("mm", "length"),
    ("cm", "length"),
    ("m", "length"),
    ("km", "length"),
    ("mg", "mass"),
    ("g", "mass"),
    ("kg", "mass"),
    ("deg", "angle"),
    ("rad", "angle"),
    ("rpm", "rotation"),
];

pub fn is_unit(name: &str) -> bool {
    UNITS.iter().any(|(unit, _)| *unit == name)
}

/// The message for a unit that is not on the list.
pub fn unknown(name: &str) -> String {
    format!("`{name}` is not a unit Zeno knows. Some that it does: s, ms, us, Hz, V, mV, A, mA, W, degC")
}
