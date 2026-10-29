# cameractl

`cameractl` controls a Sony camera over USB. It speaks Sony Camera Control PTP 3, the remote-control protocol the camera exposes in PC Remote (on some bodies, PC Control).

This repository is two things: the `cameractl` command, and the `sony-ptp3` Rust library behind it.

## Motivation

I have a Sony ZV-1M2. I don't have a background in photography, and I can't set the camera up from scratch every time the shot changes. I describe the photo to an AI. Someone by a window, an object on a desk, a picture in low light. The camera is in PC Remote mode on USB, and the AI uses `cameractl` to change whatever needs changing: ISO, aperture, shutter speed, white balance, and the other options that are a pain to remember.

That is what this CLI is for. You describe the scene, and the AI sets the camera. Someone with no photography background should be able to get a setup that fits the shot without working through the menus.

## Requirements

- A Sony camera that supports PTP 3 over USB, set to PC Remote or PC Control
- Rust (stable) and a C linker
- libusb 1.0

On macOS, Image Capture (`ptpcamerad`) takes the camera. `cameractl` stops that process by default so it can open the PTP interface. Pass `--no-release-system-client` to leave Image Capture running.

## Install

macOS:

```sh
brew install libusb pkg-config
cargo install --git https://github.com/itsnauman/cameractl
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

## Library

The library crate is `sony-ptp3`. It is not published to crates.io.

```toml
[dependencies]
sony-ptp3 = { git = "https://github.com/itsnauman/cameractl" }
```

`Camera::try_new` opens the first Sony camera in PTP mode, runs the handshake, and loads the property list. On macOS it stops Image Capture first. The USB timeout is five seconds.

```rust
fn main() -> Result<(), sony_ptp3::Error> {
    let mut camera = sony_ptp3::Camera::try_new()?;
    camera.shoot()?;
    camera.close()?;
    Ok(())
}
```

## Development

```sh
cargo test
cargo fmt --check
```

Unit tests include a device-info capture from a ZV-1M2. Other PTP 3 bodies work when they advertise the operation you call.

## License

[MIT](LICENSE)
