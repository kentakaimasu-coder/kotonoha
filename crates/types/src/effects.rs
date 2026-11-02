//! 効果(Effect)と Capability(技術仕様書 §9)。

use serde::Serialize;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub enum Effect {
    #[serde(rename = "data.read")]
    DataRead,
    #[serde(rename = "data.write")]
    DataWrite,
    #[serde(rename = "network.send")]
    NetworkSend,
    #[serde(rename = "filesystem.read")]
    FsRead,
    #[serde(rename = "filesystem.write")]
    FsWrite,
    #[serde(rename = "device.camera")]
    Camera,
    #[serde(rename = "device.location")]
    Location,
    #[serde(rename = "device.notifications")]
    Notifications,
}

pub const ALL_EFFECTS: &[Effect] = &[
    Effect::DataRead,
    Effect::DataWrite,
    Effect::NetworkSend,
    Effect::FsRead,
    Effect::FsWrite,
    Effect::Camera,
    Effect::Location,
    Effect::Notifications,
];

impl Effect {
    pub fn as_str(self) -> &'static str {
        match self {
            Effect::DataRead => "data.read",
            Effect::DataWrite => "data.write",
            Effect::NetworkSend => "network.send",
            Effect::FsRead => "filesystem.read",
            Effect::FsWrite => "filesystem.write",
            Effect::Camera => "device.camera",
            Effect::Location => "device.location",
            Effect::Notifications => "device.notifications",
        }
    }
    /// `pure` は「効果なし」を表す宣言用の名前で、Effect 値ではない(None を返す)。
    pub fn parse(s: &str) -> Option<Effect> {
        ALL_EFFECTS.iter().copied().find(|e| e.as_str() == s)
    }
}

impl fmt::Display for Effect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Capability の動詞。ソースでは日本語、VAG では英語で表す(ADR-0105)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Verb {
    Read,
    Create,
    Update,
    Delete,
}

impl Verb {
    pub fn from_ja(s: &str) -> Option<Verb> {
        Some(match s {
            "読取" => Verb::Read,
            "作成" => Verb::Create,
            "更新" => Verb::Update,
            "削除" => Verb::Delete,
            _ => return None,
        })
    }
    pub fn ja(self) -> &'static str {
        match self {
            Verb::Read => "読取",
            Verb::Create => "作成",
            Verb::Update => "更新",
            Verb::Delete => "削除",
        }
    }
    pub fn en(self) -> &'static str {
        match self {
            Verb::Read => "read",
            Verb::Create => "create",
            Verb::Update => "update",
            Verb::Delete => "delete",
        }
    }
    pub fn effect(self) -> Effect {
        match self {
            Verb::Read => Effect::DataRead,
            _ => Effect::DataWrite,
        }
    }
}

/// 資源に紐づく Capability(例 `商品.read`)。
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Capability {
    pub resource: String,
    pub verb: Verb,
}

impl Capability {
    pub fn new(resource: impl Into<String>, verb: Verb) -> Self {
        Capability {
            resource: resource.into(),
            verb,
        }
    }
    /// VAG 表記 `資源.英語動詞`
    pub fn vag(&self) -> String {
        format!("{}.{}", self.resource, self.verb.en())
    }
    /// ソース表記 `資源.日本語動詞`
    pub fn source(&self) -> String {
        format!("{}.{}", self.resource, self.verb.ja())
    }
}

impl Serialize for Capability {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.vag())
    }
}
