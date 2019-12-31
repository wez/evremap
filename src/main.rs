use anyhow::*;
use evdev::enums::{EventCode, EV_KEY as KeyCode};
use evdev::{Device, GrabMode, InputEvent, ReadFlag, TimeVal, UInputDevice};
use evdev_rs as evdev;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;
use structopt::StructOpt;

#[derive(Debug, StructOpt)]
#[structopt(
    name = "evremap",
    about = "Remap libinput evdev keyboard inputs",
    author = "Wez Furlong"
)]
struct Opt {
    /// Rather than running the remapper, list currently available devices.
    /// This is helpful to check their names when setting up the initial
    /// configuration
    #[structopt(name = "list-devices", long)]
    list_devices: bool,
}

#[derive(Debug, Clone, Eq, PartialEq)]
enum Mapping {
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

        // Second pass to apply Remap items
        for map in &self.mappings {
            if let Mapping::Remap { input, output } = map {
                if input.is_subset(&keys) {
                    for i in input {
                        keys.remove(i);
                    }
                    for o in output {
                        keys.insert(o.clone());
                    }
                }
            }
        }

        keys
    }

    fn compute_and_apply_keys(&mut self, time: &TimeVal) -> Result<()> {
        let desired_keys = self.compute_keys();
        let to_release: Vec<KeyCode> = self
            .output_keys
            .difference(&desired_keys)
            .cloned()
            .collect();

        let to_press: Vec<KeyCode> = desired_keys
            .difference(&self.output_keys)
            .cloned()
            .collect();

        if !to_release.is_empty() {
            self.emit_keys(&to_release, time, KeyEventType::Release)?;
        }
        if !to_press.is_empty() {
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

struct DeviceInfo {
    name: String,
    path: PathBuf,
}

impl DeviceInfo {
    pub fn with_path(path: PathBuf) -> Result<Self> {
        let f = std::fs::File::open(&path).context(format!("opening {}", path.display()))?;
        let mut input = Device::new().ok_or_else(|| anyhow!("failed to make new Device"))?;
        input
            .set_fd(f)
            .context(format!("assigning fd for {} to Device", path.display()))?;

        Ok(Self {
            name: input.name().unwrap_or("").to_string(),
            path,
        })
    }

    pub fn with_name(name: &str) -> Result<Self> {
        let devices = Self::obtain_device_list()?;
        for item in devices {
            if item.name == name {
                return Ok(item);
            }
        }
        bail!("No device found with name `{}`", name);
    }

    fn obtain_device_list() -> Result<Vec<DeviceInfo>> {
        let mut devices = vec![];
        for entry in std::fs::read_dir("/dev/input")? {
            let entry = entry?;

            if !entry
                .file_name()
                .to_str()
                .unwrap_or("")
                .starts_with("event")
            {
                continue;
            }
            let path = entry.path();
            if path.is_dir() {
                continue;
            }

            match DeviceInfo::with_path(path) {
                Ok(item) => devices.push(item),
                Err(err) => log::error!("{}", err),
            }
        }

        devices.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(devices)
    }
}

fn list_devices() -> Result<()> {
    let devices = DeviceInfo::obtain_device_list()?;
    for item in &devices {
        println!("Name: {}", item.name);
        println!("Path: {}", item.path.display());
        println!();
    }
    Ok(())
}

fn main() -> Result<()> {
    pretty_env_logger::init();
    let opt = Opt::from_args();

    if opt.list_devices {
        return list_devices();
    }

    let mappings = vec![
        Mapping::DualRole {
            input: KeyCode::KEY_CAPSLOCK,
            hold: vec![KeyCode::KEY_LEFTCTRL],
            tap: vec![KeyCode::KEY_ESC],
        },
        Mapping::Remap {
            input: [KeyCode::KEY_F1].into_iter().cloned().collect(),
            output: [KeyCode::KEY_BACK].into_iter().cloned().collect(),
        },
        Mapping::Remap {
            input: [KeyCode::KEY_F8].into_iter().cloned().collect(),
            output: [KeyCode::KEY_MUTE].into_iter().cloned().collect(),
        },
        Mapping::Remap {
            input: [KeyCode::KEY_F5].into_iter().cloned().collect(),
            output: [KeyCode::KEY_BRIGHTNESSDOWN].into_iter().cloned().collect(),
        },
        Mapping::Remap {
            input: [KeyCode::KEY_F6].into_iter().cloned().collect(),
            output: [KeyCode::KEY_BRIGHTNESSUP].into_iter().cloned().collect(),
        },
    ];

    log::error!("Short delay: release any keys now!");
    std::thread::sleep(Duration::new(2, 0));

    let device_info = DeviceInfo::with_name("AT Translated Set 2 keyboard")?;

    let mut mapper = InputMapper::create_mapper(device_info.path, mappings)?;

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
