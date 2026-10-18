//! Tidal-style mini-notation for robat-synth sequencers.
//!
//! One cycle is one bar. Supported syntax:
//! - notes `c3 eb4 f#2 bb`, or raw midi numbers `60` (Logic convention: C3 = 60)
//! - rest `~`, subdivision `[a b]`, polyphony `[a, b]` / `a, b`
//! - alternation per cycle `<a b c>`
//! - speed `a*2`, `[a b]/2`, weight `a@3 b`, repeat `a!3` or `a ! !`

mod parse;
mod rational;

pub use parse::{note_to_midi, parse, Ast, ParseError, DEFAULT_OCTAVE};
pub use rational::Rational;

/// A note onset produced by a pattern, in absolute cycle time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hap {
    pub start: Rational,
    pub dur: Rational,
    pub note: u8,
}

/// Patterns longer than this many cycles are truncated when tabulated.
pub const MAX_PERIOD: i64 = 128;

#[derive(Clone, Debug, PartialEq)]
pub struct Pattern {
    ast: Ast,
}

impl Pattern {
    pub fn parse(src: &str) -> Result<Self, ParseError> {
        parse(src).map(|ast| Pattern { ast })
    }

    /// All onsets in `[begin, end)`, sorted by start time.
    pub fn query(&self, begin: Rational, end: Rational) -> Vec<Hap> {
        let mut out = Vec::new();
        query(&self.ast, begin, end, &mut out);
        out.sort_by(|a, b| a.start.cmp(&b.start).then(a.note.cmp(&b.note)));
        out
    }

    /// Number of cycles after which the pattern repeats (capped at [`MAX_PERIOD`]).
    pub fn period(&self) -> i64 {
        period(&self.ast).min(MAX_PERIOD)
    }

    /// Events of every cycle in one period, with times relative to their cycle
    /// start as floats — the form the realtime engine consumes.
    pub fn tabulate(&self) -> Vec<Vec<CycleEvent>> {
        (0..self.period())
            .map(|c| {
                let c0 = Rational::int(c);
                self.query(c0, c0 + Rational::ONE)
                    .into_iter()
                    .map(|h| CycleEvent { pos: (h.start - c0).to_f64(), dur: h.dur.to_f64(), note: h.note })
                    .collect()
            })
            .collect()
    }
}

/// A tabulated event: `pos` and `dur` in cycles, `pos` within `[0, 1)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CycleEvent {
    pub pos: f64,
    pub dur: f64,
    pub note: u8,
}

fn cycles(begin: Rational, end: Rational) -> std::ops::Range<i64> {
    begin.floor()..end.ceil()
}

fn query(ast: &Ast, b: Rational, e: Rational, out: &mut Vec<Hap>) {
    if b >= e {
        return;
    }
    match ast {
        Ast::Rest => {}
        Ast::Note(n) => {
            for c in cycles(b, e) {
                let t = Rational::int(c);
                if t >= b && t < e {
                    out.push(Hap { start: t, dur: Rational::ONE, note: *n });
                }
            }
        }
        Ast::Stack(layers) => {
            for l in layers {
                query(l, b, e, out);
            }
        }
        Ast::Seq(items) => {
            let total = items.iter().fold(Rational::ZERO, |acc, (_, w)| acc + *w);
            for c in cycles(b, e) {
                let c0 = Rational::int(c);
                let mut offset = Rational::ZERO;
                for (item, w) in items {
                    let slot_lo = c0 + offset / total;
                    let slot_hi = c0 + (offset + *w) / total;
                    offset = offset + *w;
                    let lo = b.max(slot_lo);
                    let hi = e.min(slot_hi);
                    if lo >= hi {
                        continue;
                    }
                    // The item sees cycle `c`, squeezed into its slot.
                    let scale = *w / total;
                    let mut sub = Vec::new();
                    query(item, c0 + (lo - slot_lo) / scale, c0 + (hi - slot_lo) / scale, &mut sub);
                    out.extend(sub.into_iter().map(|h| Hap { start: slot_lo + (h.start - c0) * scale, dur: h.dur * scale, note: h.note }));
                }
            }
        }
        Ast::Alt(items) => {
            let n = items.len() as i64;
            for c in cycles(b, e) {
                let item = &items[c.rem_euclid(n) as usize];
                // Each item advances through its own cycles only when it plays.
                let shift = Rational::int(c - c.div_euclid(n));
                let lo = b.max(Rational::int(c));
                let hi = e.min(Rational::int(c + 1));
                let mut sub = Vec::new();
                query(item, lo - shift, hi - shift, &mut sub);
                out.extend(sub.into_iter().map(|h| Hap { start: h.start + shift, ..h }));
            }
        }
        Ast::Fast(inner, k) => {
            let mut sub = Vec::new();
            query(inner, b * *k, e * *k, &mut sub);
            out.extend(sub.into_iter().map(|h| Hap { start: h.start / *k, dur: h.dur / *k, note: h.note }));
        }
    }
}

fn period(ast: &Ast) -> i64 {
    let cap = |p: i64| p.min(MAX_PERIOD * 64);
    match ast {
        Ast::Rest | Ast::Note(_) => 1,
        Ast::Stack(v) => v.iter().fold(1, |acc, a| cap(rational::lcm(acc, period(a)))),
        Ast::Seq(v) => v.iter().fold(1, |acc, (a, _)| cap(rational::lcm(acc, period(a)))),
        Ast::Alt(v) => cap(v.len() as i64 * v.iter().fold(1, |acc, a| cap(rational::lcm(acc, period(a))))),
        Ast::Fast(inner, k) => {
            // Smallest T with T * k a multiple of the inner period.
            let pq = period(inner) * k.den();
            cap(pq / gcd(pq, k.num()))
        }
    }
}

fn gcd(a: i64, b: i64) -> i64 {
    if b == 0 {
        a.max(1)
    } else {
        gcd(b, a % b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(n: i64, d: i64) -> Rational {
        Rational::new(n, d)
    }

    fn cycle(src: &str, c: i64) -> Vec<(Rational, u8)> {
        Pattern::parse(src)
            .unwrap()
            .query(Rational::int(c), Rational::int(c + 1))
            .into_iter()
            .map(|h| (h.start - Rational::int(c), h.note))
            .collect()
    }

    #[test]
    fn subdivision() {
        assert_eq!(cycle("c d e f", 0), vec![(r(0, 1), 60), (r(1, 4), 62), (r(1, 2), 64), (r(3, 4), 65)]);
        assert_eq!(cycle("c [d e]", 3), vec![(r(0, 1), 60), (r(1, 2), 62), (r(3, 4), 64)]);
        assert_eq!(cycle("c ~ ~ e", 0), vec![(r(0, 1), 60), (r(3, 4), 64)]);
    }

    #[test]
    fn durations() {
        let p = Pattern::parse("c [d e]").unwrap();
        let haps = p.query(Rational::ZERO, Rational::ONE);
        assert_eq!(haps[0].dur, r(1, 2));
        assert_eq!(haps[1].dur, r(1, 4));
    }

    #[test]
    fn fast_and_slow() {
        assert_eq!(cycle("[c d]*2", 0), vec![(r(0, 1), 60), (r(1, 4), 62), (r(1, 2), 60), (r(3, 4), 62)]);
        assert_eq!(cycle("[c d]/2", 0), vec![(r(0, 1), 60)]);
        assert_eq!(cycle("[c d]/2", 1), vec![(r(0, 1), 62)]);
        assert_eq!(cycle("c*1.5", 0), vec![(r(0, 1), 60), (r(2, 3), 60)]);
        assert_eq!(cycle("c*1.5", 1), vec![(r(1, 3), 60)]);
    }

    #[test]
    fn alternation() {
        assert_eq!(cycle("c <d e>", 0), vec![(r(0, 1), 60), (r(1, 2), 62)]);
        assert_eq!(cycle("c <d e>", 1), vec![(r(0, 1), 60), (r(1, 2), 64)]);
        assert_eq!(cycle("c <d e>", 2), vec![(r(0, 1), 60), (r(1, 2), 62)]);
        // Nested alternation advances only when its parent slot plays.
        assert_eq!(cycle("<c <d e>>", 1), vec![(r(0, 1), 62)]);
        assert_eq!(cycle("<c <d e>>", 3), vec![(r(0, 1), 64)]);
        // Alternation inside a repeated group steps per repetition.
        assert_eq!(cycle("<c d>*2", 0), vec![(r(0, 1), 60), (r(1, 2), 62)]);
    }

    #[test]
    fn polyphony_and_weights() {
        assert_eq!(cycle("[c, e, g]", 0), vec![(r(0, 1), 60), (r(0, 1), 64), (r(0, 1), 67)]);
        assert_eq!(cycle("c e, g", 0), vec![(r(0, 1), 60), (r(0, 1), 67), (r(1, 2), 64)]);
        assert_eq!(cycle("c@3 e", 0), vec![(r(0, 1), 60), (r(3, 4), 64)]);
        assert_eq!(cycle("c!2 e", 0), vec![(r(0, 1), 60), (r(1, 3), 60), (r(2, 3), 64)]);
    }

    #[test]
    fn partial_queries_split_cleanly() {
        let p = Pattern::parse("c [d e]*2 <f g>").unwrap();
        let whole = p.query(Rational::ZERO, Rational::int(4));
        let mut parts = Vec::new();
        let mut t = Rational::ZERO;
        let step = r(3, 7);
        while t < Rational::int(4) {
            let end = (t + step).min(Rational::int(4));
            parts.extend(p.query(t, end));
            t = end;
        }
        assert_eq!(whole, parts);
    }

    #[test]
    fn periods() {
        assert_eq!(Pattern::parse("c d").unwrap().period(), 1);
        assert_eq!(Pattern::parse("c <d e>").unwrap().period(), 2);
        assert_eq!(Pattern::parse("<c <d e>>").unwrap().period(), 4);
        assert_eq!(Pattern::parse("c/3").unwrap().period(), 3);
        assert_eq!(Pattern::parse("c*1.5").unwrap().period(), 2);
        assert_eq!(Pattern::parse("<c d e>, <f g>").unwrap().period(), 6);
        let p = Pattern::parse("c <d e f>/2").unwrap();
        let table = p.tabulate();
        assert_eq!(table.len() as i64, p.period());
        // Tabulation must really repeat after one period.
        let next = p.query(Rational::int(p.period()), Rational::int(p.period() + 1));
        assert_eq!(next.len(), table[0].len());
    }
}
