//! `quote_of_day` — a bundled daily quote. Selection is deterministic per calendar day
//! (day-of-year modulo the built-in quote count), so every `splashboard` invocation on
//! the same day sees the same line and the quote advances automatically at midnight.
//! Zero-config, zero I/O, infallible — suitable for realtime dispatch.

use chrono::{DateTime, Datelike, FixedOffset, Local};

use crate::fetcher::{FetchContext, RealtimeFetcher, Safety};
use crate::payload::{Body, Payload, TextBlockData, TextData};
use crate::render::Shape;
use crate::samples;

const SHAPES: &[Shape] = &[Shape::TextBlock, Shape::Text];

/// Quote / author pairs. Classic & well-known sources: public domain or commonly
/// attributed. Ordering is stable — changing this array shifts which quote runs on
/// a given date, so append (don't reorder) when expanding.
const QUOTES: &[(&str, &str)] = &[
    (
        "The only way to do great work is to love what you do.",
        "Steve Jobs",
    ),
    ("Make it work, make it right, make it fast.", "Kent Beck"),
    (
        "Simplicity is the ultimate sophistication.",
        "Leonardo da Vinci",
    ),
    (
        "The best way to predict the future is to invent it.",
        "Alan Kay",
    ),
    (
        "Any fool can know. The point is to understand.",
        "Albert Einstein",
    ),
    ("Stay hungry, stay foolish.", "Stewart Brand"),
    ("Talk is cheap. Show me the code.", "Linus Torvalds"),
    (
        "The pessimist complains about the wind; the optimist expects it to change; the realist adjusts the sails.",
        "William Arthur Ward",
    ),
    (
        "First, solve the problem. Then, write the code.",
        "John Johnson",
    ),
    (
        "Code is like humor. When you have to explain it, it's bad.",
        "Cory House",
    ),
    (
        "Experience is the name everyone gives to their mistakes.",
        "Oscar Wilde",
    ),
    (
        "A journey of a thousand miles begins with a single step.",
        "Lao Tzu",
    ),
    ("The unexamined life is not worth living.", "Socrates"),
    (
        "Life is what happens when you're busy making other plans.",
        "John Lennon",
    ),
    (
        "Action is the foundational key to all success.",
        "Pablo Picasso",
    ),
    (
        "It always seems impossible until it's done.",
        "Nelson Mandela",
    ),
    (
        "The best error message is the one that never shows up.",
        "Thomas Fuchs",
    ),
    (
        "Perfection is achieved, not when there is nothing more to add, but when there is nothing left to take away.",
        "Antoine de Saint-Exupéry",
    ),
    (
        "Premature optimization is the root of all evil.",
        "Donald Knuth",
    ),
    (
        "Walking on water and developing software from a specification are easy if both are frozen.",
        "Edward V. Berard",
    ),
    (
        "Computers are useless. They can only give you answers.",
        "Pablo Picasso",
    ),
    (
        "The best thing about a boolean is even if you are wrong, you are only off by a bit.",
        "Anonymous",
    ),
    (
        "Programs must be written for people to read, and only incidentally for machines to execute.",
        "Harold Abelson",
    ),
    (
        "Good code is its own best documentation.",
        "Steve McConnell",
    ),
    ("Fall seven times, stand up eight.", "Japanese proverb"),
    (
        "Be yourself; everyone else is already taken.",
        "Oscar Wilde",
    ),
    ("Don't count the days, make the days count.", "Muhammad Ali"),
    (
        "What you do speaks so loudly that I cannot hear what you say.",
        "Ralph Waldo Emerson",
    ),
    (
        "The secret of getting ahead is getting started.",
        "Mark Twain",
    ),
    (
        "Whether you think you can or you think you can't, you're right.",
        "Henry Ford",
    ),
    (
        "It's not a bug — it's an undocumented feature.",
        "Anonymous",
    ),
    (
        "In theory, there is no difference between theory and practice. In practice, there is.",
        "Yogi Berra",
    ),
    (
        "The way to get started is to quit talking and begin doing.",
        "Walt Disney",
    ),
    (
        "Your time is limited, so don't waste it living someone else's life.",
        "Steve Jobs",
    ),
    (
        "Everything should be made as simple as possible, but not simpler.",
        "Albert Einstein",
    ),
    (
        "The function of good software is to make the complex appear to be simple.",
        "Grady Booch",
    ),
    (
        "Debugging is twice as hard as writing the code in the first place.",
        "Brian Kernighan",
    ),
    ("Deleted code is debugged code.", "Jeff Sickel"),
    (
        "There are only two hard things in computer science: cache invalidation and naming things.",
        "Phil Karlton",
    ),
    (
        "We are what we repeatedly do. Excellence, then, is not an act, but a habit.",
        "Aristotle",
    ),
    (
        "You miss 100% of the shots you don't take.",
        "Wayne Gretzky",
    ),
    (
        "The only impossible journey is the one you never begin.",
        "Tony Robbins",
    ),
    (
        "Happiness is not something ready made. It comes from your own actions.",
        "Dalai Lama",
    ),
    ("Measure twice, cut once.", "Carpenter's proverb"),
    (
        "If you can't explain it simply, you don't understand it well enough.",
        "Albert Einstein",
    ),
    (
        "The only limit to our realization of tomorrow is our doubts of today.",
        "Franklin D. Roosevelt",
    ),
    (
        "Rest when you're weary. Refresh and renew yourself.",
        "Ralph Marston",
    ),
    (
        "Imagination is more important than knowledge.",
        "Albert Einstein",
    ),
    (
        "In the middle of difficulty lies opportunity.",
        "Albert Einstein",
    ),
    (
        "To iterate is human, to recurse divine.",
        "L. Peter Deutsch",
    ),
];

pub struct QuoteOfDayFetcher;

impl RealtimeFetcher for QuoteOfDayFetcher {
    fn name(&self) -> &str {
        "quote_of_day"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Text => samples::text(&single_line(QUOTES[0])),
            Shape::TextBlock => samples::text_block(&[QUOTES[0].0, &format!("— {}", QUOTES[0].1)]),
            _ => return None,
        })
    }
    fn compute(&self, ctx: &FetchContext) -> Payload {
        let now = now();
        let (quote, author) = pick(&now);
        let body = match ctx.shape.unwrap_or(Shape::TextBlock) {
            Shape::Text => Body::Text(TextData {
                value: single_line((quote, author)),
            }),
            _ => Body::TextBlock(TextBlockData {
                lines: vec![quote.to_string(), format!("— {author}")],
            }),
        };
        Payload {
            icon: None,
            status: None,
            format: None,
            body,
        }
    }
}

fn now() -> DateTime<FixedOffset> {
    Local::now().fixed_offset()
}

/// Day-of-year modulo quote count — deterministic, rolls over at midnight, repeats
/// annually. Yearly repeat is fine for a splash: cycle is ~year/quotes days apart.
fn pick(now: &DateTime<FixedOffset>) -> (&'static str, &'static str) {
    let idx = (now.ordinal() as usize).saturating_sub(1) % QUOTES.len();
    QUOTES[idx]
}

fn single_line(q: (&str, &str)) -> String {
    format!("{} — {}", q.0, q.1)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use chrono::{TimeZone, Utc};

    use super::*;

    fn at(y: i32, m: u32, d: u32) -> DateTime<FixedOffset> {
        Utc.with_ymd_and_hms(y, m, d, 12, 0, 0)
            .unwrap()
            .fixed_offset()
    }

    fn ctx(shape: Option<Shape>) -> FetchContext {
        FetchContext {
            widget_id: "q".into(),
            timeout: Duration::from_secs(1),
            shape,
            ..Default::default()
        }
    }

    #[test]
    fn default_shape_is_text_block() {
        let p = QuoteOfDayFetcher.compute(&ctx(None));
        let Body::TextBlock(d) = p.body else {
            panic!("expected text_block");
        };
        assert_eq!(d.lines.len(), 2);
        assert!(d.lines[1].starts_with("— "));
    }

    #[test]
    fn text_shape_joins_with_dash() {
        let p = QuoteOfDayFetcher.compute(&ctx(Some(Shape::Text)));
        let Body::Text(d) = p.body else {
            panic!("expected text");
        };
        assert!(d.value.contains(" — "));
    }

    #[test]
    fn pick_is_stable_within_a_day() {
        let a = pick(&at(2026, 4, 24));
        let b = pick(&at(2026, 4, 24));
        assert_eq!(a, b);
    }

    #[test]
    fn pick_advances_with_day_of_year() {
        // Neighbouring days index different slots (unless we rolled across the modulo
        // boundary, which would fail in exactly one pair per quote list length).
        let a = pick(&at(2026, 4, 24));
        let b = pick(&at(2026, 4, 25));
        assert_ne!(a, b);
    }

    #[test]
    fn quote_list_is_nonempty() {
        assert!(!QUOTES.is_empty());
    }
}
