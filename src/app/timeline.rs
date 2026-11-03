//! Messages of a room, sorted and deduplicated by event ID.

use std::collections::HashSet;

use super::model::{Content, Message};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HistoryState {
    NotLoaded,
    Loading,
    /// Older messages are still available
    Partial,
    /// Reached the start of the room
    Complete,
}

#[derive(Debug)]
pub struct Timeline {
    pub messages: Vec<Message>,
    seen: HashSet<String>,
    pub prev_token: Option<String>,
    pub history: HistoryState,
}

impl Default for Timeline {
    fn default() -> Self {
        Self { messages: Vec::new(), seen: HashSet::new(), prev_token: None, history: HistoryState::NotLoaded }
    }
}

impl Timeline {
    /// Adds a live message. Returns false if it was already present.
    pub fn push(&mut self, msg: Message) -> bool {
        if !self.seen.insert(msg.event_id.clone()) {
            return false;
        }
        // usually the newest message: O(1) append
        let pos = self.messages.partition_point(|m| m.timestamp <= msg.timestamp);
        self.messages.insert(pos, msg);
        true
    }

    /// Inserts a history page; returns how many messages were new.
    pub fn add_history(&mut self, page: Vec<Message>, prev_token: Option<String>) -> usize {
        let before = self.messages.len();
        for m in page {
            if self.seen.insert(m.event_id.clone()) {
                self.messages.push(m);
            }
        }
        let added = self.messages.len() - before;
        if added > 0 {
            // stable sort: fast on nearly sorted data
            self.messages.sort_by_key(|m| m.timestamp);
        }
        self.history = if prev_token.is_some() { HistoryState::Partial } else { HistoryState::Complete };
        self.prev_token = prev_token;
        added
    }

    pub fn last(&self) -> Option<&Message> { self.messages.last() }

    /// Replaces a message after a retried decryption (`None` removes it);
    /// returns its position
    pub fn replace(&mut self, event_id: &str, msg: Option<Message>) -> Option<usize> {
        let pos = self.messages.iter().position(|m| m.event_id == event_id)?;
        match msg {
            Some(m) => self.messages[pos] = m,
            None => { self.messages.remove(pos); }
        }
        Some(pos)
    }

    /// JSON of events still encrypted, only for the given sessions (`None` = all)
    pub fn undecrypted(&self, sessions: Option<&HashSet<&str>>) -> Vec<String> {
        self.messages.iter()
            .filter_map(|m| match &m.content {
                Content::Encrypted(u) if sessions.is_none_or(|s| u.session_id.as_deref().is_some_and(|id| s.contains(id))) => {
                    Some(u.raw.clone())
                }
                _ => None,
            })
            .collect()
    }

    /// Removes a redacted message; returns its position
    pub fn remove(&mut self, event_id: &str) -> Option<usize> {
        let pos = self.messages.iter().position(|m| m.event_id == event_id)?;
        self.messages.remove(pos);
        Some(pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::model::Undecrypted;
    use chrono::DateTime;

    fn msg(id: &str, secs: i64) -> Message {
        Message {
            event_id: id.into(),
            sender: "@a:b".into(),
            sender_name: "a".into(),
            timestamp: DateTime::from_timestamp(secs, 0).unwrap(),
            content: Content::Text(id.into()),
        }
    }

    #[test]
    fn timeline_dedup_and_order() {
        let mut tl = Timeline::default();
        assert!(tl.push(msg("c", 30)));
        assert!(!tl.push(msg("c", 30)));
        let added = tl.add_history(vec![msg("a", 10), msg("b", 20), msg("c", 30)], Some("tok".into()));
        assert_eq!(added, 2);
        let ids: Vec<_> = tl.messages.iter().map(|m| m.event_id.as_str()).collect();
        assert_eq!(ids, ["a", "b", "c"]);
        assert_eq!(tl.history, HistoryState::Partial);
    }

    #[test]
    fn redecrypt_targets() {
        let mut tl = Timeline::default();
        let mut enc = |id: &str, session: &str, secs| {
            let mut m = msg(id, secs);
            m.content = Content::Encrypted(Undecrypted { session_id: Some(session.into()), raw: format!("raw-{id}") });
            tl.push(m);
        };
        enc("x", "s1", 10);
        enc("y", "s2", 20);
        tl.push(msg("z", 30));
        assert_eq!(tl.undecrypted(None), ["raw-x", "raw-y"]);
        assert_eq!(tl.undecrypted(Some(&["s2"].into())), ["raw-y"]);
        // decrypted: replaces the placeholder; an edit disappears
        assert_eq!(tl.replace("x", Some(msg("x", 10))), Some(0));
        assert_eq!(tl.replace("y", None), Some(1));
        assert!(tl.undecrypted(None).is_empty());
        assert_eq!(tl.messages.len(), 2);
    }
}
