/// Single-line text field; the cursor is a char index (Unicode-safe)
#[derive(Default, Clone)]
pub struct LineInput {
    text: String,
    cursor: usize,
}

impl LineInput {
    pub fn as_str(&self) -> &str { &self.text }
    pub fn is_empty(&self) -> bool { self.text.is_empty() }
    pub fn cursor(&self) -> usize { self.cursor }

    fn byte_idx(&self, char_idx: usize) -> usize {
        self.text.char_indices().nth(char_idx).map(|(i, _)| i).unwrap_or(self.text.len())
    }
    fn len(&self) -> usize { self.text.chars().count() }

    pub fn insert(&mut self, c: char) {
        let i = self.byte_idx(self.cursor);
        self.text.insert(i, c);
        self.cursor += 1;
    }
    pub fn insert_str(&mut self, s: &str) {
        for c in s.chars().filter(|c| !c.is_control()) {
            self.insert(c);
        }
    }
    /// Copy shown in place of a secret: one dot per character
    pub fn masked(&self) -> LineInput {
        LineInput { text: "•".repeat(self.len()), cursor: self.cursor }
    }
    pub fn set(&mut self, s: &str) {
        self.text = s.to_string();
        self.cursor = self.len();
    }
    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            let i = self.byte_idx(self.cursor);
            self.text.remove(i);
        }
    }
    pub fn delete(&mut self) {
        if self.cursor < self.len() {
            let i = self.byte_idx(self.cursor);
            self.text.remove(i);
        }
    }
    pub fn left(&mut self) { self.cursor = self.cursor.saturating_sub(1); }
    pub fn right(&mut self) { self.cursor = (self.cursor + 1).min(self.len()); }
    pub fn home(&mut self) { self.cursor = 0; }
    pub fn end(&mut self) { self.cursor = self.len(); }
    /// Ctrl+W: deletes the word before the cursor
    pub fn delete_word(&mut self) {
        let chars: Vec<char> = self.text.chars().collect();
        let mut start = self.cursor;
        while start > 0 && chars[start - 1].is_whitespace() { start -= 1; }
        while start > 0 && !chars[start - 1].is_whitespace() { start -= 1; }
        let (a, b) = (self.byte_idx(start), self.byte_idx(self.cursor));
        self.text.replace_range(a..b, "");
        self.cursor = start;
    }
    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_input_unicode() {
        let mut li = LineInput::default();
        li.insert_str("café é");
        li.left();
        li.backspace();
        assert_eq!(li.as_str(), "caféé");
        li.home();
        li.delete();
        assert_eq!(li.as_str(), "aféé");
        li.end();
        li.insert_str(" nice");
        li.delete_word();
        assert_eq!(li.as_str(), "aféé ");
    }
}
