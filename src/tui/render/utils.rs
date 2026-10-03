//! Shared rendering utilities
//!
//! Text wrapping, character boundary helpers, and token formatting used across render modules.

use ratatui::{
    style::Style,
    text::{Line, Span},
};
use unicode_width::UnicodeWidthStr;
use unicode_bidi::{BidiParagraph, EmbeddingDirection};
use arabic_reshaper::arabic_reshape;

/// Pre-wrap a Line's text content to fit within max_width, preserving the style
/// of the first span and prepending `padding` to each continuation line.
pub(crate) fn wrap_line_with_padding<'a>(
    line: Line<'a>,
    max_width: usize,
    padding: &'a str,
) -> Vec<Line<'a>> {
    if max_width == 0 {
        return vec![line];
    }
    // Use display width (not byte length) for wrapping decisions
    let total_width: usize = line.spans.iter().map(|s| s.content.width()).sum();
    if total_width <= max_width {
        return vec![line];
    }

    let padding_width = padding.width();

    // Collect all text and track style boundaries
    let mut segments: Vec<(String, Style)> = Vec::new();
    for span in &line.spans {
        segments.push((span.content.to_string(), span.style));
    }

    // Build wrapped lines
    let mut result: Vec<Line<'a>> = Vec::new();
    let mut current_spans: Vec<Span<'a>> = Vec::new();
    let mut current_width: usize = 0;

    for (text, style) in segments {
        let mut remaining = text.as_str();
        while !remaining.is_empty() {
            let available = max_width.saturating_sub(current_width);
            if available == 0 {
                result.push(Line::from(current_spans));
                current_spans = vec![Span::styled(padding.to_string(), Style::default())];
                current_width = padding_width;
                continue;
            }

            let remaining_width = remaining.width();
            if remaining_width <= available {
                current_spans.push(Span::styled(remaining.to_string(), style));
                current_width += remaining_width;
                break;
            } else {
                // Find the byte index where cumulative display width reaches `available`
                let byte_limit = char_boundary_at_width(remaining, available);
                // Look for a word break (space) within that range
                let break_at = remaining[..byte_limit]
                    .rfind(' ')
                    .map(|p| p + 1)
                    .unwrap_or(byte_limit);
                let break_at = if break_at == 0 {
                    byte_limit.max(remaining.ceil_char_boundary(1))
                } else {
                    break_at
                };
                let (chunk, rest) = remaining.split_at(break_at);
                current_spans.push(Span::styled(chunk.to_string(), style));
                remaining = rest.trim_start();
                result.push(Line::from(current_spans));
                current_spans = vec![Span::styled(padding.to_string(), Style::default())];
                current_width = padding_width;
            }
        }
    }
    if !current_spans.is_empty() {
        result.push(Line::from(current_spans));
    }
    if result.is_empty() {
        result.push(line);
    }
    result
}

/// Find the byte index in `s` where the cumulative display width first reaches or exceeds `target_width`.
/// Always returns a valid char boundary.
pub(crate) fn char_boundary_at_width(s: &str, target_width: usize) -> usize {
    let mut width = 0;
    for (idx, ch) in s.char_indices() {
        let ch_width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + ch_width > target_width {
            return idx;
        }
        width += ch_width;
    }
    s.len()
}

/// Format token count with a custom label (e.g. "1.2M total", "150K total")
pub(super) fn format_token_count_with_label(tokens: i64, label: &str) -> String {
    let tokens = tokens.max(0) as f64;
    if tokens >= 1_000_000.0 {
        format!("{:.1}M {}", tokens / 1_000_000.0, label)
    } else if tokens >= 1_000.0 {
        format!("{:.1}K {}", tokens / 1_000.0, label)
    } else if tokens > 0.0 {
        format!("{} {}", tokens as i64, label)
    } else {
        "new".to_string()
    }
}

/// Format token count as raw number without label (e.g. "150K", "1.2M")
pub(super) fn format_token_count_raw(tokens: i64) -> String {
    let tokens = tokens.max(0) as f64;
    if tokens >= 1_000_000.0 {
        format!("{:.1}M", tokens / 1_000_000.0)
    } else if tokens >= 1_000.0 {
        format!("{:.0}K", tokens / 1_000.0)
    } else if tokens > 0.0 {
        format!("{}", tokens as i64)
    } else {
        "0".to_string()
    }
}

/// Apply the Unicode BiDi algorithm and Arabic contextual shaping to a line of
/// text so it renders correctly in a terminal.
///
/// Without this, Arabic letters are emitted in logical order with isolated
/// forms, and terminals (and users reading them) show reversed or unjoined text.
/// `unicode-bidi` reorders the runs into visual order; `unicode-arabic-shaping`
/// converts the presentation forms back into properly joined Arabic.
///
/// Latin-only text is returned unchanged and untouched, so this is cheap to
/// call on every rendered line.
pub(super) fn apply_rtl(text: &str) -> String {
    // Fast path: pure ASCII/Latin needs no bidi or shaping work at all.
    if !text.chars().any(|c| is_rtl_char(c)) {
        return text.to_string();
    }

    // 1. Shape: turn isolated Arabic presentation forms into joined forms.
    //    We shape before reordering because the joining algorithm is defined on
    //    logical order, and reordering first would break the context windows.
    let shaped = if text.chars().any(is_arabic_char) {
        arabic_reshape(text)
            .chars()
            .filter(|c| !is_arabic_presentation_form(*c))
            .collect::<String>()
    } else {
        text.to_string()
    };

    // 2. Reorder: embed the paragraph in an RTL base direction and take the
    //    visual-order result. Levels are stripped because the terminal applies
    //    its own bidi handling for the characters it actually receives.
    let paragraph = BidiParagraph::new(&shaped, Some(EmbeddingDirection::RightToLeft));
    let visual: String = paragraph.visual_runs().flatten().collect();

    // 3. Reinsert the bidi control characters so the terminal does not run its
    //    own pass and undo what we just did.
    visual
}

/// True when the character belongs to a right-to-left script.
fn is_rtl_char(c: char) -> bool {
    matches!(c,
        '\u{0590}'..='\u{05FF}'   // Hebrew
        | '\u{0600}'..='\u{06FF}' // Arabic
        | '\u{0700}'..='\u{074F}' // Syriac
        | '\u{0750}'..='\u{077F}' // Arabic Supplement
        | '\u{08A0}'..='\u{08FF}' // Arabic Extended-A
        | '\u{FB1D}'..='\u{FDFF}' // Hebrew/Arabic presentation forms
        | '\u{FE70}'..='\u{FEFF}' // Arabic presentation forms-B
        | '\u{200F}'              // RIGHT-TO-LEFT MARK
        | '\u{200E}'              // LEFT-TO-RIGHT MARK
        | '\u{202B}'..='\u{202E}' // embedding/override
        | '\u{2066}'..='\u{2069}' // isolates
    )
}

/// True when the character is an Arabic-script letter or mark.
fn is_arabic_char(c: char) -> bool {
    matches!(c, '\u{0600}'..='\u{06FF}' | '\u{0750}'..='\u{077F}' | '\u{08A0}'..='\u{08FF}')
}

/// Arabic presentation forms that the shaper introduced and that must be
/// stripped before the terminal re-shapes them.
fn is_arabic_presentation_form(c: char) -> bool {
    matches!(c, '\u{FB50}'..='\u{FDFF}' | '\u{FE70}'..='\u{FEFF}')
}
