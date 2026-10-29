//! Noun-verb command tree. Agents narrow it with `--help`, then `schema`.

use std::path::PathBuf;

use clap::{Args, CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum};

const BEFORE: &str = "\
Examples:
  cameractl device get
  cameractl property ensure --name iso-sensitivity --value 400 --dry-run
  cameractl capture create --dry-run
  cameractl schema property update";

const AFTER: &str = "\
Globals, in any position: --connect-timeout DURATION, --reset-device,
--release-system-client, --no-release-system-client, --color, --no-color,
--no-interactive, --yes, --verbose, --quiet.
--yes and --no-interactive are accepted. This CLI does not prompt.
Destructive controls require --force.

Stdout is data. Stderr is messages. A terminal defaults to table output;
any other stdout defaults to json. Errors are JSON on stderr unless the
format is table.

List JSON is {items, total, page, page_size, pages}. csv and ndjson emit
items only.

Exit codes:
  0   success
  1   general error
  2   usage error
  3   not found
  4   permission denied
  5   conflict
  6   transient error; retry
  10  dry-run passed; nothing was sent";

#[derive(Debug, Parser)]
#[command(
    name = "cameractl",
    bin_name = "cameractl",
    version,
    propagate_version = true,
    disable_version_flag = true,
    disable_help_subcommand = true,
    subcommand_required = true,
    arg_required_else_help = true,
    infer_subcommands = false,
    infer_long_args = false,
    max_term_width = 100,
    about = "Control a Sony camera over USB PTP for scripts and agents.",
    before_help = BEFORE,
    after_help = AFTER
)]
pub struct Cli {
    #[command(flatten)]
    pub global: GlobalOpts,
    #[command(subcommand)]
    pub command: Cmd,
}

#[derive(Debug, Args)]
pub struct GlobalOpts {
    /// Output format. Default: table on a terminal, json otherwise.
    #[arg(long, global = true, value_enum, value_name = "json|table|csv|ndjson")]
    pub format: Option<Format>,

    /// JSON on stdout. Same as --format json.
    #[arg(long, global = true)]
    pub json: bool,

    /// Bold table headers. Default is color on a terminal unless NO_COLOR or TERM=dumb.
    #[arg(long, global = true, hide = true, overrides_with = "no_color")]
    #[allow(dead_code)]
    pub color: bool,

    /// Plain text, even on a terminal.
    #[arg(long, global = true, hide = true, overrides_with = "color")]
    pub no_color: bool,

    /// Accepted. This CLI never prompts.
    #[arg(long, global = true, hide = true)]
    #[allow(dead_code)]
    pub no_interactive: bool,

    /// Accepted. This CLI never prompts; destructive controls still require --force.
    #[arg(long, global = true, hide = true)]
    #[allow(dead_code)]
    pub yes: bool,

    /// Write the connected camera model to stderr.
    #[arg(long, global = true, hide = true, overrides_with = "quiet")]
    pub verbose: bool,

    /// Suppress stderr warnings.
    #[arg(long, global = true, hide = true, overrides_with = "verbose")]
    pub quiet: bool,

    /// USB timeout, such as 5s, 500ms, or 1m. Default 5s.
    #[arg(long, global = true, hide = true, value_name = "DURATION")]
    pub connect_timeout: Option<String>,

    /// Reopen an SDIO session. Omit for the standard remote-control session.
    #[arg(
        long,
        global = true,
        value_enum,
        value_name = "remote-control|content-transfer|remote-with-transfer"
    )]
    pub function_mode: Option<FunctionModeArg>,

    /// Reset the USB device while opening the session.
    #[arg(long, global = true, hide = true)]
    pub reset_device: bool,

    /// On macOS, stop Image Capture from holding the camera. Default on macOS.
    #[arg(
        long,
        global = true,
        hide = true,
        overrides_with = "no_release_system_client"
    )]
    pub release_system_client: bool,

    /// Leave the system camera client running.
    #[arg(
        long,
        global = true,
        hide = true,
        overrides_with = "release_system_client"
    )]
    pub no_release_system_client: bool,

    /// Print the version and exit.
    #[arg(long = "version", global = true, action = clap::ArgAction::Version)]
    pub version: (),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    Json,
    Table,
    Csv,
    Ndjson,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum FunctionModeArg {
    RemoteControl,
    ContentTransfer,
    RemoteWithTransfer,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ButtonArg {
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum MovieArg {
    Recording,
    Idle,
}

#[derive(Debug, Subcommand)]
#[command(infer_subcommands = false, infer_long_args = false)]
pub enum Cmd {
    /// Camera identity and capability counts.
    #[command(
        subcommand,
        before_help = "Examples:\n  cameractl device get\n  cameractl device get --format json"
    )]
    Device(DeviceCmd),
    /// Settings such as ISO, aperture, and white balance.
    #[command(
        subcommand,
        before_help = "Examples:\n  cameractl property list --writable\n  cameractl property get --name iso-sensitivity"
    )]
    Property(PropertyCmd),
    /// Buttons, dials, and other camera controls.
    #[command(
        subcommand,
        before_help = "Examples:\n  cameractl control list --name-contains shutter\n  cameractl control update --name movie-rec-button-hold --position down --dry-run"
    )]
    Control(ControlCmd),
    /// PTP operations this camera advertised.
    #[command(
        subcommand,
        before_help = "Examples:\n  cameractl operation list\n  cameractl operation list --name-contains object"
    )]
    Operation(OperationCmd),
    /// Events this camera can send.
    #[command(
        subcommand,
        before_help = "Examples:\n  cameractl event list\n  cameractl event get --timeout 1s"
    )]
    Event(EventCmd),
    /// Memory cards.
    #[command(
        subcommand,
        before_help = "Examples:\n  cameractl storage list\n  cameractl storage get --storage-id 0x00010001"
    )]
    Storage(StorageCmd),
    /// Files stored on the camera.
    #[command(
        subcommand,
        before_help = "Examples:\n  cameractl object list --function-mode content-transfer\n  cameractl object get --handle captured --output shot.jpg"
    )]
    Object(ObjectCmd),
    /// JPEG thumbnail of a camera file.
    #[command(
        subcommand,
        before_help = "Examples:\n  cameractl thumbnail get --handle 0x00000001 --output thumb.jpg"
    )]
    Thumbnail(ThumbnailCmd),
    /// Still pictures.
    #[command(
        subcommand,
        before_help = "Examples:\n  cameractl capture create --dry-run\n  cameractl capture create --output shot.jpg --idempotency-key shot-1"
    )]
    Capture(CaptureCmd),
    /// Zoom motor. Positive is telephoto, negative is wide, zero stops.
    #[command(
        subcommand,
        before_help = "Examples:\n  cameractl zoom update --speed 0 --dry-run\n  cameractl zoom update --speed -1"
    )]
    Zoom(ZoomCmd),
    /// Near/far focus motor. Zero does not drive the lens.
    #[command(
        subcommand,
        before_help = "Examples:\n  cameractl focus update --speed 0 --dry-run\n  cameractl focus update --speed 2"
    )]
    Focus(FocusCmd),
    /// Autofocus point in live-view coordinates.
    #[command(
        subcommand,
        before_help = "Examples:\n  cameractl focus-point update --x 320 --y 240 --dry-run"
    )]
    FocusPoint(FocusPointCmd),
    /// Touch the live-view image. X is 0..=639 and Y is 0..=479.
    #[command(
        subcommand,
        before_help = "Examples:\n  cameractl touch update --x 320 --y 240 --dry-run"
    )]
    Touch(TouchCmd),
    /// Movie recording.
    #[command(
        subcommand,
        before_help = "Examples:\n  cameractl movie ensure --state recording --dry-run\n  cameractl movie ensure --state idle"
    )]
    Movie(MovieCmd),
    /// One live-view JPEG.
    #[command(
        subcommand,
        before_help = "Examples:\n  cameractl live-view get --output frame.jpg"
    )]
    LiveView(LiveViewCmd),
    /// JSON description of commands and flags.
    #[command(
        before_help = "Examples:\n  cameractl schema --all\n  cameractl schema property update"
    )]
    Schema(SchemaArgs),
    /// Text help. Use schema for JSON.
    #[command(before_help = "Examples:\n  cameractl help\n  cameractl help property update")]
    Help(HelpArgs),
}

#[derive(Debug, Subcommand)]
#[command(infer_subcommands = false, infer_long_args = false)]
pub enum DeviceCmd {
    /// Show model, firmware, serial, and capability counts.
    #[command(
        before_help = "Examples:\n  cameractl device get\n  cameractl device get --format json"
    )]
    Get,
}

#[derive(Debug, Subcommand)]
#[command(infer_subcommands = false, infer_long_args = false)]
pub enum PropertyCmd {
    /// List properties on this camera.
    #[command(
        before_help = "Examples:\n  cameractl property list --writable --format json\n  cameractl property list --name-contains iso"
    )]
    List {
        /// Only properties the camera currently allows writing.
        #[arg(long)]
        writable: bool,
        /// Case-insensitive substring of the name, slug, or code.
        #[arg(long, value_name = "TEXT")]
        name_contains: Option<String>,
        #[command(flatten)]
        page: PageOpts,
    },
    /// Read one property, including the values it accepts.
    #[command(
        before_help = "Examples:\n  cameractl property get --name iso-sensitivity\n  cameractl property get --code 0xd21e --format json"
    )]
    Get {
        #[command(flatten)]
        selector: Selector,
    },
    /// Write a property. Prefer ensure when a retry must be safe.
    #[command(
        before_help = "Examples:\n  cameractl property update --name iso-sensitivity --value 400 --dry-run\n  cameractl property update --code 0xd21e --value \"ISO 400\""
    )]
    Update {
        #[command(flatten)]
        selector: Selector,
        /// Number or catalog label, such as 400 or ISO 400.
        #[arg(long, allow_hyphen_values = true, value_name = "VALUE")]
        value: String,
        /// Write only when the current value still matches.
        #[arg(long, allow_hyphen_values = true, value_name = "VALUE")]
        if_match: Option<String>,
        #[command(flatten)]
        side: SideEffect,
    },
    /// Write a property only when the current value differs.
    #[command(
        before_help = "Examples:\n  cameractl property ensure --name iso-sensitivity --value 400 --dry-run\n  cameractl property ensure --name iso-sensitivity --value \"ISO 400\""
    )]
    Ensure {
        #[command(flatten)]
        selector: Selector,
        /// Number or catalog label, such as 400 or ISO 400.
        #[arg(long, allow_hyphen_values = true, value_name = "VALUE")]
        value: String,
        /// Write only when the current value still matches.
        #[arg(long, allow_hyphen_values = true, value_name = "VALUE")]
        if_match: Option<String>,
        #[command(flatten)]
        side: SideEffect,
    },
}

#[derive(Debug, Subcommand)]
#[command(infer_subcommands = false, infer_long_args = false)]
pub enum ControlCmd {
    /// List controls this camera advertised.
    #[command(
        before_help = "Examples:\n  cameractl control list\n  cameractl control list --name-contains zoom"
    )]
    List {
        /// Case-insensitive substring of the name, slug, or code.
        #[arg(long, value_name = "TEXT")]
        name_contains: Option<String>,
        #[command(flatten)]
        page: PageOpts,
    },
    /// Describe one control.
    #[command(
        before_help = "Examples:\n  cameractl control get --name zoom-operation\n  cameractl control get --code 0xd2dd"
    )]
    Get {
        #[command(flatten)]
        selector: Selector,
    },
    /// Send a control value. Destructive controls also require --force.
    #[command(
        before_help = "Examples:\n  cameractl control update --name movie-rec-button-hold --position down --dry-run\n  cameractl control update --code 0xd2dd --value 0"
    )]
    Update {
        #[command(flatten)]
        selector: Selector,
        /// Numeric control value. Omit when passing --position.
        #[arg(long, allow_hyphen_values = true, value_name = "VALUE")]
        value: Option<String>,
        /// Button position. up is released and down is pressed.
        #[arg(long, value_enum, value_name = "up|down")]
        position: Option<ButtonArg>,
        /// Required for controls that format media, reset settings, or power off.
        #[arg(long)]
        force: bool,
        #[command(flatten)]
        side: SideEffect,
    },
}

#[derive(Debug, Subcommand)]
#[command(infer_subcommands = false, infer_long_args = false)]
pub enum OperationCmd {
    /// List PTP operations this camera advertised.
    #[command(
        before_help = "Examples:\n  cameractl operation list --format json\n  cameractl operation list --name-contains storage"
    )]
    List {
        /// Case-insensitive substring of the name, slug, or code.
        #[arg(long, value_name = "TEXT")]
        name_contains: Option<String>,
        #[command(flatten)]
        page: PageOpts,
    },
}

#[derive(Debug, Subcommand)]
#[command(infer_subcommands = false, infer_long_args = false)]
pub enum EventCmd {
    /// List events this camera advertised.
    #[command(
        before_help = "Examples:\n  cameractl event list\n  cameractl event list --name-contains object"
    )]
    List {
        /// Case-insensitive substring of the name, slug, or code.
        #[arg(long, value_name = "TEXT")]
        name_contains: Option<String>,
        #[command(flatten)]
        page: PageOpts,
    },
    /// Wait for one event. The event field is null when none arrives.
    #[command(
        before_help = "Examples:\n  cameractl event get --timeout 1s\n  cameractl event get --timeout 0ms --format json"
    )]
    Get {
        /// How long to wait, such as 500ms, 1s, or 0ms. Default 1s.
        #[arg(long, value_name = "DURATION", default_value = "1s")]
        timeout: String,
    },
}

#[derive(Debug, Subcommand)]
#[command(infer_subcommands = false, infer_long_args = false)]
pub enum StorageCmd {
    /// List memory cards.
    #[command(
        before_help = "Examples:\n  cameractl storage list\n  cameractl storage list --function-mode content-transfer"
    )]
    List {
        #[command(flatten)]
        page: PageOpts,
    },
    /// Describe one memory card.
    #[command(before_help = "Examples:\n  cameractl storage get --storage-id 0x00010001")]
    Get {
        /// Storage id from storage list.
        #[arg(long, value_name = "ID")]
        storage_id: String,
    },
}

#[derive(Debug, Subcommand)]
#[command(infer_subcommands = false, infer_long_args = false)]
pub enum ObjectCmd {
    /// List file handles. Card contents often need --function-mode content-transfer.
    #[command(
        before_help = "Examples:\n  cameractl object list --function-mode content-transfer\n  cameractl object list --storage-id 0x00010001 --details --page-size 20"
    )]
    List {
        /// Storage id. Omit to read every storage.
        #[arg(long, value_name = "ID")]
        storage_id: Option<String>,
        /// Parent handle. 0xffffffff lists the whole store.
        #[arg(long, value_name = "HANDLE", default_value = "0xffffffff")]
        parent: String,
        /// PTP object format code. 0 lists every format.
        #[arg(long, value_name = "CODE", default_value = "0")]
        object_format: String,
        /// Fetch filename and size for this page only.
        #[arg(long)]
        details: bool,
        #[command(flatten)]
        page: PageOpts,
    },
    /// Read file info. Pass --output to download bytes.
    #[command(
        before_help = "Examples:\n  cameractl object get --handle captured\n  cameractl object get --handle 0x00000001 --output photo.jpg"
    )]
    Get {
        /// captured, live-view, shot-with-info, camera-setting, ftp-setting, or a handle.
        #[arg(long, value_name = "HANDLE")]
        handle: String,
        /// File that receives the object bytes.
        #[arg(long, value_name = "PATH")]
        output: Option<PathBuf>,
        /// Byte offset. Nonzero requires --max-bytes.
        #[arg(long, default_value_t = 0, value_name = "N")]
        offset: u64,
        /// Read at most this many bytes from --offset.
        #[arg(long, value_name = "N")]
        max_bytes: Option<u32>,
    },
}

#[derive(Debug, Subcommand)]
#[command(infer_subcommands = false, infer_long_args = false)]
pub enum ThumbnailCmd {
    /// Save a thumbnail JPEG.
    #[command(
        before_help = "Examples:\n  cameractl thumbnail get --handle 0x00000001 --output thumb.jpg"
    )]
    Get {
        /// Object handle.
        #[arg(long, value_name = "HANDLE")]
        handle: String,
        /// File that receives the thumbnail.
        #[arg(long, value_name = "PATH")]
        output: PathBuf,
    },
}

#[derive(Debug, Subcommand)]
#[command(infer_subcommands = false, infer_long_args = false)]
pub enum CaptureCmd {
    /// Half-press, fire, then release the shutter.
    #[command(
        before_help = "Examples:\n  cameractl capture create --dry-run\n  cameractl capture create --output shot.jpg --idempotency-key shot-1",
        after_help = "If --output fails after the shutter fires, the exit code stays 0 and image_error is set. Download with: cameractl object get --handle captured --output shot.jpg"
    )]
    Create {
        /// Save the captured image here after the shutter fires.
        #[arg(long, value_name = "PATH")]
        output: Option<PathBuf>,
        #[command(flatten)]
        side: SideEffect,
    },
}

#[derive(Debug, Subcommand)]
#[command(infer_subcommands = false, infer_long_args = false)]
pub enum ZoomCmd {
    /// Set zoom speed. Positive is telephoto, negative is wide, zero stops.
    #[command(
        before_help = "Examples:\n  cameractl zoom update --speed 0 --dry-run\n  cameractl zoom update --speed 1 --idempotency-key zoom-1"
    )]
    Update {
        /// Speed from -128 to 127.
        #[arg(long, allow_hyphen_values = true, value_name = "I8")]
        speed: i8,
        #[command(flatten)]
        side: SideEffect,
    },
}

#[derive(Debug, Subcommand)]
#[command(infer_subcommands = false, infer_long_args = false)]
pub enum FocusCmd {
    /// Set near/far focus speed. Zero does not drive the lens.
    #[command(
        before_help = "Examples:\n  cameractl focus update --speed 0 --dry-run\n  cameractl focus update --speed -2"
    )]
    Update {
        /// Speed from -32768 to 32767.
        #[arg(long, allow_hyphen_values = true, value_name = "I16")]
        speed: i16,
        #[command(flatten)]
        side: SideEffect,
    },
}

#[derive(Debug, Subcommand)]
#[command(infer_subcommands = false, infer_long_args = false)]
pub enum FocusPointCmd {
    /// Move the AF point. X is 0..=639 and Y is 0..=479.
    #[command(before_help = "Examples:\n  cameractl focus-point update --x 320 --y 240 --dry-run")]
    Update {
        /// Horizontal position, 0 to 639.
        #[arg(long, value_name = "X")]
        x: u16,
        /// Vertical position, 0 to 479.
        #[arg(long, value_name = "Y")]
        y: u16,
        #[command(flatten)]
        side: SideEffect,
    },
}

#[derive(Debug, Subcommand)]
#[command(infer_subcommands = false, infer_long_args = false)]
pub enum TouchCmd {
    /// Touch live view. X is 0..=639 and Y is 0..=479.
    #[command(before_help = "Examples:\n  cameractl touch update --x 100 --y 200 --dry-run")]
    Update {
        /// Horizontal position, 0 to 639.
        #[arg(long, value_name = "X")]
        x: u16,
        /// Vertical position, 0 to 479.
        #[arg(long, value_name = "Y")]
        y: u16,
        #[command(flatten)]
        side: SideEffect,
    },
}

#[derive(Debug, Subcommand)]
#[command(infer_subcommands = false, infer_long_args = false)]
pub enum MovieCmd {
    /// Start or stop recording only when the camera is not already there.
    #[command(
        before_help = "Examples:\n  cameractl movie ensure --state recording --dry-run\n  cameractl movie ensure --state idle --idempotency-key clip-1"
    )]
    Ensure {
        /// recording or idle.
        #[arg(long, value_enum, value_name = "recording|idle")]
        state: MovieArg,
        #[command(flatten)]
        side: SideEffect,
    },
    /// Hold or release the movie button without reading record state.
    #[command(
        before_help = "Examples:\n  cameractl movie update --position down --dry-run\n  cameractl movie update --position up"
    )]
    Update {
        /// down presses the button. up releases it.
        #[arg(long, value_enum, value_name = "up|down")]
        position: ButtonArg,
        #[command(flatten)]
        side: SideEffect,
    },
}

#[derive(Debug, Subcommand)]
#[command(infer_subcommands = false, infer_long_args = false)]
pub enum LiveViewCmd {
    /// Save one live-view frame from handle live-view (0xffffc002).
    #[command(
        before_help = "Examples:\n  cameractl live-view get --output frame.jpg\n  cameractl live-view get --output frame.jpg --focal-output frame.bin"
    )]
    Get {
        /// File that receives the JPEG.
        #[arg(long, value_name = "PATH")]
        output: PathBuf,
        /// File that receives the focal-frame bytes.
        #[arg(long, value_name = "PATH")]
        focal_output: Option<PathBuf>,
    },
}

#[derive(Debug, Args)]
pub struct SchemaArgs {
    /// Print the whole command tree.
    #[arg(long)]
    pub all: bool,
    /// Command group, for example property.
    #[arg(value_name = "COMMAND")]
    pub group: Option<String>,
    /// Command verb, for example update.
    #[arg(value_name = "VERB")]
    pub verb: Option<String>,
}

#[derive(Debug, Args)]
pub struct HelpArgs {
    /// Command group, for example property.
    #[arg(value_name = "COMMAND")]
    pub group: Option<String>,
    /// Command verb, for example update.
    #[arg(value_name = "VERB")]
    pub verb: Option<String>,
}

#[derive(Debug, Args)]
pub struct Selector {
    /// Catalog slug or exact name, such as iso-sensitivity.
    #[arg(long, value_name = "SLUG")]
    pub name: Option<String>,
    /// Wire code, such as 0xd21e.
    #[arg(long, value_name = "HEX")]
    pub code: Option<String>,
}

impl Selector {
    pub fn query(&self, noun: &str) -> Result<&str, crate::error::CliError> {
        match (&self.name, &self.code) {
            (Some(name), None) => Ok(name),
            (None, Some(code)) => Ok(code),
            (None, None) => Err(crate::error::CliError::usage(
                "pass --name or --code",
                format!("Example: cameractl {noun} get --name iso-sensitivity"),
            )),
            (Some(_), Some(_)) => Err(crate::error::CliError::usage(
                "pass only one of --name and --code",
                format!("Example: cameractl {noun} get --code 0xd21e"),
            )),
        }
    }
}

pub fn check_contains(value: &Option<String>) -> Result<(), crate::error::CliError> {
    if let Some(text) = value {
        if text.trim().is_empty() {
            return Err(crate::error::CliError::usage(
                "--name-contains cannot be empty",
                "Pass text from the name, for example iso.",
            ));
        }
    }
    Ok(())
}

#[derive(Debug, Args)]
pub struct SideEffect {
    /// Show the change and exit 10. Nothing is sent to the camera.
    #[arg(long)]
    pub dry_run: bool,
    /// Replay the first success for this key instead of repeating the action.
    #[arg(long, value_name = "KEY")]
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Args)]
pub struct PageOpts {
    /// Items per page. Default is 20 on a terminal, otherwise every item.
    #[arg(long, value_name = "N")]
    pub page_size: Option<usize>,
    /// Page number, starting at 1.
    #[arg(long, value_name = "N")]
    pub page: Option<usize>,
    /// Return every item.
    #[arg(long)]
    pub page_all: bool,
}

impl Cli {
    pub fn parse(args: &[std::ffi::OsString], color: bool) -> Result<Self, clap::Error> {
        let command = Self::command().color(if color {
            clap::ColorChoice::Auto
        } else {
            clap::ColorChoice::Never
        });
        let matches = command.try_get_matches_from(args)?;
        Self::from_arg_matches(&matches)
    }
}
