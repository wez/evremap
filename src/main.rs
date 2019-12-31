use crate::mapping::*;
use crate::remapper::*;
use anyhow::*;
use std::path::PathBuf;
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

    /// Specify the configuration file to be loaded
    #[structopt(name = "config-file", long)]
    config_file: PathBuf,
}

fn main() -> Result<()> {
    pretty_env_logger::init();
    let opt = Opt::from_args();

    if opt.list_devices {
        return deviceinfo::list_devices();
    }

    let mapping_config = MappingConfig::from_file(&opt.config_file).context(format!(
        "loading --config-file={}",
        opt.config_file.display()
    ))?;

    log::error!("Short delay: release any keys now!");
    std::thread::sleep(Duration::new(2, 0));

    let device_info = deviceinfo::DeviceInfo::with_name(&mapping_config.device_name)?;

    let mut mapper = InputMapper::create_mapper(device_info.path, mapping_config.mappings)?;
    mapper.run_mapper()?;
    Ok(())
}
