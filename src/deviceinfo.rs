use anyhow::*;
use evdev_rs::Device;
use std::path::PathBuf;

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

pub fn list_devices() -> Result<()> {
    let devices = DeviceInfo::obtain_device_list()?;
    for item in &devices {
        println!("Name: {}", item.name);
        println!("Path: {}", item.path.display());
        println!();
    }
    Ok(())
}
