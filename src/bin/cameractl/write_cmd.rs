//! Commands that change the camera. Each one accepts `--dry-run`.

use std::thread;
use std::time::Duration;

use sony_ptp3::catalog::code::{control, property};
use sony_ptp3::catalog::ControlCode;
use sony_ptp3::catalog::PropertyCode;
use sony_ptp3::{ButtonPosition, Camera, DeviceProperty, PropValue};

use crate::cli::{
    ButtonArg, CaptureCmd, ControlCmd, FocusCmd, FocusPointCmd, MovieArg, MovieCmd, PropertyCmd,
    SideEffect, TouchCmd, ZoomCmd,
};
use crate::error::{CliError, Context, EXIT_DRY_RUN, EXIT_OK};
use crate::idempotency::{self, Begin};
use crate::names::{self, Resolved};
use crate::parse::code16;
use crate::present::{self, display_value};
use crate::read_cmd::{ensure_control, find_property};
use crate::render::Document;
use crate::runtime::{Log, Runtime};
use crate::session;
use crate::values::{self, parse_property_value};

const CAPTURE: &str = "capture create";
const PROPERTY_UPDATE: &str = "property update";
const PROPERTY_ENSURE: &str = "property ensure";
const CONTROL_UPDATE: &str = "control update";

#[derive(Clone, Copy)]
pub enum WriteMode {
    Update,
    Ensure,
}

#[derive(Debug)]
pub struct Assessment {
    pub writes: bool,
    pub changed: bool,
    pub desired: PropValue,
    pub changes: Vec<serde_json::Value>,
}

pub fn property(
    cmd: PropertyCmd,
    runtime: &Runtime,
    log: &mut Log,
) -> Result<(i32, Document), CliError> {
    match cmd {
        PropertyCmd::Update {
            selector,
            value,
            if_match,
            side,
        } => write_property(
            PROPERTY_UPDATE,
            "update",
            WriteMode::Update,
            &selector,
            &value,
            &if_match,
            &side,
            runtime,
            log,
        ),
        PropertyCmd::Ensure {
            selector,
            value,
            if_match,
            side,
        } => write_property(
            PROPERTY_ENSURE,
            "ensure",
            WriteMode::Ensure,
            &selector,
            &value,
            &if_match,
            &side,
            runtime,
            log,
        ),
        PropertyCmd::List { .. } | PropertyCmd::Get { .. } => {
            unreachable!("reads are dispatched separately")
        }
    }
}

pub fn control(
    cmd: ControlCmd,
    runtime: &Runtime,
    log: &mut Log,
) -> Result<(i32, Document), CliError> {
    let ControlCmd::Update {
        selector,
        value,
        position,
        force,
        side,
    } = cmd
    else {
        unreachable!("reads are dispatched separately");
    };
    let query = selector.query("control")?;
    let resolved = names::resolve("control", query, names::controls())?;
    let prepared = prepare_control(&resolved, value.as_deref(), position)?;
    if names::control_is_destructive(prepared.code()) && !side.dry_run && !force {
        return Err(CliError::usage(
            format!(
                "control {} formats media, resets the camera, or powers it off",
                control_label(prepared.code())
            ),
            format!(
                "Preview with --dry-run, then pass --force. --yes does not authorize this control. Example: cameractl control update --code {} --dry-run",
                code16(prepared.code())
            ),
        )
        .input(code16(prepared.code())));
    }
    let replay_pending =
        match idempotency::begin(&side.idempotency_key, CONTROL_UPDATE, side.dry_run)? {
            Begin::Replay(body) => return Ok((EXIT_OK, Document::One(body))),
            Begin::Run { replay_pending } => replay_pending,
        };
    let mut camera = session::open(runtime, log)?;
    ensure_control(&camera, prepared.code())?;
    let mut body = present::mutation(
        "update",
        "control",
        side.dry_run,
        true,
        true,
        !names::control_is_destructive(prepared.code()),
        side.idempotency_key.as_deref(),
        replay_pending,
        control_target(prepared.code(), &prepared),
        vec![prepared.change()],
    );
    body["requires_force"] = serde_json::json!(names::control_is_destructive(prepared.code()));
    if side.dry_run {
        return Ok(done(true, body));
    }
    prepared.send(&mut camera)?;
    let body = idempotency::finish(
        &side.idempotency_key,
        CONTROL_UPDATE,
        false,
        body,
        log,
        runtime,
    );
    Ok(done(false, body))
}

pub fn capture(
    cmd: CaptureCmd,
    runtime: &Runtime,
    log: &mut Log,
) -> Result<(i32, Document), CliError> {
    let CaptureCmd::Create { output, side } = cmd;
    let replay_pending = match idempotency::begin(&side.idempotency_key, CAPTURE, side.dry_run)? {
        Begin::Replay(body) => return Ok((EXIT_OK, Document::One(body))),
        Begin::Run { replay_pending } => replay_pending,
    };
    if let Some(path) = &output {
        crate::paths::validate(path)?;
    }
    let mut camera = session::open(runtime, log)?;
    ensure_control(&camera, control::SHUTTER_HALF_RELEASE_S1_BUTTON.0)?;
    ensure_control(&camera, control::SHUTTER_RELEASE_S2_BUTTON.0)?;
    let mut body = present::mutation(
        "create",
        "capture",
        side.dry_run,
        true,
        true,
        false,
        side.idempotency_key.as_deref(),
        replay_pending,
        serde_json::json!({
            "sequence": "half-press, full-press, release full, release half",
        }),
        vec![present::change(
            "shutter",
            serde_json::json!("idle"),
            None,
            serde_json::json!("released"),
            None,
        )],
    );
    body["output"] = serde_json::Value::Null;
    body["image_bytes"] = serde_json::Value::Null;
    body["image_error"] = serde_json::Value::Null;
    if side.dry_run {
        if let Some(path) = &output {
            let validated = crate::paths::validate(path)?;
            body["output"] = serde_json::json!(validated.display().to_string());
        }
        return Ok(done(true, body));
    }
    camera
        .shoot()
        .map_err(|err| CliError::from_ptp_in(err, Context::Control))?;
    if let Some(path) = output {
        match camera.captured_image() {
            Ok(bytes) => match crate::paths::write_bytes(&path, &bytes) {
                Ok(written) => {
                    body["output"] = serde_json::json!(written.display().to_string());
                    body["image_bytes"] = serde_json::json!(bytes.len());
                }
                Err(error) => {
                    let message = format!(
                        "shutter fired but the image was not saved: {}. Run: cameractl object get --handle captured --output shot.jpg",
                        error.message
                    );
                    log.warn(runtime, &message);
                    body["image_error"] = serde_json::json!(message);
                }
            },
            Err(error) => {
                let message = format!(
                    "shutter fired but the image was not saved: {}. Run: cameractl object get --handle captured --output shot.jpg",
                    error
                );
                log.warn(runtime, &message);
                body["image_error"] = serde_json::json!(message);
            }
        }
    }
    let body = idempotency::finish(&side.idempotency_key, CAPTURE, false, body, log, runtime);
    Ok(done(false, body))
}

pub fn zoom(cmd: ZoomCmd, runtime: &Runtime, log: &mut Log) -> Result<(i32, Document), CliError> {
    let ZoomCmd::Update { speed, side } = cmd;
    speed_command(
        "zoom update",
        "zoom",
        control::ZOOM_OPERATION.0,
        speed.into(),
        &side,
        runtime,
        log,
        |camera| camera.zoom(speed),
    )
}

pub fn focus(cmd: FocusCmd, runtime: &Runtime, log: &mut Log) -> Result<(i32, Document), CliError> {
    let FocusCmd::Update { speed, side } = cmd;
    speed_command(
        "focus update",
        "focus",
        control::NEAR_FAR.0,
        i64::from(speed),
        &side,
        runtime,
        log,
        |camera| camera.focus_near_far(speed),
    )
}

pub fn focus_point(
    cmd: FocusPointCmd,
    runtime: &Runtime,
    log: &mut Log,
) -> Result<(i32, Document), CliError> {
    let FocusPointCmd::Update { x, y, side } = cmd;
    axis("x", x, 639)?;
    axis("y", y, 479)?;
    point_command(
        "focus-point update",
        "focus-point",
        control::AF_AREA_POSITION_X_Y.0,
        x,
        y,
        &side,
        runtime,
        log,
        |camera| camera.set_af_position(x, y),
    )
}

pub fn touch(cmd: TouchCmd, runtime: &Runtime, log: &mut Log) -> Result<(i32, Document), CliError> {
    let TouchCmd::Update { x, y, side } = cmd;
    axis("x", x, 639)?;
    axis("y", y, 479)?;
    point_command(
        "touch update",
        "touch",
        control::REMOTE_TOUCH_OPERATION_X_Y.0,
        x,
        y,
        &side,
        runtime,
        log,
        |camera| camera.remote_touch(x, y),
    )
}

pub fn movie(cmd: MovieCmd, runtime: &Runtime, log: &mut Log) -> Result<(i32, Document), CliError> {
    match cmd {
        MovieCmd::Ensure { state, side } => ensure_movie(state, &side, runtime, log),
        MovieCmd::Update { position, side } => movie_button(position, &side, runtime, log),
    }
}

pub fn assess_write(
    property: &DeviceProperty,
    desired: PropValue,
    mode: WriteMode,
    if_match: Option<&PropValue>,
) -> Result<Assessment, CliError> {
    if let Some(expected) = if_match {
        if expected != &property.current {
            return Err(CliError::conflict(
                format!(
                    "current value is {} and --if-match does not match",
                    display_value(property.code, &property.current)
                ),
                format!(
                    "Run: cameractl property get --code {}",
                    code16(property.code)
                ),
            )
            .input(display_value(property.code, expected)));
        }
    }
    let changed = desired != property.current;
    let writes = match mode {
        WriteMode::Update => true,
        WriteMode::Ensure => changed,
    };
    if writes && !property.is_writable() {
        return Err(CliError::permission(
            format!(
                "{} is not writable",
                names::find(names::properties(), property.code)
                    .map(|entry| entry.name)
                    .unwrap_or("property")
            ),
            format!(
                "Run: cameractl property get --code {}",
                code16(property.code)
            ),
        ));
    }
    let changes = if writes {
        vec![present::value_change(
            "current",
            property.code,
            &property.current,
            &desired,
        )]
    } else {
        Vec::new()
    };
    Ok(Assessment {
        writes,
        changed,
        desired,
        changes,
    })
}

#[derive(Debug, PartialEq, Eq)]
pub enum MoviePlan {
    Hold { writes: bool },
    Failed,
    Unknown,
}

pub fn plan_movie(observed: i128, desired: MovieArg) -> MoviePlan {
    match (desired, observed) {
        (MovieArg::Recording, 1 | 3) => MoviePlan::Hold { writes: false },
        (MovieArg::Recording, 0) => MoviePlan::Hold { writes: true },
        (MovieArg::Idle, 0) => MoviePlan::Hold { writes: false },
        (MovieArg::Idle, 1) => MoviePlan::Hold { writes: true },
        (MovieArg::Idle, 3) => MoviePlan::Hold { writes: false },
        (_, 2) => MoviePlan::Failed,
        _ => MoviePlan::Unknown,
    }
}

#[allow(clippy::too_many_arguments)]
fn write_property(
    command: &str,
    action: &str,
    mode: WriteMode,
    selector: &crate::cli::Selector,
    raw_value: &str,
    raw_match: &Option<String>,
    side: &SideEffect,
    runtime: &Runtime,
    log: &mut Log,
) -> Result<(i32, Document), CliError> {
    let query = selector.query("property")?;
    let resolved = names::resolve("property", query, names::properties())?;
    let replay_pending = match idempotency::begin(&side.idempotency_key, command, side.dry_run)? {
        Begin::Replay(body) => return Ok((EXIT_OK, Document::One(body))),
        Begin::Run { replay_pending } => replay_pending,
    };
    let mut camera = session::open(runtime, log)?;
    let snapshot = find_property(&camera, &resolved)?.clone();
    let desired = parse_property_value(&snapshot, raw_value)?;
    let expected = match raw_match {
        Some(raw) => Some(parse_property_value(&snapshot, raw)?),
        None => None,
    };
    let assessment = assess_write(&snapshot, desired, mode, expected.as_ref())?;
    let mut body = present::mutation(
        action,
        "property",
        side.dry_run,
        assessment.writes,
        assessment.changed,
        true,
        side.idempotency_key.as_deref(),
        replay_pending,
        present::identity(names::properties(), snapshot.code),
        assessment.changes,
    );
    let mut verified = !assessment.writes;
    let mut current = snapshot.current.clone();
    if assessment.writes && !side.dry_run {
        camera
            .set_property(PropertyCode(snapshot.code), &assessment.desired)
            .map_err(|err| CliError::from_ptp_in(err, Context::Property).input(raw_value))?;
        camera.refresh_properties().map_err(|err| {
            let mut cli = CliError::from_ptp_in(err, Context::Property);
            cli.retryable = true;
            cli.message = format!(
                "the write was sent but reading it back failed: {}",
                cli.message
            );
            cli.suggestion = format!(
                "Run: cameractl property get --code {}",
                code16(snapshot.code)
            );
            cli
        })?;
        current = camera
            .property(PropertyCode(snapshot.code))
            .map(|property| property.current.clone())
            .ok_or_else(|| {
                CliError::not_found(
                    "the property disappeared after the write",
                    format!(
                        "Run: cameractl property get --code {}",
                        code16(snapshot.code)
                    ),
                )
            })?;
        if current != assessment.desired {
            return Err(CliError::general(
                format!(
                    "camera accepted the write but read back {}",
                    display_value(snapshot.code, &current)
                ),
                format!(
                    "Run: cameractl property get --code {}",
                    code16(snapshot.code)
                ),
            )
            .input(raw_value));
        }
        verified = true;
    }
    body["current"] = present::prop_json(&current);
    body["current_label"] = serde_json::json!(present::label_of(snapshot.code, &current));
    body["verified"] = serde_json::json!(verified && !side.dry_run);
    let body = idempotency::finish(
        &side.idempotency_key,
        command,
        side.dry_run,
        body,
        log,
        runtime,
    );
    Ok(done(side.dry_run, body))
}

fn ensure_movie(
    desired: MovieArg,
    side: &SideEffect,
    runtime: &Runtime,
    log: &mut Log,
) -> Result<(i32, Document), CliError> {
    let replay_pending =
        match idempotency::begin(&side.idempotency_key, "movie ensure", side.dry_run)? {
            Begin::Replay(body) => return Ok((EXIT_OK, Document::One(body))),
            Begin::Run { replay_pending } => replay_pending,
        };
    let mut camera = session::open(runtime, log)?;
    ensure_control(&camera, control::MOVIE_REC_BUTTON_HOLD.0)?;
    let observed = movie_state(&camera)?;
    let writes = match plan_movie(observed, desired) {
        MoviePlan::Hold { writes } => writes,
        MoviePlan::Failed => {
            return Err(CliError::general(
                "movie recording failed on the camera",
                "Run: cameractl property get --name movie-recording-state",
            ))
        }
        MoviePlan::Unknown => {
            return Err(CliError::general(
                format!("movie-recording-state {observed} is not a known recorder state"),
                "Run: cameractl property get --name movie-recording-state",
            ))
        }
    };
    let changed = match desired {
        MovieArg::Recording => observed != 1,
        MovieArg::Idle => observed != 0,
    };
    let changes = if changed {
        vec![present::change(
            "state",
            serde_json::json!(movie_word(observed)),
            state_label(observed),
            serde_json::json!(desired_word(desired)),
            state_label(match desired {
                MovieArg::Recording => 1,
                MovieArg::Idle => 0,
            }),
        )]
    } else {
        Vec::new()
    };
    let mut body = present::mutation(
        "ensure",
        "movie",
        side.dry_run,
        writes,
        changed,
        true,
        side.idempotency_key.as_deref(),
        replay_pending,
        {
            let mut target = present::identity(names::controls(), control::MOVIE_REC_BUTTON_HOLD.0);
            target["state"] = serde_json::json!(desired_word(desired));
            target
        },
        changes,
    );
    body["observed"] = serde_json::json!(movie_word(observed));
    body["state"] = serde_json::json!(desired_word(desired));
    if side.dry_run {
        return Ok(done(true, body));
    }
    if writes {
        toggle_movie(&mut camera)?;
    }
    let after = if changed {
        poll_movie(&mut camera, desired, writes)?
    } else {
        observed
    };
    body["state"] = serde_json::json!(movie_word(after));
    let body = idempotency::finish(
        &side.idempotency_key,
        "movie ensure",
        false,
        body,
        log,
        runtime,
    );
    Ok(done(false, body))
}

fn movie_button(
    position: ButtonArg,
    side: &SideEffect,
    runtime: &Runtime,
    log: &mut Log,
) -> Result<(i32, Document), CliError> {
    let word = match position {
        ButtonArg::Up => "up",
        ButtonArg::Down => "down",
    };
    let replay_pending =
        match idempotency::begin(&side.idempotency_key, "movie update", side.dry_run)? {
            Begin::Replay(body) => return Ok((EXIT_OK, Document::One(body))),
            Begin::Run { replay_pending } => replay_pending,
        };
    let mut camera = session::open(runtime, log)?;
    ensure_control(&camera, control::MOVIE_REC_BUTTON_HOLD.0)?;
    let body = present::mutation(
        "update",
        "movie",
        side.dry_run,
        true,
        true,
        true,
        side.idempotency_key.as_deref(),
        replay_pending,
        present::identity(names::controls(), control::MOVIE_REC_BUTTON_HOLD.0),
        vec![present::change(
            "position",
            serde_json::Value::Null,
            None,
            serde_json::json!(word),
            None,
        )],
    );
    if side.dry_run {
        return Ok(done(true, body));
    }
    camera
        .hold_movie_button(button_position(position))
        .map_err(|err| CliError::from_ptp_in(err, Context::Control))?;
    let body = idempotency::finish(
        &side.idempotency_key,
        "movie update",
        false,
        body,
        log,
        runtime,
    );
    Ok(done(false, body))
}

#[allow(clippy::too_many_arguments)]
fn speed_command(
    command: &str,
    resource: &str,
    code: u16,
    speed: i64,
    side: &SideEffect,
    runtime: &Runtime,
    log: &mut Log,
    send: impl FnOnce(&mut Camera) -> Result<(), sony_ptp3::Error>,
) -> Result<(i32, Document), CliError> {
    let replay_pending = match idempotency::begin(&side.idempotency_key, command, side.dry_run)? {
        Begin::Replay(body) => return Ok((EXIT_OK, Document::One(body))),
        Begin::Run { replay_pending } => replay_pending,
    };
    let mut camera = session::open(runtime, log)?;
    ensure_control(&camera, code)?;
    let mut target = present::identity(names::controls(), code);
    target["speed"] = serde_json::json!(speed);
    let body = present::mutation(
        "update",
        resource,
        side.dry_run,
        true,
        true,
        true,
        side.idempotency_key.as_deref(),
        replay_pending,
        target,
        vec![present::change(
            "speed",
            serde_json::Value::Null,
            None,
            serde_json::json!(speed),
            None,
        )],
    );
    if side.dry_run {
        return Ok(done(true, body));
    }
    send(&mut camera).map_err(|err| CliError::from_ptp_in(err, Context::Control))?;
    let body = idempotency::finish(&side.idempotency_key, command, false, body, log, runtime);
    Ok(done(false, body))
}

#[allow(clippy::too_many_arguments)]
fn point_command(
    command: &str,
    resource: &str,
    code: u16,
    x: u16,
    y: u16,
    side: &SideEffect,
    runtime: &Runtime,
    log: &mut Log,
    send: impl FnOnce(&mut Camera) -> Result<(), sony_ptp3::Error>,
) -> Result<(i32, Document), CliError> {
    let replay_pending = match idempotency::begin(&side.idempotency_key, command, side.dry_run)? {
        Begin::Replay(body) => return Ok((EXIT_OK, Document::One(body))),
        Begin::Run { replay_pending } => replay_pending,
    };
    let mut camera = session::open(runtime, log)?;
    ensure_control(&camera, code)?;
    let mut target = present::identity(names::controls(), code);
    target["x"] = serde_json::json!(x);
    target["y"] = serde_json::json!(y);
    let body = present::mutation(
        "update",
        resource,
        side.dry_run,
        true,
        true,
        true,
        side.idempotency_key.as_deref(),
        replay_pending,
        target,
        vec![
            present::change(
                "x",
                serde_json::Value::Null,
                None,
                serde_json::json!(x),
                None,
            ),
            present::change(
                "y",
                serde_json::Value::Null,
                None,
                serde_json::json!(y),
                None,
            ),
        ],
    );
    if side.dry_run {
        return Ok(done(true, body));
    }
    send(&mut camera).map_err(|err| CliError::from_ptp_in(err, Context::Control))?;
    let body = idempotency::finish(&side.idempotency_key, command, false, body, log, runtime);
    Ok(done(false, body))
}

enum Prepared {
    Button { code: u16, position: ButtonPosition },
    Value { code: u16, value: PropValue },
}

impl Prepared {
    fn code(&self) -> u16 {
        match self {
            Self::Button { code, .. } | Self::Value { code, .. } => *code,
        }
    }

    fn change(&self) -> serde_json::Value {
        match self {
            Self::Button { position, .. } => present::change(
                "position",
                serde_json::Value::Null,
                None,
                serde_json::json!(match position {
                    ButtonPosition::Up => "up",
                    ButtonPosition::Down => "down",
                }),
                None,
            ),
            Self::Value { code, value } => present::change(
                "value",
                serde_json::Value::Null,
                None,
                present::prop_json(value),
                present::label_of(*code, value),
            ),
        }
    }

    fn send(self, camera: &mut Camera) -> Result<(), CliError> {
        match self {
            Self::Button { code, position } => camera
                .button(ControlCode(code), position)
                .map_err(|err| CliError::from_ptp_in(err, Context::Control)),
            Self::Value { code, value } => camera
                .control(ControlCode(code), &value)
                .map_err(|err| CliError::from_ptp_in(err, Context::Control)),
        }
    }
}

fn prepare_control(
    resolved: &Resolved<'_>,
    value: Option<&str>,
    position: Option<ButtonArg>,
) -> Result<Prepared, CliError> {
    let code = resolved.code();
    let datatype = match resolved {
        Resolved::Known(entry) => entry.datatype,
        Resolved::UnknownCode(_) => 0,
    };
    match (value, position) {
        (Some(_), Some(_)) => Err(CliError::usage(
            "pass only one of --value and --position",
            "Use --position up|down for a button, or --value for a numeric control.",
        )),
        (None, None) => Err(CliError::usage(
            "pass --value or --position",
            "Example: cameractl control update --name movie-rec-button-hold --position down --dry-run",
        )),
        (None, Some(position)) => {
            if datatype != 0 && datatype != sony_ptp3::datatype::UINT16 {
                return Err(CliError::usage(
                    format!(
                        "this control is {} and does not take up or down",
                        values::datatype_name(datatype)
                    ),
                    "Pass --value with the number the control accepts.",
                ));
            }
            Ok(Prepared::Button {
                code,
                position: button_position(position),
            })
        }
        (Some(raw), None) => {
            if datatype == 0 {
                return Err(CliError::usage(
                    "this control has no documented datatype",
                    "Pass --position up or --position down, or choose a control from cameractl control list.",
                )
                .input(raw));
            }
            Ok(Prepared::Value {
                code,
                value: values::parse_scalar(datatype, raw)?,
            })
        }
    }
}

fn control_target(code: u16, prepared: &Prepared) -> serde_json::Value {
    let mut target = present::identity(names::controls(), code);
    match prepared {
        Prepared::Button { position, .. } => {
            target["position"] = serde_json::json!(match position {
                ButtonPosition::Up => "up",
                ButtonPosition::Down => "down",
            });
        }
        Prepared::Value { value, .. } => {
            target["value"] = present::prop_json(value);
        }
    }
    target
}

fn control_label(code: u16) -> String {
    names::find(names::controls(), code)
        .map(|entry| entry.slug.clone())
        .unwrap_or_else(|| code16(code))
}

fn button_position(position: ButtonArg) -> ButtonPosition {
    match position {
        ButtonArg::Up => ButtonPosition::Up,
        ButtonArg::Down => ButtonPosition::Down,
    }
}

fn axis(name: &str, value: u16, max: u16) -> Result<(), CliError> {
    if value > max {
        return Err(CliError::usage(
            format!("{name} {value} is outside 0..={max}"),
            format!("Pass --{name} from 0 to {max}."),
        )
        .input(value.to_string()));
    }
    Ok(())
}

fn movie_state(camera: &Camera) -> Result<i128, CliError> {
    let property = camera.property(property::MOVIE_RECORDING_STATE).ok_or_else(|| {
        CliError::not_found(
            "movie-recording-state is not on this camera, so ensure cannot see the recorder",
            "Hold the button directly. Run: cameractl control update --name movie-rec-button-hold --position down --dry-run",
        )
    })?;
    values::as_i128(&property.current).ok_or_else(|| {
        CliError::general(
            "movie-recording-state is not a number",
            "Run: cameractl property get --name movie-recording-state",
        )
    })
}

fn toggle_movie(camera: &mut Camera) -> Result<(), CliError> {
    camera
        .hold_movie_button(ButtonPosition::Down)
        .map_err(|err| CliError::from_ptp_in(err, Context::Control))?;
    thread::sleep(Duration::from_millis(200));
    camera.hold_movie_button(ButtonPosition::Up).map_err(|err| {
        let mut cli = CliError::from_ptp_in(err, Context::Control);
        cli.retryable = false;
        cli.suggestion = "The record button may still be held. Run: cameractl control update --name movie-rec-button-hold --position up".to_owned();
        cli
    })
}

fn poll_movie(camera: &mut Camera, desired: MovieArg, pressed: bool) -> Result<i128, CliError> {
    let mut last = 0;
    for _ in 0..20 {
        camera
            .refresh_properties()
            .map_err(|err| CliError::from_ptp_in(err, Context::Property))?;
        last = movie_state(camera)?;
        let done = match desired {
            MovieArg::Recording => last == 1,
            MovieArg::Idle => last == 0,
        };
        if done {
            return Ok(last);
        }
        if last == 2 {
            return Err(CliError::general(
                "movie recording failed while waiting",
                "Run: cameractl property get --name movie-recording-state",
            ));
        }
        thread::sleep(Duration::from_millis(200));
    }
    let state = movie_word(last);
    if pressed {
        Err(CliError::general(
            format!(
                "the movie button was pressed but the camera is still {state}. Read the state before trying again"
            ),
            "Run: cameractl property get --name movie-recording-state",
        ))
    } else {
        Err(CliError::transient(
            format!("movie state is still {state}"),
            format!(
                "Retry: cameractl movie ensure --state {}",
                desired_word(desired)
            ),
        ))
    }
}

fn desired_word(state: MovieArg) -> &'static str {
    match state {
        MovieArg::Recording => "recording",
        MovieArg::Idle => "idle",
    }
}

fn movie_word(state: i128) -> &'static str {
    match state {
        0 => "idle",
        1 => "recording",
        2 => "failed",
        3 => "waiting",
        _ => "unknown",
    }
}

fn state_label(state: i128) -> Option<&'static str> {
    sony_ptp3::catalog::value_label(property::MOVIE_RECORDING_STATE, state)
}

fn done(dry_run: bool, body: serde_json::Value) -> (i32, Document) {
    (
        if dry_run { EXIT_DRY_RUN } else { EXIT_OK },
        Document::One(body),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use sony_ptp3::{EnableState, GetSet, PropForm};

    fn sample(current: PropValue, writable: bool) -> DeviceProperty {
        DeviceProperty {
            code: 0xd21e,
            datatype: sony_ptp3::datatype::UINT32,
            get_set: if writable {
                GetSet::ReadWrite
            } else {
                GetSet::ReadOnly
            },
            enabled: EnableState::Enabled,
            factory: current.clone(),
            current,
            form: PropForm::None,
        }
    }

    #[test]
    fn ensure_skips_a_matching_value_and_update_still_writes() {
        let property = sample(PropValue::U32(400), true);
        let ensured =
            assess_write(&property, PropValue::U32(400), WriteMode::Ensure, None).unwrap();
        assert!(!ensured.writes);
        assert!(!ensured.changed);
        let updated =
            assess_write(&property, PropValue::U32(400), WriteMode::Update, None).unwrap();
        assert!(updated.writes);
        assert!(!updated.changed);
    }

    #[test]
    fn a_stale_if_match_is_a_conflict_and_a_locked_property_is_permission() {
        let property = sample(PropValue::U32(100), false);
        let conflict = assess_write(
            &property,
            PropValue::U32(400),
            WriteMode::Ensure,
            Some(&PropValue::U32(200)),
        )
        .unwrap_err();
        assert_eq!(conflict.exit, 5);
        let denied =
            assess_write(&property, PropValue::U32(400), WriteMode::Ensure, None).unwrap_err();
        assert_eq!(denied.exit, 4);
    }

    #[test]
    fn movie_plan_presses_only_to_enter_or_leave_recording() {
        assert_eq!(
            plan_movie(0, MovieArg::Recording),
            MoviePlan::Hold { writes: true }
        );
        assert_eq!(
            plan_movie(1, MovieArg::Recording),
            MoviePlan::Hold { writes: false }
        );
        assert_eq!(
            plan_movie(1, MovieArg::Idle),
            MoviePlan::Hold { writes: true }
        );
        assert_eq!(plan_movie(2, MovieArg::Idle), MoviePlan::Failed);
        assert_eq!(plan_movie(9, MovieArg::Recording), MoviePlan::Unknown);
    }
}
