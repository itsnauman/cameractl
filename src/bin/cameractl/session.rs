//! Open the single USB session a command needs.

use sony_ptp3::Camera;

use crate::error::CliError;
use crate::runtime::{Log, Runtime};

pub fn open(runtime: &Runtime, log: &mut Log) -> Result<Camera, CliError> {
    let camera = Camera::try_new_with(runtime.options.clone()).map_err(CliError::from_ptp)?;
    log.note(
        runtime,
        &format!(
            "connected {} {}",
            camera.device_info().model,
            camera.device_info().device_version
        ),
    );
    Ok(camera)
}
