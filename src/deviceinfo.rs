use anyhow::*;
use evdev_rs::Device;
use std::cmp::Ordering;
use std::num::ParseIntError;
use std::path::PathBuf;
use thiserror::Error;

const EVENTNAME: &str = "event";

#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub name: String,
    pub path: PathBuf,
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
                .starts_with(EVENTNAME)
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

        devices.sort_by(|a, b| match a.name.cmp(&b.name) {
            // If there are two equal names, sort by event number
            Ordering::Equal => compare_by_event_number(&a.path, &b.path),
            grater_or_less => grater_or_less,
        });
        Ok(devices)
    }
}

pub fn list_devices() -> Result<()> {
    let devices = DeviceInfo::obtain_device_list()?;
    for item in &devices {
        println!("Name: {}", item.name);
        println!("Path: {}", item.path.display());
        println!();
    }
    Ok(())
}

// Compare two PathBuf, which should be `/dev/input/eventX` by number X following after `event` string
// If any errors occures, return Ordering::Equal to save order, which would be without comparing by path
fn compare_by_event_number(left: &PathBuf, right: &PathBuf) -> Ordering {
    let left_number = match path_string_to_event_number(left.to_str().unwrap_or_default()) {
        Ok(order) => order,
        Err(_) => return Ordering::Equal,
    };

    let right_number = match path_string_to_event_number(right.to_str().unwrap_or_default()) {
        Ok(order) => order,
        Err(_) => return Ordering::Equal,
    };

    left_number.cmp(&right_number)
}

#[derive(Error, Debug)]
enum ComaprationError {
    #[error("`{0}` does not have 'event' name")]
    NoEventName(String),
    #[error("can not parse event number `{0}` for path `{1}`")]
    ParseEventNumber(ParseIntError, String),
}

fn path_string_to_event_number(path: &str) -> Result<u32, ComaprationError> {
    let index = match path.rfind(EVENTNAME) {
        Some(index) => index,
        None => return Err(ComaprationError::NoEventName(String::from(path))),
    };
    let string_number: String = path.chars().skip(index + EVENTNAME.len()).collect();

    string_number
        .parse::<u32>()
        .map_err(|error| ComaprationError::ParseEventNumber(error, String::from(path)))
}
