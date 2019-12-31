use crate::mapping::*;
use crate::remapper::*;
use anyhow::*;
use std::time::Duration;
use structopt::StructOpt;

mod deviceinfo;
mod mapping;
mod remapper;

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

fn main() -> Result<()> {
    pretty_env_logger::init();
    let opt = Opt::from_args();

    if opt.list_devices {
        return deviceinfo::list_devices();
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

    let device_info = deviceinfo::DeviceInfo::with_name("AT Translated Set 2 keyboard")?;

    let mut mapper = InputMapper::create_mapper(device_info.path, mappings)?;
    mapper.run_mapper()?;
    Ok(())
}
