// matrix-sdk with crypto enabled produces deeply nested futures
#![recursion_limit = "256"]

mod app;
mod commands;
mod config;
mod keys;
mod matrix;
mod theme;
mod ui;

use std::io;
use std::time::Duration;

use anyhow::Result;
use crossterm::{
    event::{DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, EventStream},
    cursor::SetCursorStyle,
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures::StreamExt;
use matrix_sdk::Client;
use ratatui::{backend::CrosstermBackend, Terminal};
use tokio::sync::mpsc;

use app::{
    event::{AppEvent, Command},
    model::AccountEntry,
    popup::{LoginForm, Popup},
    App,
};
use commands::{Executor, SessionChange};
use config::{AccountConfig, UiConfig};
use matrix::session::Restore;

/// UI timer interval (fades, error expiry, debounced searches)
const TICK: Duration = Duration::from_millis(100);

#[tokio::main]
async fn main() -> Result<()> {
    // ── Config and login, before entering the TUI ─────────────────────────
    let arg = std::env::args().nth(1);
    if arg.as_deref().is_some_and(|a| a.starts_with('-')) {
        eprintln!("Usage: {} [@user:server]\n(with no arguments the last used account is opened)", config::APP_NAME);
        return Ok(());
    }
    let migrated = config::migrate_legacy()?;
    if migrated {
        eprintln!("matrix-tui config and session moved to {}", config::config_dir().display());
    }
    for note in config::ensure_files()? {
        eprintln!("{note}");
    }
    let account = choose_account(arg)?;

    // saved session: go straight in; otherwise log in inside the TUI
    let mut ready = None;
    let mut form = LoginForm::default();
    let mut auto_password = None;
    if let Some(acc) = account {
        eprintln!("Signing in as {}…", acc.username);
        match matrix::session::restore(&acc.username).await? {
            Restore::Ready(client) => ready = Some((client, acc.username)),
            Restore::NeedLogin(note) => {
                form = LoginForm::new(&acc.username, &acc.homeserver, note.map(str::to_owned));
                // password in account.toml: try it without asking
                auto_password = acc.password.filter(|p| !p.is_empty());
            }
        }
    }

    let (ui_cfg, ui_error) = config::load_ui();

    // ── Terminal ───────────────────────────────────────────────────────────
    install_panic_hook();
    enable_raw_mode()?;
    // mouse capture makes the wheel arrive as scroll events instead of arrow keys
    execute!(io::stdout(), EnterAlternateScreen, EnableBracketedPaste, EnableMouseCapture, SetCursorStyle::SteadyBar)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    terminal.clear()?;

    let result = async {
        let (client, username) = match ready {
            Some(r) => r,
            None => match login_screen(&mut terminal, &ui_cfg, form, auto_password).await? {
                Some(r) => r,
                None => return Ok(None),
            },
        };
        config::set_last_account(&username);
        if migrated {
            let c = client.clone();
            tokio::spawn(async move { matrix::session::rename_device(&c).await });
        }
        run_sessions(&mut terminal, client, username, ui_cfg, ui_error).await
    }
    .await;

    restore_terminal();
    terminal.show_cursor()?;
    if let Ok(Some(farewell)) = &result {
        eprintln!("{farewell}");
    }
    result.map(|_| ())
}

/// Account to open: the CLI argument, else the last used, else the first
/// known one. `None` on first start: login happens inside the TUI.
fn choose_account(arg: Option<String>) -> Result<Option<AccountConfig>> {
    let configured = config::load_accounts()?;
    let known = matrix::session::known_accounts(&configured);
    let find = |name: &str| known.iter().find(|a| a.username == name).cloned();
    if let Some(name) = arg {
        // unknown account: the server is derived from the MXID
        return Ok(Some(find(&name).unwrap_or(AccountConfig { username: name, ..Default::default() })));
    }
    Ok(config::last_account().and_then(|name| find(&name)).or_else(|| known.into_iter().next()))
}

/// Other known accounts, for the account switcher
fn other_accounts(current: &str) -> Vec<AccountEntry> {
    let configured = config::load_accounts().unwrap_or_default();
    matrix::session::known_accounts(&configured).into_iter()
        .filter(|a| a.username != current)
        .map(|a| AccountEntry { has_session: matrix::session::has_session(&a.username), username: a.username })
        .collect()
}

enum Outcome {
    Quit,
    Session(SessionChange),
}

/// Runs one session after another until the user quits. Returns an optional
/// message to print once the TUI is closed.
async fn run_sessions(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    mut client: Client,
    mut username: String,
    ui: UiConfig,
    mut ui_error: Option<String>,
) -> Result<Option<String>> {
    let mut switched = false;
    loop {
        let user_id = client.user_id().map(|u| u.to_string()).unwrap_or(username.clone());
        let mut app = App::new(ui.clone(), user_id.clone());
        app.device_id = client.device_id().map(|d| d.to_string()).unwrap_or_default();
        app.accounts = other_accounts(&username);
        if let Some(e) = ui_error.take() { app.push_error(e); }
        if switched { app.set_status(format!("Signed in as {user_id}")); }

        match run(terminal, &mut app, client, &username).await? {
            Outcome::Quit => return Ok(None),
            Outcome::Session(SessionChange::Switch { client: c, username: u }) => {
                config::set_last_account(&u);
                (client, username) = (c, u);
            }
            Outcome::Session(SessionChange::LoggedOut) => {
                // switch to the first other account that opens without a password
                let mut next = None;
                for acc in other_accounts(&username).into_iter().filter(|a| a.has_session) {
                    if let Ok(Restore::Ready(c)) = matrix::session::restore(&acc.username).await {
                        next = Some((c, acc.username));
                        break;
                    }
                }
                let next = match next {
                    Some(n) => Some(n),
                    None => login_screen(terminal, &ui, LoginForm::default(), None).await?,
                };
                let Some((c, u)) = next else {
                    return Ok(Some(format!("Logged out of {user_id}.")));
                };
                config::set_last_account(&u);
                (client, username) = (c, u);
            }
        }
        switched = true;
    }
}

/// Full-screen login (first start, expired session, after logging out of the
/// last account). `None` if the user gives up (Esc, Ctrl+C).
async fn login_screen(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ui: &UiConfig,
    mut form: LoginForm,
    auto_password: Option<String>,
) -> Result<Option<(Client, String)>> {
    let mut app = App::new(ui.clone(), String::new());
    if let Some(password) = auto_password {
        form.busy = true;
        app.pending.push(Command::Login { homeserver: form.server.as_str().to_string(), username: form.user.as_str().to_string(), password });
    }
    app.popup = Some(Popup::Login(form));

    let mut keys = EventStream::new();
    let mut attempt: Option<tokio::task::JoinHandle<(String, Result<Client>)>> = None;
    loop {
        for cmd in app.pending.drain(..) {
            if let Command::Login { homeserver, username, password } = cmd {
                attempt = Some(tokio::spawn(async move {
                    let res = matrix::session::login(&homeserver, &username, &password).await;
                    (username.trim().to_string(), res)
                }));
            }
        }
        // Esc closes the popup: without an account there is nothing else to show
        if app.popup.is_none() { return Ok(None); }
        terminal.draw(|f| ui::render_login(f, &app))?;

        tokio::select! {
            ev = keys.next() => match ev {
                Some(Ok(ev)) => if keys::handle_event(ev, &mut app) { return Ok(None); },
                _ => return Ok(None),
            },
            Some(res) = async { Some(attempt.as_mut()?.await) }, if attempt.is_some() => {
                attempt = None;
                match res {
                    Ok((username, Ok(client))) => return Ok(Some((client, username))),
                    Ok((_, Err(e))) => app.apply(AppEvent::LoginFailed(commands::format_error(&e))),
                    Err(e) => app.apply(AppEvent::LoginFailed(format!("Login interrupted: {e}"))),
                }
            }
        }
    }
}

/// One session: sync, commands and UI of a single account
async fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut App, client: Client, username: &str) -> Result<Outcome> {
    // fresh channels per session: tasks of the previous one can no longer deliver events
    let (tx, mut rx) = mpsc::unbounded_channel::<AppEvent>();
    let (session_tx, mut session_rx) = mpsc::unbounded_channel::<SessionChange>();
    let sync = tokio::spawn(matrix::sync::run(client.clone(), tx.clone()));
    let executor = Executor::new(client, username.to_string(), tx, session_tx, &app.ui);

    let mut keys = EventStream::new();
    let mut ticker = tokio::time::interval(TICK);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut dirty = true;

    let outcome = loop {
        for cmd in app.pending.drain(..) {
            executor.run(cmd);
        }
        if dirty {
            terminal.draw(|f| ui::render(f, app))?;
            dirty = false;
        }

        tokio::select! {
            ev = keys.next() => match ev {
                Some(Ok(ev)) => {
                    if keys::handle_event(ev, app) { break Outcome::Quit; }
                    dirty = true;
                }
                Some(Err(e)) => { app.push_error(format!("Terminal error: {e}")); dirty = true; }
                None => break Outcome::Quit,
            },
            Some(ev) = rx.recv() => {
                app.apply(ev);
                // a single redraw for a burst of events
                while let Ok(ev) = rx.try_recv() { app.apply(ev); }
                dirty = true;
            }
            Some(change) = session_rx.recv() => break Outcome::Session(change),
            _ = ticker.tick() => dirty |= app.tick(),
        }
    };
    sync.abort();
    Ok(outcome)
}

fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(
        io::stdout(),
        SetCursorStyle::DefaultUserShape,
        DisableMouseCapture,
        DisableBracketedPaste,
        LeaveAlternateScreen
    );
}

/// Restores the terminal before the panic message is printed
fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        default(info);
    }));
}
