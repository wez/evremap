use crate::deviceinfo::DeviceInfo;
use crate::mapping::*;
use crate::remapper::*;
use anyhow::Error;
use anyhow::{Context, Result};
use clap::Parser;
use std::path::PathBuf;
use std::time::Duration;

mod deviceinfo;
mod mapping;
mod remapper;

/// Remap libinput evdev keyboard inputs
#[derive(Debug, Parser)]
#[command(name = "evremap", about, author = "Wez Furlong")]
enum Opt {
    /// Rather than running the remapper, list currently available devices.
    /// This is helpful to check their names when setting up the initial
    /// configuration
    ListDevices,

    /// Show a list of possible KEY_XXX values
    ListKeys,

    /// Listen to events and print them out to facilitate learning
    /// which keys/buttons have which labels for your device(s)
    DebugEvents {
        /// Specify the device name of interest
        #[arg(long)]
        device_name: Option<String>,

        /// Specify the phys device in case multiple devices have
        /// the same name
        #[arg(long)]
        phys: Option<String>,

        /// Specify the path
        #[arg(long)]
        path: Option<String>,
    },

    /// Load a remapper config and run the remapper.
    /// This usually requires running as root to obtain exclusive access
    /// to the input devices.
    Remap {
        /// Specify the configuration file to be loaded
        #[arg(name = "CONFIG-FILE")]
        config_file: PathBuf,

        /// Number of seconds for user to release keys on startup
        #[arg(short, long, default_value = "2")]
        delay: f64,

        /// Override the device path specified by the config file
        #[arg(long)]
        path: Option<String>,

        /// Override the device name specified by the config file
        #[arg(long)]
        device_name: Option<String>,

        /// Override the phys device specified by the config file
        #[arg(long)]
        phys: Option<String>,

        /// If the device isn't found on startup, wait forever
        /// until the device is plugged in. This works by polling
        /// the set of devices every few seconds. It is not as
        /// efficient as setting up a udev rule to spawn evremap,
        /// but is simpler to setup ad-hoc.
        #[arg(long)]
        wait_for_device: bool,
    },
}

pub fn list_keys() -> Result<()> {
    let mut keys: Vec<String> = EventCode::EV_KEY(KeyCode::KEY_RESERVED)
        .iter()
        .filter_map(|code| match code {
            EventCode::EV_KEY(_) => Some(format!("{}", code)),
            _ => None,
        })
        .collect();
    keys.sort();
    for key in keys {
        println!("{}", key);
    }
    Ok(())
}

fn setup_logger() {
    let mut builder = env_logger::Builder::new();
    builder.filter_level(log::LevelFilter::Info);
    let env = env_logger::Env::new()
        .filter("EVREMAP_LOG")
        .write_style("EVREMAP_LOG_STYLE");
    builder.parse_env(env);
    builder.init();
}

fn get_device(
    path: Option<&str>,
    name: Option<&str>,
    phys: Option<&str>,
    wait_for_device: bool,
) -> anyhow::Result<DeviceInfo> {
    if let Some(path) = path {
        match deviceinfo::DeviceInfo::with_path(path.into()) {
            Ok(dev) => return Ok(dev),
            Err(err) if !wait_for_device => return Err(err),
            Err(err) => {
                log::warn!("{err:#}. Will wait until it is attached.");
            }
        }
    } else if let Some(name) = name {
        match deviceinfo::DeviceInfo::with_name(name, phys) {
            Ok(dev) => return Ok(dev),
            Err(err) if !wait_for_device => return Err(err),
            Err(err) => {
                log::warn!("{err:#}. Will wait until it is attached.");
            }
        }
    } else {
        return Err(Error::msg("device or path is required"));
    }

    const MAX_SLEEP: Duration = Duration::from_secs(10);
    const ONE_SECOND: Duration = Duration::from_secs(1);
    let mut sleep = ONE_SECOND;

    loop {
        std::thread::sleep(sleep);
        sleep = (sleep + ONE_SECOND).min(MAX_SLEEP);
        if let Some(path) = path {
            match deviceinfo::DeviceInfo::with_path(path.into()) {
                Ok(dev) => return Ok(dev),
                Err(err) => {
                    log::debug!("{err:#}");
                }
            }
        } else if let Some(name) = name {
            match deviceinfo::DeviceInfo::with_name(name, phys) {
                Ok(dev) => return Ok(dev),
                Err(err) => {
                    log::debug!("{err:#}");
                }
            }
        }
    }
}

fn debug_events(device: DeviceInfo) -> Result<()> {
    let f =
        std::fs::File::open(&device.path).context(format!("opening {}", device.path.display()))?;
    let input = evdev_rs::Device::new_from_file(f).with_context(|| {
        format!(
            "failed to create new Device from file {}",
            device.path.display()
        )
    })?;

    loop {
        let (status, event) =
            input.next_event(evdev_rs::ReadFlag::NORMAL | evdev_rs::ReadFlag::BLOCKING)?;
        match status {
            evdev_rs::ReadStatus::Success => {
                if let EventCode::EV_KEY(key) = event.event_code {
                    log::info!("{key:?} {}", event.value);
                }
            }
            evdev_rs::ReadStatus::Sync => anyhow::bail!("ReadStatus::Sync!"),
        }
    }
}

fn main() -> Result<()> {
    setup_logger();
    let opt = Opt::parse();

    match opt {
        Opt::ListDevices => deviceinfo::list_devices(),
        Opt::ListKeys => list_keys(),
        Opt::DebugEvents {
            path,
            device_name,
            phys,
        } => {
            let device_info = get_device(
                path.as_deref(),
                device_name.as_deref(),
                phys.as_deref(),
                false,
            )?;
            debug_events(device_info)
        }
        Opt::Remap {
            path,
            config_file,
            delay,
            device_name,
            phys,
            wait_for_device,
        } => {
            let mut mapping_config = MappingConfig::from_file(&config_file).context(format!(
                "loading MappingConfig from {}",
                config_file.display()
            ))?;

            if let Some(device) = device_name {
                mapping_config.device_name = Some(device);
            }
            if let Some(phys) = phys {
                mapping_config.phys = Some(phys);
            }
            if let Some(path) = path {
                mapping_config.path = Some(path);
            }

            log::warn!("Short delay: release any keys now!");
            std::thread::sleep(Duration::from_secs_f64(delay));

            let device_info = get_device(
                mapping_config.path.as_deref(),
                mapping_config.device_name.as_deref(),
                mapping_config.phys.as_deref(),
                wait_for_device,
            )?;

            let mut mapper = InputMapper::create_mapper(device_info.path, mapping_config.mappings)?;
            mapper.run_mapper()
        }
    }
}
