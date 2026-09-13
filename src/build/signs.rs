//! AerodromeSign from X-Plane sign strings (`{@Y}A{^r}` style).

use super::Ctx;
use crate::model::codes::signtype;
use crate::model::feature::opt;
use crate::model::{AmdbFeature, Layer};
use geo_types::Point;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Decoded {
    pub front: String,
    pub back: String,
    pub signtype: i64,
}

fn glyph(tok: &str) -> String {
    match tok {
        "^u" => "↑",
        "^d" => "↓",
        "^l" => "←",
        "^r" => "→",
        "^lu" | "^ul" => "↖",
        "^ru" | "^ur" => "↗",
        "^ld" | "^dl" => "↙",
        "^rd" | "^dr" => "↘",
        "r1" => "I",
        "r2" => "II",
        "r3" => "III",
        "hazard" => "[HAZARD]",
        "safety" => "[SAFETY]",
        "critical" => "[CRITICAL]",
        "no-entry" => "⛔",
        "_" => " ",
        "*" => "·",
        "|" => "|",
        "comma" => ",",
        other => other,
    }
    .to_string()
}

pub fn decode(raw: &str) -> Decoded {
    let mut d = Decoded::default();
    let mut front = true;
    let mut style: Option<char> = None;
    let mut first_style: Option<char> = None;
    let mut chars = raw.chars().peekable();
    let push = |d: &mut Decoded, front: bool, s: &str| {
        if front {
            d.front.push_str(s)
        } else {
            d.back.push_str(s)
        }
    };
    while let Some(c) = chars.next() {
        if c == '{' {
            let mut inner = String::new();
            for c2 in chars.by_ref() {
                if c2 == '}' {
                    break;
                }
                inner.push(c2);
            }
            for tok in inner.split(',') {
                let tok = tok.trim();
                if tok.is_empty() {
                    continue;
                }
                if tok == "@@" {
                    front = false;
                    continue;
                }
                if let Some(s) = tok.strip_prefix('@') {
                    let ch = s.chars().next().unwrap_or('Y').to_ascii_uppercase();
                    style = Some(ch);
                    if first_style.is_none() {
                        first_style = Some(ch);
                    }
                    continue;
                }
                push(&mut d, front, &glyph(tok));
            }
        } else {
            push(&mut d, front, &c.to_string());
        }
    }
    let _ = style;
    d.signtype = match first_style {
        Some('R') => signtype::MANDATORY,
        Some('Y') => signtype::INFORMATION_DIRECTION,
        Some('L') => signtype::LOCATION,
        Some('B') => signtype::DISTANCE_REMAINING,
        _ => signtype::UNKNOWN,
    };
    d
}

pub fn build(ctx: &mut Ctx) {
    for s in &ctx.src.signs {
        let dec = decode(&s.raw);
        let c = ctx.p(s.pos);
        let signtype = if s.size >= 4 { signtype::DISTANCE_REMAINING } else { dec.signtype };
        let height = match s.size { 1 => 0.6, 2 => 0.8, 3 => 1.2, 4 => 1.2, _ => 0.8 };
        ctx.push(
            AmdbFeature::new(Layer::AerodromeSign, Point(c))
                .with("signtype", signtype)
                .with("msgfront", dec.front.clone())
                .with("msgback", if dec.back.is_empty() { serde_json::Value::Null } else { dec.back.clone().into() })
                .with("signdir", super::runway::round1(s.heading_deg))
                .with("size", s.size)
                .with("height", height)
                .with("raw", s.raw.clone())
                .with("lighting", opt(None::<bool>))
                .with("source", s.source),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_xplane_sign_syntax() {
        let d = decode("{@Y}A{^r}{@@}B{^l}");
        assert_eq!(d.front, "A→");
        assert_eq!(d.back, "B←");
        assert_eq!(d.signtype, signtype::INFORMATION_DIRECTION);
        let m = decode("{@R}25L-07R{@L}M23");
        assert_eq!(m.front, "25L-07RM23");
        assert_eq!(m.signtype, signtype::MANDATORY);
        let b = decode("{@B}1{@@}7");
        assert_eq!((b.front.as_str(), b.back.as_str()), ("1", "7"));
        assert_eq!(b.signtype, signtype::DISTANCE_REMAINING);
    }
}
