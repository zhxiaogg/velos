//! A human-readable memory size (`8G`, `512M`, `1048576`) wrapping a byte count.
//!
//! A semantic type (Principle #1): memory is never a bare integer. Parsed from
//! and rendered as a compact human string; base-1024.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A memory quantity in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Memory(u64);

impl Memory {
    /// Construct from a raw byte count.
    pub fn from_bytes(bytes: u64) -> Self {
        Memory(bytes)
    }

    /// The value in bytes.
    pub fn bytes(self) -> u64 {
        self.0
    }
}

/// Why a memory string could not be parsed.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MemoryParseError {
    #[error("memory value is empty")]
    Empty,
    #[error("invalid memory value {0:?}: expected a number optionally suffixed with K/M/G")]
    Invalid(String),
}

impl FromStr for Memory {
    type Err = MemoryParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err(MemoryParseError::Empty);
        }
        let lower = trimmed.to_ascii_lowercase();
        let (digits, mult) = if let Some(n) = strip_unit(&lower, "gb", "g") {
            (n, 1024u64 * 1024 * 1024)
        } else if let Some(n) = strip_unit(&lower, "mb", "m") {
            (n, 1024u64 * 1024)
        } else if let Some(n) = strip_unit(&lower, "kb", "k") {
            (n, 1024u64)
        } else if let Some(n) = lower.strip_suffix('b') {
            (n, 1u64)
        } else {
            (lower.as_str(), 1u64)
        };
        let value: u64 = digits
            .trim()
            .parse()
            .map_err(|_| MemoryParseError::Invalid(s.to_string()))?;
        value
            .checked_mul(mult)
            .map(Memory)
            .ok_or_else(|| MemoryParseError::Invalid(s.to_string()))
    }
}

/// Strip a two- then one-char unit suffix, returning the remaining digits.
fn strip_unit<'a>(s: &'a str, long: &str, short: &str) -> Option<&'a str> {
    s.strip_suffix(long).or_else(|| s.strip_suffix(short))
}

impl fmt::Display for Memory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const G: u64 = 1024 * 1024 * 1024;
        const M: u64 = 1024 * 1024;
        const K: u64 = 1024;
        let b = self.0;
        if b != 0 && b.is_multiple_of(G) {
            write!(f, "{}G", b / G)
        } else if b != 0 && b.is_multiple_of(M) {
            write!(f, "{}M", b / M)
        } else if b != 0 && b.is_multiple_of(K) {
            write!(f, "{}K", b / K)
        } else {
            write!(f, "{b}")
        }
    }
}

impl Serialize for Memory {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Memory {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
#[cfg_attr(test, allow(clippy::unwrap_used))]
mod tests {
    use super::*;

    #[test]
    fn parses_suffixes_base_1024() {
        assert_eq!(
            "8G".parse::<Memory>().unwrap().bytes(),
            8 * 1024 * 1024 * 1024
        );
        assert_eq!("512M".parse::<Memory>().unwrap().bytes(), 512 * 1024 * 1024);
        assert_eq!("2kb".parse::<Memory>().unwrap().bytes(), 2048);
        assert_eq!("1024".parse::<Memory>().unwrap().bytes(), 1024);
        assert_eq!("4096b".parse::<Memory>().unwrap().bytes(), 4096);
    }

    #[test]
    fn rejects_bad_input() {
        assert_eq!("".parse::<Memory>(), Err(MemoryParseError::Empty));
        assert!("8x".parse::<Memory>().is_err());
        assert!("abc".parse::<Memory>().is_err());
    }

    #[test]
    fn display_round_trips() {
        let m = "8G".parse::<Memory>().unwrap();
        assert_eq!(m.to_string(), "8G");
        assert_eq!(m.to_string().parse::<Memory>().unwrap(), m);
    }

    #[test]
    fn serde_is_a_string() {
        let m = Memory::from_bytes(8 * 1024 * 1024 * 1024);
        let json = serde_json::to_string(&m).unwrap();
        assert_eq!(json, "\"8G\"");
        assert_eq!(serde_json::from_str::<Memory>(&json).unwrap(), m);
    }
}
