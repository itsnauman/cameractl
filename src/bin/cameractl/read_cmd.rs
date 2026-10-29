//! Commands that do not change the camera.

use std::time::Duration;

use sony_ptp3::catalog::{ControlCode, PropertyCode};
use sony_ptp3::object;
use sony_ptp3::Error;

use crate::cli::{
    check_contains, DeviceCmd, EventCmd, LiveViewCmd, ObjectCmd, OperationCmd, PropertyCmd,
    StorageCmd, ThumbnailCmd,
};
use crate::error::{CliError, Context, EXIT_OK};
use crate::names::{self, Resolved};
use crate::page::Page;
use crate::parse::{self, code32};
use crate::present::{self, Column};
use crate::render::{Document, ListOut};
use crate::runtime::{self, Log, Runtime};
use crate::session;

pub fn device(
    cmd: DeviceCmd,
    runtime: &Runtime,
    log: &mut Log,
) -> Result<(i32, Document), CliError> {
    match cmd {
        DeviceCmd::Get => {
            let camera = session::open(runtime, log)?;
            let body = present::device_json(
                &camera,
                runtime::session_mode(runtime.options.function_mode),
            );
            Ok((EXIT_OK, Document::One(body)))
        }
    }
}

pub fn property(
    cmd: PropertyCmd,
    runtime: &Runtime,
    log: &mut Log,
) -> Result<(i32, Document), CliError> {
    match cmd {
        PropertyCmd::List {
            writable,
            name_contains,
            page,
        } => {
            check_contains(&name_contains)?;
            let page = page_of(&page, runtime)?;
            let camera = session::open(runtime, log)?;
            let mut properties = camera.properties().to_vec();
            properties.retain(|property| !writable || property.is_writable());
            properties.retain(|property| {
                let entry = names::find(names::properties(), property.code);
                matches_filter(
                    &name_contains,
                    entry.map(|entry| entry.name),
                    entry.map(|entry| entry.slug.as_str()),
                    &parse::code16(property.code),
                )
            });
            properties.sort_by_key(|property| property.code);
            let items = properties.iter().map(present::property_json).collect();
            Ok((EXIT_OK, paged(items, &page, present::PROPERTY_COLUMNS)))
        }
        PropertyCmd::Get { selector } => {
            let query = selector.query("property")?;
            let resolved = names::resolve("property", query, names::properties())?;
            let camera = session::open(runtime, log)?;
            let property = find_property(&camera, &resolved)?;
            Ok((EXIT_OK, Document::One(present::property_json(property))))
        }
        PropertyCmd::Update { .. } | PropertyCmd::Ensure { .. } => {
            unreachable!("writes are dispatched separately")
        }
    }
}

pub fn control_read(
    cmd: &crate::cli::ControlCmd,
    runtime: &Runtime,
    log: &mut Log,
) -> Result<Option<(i32, Document)>, CliError> {
    match cmd {
        crate::cli::ControlCmd::List {
            name_contains,
            page,
        } => {
            check_contains(name_contains)?;
            let page = page_of(page, runtime)?;
            let camera = session::open(runtime, log)?;
            let mut codes = camera.ext_device_info().controls.clone();
            codes.sort_unstable();
            codes.dedup();
            let items: Vec<_> = codes
                .into_iter()
                .map(present::control_json)
                .filter(|item| {
                    matches_filter(
                        name_contains,
                        item.get("name").and_then(|value| value.as_str()),
                        item.get("slug").and_then(|value| value.as_str()),
                        item.get("code")
                            .and_then(|value| value.as_str())
                            .unwrap_or(""),
                    )
                })
                .collect();
            Ok(Some((
                EXIT_OK,
                paged(items, &page, present::CONTROL_COLUMNS),
            )))
        }
        crate::cli::ControlCmd::Get { selector } => {
            let query = selector.query("control")?;
            let resolved = names::resolve("control", query, names::controls())?;
            let camera = session::open(runtime, log)?;
            ensure_control(&camera, resolved.code())?;
            Ok(Some((
                EXIT_OK,
                Document::One(present::control_json(resolved.code())),
            )))
        }
        crate::cli::ControlCmd::Update { .. } => Ok(None),
    }
}

pub fn operation(
    cmd: OperationCmd,
    runtime: &Runtime,
    log: &mut Log,
) -> Result<(i32, Document), CliError> {
    match cmd {
        OperationCmd::List {
            name_contains,
            page,
        } => {
            check_contains(&name_contains)?;
            let page = page_of(&page, runtime)?;
            let camera = session::open(runtime, log)?;
            Ok((
                EXIT_OK,
                code_list(
                    &camera.device_info().operations,
                    names::operations(),
                    &name_contains,
                    &page,
                ),
            ))
        }
    }
}

pub fn event(cmd: EventCmd, runtime: &Runtime, log: &mut Log) -> Result<(i32, Document), CliError> {
    match cmd {
        EventCmd::List {
            name_contains,
            page,
        } => {
            check_contains(&name_contains)?;
            let page = page_of(&page, runtime)?;
            let camera = session::open(runtime, log)?;
            Ok((
                EXIT_OK,
                code_list(
                    &camera.device_info().events,
                    names::events(),
                    &name_contains,
                    &page,
                ),
            ))
        }
        EventCmd::Get { timeout } => {
            let timeout = parse::duration(&timeout, Duration::ZERO, Duration::from_secs(60))?;
            let mut camera = session::open(runtime, log)?;
            let event = camera
                .poll_event(timeout)
                .map_err(|err| CliError::from_ptp_in(err, Context::Event))?;
            let body = match event {
                None => serde_json::json!({"event": null}),
                Some(event) => serde_json::json!({"event": present::event_json(&event)}),
            };
            Ok((EXIT_OK, Document::One(body)))
        }
    }
}

pub fn storage(
    cmd: StorageCmd,
    runtime: &Runtime,
    log: &mut Log,
) -> Result<(i32, Document), CliError> {
    match cmd {
        StorageCmd::List { page } => {
            let page = page_of(&page, runtime)?;
            let mut camera = session::open(runtime, log)?;
            let ids = camera
                .storage_ids()
                .map_err(|err| CliError::from_ptp_in(err, Context::Media))?;
            let mut items = Vec::with_capacity(ids.len());
            for id in ids {
                let info = camera
                    .storage_info(id)
                    .map_err(|err| CliError::from_ptp_in(err, Context::Media).input(code32(id)))?;
                items.push(present::storage_json(id, &info));
            }
            Ok((EXIT_OK, paged(items, &page, present::STORAGE_COLUMNS)))
        }
        StorageCmd::Get { storage_id } => {
            let id = parse::u32_code(&storage_id)?;
            let mut camera = session::open(runtime, log)?;
            let ids = camera
                .storage_ids()
                .map_err(|err| CliError::from_ptp_in(err, Context::Media))?;
            if !ids.contains(&id) {
                return Err(CliError::not_found(
                    format!(
                        "storage {} is not on {}",
                        code32(id),
                        camera.device_info().model
                    ),
                    "Run: cameractl storage list --format json",
                )
                .input(storage_id));
            }
            let info = camera
                .storage_info(id)
                .map_err(|err| CliError::from_ptp_in(err, Context::Media).input(code32(id)))?;
            Ok((EXIT_OK, Document::One(present::storage_json(id, &info))))
        }
    }
}

pub fn object(
    cmd: ObjectCmd,
    runtime: &Runtime,
    log: &mut Log,
) -> Result<(i32, Document), CliError> {
    match cmd {
        ObjectCmd::List {
            storage_id,
            parent,
            object_format,
            details,
            page,
        } => {
            let parent = parse::u32_code(&parent)?;
            let format_code = parse::u32_code(&object_format)?;
            let storage_filter = storage_id.as_deref().map(parse::u32_code).transpose()?;
            let page = page_of(&page, runtime)?;
            let mut camera = session::open(runtime, log)?;
            let ids = match storage_filter {
                Some(id) => {
                    let known = camera
                        .storage_ids()
                        .map_err(|err| CliError::from_ptp_in(err, Context::Media))?;
                    if !known.contains(&id) {
                        return Err(CliError::not_found(
                            format!(
                                "storage {} is not on {}",
                                code32(id),
                                camera.device_info().model
                            ),
                            "Run: cameractl storage list --format json",
                        )
                        .input(code32(id)));
                    }
                    vec![id]
                }
                None => camera
                    .storage_ids()
                    .map_err(|err| CliError::from_ptp_in(err, Context::Media))?,
            };
            let mut handles = Vec::new();
            for id in ids {
                let found = camera
                    .object_handles(id, format_code, parent)
                    .map_err(|err| CliError::from_ptp_in(err, Context::Media).input(code32(id)))?;
                for handle in found {
                    handles.push((id, handle));
                }
            }
            handles.sort_unstable();
            let window = page.window(handles.len());
            let mut items = Vec::new();
            for &(storage, handle) in &handles[window.start..window.end] {
                if details {
                    let info = camera
                        .object_info(handle)
                        .map_err(|err| media_error(err, Some(handle)).input(code32(handle)))?;
                    items.push(present::object_json(handle, &info));
                } else {
                    items.push(serde_json::json!({
                        "handle": code32(handle),
                        "storage_id": code32(storage),
                    }));
                }
            }
            let columns: &'static [Column] = if details {
                present::OBJECT_DETAIL_COLUMNS
            } else {
                present::OBJECT_COLUMNS
            };
            Ok((
                EXIT_OK,
                Document::Many(ListOut::page(items, handles.len(), window, columns)),
            ))
        }
        ObjectCmd::Get {
            handle,
            output,
            offset,
            max_bytes,
        } => {
            let handle = prepare_object_read(&handle, output.as_deref(), offset, max_bytes)?;
            if let Some(path) = &output {
                crate::paths::validate(path)?;
            }
            let mut camera = session::open(runtime, log)?;
            let info = camera
                .object_info(handle)
                .map_err(|err| media_error(err, Some(handle)).input(code32(handle)))?;
            let mut body = present::object_json(handle, &info);
            body["offset"] = serde_json::json!(offset);
            body["max_bytes"] = match max_bytes {
                Some(max) => serde_json::json!(max),
                None => serde_json::Value::Null,
            };
            body["output"] = serde_json::Value::Null;
            body["bytes"] = serde_json::Value::Null;
            if let Some(path) = output {
                let bytes = if let Some(max) = max_bytes {
                    camera.partial_large_object(handle, offset, max)
                } else {
                    camera.object(handle)
                }
                .map_err(|err| media_error(err, Some(handle)).input(code32(handle)))?;
                let written = crate::paths::write_bytes(&path, &bytes)?;
                body["output"] = serde_json::json!(written.display().to_string());
                body["bytes"] = serde_json::json!(bytes.len());
            }
            Ok((EXIT_OK, Document::One(body)))
        }
    }
}

pub fn thumbnail(
    cmd: ThumbnailCmd,
    runtime: &Runtime,
    log: &mut Log,
) -> Result<(i32, Document), CliError> {
    match cmd {
        ThumbnailCmd::Get { handle, output } => {
            let handle = parse::handle(&handle)?;
            crate::paths::validate(&output)?;
            let mut camera = session::open(runtime, log)?;
            let bytes = camera
                .thumbnail(handle)
                .map_err(|err| media_error(err, Some(handle)).input(code32(handle)))?;
            let written = crate::paths::write_bytes(&output, &bytes)?;
            Ok((
                EXIT_OK,
                Document::One(serde_json::json!({
                    "handle": code32(handle),
                    "output": written.display().to_string(),
                    "bytes": bytes.len(),
                })),
            ))
        }
    }
}

pub fn live_view(
    cmd: LiveViewCmd,
    runtime: &Runtime,
    log: &mut Log,
) -> Result<(i32, Document), CliError> {
    match cmd {
        LiveViewCmd::Get {
            output,
            focal_output,
        } => {
            crate::paths::validate(&output)?;
            if let Some(path) = &focal_output {
                crate::paths::validate(path)?;
            }
            let mut camera = session::open(runtime, log)?;
            let frame = camera
                .live_view_frame()
                .map_err(|err| media_error(err, Some(object::LIVE_VIEW)).input("live-view"))?;
            let written = crate::paths::write_bytes(&output, &frame.jpeg)?;
            let focal_written = if let Some(path) = focal_output {
                Some(crate::paths::write_bytes(&path, &frame.focal_frame)?)
            } else {
                None
            };
            Ok((
                EXIT_OK,
                Document::One(serde_json::json!({
                    "output": written.display().to_string(),
                    "bytes": frame.jpeg.len(),
                    "focal_output": focal_written.map(|path| path.display().to_string()),
                    "focal_bytes": frame.focal_frame.len(),
                })),
            ))
        }
    }
}

pub fn find_property<'a>(
    camera: &'a sony_ptp3::Camera,
    resolved: &Resolved<'_>,
) -> Result<&'a sony_ptp3::DeviceProperty, CliError> {
    let code = resolved.code();
    camera.property(PropertyCode(code)).ok_or_else(|| {
        let shown = match resolved {
            Resolved::Known(entry) => entry.slug.clone(),
            Resolved::UnknownCode(code) => parse::code16(*code),
        };
        CliError::not_found(
            format!("property {shown} is not on {}", camera.device_info().model),
            "Run: cameractl property list --format json",
        )
        .input(shown)
    })
}

pub fn ensure_control(camera: &sony_ptp3::Camera, code: u16) -> Result<(), CliError> {
    if camera.supports_control(ControlCode(code)) {
        return Ok(());
    }
    let name = names::find(names::controls(), code)
        .map(|entry| entry.name.to_owned())
        .unwrap_or_else(|| parse::code16(code));
    Err(CliError::not_found(
        format!(
            "{} does not support control {name}",
            camera.device_info().model
        ),
        "Run: cameractl control list --format json",
    )
    .input(parse::code16(code)))
}

fn prepare_object_read(
    handle: &str,
    output: Option<&std::path::Path>,
    offset: u64,
    max_bytes: Option<u32>,
) -> Result<u32, CliError> {
    let handle = parse::handle(handle)?;
    if offset > 0 && max_bytes.is_none() {
        return Err(CliError::usage(
            "--offset requires --max-bytes",
            "Pass --max-bytes 1048576, or omit --offset for the whole object.",
        ));
    }
    if max_bytes == Some(0) {
        return Err(CliError::usage(
            "--max-bytes must be at least 1",
            "Pass a positive --max-bytes.",
        ));
    }
    if max_bytes == Some(u32::MAX) {
        return Err(CliError::usage(
            "--max-bytes cannot be 4294967295",
            "SDIO_GetPartialLargeObject rejects 0xffffffff. Pass a smaller --max-bytes.",
        ));
    }
    if (offset > 0 || max_bytes.is_some()) && output.is_none() {
        return Err(CliError::usage(
            "a partial read requires --output",
            "Pass --output chunk.bin.",
        ));
    }
    Ok(handle)
}

fn code_list(
    codes: &[u16],
    entries: &[names::CatalogEntry],
    filter: &Option<String>,
    page: &Page,
) -> Document {
    let mut codes = codes.to_vec();
    codes.sort_unstable();
    codes.dedup();
    let items: Vec<_> = codes
        .into_iter()
        .map(|code| present::identity(entries, code))
        .filter(|item| {
            matches_filter(
                filter,
                item.get("name").and_then(|value| value.as_str()),
                item.get("slug").and_then(|value| value.as_str()),
                item.get("code")
                    .and_then(|value| value.as_str())
                    .unwrap_or(""),
            )
        })
        .collect();
    paged(items, page, present::CODE_COLUMNS)
}

fn page_of(page: &crate::cli::PageOpts, runtime: &Runtime) -> Result<Page, CliError> {
    crate::page::resolve(
        page.page,
        page.page_size,
        page.page_all,
        runtime.stdout_is_tty,
    )
}

fn paged(items: Vec<serde_json::Value>, page: &Page, columns: &'static [Column]) -> Document {
    let window = page.window(items.len());
    let slice = items[window.start..window.end].to_vec();
    Document::Many(ListOut::page(slice, items.len(), window, columns))
}

fn matches_filter(
    filter: &Option<String>,
    name: Option<&str>,
    slug: Option<&str>,
    code: &str,
) -> bool {
    let Some(filter) = filter else {
        return true;
    };
    let filter = filter.to_ascii_lowercase();
    name.is_some_and(|name| name.to_ascii_lowercase().contains(&filter))
        || slug.is_some_and(|slug| slug.contains(&filter))
        || code.to_ascii_lowercase().contains(&filter)
}

fn media_error(error: Error, handle: Option<u32>) -> CliError {
    let mut cli = CliError::from_ptp_in(error, Context::Media);
    if handle == Some(object::LIVE_VIEW) {
        cli.suggestion = "Run: cameractl live-view get --output frame.jpg".to_owned();
    }
    cli
}
