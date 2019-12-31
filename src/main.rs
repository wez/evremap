use anyhow::*;
use evdev::enums::{EventCode, EV_KEY as KeyCode};
use evdev::{Device, GrabMode, InputEvent, ReadFlag, TimeVal, UInputDevice};
use evdev_rs as evdev;
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

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

struct InputPair {
    input: Device,
    output: UInputDevice,
    /// If present in this map, the key is down since the instant
    /// of its associated value
    input_state: HashMap<KeyCode, TimeVal>,

    /// The most recent candidate for a tap function is held here
    tapping: Option<KeyCode>,
}

impl InputPair {
    pub fn create_mapper<P: AsRef<Path>>(path: P) -> Result<Self> {
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
        })
    }

    pub fn update_with_event(&mut self, event: &InputEvent, code: KeyCode) -> Result<()> {
        let event_type = KeyEventType::from_value(event.value);
        match event_type {
            KeyEventType::Release => {
                let pressed_at = self.input_state.remove(&code);

                if pressed_at.is_none() {
                    self.sync_event(event)?;
                    return Ok(());
                }

                if code == KeyCode::KEY_CAPSLOCK {
                    if let Some(pressed_at) = pressed_at {
                        // If released quickly enough, becomes an ESC key press.
                        // Regardless, we'll release the CTRL value that we mapped it to first.
                        self.emit_key(KeyCode::KEY_LEFTCTRL, &event.time, KeyEventType::Release)?;

                        // If no other key went down since the caps key, then this may be a short
                        // tap on that key; if so, remap to escape
                        if let Some(KeyCode::KEY_CAPSLOCK) = self.tapping.take() {
                            if timeval_diff(&event.time, &pressed_at) <= Duration::from_millis(200)
                            {
                                self.emit_key(KeyCode::KEY_ESC, &event.time, KeyEventType::Press)?;
                                self.emit_key(
                                    KeyCode::KEY_ESC,
                                    &event.time,
                                    KeyEventType::Release,
                                )?;
                            }
                        }
                    } else {
                        self.sync_event(event)?;
                    }
                } else {
                    self.sync_event(event)?;
                }
            }
            KeyEventType::Press => {
                self.input_state.insert(code.clone(), event.time.clone());

                if code == KeyCode::KEY_CAPSLOCK {
                    // Remap caps to ctrl
                    self.emit_key(KeyCode::KEY_LEFTCTRL, &event.time, KeyEventType::Press)?;
                    self.tapping.replace(KeyCode::KEY_CAPSLOCK);
                } else {
                    self.cancel_pending_tap();
                    self.sync_event(event)?;
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

    fn emit_key(&self, key: KeyCode, time: &TimeVal, event_type: KeyEventType) -> Result<()> {
        let event = make_event(key, time, event_type);
        self.sync_event(&event)?;
        Ok(())
    }

    fn sync_event(&self, event: &InputEvent) -> Result<()> {
        println!("OUT: {:?}", event);
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
    println!("Short delay: release any keys now!");
    std::thread::sleep(Duration::new(2, 0));

    let path = "/dev/input/event2";

    let mut pair = InputPair::create_mapper(path)?;

    println!("Going into read loop");
    loop {
        let (status, event) = pair
            .input
            .next_event(ReadFlag::NORMAL | ReadFlag::BLOCKING)?;
        match status {
            evdev::ReadStatus::Success => {
                if let EventCode::EV_KEY(ref key) = event.event_code {
                    println!("IN {:?}", event);
                    pair.update_with_event(&event, key.clone())?;
                } else {
                    println!("PASSTHRU {:?}", event);
                    pair.output.write_event(&event)?;
                }
            }
            evdev::ReadStatus::Sync => bail!("ReadStatus::Sync!"),
        }
    }
}
