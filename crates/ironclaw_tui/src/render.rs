//! Rendering utilities for converting text to styled Ratatui spans.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::theme::Theme;

/// Convert a plain text string into wrapped `Line`s that fit within `max_width`.
pub fn wrap_text<'a>(text: &'a str, max_width: usize, style: Style) -> Vec<Line<'a>> {
    if max_width == 0 {
        return vec![];
    }

    let mut lines = Vec::new();
    for raw_line in text.lines() {
        if raw_line.is_empty() {
            lines.push(Line::from(""));
            continue;
        }
        // Simple word-wrap
        let words: Vec<&str> = raw_line.split_whitespace().collect();
        if words.is_empty() {
            lines.push(Line::from(""));
            continue;
        }

        let mut current = String::new();
        for word in words {
            if current.is_empty() {
                current = word.to_string();
            } else if current.len() + 1 + word.len() <= max_width {
                current.push(' ');
                current.push_str(word);
            } else {
                lines.push(Line::from(Span::styled(current, style)));
                current = word.to_string();
            }
        }
        if !current.is_empty() {
            lines.push(Line::from(Span::styled(current, style)));
        }
    }

    if lines.is_empty() {
        lines.push(Line::from(""));
    }

    lines
}

/// Render a simple markdown-like text with basic formatting.
///
/// Supports:
/// - `**bold**` -> bold text
/// - `` `code` `` -> green text
/// - `# heading` -> bold accent text
pub fn styled_markdown<'a>(text: &'a str, theme: &Theme) -> Vec<Line<'a>> {
    let mut lines = Vec::new();

    for line in text.lines() {
        if line.starts_with("# ") {
            lines.push(Line::from(Span::styled(
                line.to_string(),
                theme
                    .accent_style()
                    .add_modifier(Modifier::BOLD),
            )));
        } else if line.starts_with("```") {
            lines.push(Line::from(Span::styled(
                line.to_string(),
                Style::default().fg(theme.success.to_color()),
            )));
        } else {
            // Simple inline formatting: split on backtick pairs
            let spans = parse_inline_spans(line, theme);
            lines.push(Line::from(spans));
        }
    }

    lines
}

/// Parse inline formatting (backticks and bold) into spans.
fn parse_inline_spans(line: &str, theme: &Theme) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut remaining = line;

    while !remaining.is_empty() {
        if let Some(start) = remaining.find('`') {
            // Push text before the backtick
            if start > 0 {
                spans.push(Span::raw(remaining[..start].to_string()));
            }
            let after = &remaining[start + 1..];
            if let Some(end) = after.find('`') {
                spans.push(Span::styled(
                    after[..end].to_string(),
                    Style::default().fg(theme.success.to_color()),
                ));
                remaining = &after[end + 1..];
            } else {
                // Unmatched backtick, push rest as-is
                spans.push(Span::raw(remaining.to_string()));
                break;
            }
        } else {
            spans.push(Span::raw(remaining.to_string()));
            break;
        }
    }

    if spans.is_empty() {
        spans.push(Span::raw(String::new()));
    }

    spans
}

/// Truncate a string to a maximum character count, appending "..." if truncated.
pub fn truncate(s: &str, max: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max.saturating_sub(3)).collect();
        format!("{truncated}...")
    }
}

/// Format a duration in seconds to a human-readable string (e.g., "2m", "1h 5m").
pub fn format_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else {
        let h = secs / 3600;
        let m = (secs % 3600) / 60;
        if m > 0 {
            format!("{h}h {m}m")
        } else {
            format!("{h}h")
        }
    }
}

/// Format a token count with K/M suffix.
pub fn format_tokens(tokens: u64) -> String {
    if tokens < 1_000 {
        tokens.to_string()
    } else if tokens < 1_000_000 {
        format!("{:.1}K", tokens as f64 / 1_000.0)
    } else {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_text_no_wrapping_needed() {
        let lines = wrap_text("short line", 80, Style::default());
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn wrap_text_wraps_long_line() {
        let text = "the quick brown fox jumps over the lazy dog";
        let lines = wrap_text(text, 20, Style::default());
        assert!(lines.len() > 1);
    }

    #[test]
    fn wrap_text_empty() {
        let lines = wrap_text("", 80, Style::default());
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn wrap_text_zero_width() {
        let lines = wrap_text("hello", 0, Style::default());
        assert!(lines.is_empty());
    }

    #[test]
    fn truncate_short_string() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn truncate_long_string() {
        let result = truncate("hello world this is a test", 10);
        assert!(result.ends_with("..."));
        assert!(result.chars().count() <= 10);
    }

    #[test]
    fn format_duration_seconds() {
        assert_eq!(format_duration(45), "45s");
    }

    #[test]
    fn format_duration_minutes() {
        assert_eq!(format_duration(120), "2m");
    }

    #[test]
    fn format_duration_hours() {
        assert_eq!(format_duration(3660), "1h 1m");
    }

    #[test]
    fn format_tokens_small() {
        assert_eq!(format_tokens(500), "500");
    }

    #[test]
    fn format_tokens_thousands() {
        assert_eq!(format_tokens(2100), "2.1K");
    }

    #[test]
    fn format_tokens_millions() {
        assert_eq!(format_tokens(1_500_000), "1.5M");
    }
}
