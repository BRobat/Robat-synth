use crate::rational::Rational;
use std::fmt;

/// Pattern syntax tree. Times are relative to one cycle (one bar).
#[derive(Clone, Debug, PartialEq)]
pub enum Ast {
    Note(u8),
    Rest,
    /// Items share one cycle in proportion to their weights (`a@3 b`).
    Seq(Vec<(Ast, Rational)>),
    /// Items play simultaneously (`a, b`).
    Stack(Vec<Ast>),
    /// One item per cycle, in turn (`<a b c>`).
    Alt(Vec<Ast>),
    /// Speed up by factor (`a*2`); slow down is a factor < 1 (`a/2`).
    Fast(Box<Ast>, Rational),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParseError {
    /// Character column the error refers to.
    pub pos: usize,
    pub msg: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "col {}: {}", self.pos + 1, self.msg)
    }
}

impl std::error::Error for ParseError {}

/// Default octave when a note name has none (`c` == `c3`).
pub const DEFAULT_OCTAVE: i32 = 3;

/// Logic Pro convention: C3 = MIDI 60.
pub fn note_to_midi(pitch_class: i32, octave: i32) -> Option<u8> {
    let n = 12 * (octave + 2) + pitch_class;
    (0..=127).contains(&n).then_some(n as u8)
}

pub fn parse(src: &str) -> Result<Ast, ParseError> {
    let mut p = Parser { chars: src.chars().collect(), pos: 0 };
    let ast = p.stack(None)?;
    p.skip_ws();
    if let Some(c) = p.peek() {
        return Err(p.err(format!("unexpected '{c}'")));
    }
    Ok(ast)
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
}

impl Parser {
    fn err(&self, msg: impl Into<String>) -> ParseError {
        ParseError { pos: self.pos, msg: msg.into() }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek();
        self.pos += 1;
        c
    }

    fn skip_ws(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.pos += 1;
        }
    }

    fn eat(&mut self, c: char) -> bool {
        self.skip_ws();
        if self.peek() == Some(c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    /// `seq (',' seq)*`, terminated by `close` (or end of input).
    fn stack(&mut self, close: Option<char>) -> Result<Ast, ParseError> {
        let mut layers = vec![self.seq(close)?];
        while self.eat(',') {
            layers.push(self.seq(close)?);
        }
        Ok(if layers.len() == 1 { layers.pop().unwrap() } else { Ast::Stack(layers) })
    }

    fn seq(&mut self, close: Option<char>) -> Result<Ast, ParseError> {
        let mut items: Vec<(Ast, Rational)> = Vec::new();
        loop {
            self.skip_ws();
            match self.peek() {
                None => break,
                Some(c) if Some(c) == close || c == ',' => break,
                Some(']') | Some('>') => return Err(self.err("unmatched closing bracket")),
                Some('!') => {
                    // Bare `!` repeats the previous step.
                    self.pos += 1;
                    let last = items.last().cloned().ok_or_else(|| self.err("'!' with nothing to repeat"))?;
                    items.push(last);
                }
                Some(_) => {
                    let (term, weight, reps) = self.term()?;
                    for _ in 0..reps {
                        items.push((term.clone(), weight));
                    }
                }
            }
        }
        Ok(match items.len() {
            0 => Ast::Rest,
            1 if items[0].1 == Rational::ONE => items.pop().unwrap().0,
            _ => Ast::Seq(items),
        })
    }

    /// An atom followed by modifiers: `*n`, `/n`, `@w`, `!n`.
    fn term(&mut self) -> Result<(Ast, Rational, usize), ParseError> {
        let mut ast = self.atom()?;
        let mut weight = Rational::ONE;
        let mut reps = 1;
        loop {
            match self.peek() {
                Some('*') => {
                    self.pos += 1;
                    let k = self.positive_number()?;
                    ast = Ast::Fast(Box::new(ast), k);
                }
                Some('/') => {
                    self.pos += 1;
                    let k = self.positive_number()?;
                    ast = Ast::Fast(Box::new(ast), Rational::ONE / k);
                }
                Some('@') => {
                    self.pos += 1;
                    weight = self.positive_number()?;
                }
                Some('!') if self.chars.get(self.pos + 1).is_some_and(|c| c.is_ascii_digit()) => {
                    self.pos += 1;
                    let n = self.positive_number()?;
                    if n.den() != 1 || n.num() > 64 {
                        return Err(self.err("'!' needs a whole number up to 64"));
                    }
                    reps = n.num() as usize;
                }
                _ => break,
            }
        }
        Ok((ast, weight, reps))
    }

    fn atom(&mut self) -> Result<Ast, ParseError> {
        self.skip_ws();
        let start = self.pos;
        match self.peek() {
            Some('~') => {
                self.pos += 1;
                Ok(Ast::Rest)
            }
            Some('[') => {
                self.pos += 1;
                let inner = self.stack(Some(']'))?;
                if !self.eat(']') {
                    return Err(ParseError { pos: start, msg: "unclosed '['".into() });
                }
                Ok(inner)
            }
            Some('<') => {
                self.pos += 1;
                let inner = self.seq(Some('>'))?;
                if !self.eat('>') {
                    return Err(ParseError { pos: start, msg: "unclosed '<'".into() });
                }
                Ok(match inner {
                    Ast::Seq(items) => Ast::Alt(items.into_iter().map(|(a, _)| a).collect()),
                    other => other,
                })
            }
            Some(c) if c.is_ascii_digit() => {
                let n = self.integer()?;
                u8::try_from(n)
                    .ok()
                    .filter(|n| *n <= 127)
                    .map(Ast::Note)
                    .ok_or(ParseError { pos: start, msg: "midi note must be 0-127".into() })
            }
            Some(c) if c.is_ascii_alphabetic() => self.note_name(),
            Some(c) => Err(self.err(format!("unexpected '{c}'"))),
            None => Err(self.err("unexpected end")),
        }
    }

    fn note_name(&mut self) -> Result<Ast, ParseError> {
        let start = self.pos;
        let letter = self.bump().unwrap().to_ascii_lowercase();
        let mut pc = match letter {
            'c' => 0,
            'd' => 2,
            'e' => 4,
            'f' => 5,
            'g' => 7,
            'a' => 9,
            'b' => 11,
            _ => return Err(ParseError { pos: start, msg: format!("unknown note '{letter}'") }),
        };
        match self.peek() {
            Some('#') | Some('s') => {
                pc += 1;
                self.pos += 1;
            }
            Some('b') => {
                pc -= 1;
                self.pos += 1;
            }
            _ => {}
        }
        let octave = match self.peek() {
            Some(c) if c.is_ascii_digit() || c == '-' => self.integer()? as i32,
            _ => DEFAULT_OCTAVE,
        };
        if self.peek().is_some_and(|c| c.is_ascii_alphanumeric()) {
            return Err(self.err("malformed note"));
        }
        note_to_midi(pc, octave)
            .map(Ast::Note)
            .ok_or(ParseError { pos: start, msg: "note out of midi range".into() })
    }

    fn integer(&mut self) -> Result<i64, ParseError> {
        let start = self.pos;
        if self.peek() == Some('-') {
            self.pos += 1;
        }
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.pos += 1;
        }
        let s: String = self.chars[start..self.pos].iter().collect();
        s.parse().map_err(|_| ParseError { pos: start, msg: "expected a number".into() })
    }

    fn positive_number(&mut self) -> Result<Rational, ParseError> {
        let start = self.pos;
        while self.peek().is_some_and(|c| c.is_ascii_digit() || c == '.') {
            self.pos += 1;
        }
        let s: String = self.chars[start..self.pos].iter().collect();
        match Rational::from_decimal(&s) {
            Some(r) if r > Rational::ZERO && r.num() <= 1024 * r.den() => Ok(r),
            _ => Err(ParseError { pos: start, msg: "expected a positive number".into() }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_names() {
        assert_eq!(parse("c3").unwrap(), Ast::Note(60));
        assert_eq!(parse("c").unwrap(), Ast::Note(60));
        assert_eq!(parse("a3").unwrap(), Ast::Note(69));
        assert_eq!(parse("eb4").unwrap(), Ast::Note(75));
        assert_eq!(parse("f#2").unwrap(), Ast::Note(54));
        assert_eq!(parse("bb").unwrap(), Ast::Note(70));
        assert_eq!(parse("c-2").unwrap(), Ast::Note(0));
        assert_eq!(parse("64").unwrap(), Ast::Note(64));
    }

    #[test]
    fn structure() {
        let ast = parse("c [d e]*2 <f g>").unwrap();
        let Ast::Seq(items) = ast else { panic!() };
        assert_eq!(items.len(), 3);
        assert!(matches!(items[1].0, Ast::Fast(_, _)));
        assert!(matches!(items[2].0, Ast::Alt(ref v) if v.len() == 2));
        assert!(matches!(parse("c, e, g").unwrap(), Ast::Stack(ref v) if v.len() == 3));
        assert!(matches!(parse("c!3").unwrap(), Ast::Seq(ref v) if v.len() == 3));
        assert!(matches!(parse("c ! !").unwrap(), Ast::Seq(ref v) if v.len() == 3));
        assert_eq!(parse("").unwrap(), Ast::Rest);
    }

    #[test]
    fn errors_have_positions() {
        assert_eq!(parse("c [d e").unwrap_err().pos, 2);
        assert_eq!(parse("c x").unwrap_err().pos, 2);
        assert_eq!(parse("c*0").unwrap_err().pos, 2);
        assert!(parse("c ]").is_err());
        assert!(parse("200").is_err());
        assert!(parse("cq").is_err());
    }
}
