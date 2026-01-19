# evremap

*A keyboard input remapper for Linux/Wayland systems, written by <a href="https://github.com/wez/">@wez</a>*

## Why?

I couldn't find a good solution for the following:

* Remap the `CAPSLOCK` key so that it produces `CTRL` when held, but `ESC` if tapped
* Remap N keys to M keys.  Eg: `F3` -> `CTRL+c`, and `ALT+LEFT` to `HOME`

## How?

`evremap` works by grabbing exclusive access to an input device and maintaining
a model of the keys that are pressed.  It then applies your remapping configuration
to produce the effective set of pressed keys and emits appropriate changes to a virtual
output device.

Because `evremap` targets the evdev layer of libinput, its remapping
is effective system-wide: in Wayland, X11 and the linux console.

## Configuration

Here's an example configuration that makes capslock useful:

```toml
# The name of the device to remap.
# Run `sudo evremap list-devices` to see the devices available
# on your system.
device_name = "AT Translated Set 2 keyboard"

# If you have multiple devices with the same name, you can optionally
# specify the `phys` value that is printed by the `list-devices` subcommand
# phys = "usb-0000:07:00.3-2.1.1/input0"

# If you specify path, device_name and phys are ignored
# path = "/dev/input/by-id/usb-SINO_WEALTH_Gaming_KB-event-kbd"

# Configure CAPSLOCK as a Dual Role key.
# Holding it produces LEFTCTRL, but tapping it
# will produce ESC.
# Both `tap` and `hold` can expand to multiple output keys.
[[dual_role]]
input = "KEY_CAPSLOCK"
hold = ["KEY_LEFTCTRL"]
tap = ["KEY_ESC"]
```

You can also express simple remapping entries:

```toml
# This config snippet is useful if your keyboard has an arrow
# cluster, but doesn't have page up, page down, home or end
# keys.  Here we're configuring ALT+arrow to map to those functions.
[[remap]]
input = ["KEY_LEFTALT", "KEY_UP"]
output = ["KEY_PAGEUP"]

[[remap]]
input = ["KEY_LEFTALT", "KEY_DOWN"]
output = ["KEY_PAGEDOWN"]

[[remap]]
input = ["KEY_LEFTALT", "KEY_LEFT"]
output = ["KEY_HOME"]

[[remap]]
input = ["KEY_LEFTALT", "KEY_RIGHT"]
output = ["KEY_END"]
```

When applying remapping configuration, ordering is important:

* Dual Role entries are always processed first
* Remap entries are applied in the order that they appear in
  your configuration file

Here's an example where ordering is important: on the PixelBook Go keyboard,
the function key row has alternate functions on the keycaps.  It is natural
to want the mute button to mute by default, but to emit the F8 key when
holding alt.  We can express that with the following configuration:

```toml
[[remap]]
input = ["KEY_LEFTALT", "KEY_F8"]
# When our `input` is matched, our list of `output` is prevented from
# matching as the `input` of subsequent rules.
output = ["KEY_F8"]

[[remap]]
input = ["KEY_F8"]
output = ["KEY_MUTE"]
```

* How do I list available input devices?
  `sudo evremap list-devices`

* How do I list available key codes?
  `evremap list-keys`

* Is there a GUI for editing the config file?
  Yes, take a look at [Evremap-GUI](https://github.com/M8850/Evremap-GUI)

## Building it

```console
$ sudo dnf install libevdev-devel # redhat/centos
## or
$ sudo apt install libevdev-dev pkg-config # debian/ubuntu

$ cargo build --release
```

## Running it

To run the remapper, invoke it *as root* (so that it can grab exclusive access to the input device):

```console
$ sudo target/release/evremap remap my-config-file.toml
```

Or, grant an unprivileged user access to `evdev` and `uinput`.
On Ubuntu, this can be configured by running the following commands and rebooting:

```
sudo gpasswd -a YOUR_USER input
echo 'KERNEL=="uinput", GROUP="input"' | sudo tee /etc/udev/rules.d/input.rules
```

For some platforms, you might need to create an `input` group first and run:
```
echo 'KERNEL=="event*", NAME="input/%k", MODE="660", GROUP="input"' | sudo tee /etc/udev/rules.d/input.rules
```
as well.

## Systemd

A sample system service unit is included in the repo.  You'll want to adjust the paths to match
your system and then install and enable it:

```console
$ sudo cp evremap.service /usr/lib/systemd/system/
$ sudo systemctl daemon-reload
$ sudo systemctl enable evremap.service
$ sudo systemctl start evremap.service
```

## Runit

If you're using Runit instead of Systemd, follow these steps to create a service.

* Create a directory called `evremap` and create a file called `run` under it
```console
sudo mkdir /etc/sv/evremap
sudo touch /etc/sv/evremap/run
```

* Copy these lines into the run file
```console
#!/bin/sh
set -e 
exec <PATH_TO_EVREMAP> remap <CONFIG>
```

Replace `<PATH_TO_EVREMAP>` with the path to your evremap executable and `<CONFIG>` with the path to your configuration file.

* Finally, symlink the evremap directory to `/var/service`
```console
sudo ln -s /etc/sv/evremap /var/service
```

## OpenRC

To make an OpenRC service, create the file `/etc/init.d/evremap` with the following contents...

```console
#!/usr/bin/openrc-run

supervisor=supervise-daemon
command="<PATH_TO_EVREMAP>"
command_args="remap <CONFIG>"
```

Replace `<PATH_TO_EVREMAP>` with the path to your evremap executable and `<CONFIG>` with the path to your configuration file.

Make the file executable...

```console
chmod +x /etc/init.d/evremap
```

Enable the service with...

```console
rc-update add evremap
```

Start the service with...

```console
rc-service evremap start
```

## How do I make this execute a command when a key is pressed?

That feature is not implemented.
