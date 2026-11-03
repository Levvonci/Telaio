//! Configuration lives in two files under ~/.config/telaio/:
//!
//! - `account.toml` — login data (one or more accounts)
//! - `ui.toml`      — theme, layout and behaviour

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

// ── Accounts ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct AccountConfig {
    pub homeserver: String,
    pub username: String,
    /// Only needed for the first login: the session is saved afterwards
    pub password: Option<String>,
}

impl AccountConfig {
    /// Actually filled in (not empty and not the template placeholder)
    pub fn is_set(&self) -> bool {
        let user = self.username.trim();
        !user.is_empty() && !PLACEHOLDER_USERS.contains(&user)
    }
}

/// account.toml: one account at the top level (original format) and/or more
/// in `[[account]]` tables
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct AccountsFile {
    #[serde(flatten)]
    main: AccountConfig,
    account: Vec<AccountConfig>,
}

/// The second entry is the placeholder of older, Italian templates
const PLACEHOLDER_USERS: [&str; 2] = ["@user:matrix.org", "@utente:matrix.org"];

// ── Interface ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    pub theme: ThemeConfig,
    pub layout: LayoutConfig,
    pub behavior: BehaviorConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ThemeConfig {
    pub background: String,
    pub surface: String,
    pub text: String,
    pub border: String,
    pub muted: String,
    pub timestamp: String,
    pub accent: String,
    pub unread: String,
    pub error: String,
    pub success: String,
    pub sender_colors: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct LayoutConfig {
    pub show_spaces: bool,
    pub show_rooms: bool,
    pub spaces_width: u16,
    pub rooms_width: u16,
    pub show_message_preview: bool,
    /// Home lists every room; if false only rooms outside any space
    pub home_all_rooms: bool,
    /// Popup and error bar border: "plain", "rounded", "double", "thick"
    pub border_type: String,
    pub time_format: String,
    /// Unicode symbols; turn off if the terminal renders them badly
    pub icons: bool,
    /// Group consecutive messages from the same sender
    pub group_messages: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct BehaviorConfig {
    pub history_limit: u64,
    pub status_timeout_secs: u64,
    pub status_fade_secs: f32,
    /// 0 = errors stay until dismissed
    pub error_timeout_secs: u64,
    /// Empty = `Matrix_Download` inside the system Downloads folder
    pub download_dir: String,
    /// Program used to open attachments; empty = system default (xdg-open)
    pub open_command: String,
    pub max_upload_mb: u64,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            background: "#0f0f0f".into(),
            surface: "#1c1918".into(),
            text: "#f0f0f0".into(),
            border: "#f0f0f0".into(),
            muted: "#b4bdc3".into(),
            timestamp: "#6c7174".into(),
            accent: "#b77e64".into(),
            unread: "#de6e7c".into(),
            error: "#de6e7c".into(),
            success: "#819b69".into(),
            sender_colors: ["#de6e7c", "#819b69", "#b77e64", "#b279a7", "#66a5ad", "#b4bdc3"]
                .iter().map(|s| s.to_string()).collect(),
        }
    }
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            show_spaces: true,
            show_rooms: true,
            spaces_width: 22,
            rooms_width: 30,
            show_message_preview: true,
            home_all_rooms: true,
            border_type: "rounded".into(),
            time_format: "%H:%M".into(),
            icons: true,
            group_messages: true,
        }
    }
}

impl Default for BehaviorConfig {
    fn default() -> Self {
        Self {
            history_limit: 50,
            status_timeout_secs: 3,
            status_fade_secs: 1.5,
            error_timeout_secs: 10,
            download_dir: String::new(),
            open_command: String::new(),
            max_upload_mb: 100,
        }
    }
}

pub const DOWNLOAD_SUBDIR: &str = "Matrix_Download";

impl BehaviorConfig {
    pub fn download_dir(&self) -> PathBuf {
        match self.download_dir.trim() {
            "" => dirs::download_dir().or_else(dirs::home_dir).unwrap_or_else(|| PathBuf::from(".")).join(DOWNLOAD_SUBDIR),
            dir => expand_tilde(dir),
        }
    }

    /// Program and arguments used to open a file
    pub fn open_command(&self) -> Vec<String> {
        let custom: Vec<String> = self.open_command.split_whitespace().map(str::to_owned).collect();
        if !custom.is_empty() { return custom; }
        vec![if cfg!(target_os = "macos") { "open" } else { "xdg-open" }.to_string()]
    }
}

// ── Paths ──────────────────────────────────────────────────────────────────

/// Used for folders, the Matrix device name and the app-id
pub const APP_NAME: &str = "telaio";
/// Name of the prototype: its folders are migrated on first start
const LEGACY_NAME: &str = "matrix-tui";

fn base_config() -> PathBuf { dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")) }
fn base_data() -> PathBuf { dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")) }

pub fn config_dir() -> PathBuf {
    base_config().join(APP_NAME)
}

pub fn account_path() -> PathBuf { config_dir().join("account.toml") }
pub fn ui_path() -> PathBuf { config_dir().join("ui.toml") }

pub fn data_root() -> PathBuf { base_data().join(APP_NAME) }

/// Per-account data folder: session and SQLite store
pub fn data_dir_for(username: &str) -> PathBuf {
    let safe: String = username
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' })
        .collect();
    data_root().join(safe)
}

/// Last used account, reopened on the next start
pub fn last_account() -> Option<String> {
    let raw = std::fs::read_to_string(data_root().join("last_account")).ok()?;
    Some(raw.trim().to_string()).filter(|s| !s.is_empty())
}

/// Best effort: errors are ignored
pub fn set_last_account(username: &str) {
    let _ = std::fs::create_dir_all(data_root());
    let _ = std::fs::write(data_root().join("last_account"), username);
}

pub fn expand_tilde(p: &str) -> PathBuf {
    let home = dirs::home_dir();
    match (p, home) {
        ("~", Some(h)) => h,
        (p, Some(h)) if p.starts_with("~/") => h.join(&p[2..]),
        (p, _) => PathBuf::from(p),
    }
}

// ── Loading ────────────────────────────────────────────────────────────────

/// Accounts from account.toml (only the filled-in ones)
pub fn load_accounts() -> Result<Vec<AccountConfig>> {
    let path = account_path();
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("Cannot read {}", path.display()))?;
    parse_accounts(&raw).with_context(|| format!("Cannot parse {}", path.display()))
}

fn parse_accounts(raw: &str) -> Result<Vec<AccountConfig>> {
    let file: AccountsFile = toml::from_str(raw)?;
    Ok(std::iter::once(file.main).chain(file.account).filter(AccountConfig::is_set).collect())
}

/// Loads `ui.toml`; if it is malformed, falls back to the defaults and returns
/// the error to show in the interface
pub fn load_ui() -> (UiConfig, Option<String>) {
    let Ok(raw) = std::fs::read_to_string(ui_path()) else {
        return (UiConfig::default(), None);
    };
    match toml::from_str(&raw) {
        Ok(cfg) => (cfg, None),
        Err(e) => (UiConfig::default(), Some(format!("Invalid ui.toml, using defaults: {}", e.message()))),
    }
}

/// Moves config and data of the prototype ("matrix-tui") under the new name,
/// keeping session and settings. Returns true if anything was migrated.
pub fn migrate_legacy() -> Result<bool> {
    let mut migrated = false;
    for (old, new) in [
        (base_config().join(LEGACY_NAME), config_dir()),
        (base_data().join(LEGACY_NAME), base_data().join(APP_NAME)),
    ] {
        if old.is_dir() && !new.exists() {
            std::fs::rename(&old, &new)
                .with_context(|| format!("Cannot move {} to {}", old.display(), new.display()))?;
            migrated = true;
        }
    }
    if migrated {
        // the old name also appears in the config file comments
        for path in [account_path(), ui_path()] {
            if let Ok(raw) = std::fs::read_to_string(&path) {
                let updated = raw.replace(&format!("# {LEGACY_NAME} —"), &format!("# {APP_NAME} —"))
                    .replace(&format!("share/{LEGACY_NAME}/"), &format!("share/{APP_NAME}/"));
                if updated != raw { std::fs::write(&path, updated)?; }
            }
        }
    }
    Ok(migrated)
}

/// Creates missing config files; returns informational notes
pub fn ensure_files() -> Result<Vec<String>> {
    std::fs::create_dir_all(config_dir())?;
    let mut notes = Vec::new();
    if !account_path().exists() {
        write_private(&account_path(), ACCOUNT_EXAMPLE)?;
        notes.push(format!("Created {}", account_path().display()));
    }
    if !ui_path().exists() {
        std::fs::write(ui_path(), ui_template(&UiConfig::default()))?;
        notes.push(format!("Created {}", ui_path().display()));
    }
    Ok(notes)
}

/// Writes a file readable only by its owner (it holds credentials)
pub fn write_private(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn toml_str(s: &str) -> String {
    toml::Value::String(s.to_owned()).to_string()
}

const ACCOUNT_EXAMPLE: &str = "\
# telaio — login data (optional)
# You don't need to fill this in: on first start telaio asks for credentials
# and saves the session. List accounts here to skip typing them.
homeserver = \"https://matrix.org\"
username   = \"@user:matrix.org\"

# Only needed for the first login: afterwards the session is saved in
# ~/.local/share/telaio/ and this line can be removed.
# If missing, the password is asked on the login screen.
# password = \"YOUR_PASSWORD\"

# More accounts (switch with F2 inside telaio, or start with
# `telaio @user:server`). They can also be added from F2.
# [[account]]
# homeserver = \"https://matrix.org\"
# username   = \"@other:matrix.org\"
";

pub fn ui_template(ui: &UiConfig) -> String {
    let t = &ui.theme;
    let l = &ui.layout;
    let b = &ui.behavior;
    let senders = t.sender_colors.iter().map(|c| toml_str(c)).collect::<Vec<_>>().join(", ");
    format!(
"# telaio — interface settings
# Colors: \"#rrggbb\" or names (black, red, green, yellow, blue, magenta, cyan, white, gray)

[theme]
background = {background}
surface    = {surface}   # selected item background
text       = {text}
border     = {border}   # separator lines
muted      = {muted}   # secondary text
timestamp  = {timestamp}
accent     = {accent}   # active panel, selection
unread     = {unread}
error      = {error}
success    = {success}
sender_colors = [{senders}]

[layout]
show_spaces          = {show_spaces}   # space bar visible at start (Ctrl+S)
show_rooms           = {show_rooms}   # room bar visible at start (Ctrl+R)
spaces_width         = {spaces_width}
rooms_width          = {rooms_width}
show_message_preview = {show_message_preview}
home_all_rooms       = {home_all_rooms}   # Home lists every room, not only those outside spaces
border_type          = {border_type}   # popups and errors: plain | rounded | double | thick
time_format          = {time_format}
icons                = {icons}
group_messages       = {group_messages}

[behavior]
history_limit       = {history_limit}   # messages per history page
status_timeout_secs = {status_timeout_secs}
status_fade_secs    = {status_fade_secs:?}
error_timeout_secs  = {error_timeout_secs}   # 0 = errors stay until Ctrl+E
download_dir        = {download_dir}   # empty = Downloads/Matrix_Download
open_command        = {open_command}   # opens attachments; empty = system default (xdg-open)
max_upload_mb       = {max_upload_mb}
",
        background = toml_str(&t.background),
        surface = toml_str(&t.surface),
        text = toml_str(&t.text),
        border = toml_str(&t.border),
        muted = toml_str(&t.muted),
        timestamp = toml_str(&t.timestamp),
        accent = toml_str(&t.accent),
        unread = toml_str(&t.unread),
        error = toml_str(&t.error),
        success = toml_str(&t.success),
        show_spaces = l.show_spaces,
        show_rooms = l.show_rooms,
        spaces_width = l.spaces_width,
        rooms_width = l.rooms_width,
        show_message_preview = l.show_message_preview,
        home_all_rooms = l.home_all_rooms,
        border_type = toml_str(&l.border_type),
        time_format = toml_str(&l.time_format),
        icons = l.icons,
        group_messages = l.group_messages,
        history_limit = b.history_limit,
        status_timeout_secs = b.status_timeout_secs,
        status_fade_secs = b.status_fade_secs,
        error_timeout_secs = b.error_timeout_secs,
        download_dir = toml_str(&b.download_dir),
        open_command = toml_str(&b.open_command),
        max_upload_mb = b.max_upload_mb,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_roundtrip() {
        let ui: UiConfig = toml::from_str(&ui_template(&UiConfig::default())).unwrap();
        assert_eq!(ui.layout.rooms_width, LayoutConfig::default().rooms_width);
        assert_eq!(ui.theme.background, "#0f0f0f");
        assert_eq!(ui.theme.sender_colors.len(), 6);
    }

    #[test]
    fn download_and_open_defaults() {
        let b = BehaviorConfig::default();
        assert!(b.download_dir().ends_with(DOWNLOAD_SUBDIR));
        assert_eq!(BehaviorConfig { download_dir: "/tmp/x".into(), ..b.clone() }.download_dir(), PathBuf::from("/tmp/x"));
        assert_eq!(BehaviorConfig { open_command: "mpv --fs".into(), ..b }.open_command(), ["mpv", "--fs"]);
    }

    #[test]
    fn account_example_parses() {
        // the untouched template does not count as an account
        assert!(parse_accounts(ACCOUNT_EXAMPLE).unwrap().is_empty());
    }

    #[test]
    fn several_accounts() {
        let raw = "homeserver = \"https://a.org\"\nusername = \"@me:a.org\"\n\n\
                   [[account]]\nhomeserver = \"https://b.org\"\nusername = \"@me:b.org\"\npassword = \"x\"\n";
        let accs = parse_accounts(raw).unwrap();
        assert_eq!(accs.len(), 2);
        assert_eq!(accs[1].username, "@me:b.org");
        assert_eq!(accs[1].password.as_deref(), Some("x"));
        // only [[account]] tables, nothing at the top level
        let only = parse_accounts("[[account]]\nusername = \"@c:c.org\"\n").unwrap();
        assert_eq!(only.len(), 1);
    }
}
