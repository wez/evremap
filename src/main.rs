use anyhow::*;
use evdev::enums::{EventCode, EV_KEY as KeyCode};
use evdev::{Device, GrabMode, InputEvent, ReadFlag, TimeVal, UInputDevice};
use evdev_rs as evdev;
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Clone, Eq, PartialEq)]
enum Mapping {
    DualRole {
        input: KeyCode,
        hold: Vec<KeyCode>,
        tap: Vec<KeyCode>,
    },
    Remap {
        input: Vec<KeyCode>,
        output: Vec<KeyCode>,
    },
}

#[derive(Clone, Copy, Debug)]
enum KeyEventType {
    Release,
    Press,
    Repeat,
    Unknown(i32),
}

impl KeyEventType {
    fn from_value(value: i32) -> Self {
        match value {
            0 => KeyEventType::Release,
            1 => KeyEventType::Press,
            2 => KeyEventType::Repeat,
            _ => KeyEventType::Unknown(value),
        }
    }

    fn value(&self) -> i32 {
        match self {
            Self::Release => 0,
            Self::Press => 1,
            Self::Repeat => 2,
            Self::Unknown(n) => *n,
        }
    }
}

fn timeval_diff(newer: &TimeVal, older: &TimeVal) -> Duration {
    const MICROS_PER_SECOND: i64 = 1000000;
    let secs = newer.tv_sec - older.tv_sec;
    let usecs = newer.tv_usec - older.tv_usec;

    let (secs, usecs) = if usecs < 0 {
        (secs - 1, usecs + MICROS_PER_SECOND)
    } else {
        (secs, usecs)
    };

    Duration::from_micros(((secs * MICROS_PER_SECOND) + usecs) as u64)
}

struct InputMapper {
    input: Device,
    output: UInputDevice,
    /// If present in this map, the key is down since the instant
    /// of its associated value
    input_state: HashMap<KeyCode, TimeVal>,

    mappings: Vec<Mapping>,

    /// The most recent candidate for a tap function is held here
    tapping: Option<KeyCode>,
}

impl InputMapper {
    pub fn create_mapper<P: AsRef<Path>>(path: P, mappings: Vec<Mapping>) -> Result<Self> {
        let path = path.as_ref();
        let f = std::fs::File::open(path).context(format!("opening {}", path.display()))?;
        let mut input = Device::new().ok_or_else(|| anyhow!("failed to make new Device"))?;
        input
            .set_fd(f)
            .context(format!("assigning fd for {} to Device", path.display()))?;

        input.set_name(&format!("evremap Virtual input for {}", path.display()));
        let output = UInputDevice::create_from_device(&input)
            .context(format!("creating UInputDevice from {}", path.display()))?;

        input
            .grab(GrabMode::Grab)
            .context(format!("grabbing exclusive access on {}", path.display()))?;

        Ok(Self {
            input,
            output,
            input_state: HashMap::new(),
            tapping: None,
            mappings,
        })
    }

    fn lookup_mapping(&self, code: KeyCode) -> Option<Mapping> {
        let mut candidates = vec![];

        for map in &self.mappings {
            match map {
                Mapping::DualRole { input, .. } => {
                    if *input == code {
                        // A DualRole mapping has the highest precedence
                        // so we've found our match
                        return Some(map.clone());
                    }
                }
                Mapping::Remap { input, .. } => {
                    // Look for a mapping that includes the current key.
                    // If part of a chord, all of its component keys must
                    // also be pressed.
                    let mut code_matched = false;
                    let mut all_matched = true;
                    for i in input {
                        if *i == code {
                            code_matched = true;
                        } else if !self.input_state.contains_key(i) {
                            all_matched = false;
                            break;
                        }
                    }
                    if code_matched && all_matched {
                        candidates.push(map);
                    }
                }
            }
        }

        // Any matches must be Remap entries.  We want the one
        // with the most active keys
        candidates.sort_by(|a, b| match (a, b) {
            (Mapping::Remap { input: input_a, .. }, Mapping::Remap { input: input_b, .. }) => {
                input_a.len().cmp(&input_b.len()).reverse()
            }
            _ => unreachable!(),
        });

        candidates.get(0).map(|&m| m.clone())
    }

    pub fn update_with_event(&mut self, event: &InputEvent, code: KeyCode) -> Result<()> {
        let event_type = KeyEventType::from_value(event.value);
        match event_type {
            KeyEventType::Release => {
                let pressed_at = match self.input_state.remove(&code) {
                    None => {
                        self.sync_event(event)?;
                        return Ok(());
                    }
                    Some(p) => p,
                };

                match self.lookup_mapping(code.clone()) {
                    Some(Mapping::DualRole { hold, tap, .. }) => {
                        // If released quickly enough, becomes a tap press.
                        // Regardless: release the hold keys
                        self.emit_keys(&hold, &event.time, KeyEventType::Release)?;

                        if let Some(tapping) = self.tapping.take() {
                            if tapping == code
                                && timeval_diff(&event.time, &pressed_at)
                                    <= Duration::from_millis(200)
                            {
                                self.emit_keys(&tap, &event.time, KeyEventType::Press)?;
                                self.emit_keys(&tap, &event.time, KeyEventType::Release)?;
                            }
                        }
                    }
                    Some(Mapping::Remap { .. }) => {
                        unreachable!();
                    }
                    None => {
                        // Just pass it through
                        self.sync_event(event)?;
                    }
                }
            }
            KeyEventType::Press => {
                self.input_state.insert(code.clone(), event.time.clone());

                match self.lookup_mapping(code.clone()) {
                    Some(Mapping::DualRole { hold, .. }) => {
                        self.emit_keys(&hold, &event.time, KeyEventType::Press)?;
                        self.tapping.replace(code);
                    }
                    Some(Mapping::Remap { .. }) => {
                        unreachable!();
                    }
                    None => {
                        // Just pass it through
                        self.cancel_pending_tap();
                        self.sync_event(event)?;
                    }
                }
            }
            KeyEventType::Repeat => {
                self.sync_event(event)?;
            }
            KeyEventType::Unknown(_) => {
                self.sync_event(event)?;
            }
        }

        Ok(())
    }

    fn cancel_pending_tap(&mut self) {
        self.tapping.take();
    }

    fn emit_keys(&self, key: &[KeyCode], time: &TimeVal, event_type: KeyEventType) -> Result<()> {
        for k in key {
            let event = make_event(k.clone(), time, event_type);
            log::trace!("OUT: {:?}", event);
            self.output.write_event(&event)?;
        }
        self.output.write_event(&InputEvent::new(
            time,
            &EventCode::EV_SYN(evdev_rs::enums::EV_SYN::SYN_REPORT),
            0,
        ))?;
        Ok(())
    }

    fn emit_key(&self, key: KeyCode, time: &TimeVal, event_type: KeyEventType) -> Result<()> {
        let event = make_event(key, time, event_type);
        self.sync_event(&event)?;
        Ok(())
    }

    fn sync_event(&self, event: &InputEvent) -> Result<()> {
        log::trace!("OUT: {:?}", event);
        self.output.write_event(&event)?;
        self.output.write_event(&InputEvent::new(
            &event.time,
            &EventCode::EV_SYN(evdev_rs::enums::EV_SYN::SYN_REPORT),
            0,
        ))?;
        Ok(())
    }
}

fn make_event(key: KeyCode, time: &TimeVal, event_type: KeyEventType) -> InputEvent {
    InputEvent::new(time, &EventCode::EV_KEY(key), event_type.value())
}

fn main() -> Result<()> {
    pretty_env_logger::init();

    let mappings = vec![Mapping::DualRole {
        input: KeyCode::KEY_CAPSLOCK,
        hold: vec![KeyCode::KEY_LEFTCTRL],
        tap: vec![KeyCode::KEY_ESC],
    }];

    log::error!("Short delay: release any keys now!");
    std::thread::sleep(Duration::new(2, 0));

    let path = "/dev/input/event2";

    let mut mapper = InputMapper::create_mapper(path, mappings)?;

    log::error!("Going into read loop");
    loop {
        let (status, event) = mapper
            .input
            .next_event(ReadFlag::NORMAL | ReadFlag::BLOCKING)?;
        match status {
            evdev::ReadStatus::Success => {
                if let EventCode::EV_KEY(ref key) = event.event_code {
                    log::trace!("IN {:?}", event);
                    mapper.update_with_event(&event, key.clone())?;
                } else {
                    log::trace!("PASSTHRU {:?}", event);
                    mapper.output.write_event(&event)?;
                }
            }
            evdev::ReadStatus::Sync => bail!("ReadStatus::Sync!"),
        }
    }
}
