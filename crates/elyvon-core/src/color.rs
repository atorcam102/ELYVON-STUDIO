//! sRGB colours encoded as `#RRGGBB` strings on the wire and on disk.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use ts_rs::TS;

/// An opaque 8-bit sRGB colour.
#[derive(Clone, Copy, PartialEq, Eq, Hash, TS)]
#[ts(export, type = "string")]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid colour {0:?}: expected #RRGGBB")]
pub struct ColorParseError(pub String);

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub fn to_hex(self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }
}

impl fmt::Debug for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Color({})", self.to_hex())
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl FromStr for Color {
    type Err = ColorParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || ColorParseError(s.to_owned());
        let hex = s.strip_prefix('#').ok_or_else(err)?;
        if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(err());
        }
        let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| err());
        Ok(Self::rgb(channel(0)?, channel(2)?, channel(4)?))
    }
}

impl Serialize for Color {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_formats_round_trip() {
        let c: Color = "#3a7BfF".parse().unwrap();
        assert_eq!(c, Color::rgb(0x3A, 0x7B, 0xFF));
        assert_eq!(c.to_hex(), "#3A7BFF");
    }

    #[test]
    fn rejects_malformed_values() {
        for bad in [
            "", "#", "3A7BFF", "#3A7BF", "#3A7BFFF", "#GGGGGG", "#3A7B+F", "#ÿÿÿ",
        ] {
            assert!(bad.parse::<Color>().is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn serde_uses_hex_strings() {
        let json = serde_json::to_string(&Color::rgb(1, 2, 3)).unwrap();
        assert_eq!(json, "\"#010203\"");
        let back: Color = serde_json::from_str(&json).unwrap();
        assert_eq!(back, Color::rgb(1, 2, 3));
        assert!(serde_json::from_str::<Color>("\"red\"").is_err());
    }
}
