//! Parsing of user-typed addresses (pure functions).

use anyhow::{bail, Context, Result};
use matrix_sdk::ruma::{OwnedRoomOrAliasId, OwnedServerName, OwnedUserId, RoomOrAliasId, UserId};

/// Parses a room or space address. Accepts `#alias:server`, `!id:server`,
/// `https://matrix.to/#/...` links and `matrix:` URIs, with optional `?via=server`.
pub fn parse_room_address(input: &str) -> Result<(OwnedRoomOrAliasId, Vec<OwnedServerName>)> {
    let input = input.trim();
    let rest = strip_matrix_to(input);
    let (id_part, query) = rest.split_once('?').unwrap_or((rest, ""));

    let id_str = if let Some(uri) = id_part.strip_prefix("matrix:") {
        // matrix:r/alias:server or matrix:roomid/id:server (a trailing /e/event is ignored)
        let mut parts = uri.split('/');
        match (parts.next(), parts.next()) {
            (Some("r"), Some(a)) => format!("#{a}"),
            (Some("roomid"), Some(r)) => format!("!{r}"),
            _ => bail!("Unrecognized matrix: URI"),
        }
    } else {
        id_part.split('/').next().unwrap_or(id_part).to_string() // drop /$event
    };
    let id_str = percent_decode(&id_str);

    let id = RoomOrAliasId::parse(&id_str)
        .with_context(|| format!("“{id_str}” is not a valid address (e.g. #room:server or !id:server)"))?;

    let mut via: Vec<OwnedServerName> = query
        .split('&')
        .filter_map(|kv| kv.strip_prefix("via="))
        .filter_map(|s| s.parse().ok())
        .collect();
    if let Some(server) = id.server_name() {
        if !via.iter().any(|v| v == server) {
            via.push(server.to_owned());
        }
    }
    Ok((id, via))
}

/// Recognizes a user MXID, also as a matrix.to link or `matrix:u/` URI
pub fn parse_user_id(input: &str) -> Option<OwnedUserId> {
    let rest = strip_matrix_to(input.trim());
    let rest = rest.split('?').next().unwrap_or(rest);
    let id = match rest.strip_prefix("matrix:u/") {
        Some(u) => format!("@{u}"),
        None => rest.to_string(),
    };
    UserId::parse(percent_decode(&id)).ok()
}

fn strip_matrix_to(s: &str) -> &str {
    s.strip_prefix("https://matrix.to/#/")
        .or_else(|| s.strip_prefix("http://matrix.to/#/"))
        .unwrap_or(s)
}

fn percent_decode(s: &str) -> String {
    s.replace("%23", "#").replace("%21", "!").replace("%40", "@").replace("%3A", ":").replace("%3a", ":")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn room_addresses() {
        let (id, via) = parse_room_address("#rust:matrix.org").unwrap();
        assert_eq!(id.as_str(), "#rust:matrix.org");
        assert_eq!(via[0].as_str(), "matrix.org");

        let (id, via) = parse_room_address("https://matrix.to/#/!abc:example.org?via=matrix.org&via=example.org").unwrap();
        assert_eq!(id.as_str(), "!abc:example.org");
        assert_eq!(via.len(), 2);

        let (id, _) = parse_room_address("https://matrix.to/#/%23thesis%3Aexample.org").unwrap();
        assert_eq!(id.as_str(), "#thesis:example.org");

        let (id, _) = parse_room_address("matrix:r/rust:matrix.org").unwrap();
        assert_eq!(id.as_str(), "#rust:matrix.org");

        assert!(parse_room_address("not an address").is_err());
        assert!(parse_room_address("rust").is_err());
    }

    #[test]
    fn user_ids() {
        assert_eq!(parse_user_id("@alice:example.org").unwrap().as_str(), "@alice:example.org");
        assert_eq!(parse_user_id("https://matrix.to/#/@bob:matrix.org").unwrap().as_str(), "@bob:matrix.org");
        assert_eq!(parse_user_id("matrix:u/carl:x.org").unwrap().as_str(), "@carl:x.org");
        assert!(parse_user_id("alice").is_none());
    }
}
