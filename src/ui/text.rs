//! Measuring, truncating and wrapping text (widths in terminal columns).

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Truncates to `max` columns, appending "…"
pub fn truncate(s: &str, max: usize) -> String {
    if s.width() <= max { return s.to_string(); }
    if max == 0 { return String::new(); }
    let mut out = String::new();
    let mut w = 0;
    for c in s.chars() {
        let cw = c.width().unwrap_or(0);
        if w + cw + 1 > max { break; }
        out.push(c);
        w += cw;
    }
    out.push('…');
    out
}

/// Word wrap at `width`; words that are too long are split
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = Vec::new();
    for raw in text.split('\n') {
        let raw = raw.replace('\t', "    ");
        let raw: String = raw.chars().filter(|c| !c.is_control()).collect();
        let mut line = String::new();
        let mut w = 0;
        for word in raw.split(' ') {
            let ww = word.width();
            let sep = usize::from(!line.is_empty());
            if w + sep + ww <= width {
                if sep == 1 { line.push(' '); }
                line.push_str(word);
                w += sep + ww;
                continue;
            }
            if !line.is_empty() {
                out.push(std::mem::take(&mut line));
                w = 0;
            }
            for c in word.chars() {
                let cw = c.width().unwrap_or(0);
                if w + cw > width {
                    out.push(std::mem::take(&mut line));
                    w = 0;
                }
                line.push(c);
                w += cw;
            }
        }
        out.push(line);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapping() {
        assert_eq!(wrap("hi to all of you", 8), ["hi to", "all of", "you"]);
        assert_eq!(wrap("abcdefghij", 4), ["abcd", "efgh", "ij"]);
        assert_eq!(wrap("line1\nline2", 20), ["line1", "line2"]);
        assert!(wrap("naïve café déjà vu", 6).iter().all(|l| l.width() <= 6));
    }

    #[test]
    fn truncating() {
        assert_eq!(truncate("abc", 5), "abc");
        assert_eq!(truncate("àèìòù", 3), "àè…");
        assert_eq!(truncate("abc", 0), "");
    }
}
