//! UI colors, resolved once at startup.

use ratatui::style::Color;
use ratatui::widgets::BorderType;

use crate::config::UiConfig;

#[derive(Debug, Clone)]
pub struct Theme {
    pub background: Color,
    /// Selected item background
    pub surface: Color,
    pub text: Color,
    /// Column separators
    pub border: Color,
    pub muted: Color,
    pub timestamp: Color,
    pub accent: Color,
    pub unread: Color,
    pub error: Color,
    pub success: Color,
    pub sender_colors: Vec<Color>,
    pub border_type: BorderType,
}

impl Theme {
    pub fn from_config(ui: &UiConfig) -> Self {
        let t = &ui.theme;
        let mut sender_colors: Vec<Color> = t.sender_colors.iter().map(|c| parse_color(c)).collect();
        if sender_colors.is_empty() {
            sender_colors.push(parse_color(&t.accent));
        }
        let border_type = match ui.layout.border_type.to_lowercase().as_str() {
            "plain" => BorderType::Plain,
            "double" => BorderType::Double,
            "thick" => BorderType::Thick,
            _ => BorderType::Rounded,
        };
        Self {
            background: parse_color(&t.background),
            surface: parse_color(&t.surface),
            text: parse_color(&t.text),
            border: parse_color(&t.border),
            muted: parse_color(&t.muted),
            timestamp: parse_color(&t.timestamp),
            accent: parse_color(&t.accent),
            unread: parse_color(&t.unread),
            error: parse_color(&t.error),
            success: parse_color(&t.success),
            sender_colors,
            border_type,
        }
    }

    /// Stable color per sender (djb2 hash of the ID)
    pub fn sender_color(&self, user_id: &str) -> Color {
        let h = user_id.bytes().fold(5381usize, |acc, b| acc.wrapping_mul(33).wrapping_add(b as usize));
        self.sender_colors[h % self.sender_colors.len()]
    }
}

/// Parses "#rrggbb" or a color name
pub fn parse_color(s: &str) -> Color {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#') {
        if hex.len() == 6 {
            if let Ok(v) = u32::from_str_radix(hex, 16) {
                return Color::Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8);
            }
        }
    }
    match s.to_lowercase().as_str() {
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" => Color::Magenta,
        "cyan" => Color::Cyan,
        "white" => Color::White,
        "gray" | "grey" => Color::Gray,
        "darkgray" | "darkgrey" => Color::DarkGray,
        _ => Color::Reset,
    }
}

/// Interpolates two RGB colors (t = 1.0 → a, t = 0.0 → b)
pub fn blend(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    match (a, b) {
        (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) => {
            let mix = |x: u8, y: u8| (x as f32 * t + y as f32 * (1.0 - t)).round() as u8;
            Color::Rgb(mix(r1, r2), mix(g1, g2), mix(b1, b2))
        }
        _ => a,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors() {
        assert_eq!(parse_color("#ff0080"), Color::Rgb(255, 0, 128));
        assert_eq!(parse_color("Red"), Color::Red);
        assert_eq!(parse_color("nope"), Color::Reset);
        assert_eq!(blend(Color::Rgb(200, 0, 0), Color::Rgb(0, 0, 0), 0.5), Color::Rgb(100, 0, 0));
    }
}
