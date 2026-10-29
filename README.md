# cameractl

`cameractl` controls a Sony camera over USB. It speaks Sony Camera Control PTP 3, the remote-control protocol the camera exposes in PC Remote (on some bodies, PC Control).

This repository is two things: the `cameractl` command, and the `sony-ptp3` Rust library behind it.

## Requirements

- A Sony camera that supports PTP 3 over USB, set to PC Remote or PC Control
- Rust (stable) and a C linker
- libusb 1.0

On macOS, Image Capture (`ptpcamerad`) takes the camera. `cameractl` stops that process by default so it can open the PTP interface. Pass `--no-release-system-client` to leave Image Capture running.

On Linux, the process needs permission to open the USB device. This `udev` rule matches Sony's vendor id `054c` and avoids running as root:

```
SUBSYSTEM=="usb", ATTR{idVendor}=="054c", MODE="0660", GROUP="plugdev"
```

Reload the rules, replug the camera, and make sure your user is in `plugdev`.

## Install

macOS:

```sh
brew install libusb pkg-config
cargo install --git https://github.com/itsnauman/cameractl
```

Debian and Ubuntu:

```sh
sudo apt install libusb-1.0-dev pkg-config build-essential
cargo install --git https://github.com/itsnauman/cameractl
```

From a clone:

```sh
cargo build --release
./target/release/cameractl --help
```

## Usage

```sh
cameractl device get
cameractl property list --writable
cameractl property ensure --name iso-sensitivity --value 400 --dry-run
cameractl capture create --output shot.jpg
cameractl object list --function-mode content-transfer
cameractl schema property update
```

A terminal prints a table. A pipe or any other stdout prints JSON. `--format` accepts `json`, `table`, `csv`, or `ndjson`. `--json` is `--format json`.

`--dry-run` prints the change and exits 10. Nothing is sent to the camera.

| Code | Meaning |
| --- | --- |
| 0 | Success |
| 1 | General error |
| 2 | Usage error |
| 3 | Not found |
| 4 | Permission denied |
| 5 | Conflict |
| 6 | Transient error; retry |
| 10 | Dry run passed |

Commands: `device`, `property`, `control`, `operation`, `event`, `storage`, `object`, `thumbnail`, `capture`, `zoom`, `focus`, `focus-point`, `touch`, `movie`, `live-view`, `schema`, `help`.

`schema` prints the command tree as JSON. `cameractl schema --all` prints every command. `help` prints the same tree as text.

A write is sent only when the connected camera advertised that operation or property. Anything else returns an error that names the model.

Controls that format media, reset settings, or power the camera off also require `--force`.

Listing files on the memory card usually needs `--function-mode content-transfer`.

## Library

```toml
[dependencies]
sony-ptp3 = { git = "https://github.com/itsnauman/cameractl" }
```

```rust
fn main() -> Result<(), sony_ptp3::Error> {
    let mut camera = sony_ptp3::Camera::try_new()?;
    camera.shoot()?;
    camera.close()?;
    Ok(())
}
```

`catalog` names the operations, properties, controls, and events from the PTP 3 reference. `Camera` refuses a code the connected camera did not advertise.

The crate is not published to crates.io. Depend on it from this repository, as above.

## Development

```sh
cargo test
cargo fmt --check
```

Unit tests include a device-info capture from a ZV-1M2. Other PTP 3 bodies work when they advertise the operation you call.

## License

[MIT](LICENSE)
