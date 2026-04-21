use std::sync::OnceLock;
use std::time::Instant;

use ratatui::{Frame, layout::Rect, widgets::Paragraph};

use crate::payload::{Body, LinesData};

use super::{RenderOptions, Renderer, Shape};

const DEFAULT_CHARS_PER_SECOND: f64 = 40.0;

/// Typewriter-style progressive reveal over the `Lines` shape. Each frame shows
/// `elapsed * chars_per_second` characters of the payload, truncating to whatever has "typed"
/// by now. Only animates inside a render loop — on cached fast-path draws it appears fully
/// typed, which is intentional (we don't slow repeat invocations for effect).
pub struct AnimatedTypewriterRenderer;

impl Renderer for AnimatedTypewriterRenderer {
    fn name(&self) -> &str {
        "animated_typewriter"
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::Lines]
    }
    fn animates(&self) -> bool {
        true
    }
    fn render(&self, frame: &mut Frame, area: Rect, body: &Body, _opts: &RenderOptions) {
        if let Body::Lines(d) = body {
            render_typewriter(frame, area, d);
        }
    }
}

fn render_typewriter(frame: &mut Frame, area: Rect, data: &LinesData) {
    let total: usize = data.lines.iter().map(|l| l.chars().count() + 1).sum();
    let budget = chars_revealed(total);
    let revealed = reveal(&data.lines, budget);
    frame.render_widget(Paragraph::new(revealed), area);
}

/// Characters that should be visible given the time elapsed since render loop start.
fn chars_revealed(total: usize) -> usize {
    let elapsed = process_start().elapsed().as_secs_f64();
    let budget = (elapsed * DEFAULT_CHARS_PER_SECOND).floor() as usize;
    budget.min(total)
}

fn reveal(lines: &[String], budget: usize) -> String {
    let mut remaining = budget;
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            if remaining == 0 {
                break;
            }
            out.push('\n');
            remaining -= 1;
        }
        let take = remaining.min(line.chars().count());
        out.extend(line.chars().take(take));
        remaining -= take;
        if remaining == 0 {
            break;
        }
    }
    out
}

/// First-caller-wins timestamp. Shared across renderer invocations so a multi-frame loop sees
/// monotonically increasing elapsed time.
fn process_start() -> Instant {
    static START: OnceLock<Instant> = OnceLock::new();
    *START.get_or_init(Instant::now)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reveal_truncates_at_budget() {
        let lines = vec!["hello".to_string(), "world".to_string()];
        assert_eq!(reveal(&lines, 3), "hel");
        assert_eq!(reveal(&lines, 5), "hello");
        assert_eq!(reveal(&lines, 6), "hello\n");
        assert_eq!(reveal(&lines, 11), "hello\nworld");
        assert_eq!(reveal(&lines, 99), "hello\nworld");
    }

    #[test]
    fn empty_budget_yields_empty_string() {
        let lines = vec!["x".to_string()];
        assert_eq!(reveal(&lines, 0), "");
    }
}
