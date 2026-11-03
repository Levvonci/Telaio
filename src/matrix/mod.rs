//! The only place that talks to matrix-sdk.

pub mod actions;
pub mod address;
pub mod convert;
pub mod history;
pub mod search;
pub mod session;
pub mod sync;
pub mod verify;

use anyhow::{Context, Result};
use matrix_sdk::{ruma::RoomId, Client, Room};
use tokio::sync::mpsc::UnboundedSender;

use crate::app::event::AppEvent;

pub type Tx = UnboundedSender<AppEvent>;

fn get_room(client: &Client, room_id: &str) -> Result<Room> {
    let rid = RoomId::parse(room_id).context("Invalid room ID")?;
    client.get_room(&rid).context("Room not found")
}

pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 { format!("{bytes} B") } else { format!("{v:.1} {}", UNITS[u]) }
}

/// Compact error message (first line, at most 200 characters)
pub fn short_error(e: &dyn std::fmt::Display) -> String {
    let s = e.to_string();
    s.lines().next().unwrap_or(&s).chars().take(200).collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn sizes() {
        assert_eq!(super::human_size(512), "512 B");
        assert_eq!(super::human_size(1536), "1.5 KB");
    }
}
