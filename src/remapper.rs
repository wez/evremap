use crate::mapping::*;
use anyhow::*;
use evdev_rs::{Device, GrabMode, InputEvent, ReadFlag, TimeVal, UInputDevice};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Duration;

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
    const MICROS_PER_SECOND: libc::time_t = 1000000;
    let secs = newer.tv_sec - older.tv_sec;
    let usecs = newer.tv_usec - older.tv_usec;

    let (secs, usecs) = if usecs < 0 {
        (secs - 1, usecs + MICROS_PER_SECOND)
    } else {
        (secs, usecs)
    };

    Duration::from_micros(((secs * MICROS_PER_SECOND) + usecs) as u64)
}

pub struct InputMapper {
    input: Device,
    output: UInputDevice,
    /// If present in this map, the key is down since the instant
    /// of its associated value
    input_state: HashMap<KeyCode, TimeVal>,

    mappings: Vec<Mapping>,

    /// The most recent candidate for a tap function is held here
    tapping: Option<KeyCode>,

    output_keys: HashSet<KeyCode>,
}

fn enable_key_code(input: &mut Device, key: KeyCode) -> Result<()> {
    input
        .enable(&EventCode::EV_KEY(key.clone()))
        .context(format!("enable key {:?}", key))?;
    Ok(())
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

        // Ensure that any remapped keys are supported by the generated output device
        for map in &mappings {
            match map {
                Mapping::DualRole { tap, hold, .. } => {
                    for t in tap {
                        enable_key_code(&mut input, t.clone())?;
                    }
                    for h in hold {
                        enable_key_code(&mut input, h.clone())?;
                    }
                }
                Mapping::Remap { output, .. } => {
                    for o in output {
                        enable_key_code(&mut input, o.clone())?;
                    }
                }
            }
        }

        let output = UInputDevice::create_from_device(&input)
            .context(format!("creating UInputDevice from {}", path.display()))?;

        input
            .grab(GrabMode::Grab)
            .context(format!("grabbing exclusive access on {}", path.display()))?;

        Ok(Self {
            input,
            output,
            input_state: HashMap::new(),
            output_keys: HashSet::new(),
            tapping: None,
            mappings,
        })
    }

    pub fn run_mapper(&mut self) -> Result<()> {
        log::info!("Going into read loop");
        loop {
            let (status, event) = self
                .input
                .next_event(ReadFlag::NORMAL | ReadFlag::BLOCKING)?;
            match status {
                evdev_rs::ReadStatus::Success => {
                    if let EventCode::EV_KEY(ref key) = event.event_code {
                        log::trace!("IN {:?}", event);
                        self.update_with_event(&event, key.clone())?;
                    } else {
                        log::trace!("PASSTHRU {:?}", event);
                        self.output.write_event(&event)?;
                    }
                }
                evdev_rs::ReadStatus::Sync => bail!("ReadStatus::Sync!"),
            }
        }
    }

    /// Compute the effective set of keys that are pressed
    fn compute_keys(&self) -> HashSet<KeyCode> {
        // Start with the input keys
        let mut keys: HashSet<KeyCode> = self.input_state.keys().cloned().collect();

        // First phase is to apply any DualRole mappings as they are likely to
        // be used to produce modifiers when held.
        for map in &self.mappings {
            if let Mapping::DualRole { input, hold, .. } = map {
                if keys.contains(input) {
                    keys.remove(input);
                    for h in hold {
                        keys.insert(h.clone());
                    }
                }
            }
        }

        let mut keys_minus_remapped = keys.clone();

        // Second pass to apply Remap items
        for map in &self.mappings {
            if let Mapping::Remap { input, output } = map {
                if input.is_subset(&keys_minus_remapped) {
                    for i in input {
                        keys.remove(i);
                        if !is_modifier(i) {
                            keys_minus_remapped.remove(i);
                        }
                    }
                    for o in output {
                        keys.insert(o.clone());
                        // Outputs that apply are not visible as
                        // inputs for later remap rules
                        if !is_modifier(o) {
                            keys_minus_remapped.remove(o);
                        }
                    }
                }
            }
        }

        keys
    }

    /// Compute the difference between our desired set of keys
    /// and the set of keys that are currently pressed in the
    /// output device.
    /// Release any keys that should not be pressed, and then
    /// press any keys that should be pressed.
    ///
    /// When releasing, release modifiers last so that mappings
    /// that produce eg: CTRL-C don't emit a random C character
    /// when released.
    ///
    /// Similarly, when pressing, emit modifiers first so that
    /// we don't emit C and then CTRL for such a mapping.
    fn compute_and_apply_keys(&mut self, time: &TimeVal) -> Result<()> {
        let desired_keys = self.compute_keys();
        let mut to_release: Vec<KeyCode> = self
            .output_keys
            .difference(&desired_keys)
            .cloned()
            .collect();

        let mut to_press: Vec<KeyCode> = desired_keys
            .difference(&self.output_keys)
            .cloned()
            .collect();

        if !to_release.is_empty() {
            to_release.sort_by(modifiers_last);
            self.emit_keys(&to_release, time, KeyEventType::Release)?;
        }
        if !to_press.is_empty() {
            to_press.sort_by(modifiers_first);
            self.emit_keys(&to_press, time, KeyEventType::Press)?;
        }
        Ok(())
    }

    fn lookup_dual_role_mapping(&self, code: KeyCode) -> Option<Mapping> {
        for map in &self.mappings {
            if let Mapping::DualRole { input, .. } = map {
                if *input == code {
                    // A DualRole mapping has the highest precedence
                    // so we've found our match
                    return Some(map.clone());
                }
            }
        }
        None
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
                        self.write_event_and_sync(event)?;
                        return Ok(());
                    }
                    Some(p) => p,
                };

                self.compute_and_apply_keys(&event.time)?;

                if let Some(Mapping::DualRole { tap, .. }) =
                    self.lookup_dual_role_mapping(code.clone())
                {
                    // If released quickly enough, becomes a tap press.
                    if let Some(tapping) = self.tapping.take() {
                        if tapping == code
                            && timeval_diff(&event.time, &pressed_at) <= Duration::from_millis(200)
                        {
                            self.emit_keys(&tap, &event.time, KeyEventType::Press)?;
                            self.emit_keys(&tap, &event.time, KeyEventType::Release)?;
                        }
                    }
                }
            }
            KeyEventType::Press => {
                self.input_state.insert(code.clone(), event.time.clone());

                match self.lookup_mapping(code.clone()) {
                    Some(_) => {
                        self.compute_and_apply_keys(&event.time)?;
                        self.tapping.replace(code);
                    }
                    None => {
                        // Just pass it through
                        self.cancel_pending_tap();
                        self.compute_and_apply_keys(&event.time)?;
                    }
                }
            }
            KeyEventType::Repeat => {
                match self.lookup_mapping(code.clone()) {
                    Some(Mapping::DualRole { hold, .. }) => {
                        self.emit_keys(&hold, &event.time, KeyEventType::Repeat)?;
                    }
                    Some(Mapping::Remap { output, .. }) => {
                        let output: Vec<KeyCode> = output.iter().cloned().collect();
                        self.emit_keys(&output, &event.time, KeyEventType::Repeat)?;
                    }
                    None => {
                        // Just pass it through
                        self.cancel_pending_tap();
                        self.write_event_and_sync(event)?;
                    }
                }
            }
            KeyEventType::Unknown(_) => {
                self.write_event_and_sync(event)?;
            }
        }

        Ok(())
    }

    fn cancel_pending_tap(&mut self) {
        self.tapping.take();
    }

    fn emit_keys(
        &mut self,
        key: &[KeyCode],
        time: &TimeVal,
        event_type: KeyEventType,
    ) -> Result<()> {
        for k in key {
            let event = make_event(k.clone(), time, event_type);
            self.write_event(&event)?;
        }
        self.generate_sync_event(time)?;
        Ok(())
    }

    fn write_event_and_sync(&mut self, event: &InputEvent) -> Result<()> {
        self.write_event(event)?;
        self.generate_sync_event(&event.time)?;
        Ok(())
    }

    fn write_event(&mut self, event: &InputEvent) -> Result<()> {
        log::trace!("OUT: {:?}", event);
        self.output.write_event(&event)?;
        if let EventCode::EV_KEY(ref key) = event.event_code {
            let event_type = KeyEventType::from_value(event.value);
            match event_type {
                KeyEventType::Press | KeyEventType::Repeat => {
                    self.output_keys.insert(key.clone());
                }
                KeyEventType::Release => {
                    self.output_keys.remove(key);
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn generate_sync_event(&self, time: &TimeVal) -> Result<()> {
        self.output.write_event(&InputEvent::new(
            time,
            &EventCode::EV_SYN(evdev_rs::enums::EV_SYN::SYN_REPORT),
            0,
        ))?;
        Ok(())
    }
}

fn make_event(key: KeyCode, time: &TimeVal, event_type: KeyEventType) -> InputEvent {
    InputEvent::new(time, &EventCode::EV_KEY(key), event_type.value())
}

fn is_modifier(key: &KeyCode) -> bool {
    match key {
        KeyCode::KEY_FN
        | KeyCode::KEY_LEFTALT
        | KeyCode::KEY_RIGHTALT
        | KeyCode::KEY_LEFTMETA
        | KeyCode::KEY_RIGHTMETA
        | KeyCode::KEY_LEFTCTRL
        | KeyCode::KEY_RIGHTCTRL
        | KeyCode::KEY_LEFTSHIFT
        | KeyCode::KEY_RIGHTSHIFT => true,
        _ => false,
    }
}

/// Orders modifier keys ahead of non-modifier keys.
/// Unfortunately the underlying type doesn't allow direct
/// comparison, but that's ok for our purposes.
fn modifiers_first(a: &KeyCode, b: &KeyCode) -> Ordering {
    if is_modifier(a) {
        if is_modifier(b) {
            Ordering::Equal
        } else {
            Ordering::Less
        }
    } else if is_modifier(b) {
        Ordering::Greater
    } else {
        // Neither are modifiers
        Ordering::Equal
    }
}

fn modifiers_last(a: &KeyCode, b: &KeyCode) -> Ordering {
    modifiers_first(a, b).reverse()
}
