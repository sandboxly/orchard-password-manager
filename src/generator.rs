//! Random passwords: characters from the kinds chosen, drawn with the
//! operating system's secure random source (no bias: indexes are rejection
//! sampled), with at least one of each kind chosen.

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

/// What to generate. Remembered in the settings (nothing secret).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Options {
    pub length: u16,
    pub lower: bool,
    pub upper: bool,
    pub digits: bool,
    pub symbols: bool,
    /// Leave out characters that are easy to mix up when read or typed.
    pub no_lookalikes: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { length: 20, lower: true, upper: true, digits: true, symbols: true, no_lookalikes: false }
    }
}

pub const MIN_LENGTH: u16 = 4;
pub const MAX_LENGTH: u16 = 128;

const LOWER: &str = "abcdefghijklmnopqrstuvwxyz";
const UPPER: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const DIGITS: &str = "0123456789";
const SYMBOLS: &str = "!#$%&*+-=?@^_~.,:;/()[]{}<>";
const LOOKALIKES: &str = "lI1O0o|";

impl Options {
    /// The kinds chosen, each as its characters.
    fn sets(&self) -> Vec<Vec<char>> {
        [(self.lower, LOWER), (self.upper, UPPER), (self.digits, DIGITS), (self.symbols, SYMBOLS)]
            .into_iter()
            .filter(|(on, _)| *on)
            .map(|(_, set)| set.chars().filter(|c| !(self.no_lookalikes && LOOKALIKES.contains(*c))).collect())
            .collect()
    }

    /// How hard to guess, in bits (length × log2 of the characters to pick from).
    pub fn bits(&self) -> u32 {
        let pool: usize = self.sets().iter().map(Vec::len).sum();
        if pool < 2 {
            return 0;
        }
        (f64::from(self.length) * (pool as f64).log2()) as u32
    }
}

/// A word for `bits` (an attacker's guesses; roughly the usual bands).
pub fn strength(bits: u32) -> &'static str {
    match bits {
        0..40 => "weak",
        40..64 => "fair",
        64..100 => "strong",
        _ => "very strong",
    }
}

/// A uniform random number below `n` from the OS.
fn below(n: usize) -> usize {
    let n = n as u64;
    // Reject the top end that would make low numbers likelier.
    let zone = u64::MAX - u64::MAX % n;
    loop {
        let mut b = [0u8; 8];
        getrandom::fill(&mut b).expect("the system's random source");
        let x = u64::from_le_bytes(b);
        if x < zone {
            return (x % n) as usize;
        }
    }
}

/// A new password; `None` if no kind of character is chosen.
pub fn generate(o: &Options) -> Option<Zeroizing<String>> {
    let sets = o.sets();
    if sets.iter().all(Vec::is_empty) {
        return None;
    }
    let len = o.length.clamp(MIN_LENGTH, MAX_LENGTH) as usize;
    let all: Vec<char> = sets.concat();
    let mut chars: Zeroizing<Vec<char>> = Zeroizing::new(Vec::with_capacity(len));
    // One of each kind (so a site's "needs a digit" rule holds), the rest
    // from all of them, then shuffled so those aren't always first.
    for set in sets.iter().filter(|s| !s.is_empty()).take(len) {
        chars.push(set[below(set.len())]);
    }
    while chars.len() < len {
        chars.push(all[below(all.len())]);
    }
    for i in (1..chars.len()).rev() {
        chars.swap(i, below(i + 1));
    }
    let mut out = Zeroizing::new(String::with_capacity(len * 4));
    out.extend(chars.iter());
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_the_length_and_one_of_each_kind() {
        let o = Options::default();
        for _ in 0..200 {
            let p = generate(&o).unwrap();
            assert_eq!(p.chars().count(), 20);
            assert!(p.chars().any(|c| c.is_ascii_lowercase()));
            assert!(p.chars().any(|c| c.is_ascii_uppercase()));
            assert!(p.chars().any(|c| c.is_ascii_digit()));
            assert!(p.chars().any(|c| SYMBOLS.contains(c)));
        }
    }

    #[test]
    fn only_the_kinds_chosen() {
        let o = Options { lower: false, upper: false, symbols: false, no_lookalikes: true, ..Options::default() };
        let p = generate(&o).unwrap();
        assert!(p.chars().all(|c| c.is_ascii_digit() && c != '0' && c != '1'));
        assert!(generate(&Options { digits: false, ..o }).is_none());
    }

    #[test]
    fn different_every_time() {
        let o = Options::default();
        assert_ne!(generate(&o), generate(&o));
    }

    #[test]
    fn strength_from_length_and_pool() {
        // 20 from 26 + 26 + 10 + 27 = 89 characters: about 129 bits.
        assert_eq!(Options::default().bits(), 129);
        assert_eq!(strength(Options::default().bits()), "very strong");
        let pin = Options { length: 4, lower: false, upper: false, symbols: false, ..Options::default() };
        assert_eq!(strength(pin.bits()), "weak");
    }
}
