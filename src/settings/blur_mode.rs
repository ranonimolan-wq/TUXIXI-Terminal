// SPDX-License-Identifier: GPL-3.0
// Copyright (c) 2026 ranonimolan-wq / TUXIXI. All rights reserved.

//! Terminal background blur modes stored in `config.json`.
//!
//! The enum is serde-serialized in lowercase form (`off`, `static`,
//! `smoky`) so the JSON representation stays stable across releases.

use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BlurMode {
    Off,
    Static,
    Smoky,
}

impl BlurMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            BlurMode::Off => "off",
            BlurMode::Static => "static",
            BlurMode::Smoky => "smoky",
        }
    }

    /// Parse a blur mode from its config-file string form.
    /// Returns `None` for any name outside `off` / `static` / `smoky`.
    fn parse_mode(s: &str) -> Option<Self> {
        match s {
            "off" => Some(BlurMode::Off),
            "static" => Some(BlurMode::Static),
            "smoky" => Some(BlurMode::Smoky),
            _ => None,
        }
    }

    /// `off -> static -> smoky -> off`
    pub fn next(&self) -> Self {
        match self {
            BlurMode::Off => BlurMode::Static,
            BlurMode::Static => BlurMode::Smoky,
            BlurMode::Smoky => BlurMode::Off,
        }
    }
}

impl fmt::Display for BlurMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for BlurMode {
    type Err = ();

    /// Parse from the lowercase config representation; any other
    /// value is an error so callers can fall back to the default.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse_mode(s).ok_or(())
    }
}
