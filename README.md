# Telaio

A minimal Matrix client for the terminal, written in Rust.

*Telaio* is Italian for *loom*: the frame that weaves warp and weft into
cloth. Matrix is a federated network of conversations; this is a small tool
to weave them together in a terminal window.

```
 Spaces              │ Home                        │ # Thesis — telaio  ✓ encrypted  Talking about the…   12 members
                     │                             │───────────────────────────────────────────────────────────────────
▌⌂ Home            ● │ PEOPLE                      │                     start of the conversation
 ◆ University        │ @ Supervisor              1 │──────────────────────────────── Today ────────────────────────────
                     │                             │ bob  00:05
                     │ ROOMS                       │   I uploaded the new draft, let me know what you think when you
                     │▌# Thesis — telaio           │   have time.
                     │   * alice approves    01:03 │   Especially chapter 3
                     │ # Rust Users              3 │
                     │                             │▌me  00:35
                     │                             │   ▤ chapter3.pdf
                     │                             │   file · 1.2 MB · application/pdf   Enter open · s save
                     │                             │
                     │                             │ * alice approves  01:03
                     │                             │───────────────────────────────────────────────────────────────────
                     │                             │ NORMAL  @me › press i to write · F1 help
```

It started as a university thesis project. The goal is not to replicate every
feature of Element, but to cover what is needed day to day — rooms, spaces,
direct messages, attachments, end-to-end encryption — with a quiet interface
and vim-style keys.

## What it does

- **End-to-end encryption** via matrix-sdk. Rooms created from Telaio are
  encrypted; the room header always shows whether the current room is.
- **Session verification** with emojis (SAS) against your other devices, or
  with your recovery key. Messages that arrived before the keys are decrypted
  in place once the keys show up.
- **Spaces**: browse a space's rooms, including the ones you haven't joined
  yet; create spaces; add or remove rooms from them.
- **Attachments**: send any file (with path completion), open received ones
  with the system default application, or save a copy.
- **Join anything by address**: `#alias:server`, `!id:server`, `matrix.to`
  links, `matrix:` URIs, or search the server directory.
- **Direct messages** with user directory search; **invites** shown at the top
  of Home.
- **Moderation**: kick, ban, unban, topic and name changes, message deletion —
  with permission checks before the request is sent.
- **Multiple accounts**, one at a time, switchable without restarting.
- **Mouse support**: scroll wheel and clicks; `Shift`+drag still selects text.

## Installing

Requires Rust 1.96 or newer.

```bash
git clone https://github.com/Levvonci/Telaio.git
cd Telaio
cargo install --path . --locked
telaio
```

The first build takes a while: matrix-sdk and its bundled SQLite are large.

## First run

Telaio opens a login screen asking for your user ID (`@you:server`), an
optional homeserver (otherwise discovered from the user ID) and your password.
After that the session is stored locally and later starts go straight to your
rooms, resuming the sync where it stopped.

If the session is not verified yet, the input line shows `⚠ unverified (F2)`.
Press `F2` and either compare emojis with another logged-in device (Element,
for instance) or paste your recovery key.

To open a specific account: `telaio @you:server`.

## Using it

The interface has two modes, like vim. The current one is shown at the start
of the input line.

- **NORMAL** — keys are commands. `Tab` moves between the space bar, the room
  list and the messages; `j`/`k` or the arrows move inside a panel.
- **INSERT** — keys go into the message field. Enter it with `i`, `Enter` on a
  room, or a click on the input line; leave it with `Esc`.

| Key                 | Action                                                    |
|---------------------|-----------------------------------------------------------|
| `i` / `Esc`         | insert / normal mode                                      |
| `Tab` / `Shift+Tab` | next / previous panel                                     |
| `Ctrl+S` / `Ctrl+R` | show or hide the space bar / room list                    |
| `Ctrl+K`            | jump to any room by name                                  |
| `a`                 | join a room or space (address or directory search)        |
| `d`                 | new direct chat                                           |
| `n`                 | new space (in the space bar) / new room (in the room list)|
| `m`                 | add or remove the current room from your spaces           |
| `I` / `L`           | invite someone / leave the room or space                  |
| `Ctrl+O`, `f`       | attach a file                                             |
| `↑` `↓`             | select a message, also while typing                       |
| `Enter` / `s`       | open / save the selected attachment                       |
| `x`                 | delete the selected message                               |
| `F2`                | account: verification, recovery key, switch, log out      |
| `F1`, `?`           | all keys                                                  |
| `Ctrl+C`            | quit                                                      |

Commands typed in the input line: `/file`, `/join`, `/dm`, `/invite`,
`/leave`, `/kick @user [reason]`, `/ban`, `/unban`, `/topic text`,
`/name text`, `/verify`, `/recovery`, `/account`, `/help`. Users can be given
by MXID or display name. Messages support Markdown; start with `//` to send
text that begins with `/`.

## Configuration

Nothing needs to be edited by hand. Two files are created in
`~/.config/telaio/`:

- **`account.toml`** — optional list of accounts (permissions `600`). A
  password here allows an unattended first login; it can be removed afterwards.

  ```toml
  homeserver = "https://matrix.org"
  username   = "@you:matrix.org"

  [[account]]
  homeserver = "https://example.org"
  username   = "@work:example.org"
  ```

- **`ui.toml`** — colors, column widths, time format, Unicode icons on/off,
  message grouping, timeouts, download folder and the program used to open
  attachments. Every option is listed with its default in the generated file.

Attachments are downloaded to `Matrix_Download` inside your system Downloads
folder. Opening the same attachment twice reuses the file already there.
By default files are opened with `xdg-open` (`open` on macOS); set
`open_command` to use something else, e.g. `"mpv"`.

Session tokens and the encrypted SQLite store live in
`~/.local/share/telaio/<user>/`. Logging out from `F2` deletes them.

## Known limitations

- Emoji verification only between your own devices, through to-device
  requests: no QR codes, no in-room verification, no verifying other users.
- Cross-signing and key backup must be set up with another client first;
  Telaio uses them but doesn't create them.
- No replies, edits, reactions, threads, typing indicators or read markers
  from other people yet.
- Single-line input.

## Development

```bash
cargo build
cargo test                                    # unit and rendering tests
cargo clippy --all-targets                    # kept warning-free
cargo test preview -- --ignored --nocapture   # print the UI with sample data
```

The rendering tests draw the interface on ratatui's `TestBackend` with sample
data and simulated key and mouse events, so most of the UI can be checked
without a homeserver. To try login and configuration without touching your
real files:

```bash
XDG_CONFIG_HOME=target/x/cfg XDG_DATA_HOME=target/x/data cargo run
```

### How it is put together

```
src/
  main.rs       account selection, terminal setup, session loop
  commands.rs   runs each Command in a tokio task, reports back as AppEvent
  keys.rs       keyboard, paste and mouse → state changes and Commands
  config.rs     account.toml and ui.toml
  theme.rs      color palette, resolved once
  app/          application state, no terminal or network code
  matrix/       the only module that talks to matrix-sdk
  ui/           ratatui rendering, one file per area, plus tests
```

The UI never waits on the network. Key handlers push `Command`s; the main loop
hands them to background tasks; results come back as `AppEvent`s on a channel
and are applied to the state in one place (`App::apply`). The screen is redrawn
only when something changed, once per burst of events, and the message view
builds only the lines that fit on screen.

## License

Telaio is free software, released under the
[GNU General Public License v3.0 or later](LICENSE).
