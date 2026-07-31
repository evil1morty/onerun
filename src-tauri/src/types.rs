use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::util::UrlConfidence;

// ── Runtime state ──────────────────────────────────

/// Per-command process state.  Keyed by `"{project_id}::{label}"` in the map.
#[derive(Default)]
pub struct ProcessState {
    pub pid: Option<u32>,
    pub running: bool,
    pub logs: VecDeque<LogLine>,
    pub log_bytes: usize,           // running total of `logs[..].text.len()` — caps buffer by size, not just count
    pub detected_url: Option<String>,
    pub url_confidence: UrlConfidence,
    pub job_handle: Option<usize>,  // per-process job object for reliable kill
    pub epoch: u64,                 // generation counter — prevents stale wait threads from corrupting state
}

pub struct AppState {
    pub processes: Arc<Mutex<HashMap<String, ProcessState>>>,
    /// Process key currently displayed in the log panel, or `None` when the
    /// panel is closed. Reader threads only emit `process-log` events for this
    /// key — everything else is buffered in `ProcessState::logs` and fetched
    /// on demand, so background output costs nothing on the IPC bridge.
    pub log_viewer: Arc<Mutex<Option<String>>>,
    pub config_path: Mutex<PathBuf>,
    pub settings_path: Mutex<PathBuf>,
    pub force_close: Mutex<bool>,
}

// ── Settings ───────────────────────────────────────

#[derive(Serialize, Deserialize, Clone)]
pub struct Settings {
    pub claude_command: String,
    #[serde(default = "default_claude_mode")]
    pub claude_mode: String, // "window" or "tab"
    pub editor_command: String,
    pub theme: String,
    #[serde(default = "default_width")]
    pub width: u32,
    #[serde(default = "default_height")]
    pub height: u32,
    #[serde(default)]
    pub autostart: bool,
    #[serde(default)]
    pub tag_order: Vec<String>,
    #[serde(default = "default_tags_visible")]
    pub tags_visible: bool,
    /// Parent folder for newly created projects. Empty = Documents/projects.
    #[serde(default)]
    pub projects_dir: String,
}

fn default_claude_mode() -> String { "tab".into() }
fn default_width() -> u32 { 580 }
fn default_height() -> u32 { 680 }
fn default_tags_visible() -> bool { true }

impl Default for Settings {
    fn default() -> Self {
        Self {
            claude_command: "claude".into(),
            claude_mode: "tab".into(),
            editor_command: "code".into(),
            theme: "system".into(),
            width: 580,
            height: 680,
            autostart: false,
            tag_order: Vec::new(),
            tags_visible: true,
            projects_dir: String::new(),
        }
    }
}

// ── Persisted config ───────────────────────────────

#[derive(Serialize, Deserialize, Clone)]
pub struct ProjectConfig {
    pub id: String,
    pub name: String,
    pub directory: String,
    pub framework: Option<String>,
    pub commands: Vec<CommandDef>,
    #[serde(default)]
    pub env: Vec<EnvVar>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub pinned: bool,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct EnvVar {
    pub key: String,
    pub value: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct CommandDef {
    pub label: String,
    pub cmd: String,
}

// ── Event payloads ─────────────────────────────────

/// Which pipe a log line came from. Serializes to `"stdout"` / `"stderr"`,
/// matching the CSS class names the frontend applies.
#[derive(Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Stream {
    Stdout,
    Stderr,
}

#[derive(Serialize, Clone)]
pub struct LogPayload {
    pub id: String,
    pub label: String,
    pub text: String,
    pub stream: Stream,
}

#[derive(Serialize, Clone)]
pub struct StatusPayload {
    pub id: String,
    pub label: String,
    pub running: bool,
    pub url: Option<String>,
}

#[derive(Serialize, Clone)]
pub struct LogLine {
    pub text: String,
    pub stream: Stream,
}

/// Per-command status returned by get_all_status.
#[derive(Serialize, Clone)]
pub struct CmdStatusPayload {
    pub running: bool,
    pub url: Option<String>,
}

// ── Scan result ────────────────────────────────────

#[derive(Serialize)]
pub struct ScanResult {
    pub name: String,
    pub framework: Option<String>,
    pub commands: Vec<CommandDef>,
}

// ── Key helpers ────────────────────────────────────

/// Build the composite key used in the process map.
pub fn process_key(id: &str, label: &str) -> String {
    format!("{}::{}", id, label)
}

/// Parse a composite key back to (project_id, label).
pub fn parse_key(key: &str) -> (&str, &str) {
    key.split_once("::").unwrap_or((key, ""))
}
