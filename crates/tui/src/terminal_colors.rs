use std::collections::{BTreeSet, VecDeque};

use crate::colors::Rgb;
use crate::terminal::{NegotiationReply, parse_keyboard_protocol_negotiation_sequence};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TerminalColors {
    pub foreground: Option<Rgb>,
    pub background: Option<Rgb>,
    pub palette: Option<[Rgb; 16]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum OscColorTarget {
    Foreground,
    Background,
    Palette(u16),
}

const PALETTE_SIZE: usize = 16;
const REPLY_COUNT: usize = 2 + PALETTE_SIZE;
pub const TERMINAL_COLOR_QUERY: &str = concat!(
    "\x1b]10;?\x07\x1b]11;?\x07",
    "\x1b]4;0;?\x07\x1b]4;1;?\x07\x1b]4;2;?\x07\x1b]4;3;?\x07",
    "\x1b]4;4;?\x07\x1b]4;5;?\x07\x1b]4;6;?\x07\x1b]4;7;?\x07",
    "\x1b]4;8;?\x07\x1b]4;9;?\x07\x1b]4;10;?\x07\x1b]4;11;?\x07",
    "\x1b]4;12;?\x07\x1b]4;13;?\x07\x1b]4;14;?\x07\x1b]4;15;?\x07",
    "\x1b[c"
);

pub fn parse_osc_color_response(data: &str) -> Option<(OscColorTarget, Option<Rgb>)> {
    let body = data.strip_prefix("\x1b]")?;
    let body = body
        .strip_suffix('\x07')
        .or_else(|| body.strip_suffix("\x1b\\"))?;
    let (target, value) = if let Some(value) = body.strip_prefix("10;") {
        (OscColorTarget::Foreground, value)
    } else if let Some(value) = body.strip_prefix("11;") {
        (OscColorTarget::Background, value)
    } else {
        let (index, value) = body.strip_prefix("4;")?.split_once(';')?;
        if !(1..=3).contains(&index.len()) || !index.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        (OscColorTarget::Palette(index.parse().ok()?), value)
    };
    if value.contains(['\x07', '\x1b']) {
        return None;
    }
    Some((target, parse_osc_color_value(value)))
}

fn parse_osc_hex_channel(channel: &str) -> Option<u8> {
    if channel.is_empty() || !channel.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let value = channel
        .chars()
        .filter_map(|c| c.to_digit(16))
        .fold(0.0, |total, digit| total * 16.0 + digit as f64);
    let max = 16f64.powi(channel.len() as i32) - 1.0;
    Some(((value / max) * 255.0).round() as u8)
}

fn parse_osc_color_value(raw: &str) -> Option<Rgb> {
    let value = raw.trim();
    if let Some(hex) = value.strip_prefix('#') {
        if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let width = match hex.len() {
            6 => 2,
            12 => 4,
            _ => return None,
        };
        let channel = |at: usize| parse_osc_hex_channel(&hex[at * width..(at + 1) * width]);
        return Some(Rgb::new(channel(0)?, channel(1)?, channel(2)?));
    }
    let prefix = value.find(':').map_or(0, |colon| {
        let scheme = &value[..colon];
        if scheme.eq_ignore_ascii_case("rgb") || scheme.eq_ignore_ascii_case("rgba") {
            colon + 1
        } else {
            0
        }
    });
    let mut parts = value[prefix..].split('/');
    let (r, g, b) = (parts.next()?, parts.next()?, parts.next()?);
    Some(Rgb::new(
        parse_osc_hex_channel(r)?,
        parse_osc_hex_channel(g)?,
        parse_osc_hex_channel(b)?,
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Delivery {
    Until(u64),
    Late,
    Done,
}

#[derive(Debug)]
struct Query {
    foreground: Option<Rgb>,
    background: Option<Rgb>,
    palette: [Option<Rgb>; PALETTE_SIZE],
    replied: BTreeSet<OscColorTarget>,
    delivery: Delivery,
}

impl Query {
    fn result(&self) -> TerminalColors {
        let mut palette = [Rgb::new(0, 0, 0); PALETTE_SIZE];
        let complete = self
            .palette
            .iter()
            .zip(palette.iter_mut())
            .all(|(reported, slot)| reported.map(|color| *slot = color).is_some());
        TerminalColors {
            foreground: self.foreground,
            background: self.background,
            palette: complete.then_some(palette),
        }
    }
}

#[derive(Debug, Default)]
pub struct TerminalColorQueries {
    pending: VecDeque<Query>,
    ready: VecDeque<TerminalColors>,
}

impl TerminalColorQueries {
    pub fn start(&mut self, now: u64, timeout_ms: u64) -> &'static str {
        self.pending.push_back(Query {
            foreground: None,
            background: None,
            palette: [None; PALETTE_SIZE],
            replied: BTreeSet::new(),
            delivery: Delivery::Until(now + timeout_ms),
        });
        TERMINAL_COLOR_QUERY
    }

    pub fn consume(&mut self, data: &str) -> bool {
        let Some(query) = self.pending.front_mut() else {
            return false;
        };
        if parse_keyboard_protocol_negotiation_sequence(data)
            == Some(NegotiationReply::DeviceAttributes)
        {
            if query.delivery != Delivery::Done {
                self.ready.push_back(query.result());
            }
            self.pending.pop_front();
            return true;
        }
        let Some((target, rgb)) = parse_osc_color_response(data) else {
            return false;
        };
        if query.delivery == Delivery::Done || !query.replied.insert(target) {
            return true;
        }
        match target {
            OscColorTarget::Foreground => query.foreground = rgb,
            OscColorTarget::Background => query.background = rgb,
            OscColorTarget::Palette(index) => {
                if let Some(slot) = query.palette.get_mut(index as usize) {
                    *slot = rgb;
                }
            }
        }
        if query.replied.len() == REPLY_COUNT {
            query.delivery = Delivery::Done;
            self.ready.push_back(query.result());
        }
        true
    }

    pub fn take(&mut self, now: u64) -> Option<TerminalColors> {
        for query in &mut self.pending {
            if let Delivery::Until(deadline) = query.delivery
                && now >= deadline
            {
                query.delivery = Delivery::Late;
                self.ready.push_back(query.result());
            }
        }
        self.ready.pop_front()
    }

    pub fn deadline(&self) -> Option<u64> {
        self.pending
            .iter()
            .filter_map(|query| match query.delivery {
                Delivery::Until(deadline) => Some(deadline),
                _ => None,
            })
            .min()
    }
}
