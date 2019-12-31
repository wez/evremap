pub use evdev_rs::enums::{EventCode, EV_KEY as KeyCode};
use std::collections::HashSet;

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Mapping {
    DualRole {
        input: KeyCode,
        hold: Vec<KeyCode>,
        tap: Vec<KeyCode>,
    },
    Remap {
        input: HashSet<KeyCode>,
        output: HashSet<KeyCode>,
    },
}
