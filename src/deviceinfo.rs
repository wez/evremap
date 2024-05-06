use anyhow::{bail, Context, Result};
use evdev_rs::{Device, DeviceWrapper};
use std::cmp::Ordering;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub name: String,
    pub path: PathBuf,
    pub phys: String,
}

impl DeviceInfo {
    pub fn with_path(path: PathBuf) -> Result<Self> {
        let f = std::fs::File::open(&path).context(format!("opening {}", path.display()))?;
        let input = Device::new_from_file(f)
            .with_context(|| format!("failed to create new Device from file {}", path.display()))?;

        Ok(Self {
            name: input.name().unwrap_or("").to_string(),
            phys: input.phys().unwrap_or("").to_string(),
            path,
        })
    }

    pub fn with_name(name: &str, phys: Option<&str>) -> Result<Self> {
        let mut devices = Self::obtain_device_list()?;

        if let Some(phys) = phys {
            match devices.iter().position(|item| item.phys == phys) {
                Some(idx) => return Ok(devices.remove(idx)),
                None => {
                    bail!(
                        "Requested device `{}` with phys=`{}` was not found",
                        name,
                        phys
                    );
                }
            }
        }

        let mut devices_with_name: Vec<_> = devices
            .into_iter()
            .filter(|item| item.name == name)
            .collect();

        if devices_with_name.is_empty() {
            bail!("No device found with name `{}`", name);
        }

        if devices_with_name.len() > 1 {
            log::warn!("The following devices match name `{}`:", name);
            for dev in &devices_with_name {
                log::warn!("{:?}", dev);
            }
            log::warn!(
                "evremap will use the first entry. If you want to \
                       use one of the others, add the corresponding phys \
                       value to your configuration, for example, \
                       `phys = \"{}\"` for the second entry in the list.",
                devices_with_name[1].phys
            );
        }

        Ok(devices_with_name.remove(0))
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
                Err(err) => log::error!("{:#}", err),
            }
        }

        // Order by name, but when multiple devices have the same name,
        // order by the event device unit number
        devices.sort_by(|a, b| match a.name.cmp(&b.name) {
            Ordering::Equal => {
                event_number_from_path(&a.path).cmp(&event_number_from_path(&b.path))
            }
            different => different,
        });
        Ok(devices)
    }
}

fn event_number_from_path(path: &PathBuf) -> u32 {
    match path.to_str() {
        Some(s) => match s.rfind("event") {
            Some(idx) => s[idx + 5..].parse().unwrap_or(0),
            None => 0,
        },
        None => 0,
    }
}

pub fn list_devices() -> Result<()> {
    let devices = DeviceInfo::obtain_device_list()?;
    for item in &devices {
        println!("Name: {}", item.name);
        println!("Path: {}", item.path.display());
        println!("Phys: {}", item.phys);
        println!();
    }
    Ok(())
}
