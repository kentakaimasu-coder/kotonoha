//! 型の表現(技術仕様書 §4、正式定義は docs/spec/type_system.md)。

use serde::Serialize;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, PartialOrd, Ord)]
pub enum Type {
    Bool,
    Int,
    Decimal,
    Str,
    /// 日時(タイムゾーン必須。リテラル構文は未定義: ADR-0106)
    DateTime,
    Date,
    Id,
    Bytes,
    /// 金額<通貨コード>
    Money(String),
    Duration,
    Url,
    Email,
    Array(Box<Type>),
    List(Box<Type>),
    Option(Box<Type>),
    Result(Box<Type>, Box<Type>),
    Record(Vec<(String, Type)>),
    /// ユーザー定義データ(名前で同一性を判定する名目型)
    Data(String),
    /// ユーザー定義列挙
    Enum(String),
    /// 値を返さない
    Unit,
    /// エラー回復用(どの型とも一致したとみなし、連鎖エラーを防ぐ)
    Error,
}

pub const BUILTIN_TYPE_NAMES: &[&str] = &[
    "真偽",
    "整数",
    "小数",
    "文字列",
    "日時",
    "日付",
    "識別子",
    "バイト列",
    "金額",
    "期間",
    "URL",
    "メールアドレス",
    "配列",
    "一覧",
    "選択",
    "結果",
];

impl Type {
    pub fn is_error(&self) -> bool {
        matches!(self, Type::Error)
    }
    pub fn is_numeric(&self) -> bool {
        matches!(self, Type::Int | Type::Decimal | Type::Money(_))
    }
    /// 構造的同一性(Error はワイルドカード)
    pub fn same(&self, other: &Type) -> bool {
        match (self, other) {
            (Type::Error, _) | (_, Type::Error) => true,
            (Type::Array(a), Type::Array(b)) | (Type::List(a), Type::List(b)) | (Type::Option(a), Type::Option(b)) => a.same(b),
            (Type::Result(a, e), Type::Result(b, f)) => a.same(b) && e.same(f),
            (Type::Record(a), Type::Record(b)) => a.len() == b.len() && a.iter().zip(b).all(|((n1, t1), (n2, t2))| n1 == n2 && t1.same(t2)),
            _ => self == other,
        }
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Bool => f.write_str("真偽"),
            Type::Int => f.write_str("整数"),
            Type::Decimal => f.write_str("小数"),
            Type::Str => f.write_str("文字列"),
            Type::DateTime => f.write_str("日時"),
            Type::Date => f.write_str("日付"),
            Type::Id => f.write_str("識別子"),
            Type::Bytes => f.write_str("バイト列"),
            Type::Money(c) => write!(f, "金額<{c}>"),
            Type::Duration => f.write_str("期間"),
            Type::Url => f.write_str("URL"),
            Type::Email => f.write_str("メールアドレス"),
            Type::Array(t) => write!(f, "配列<{t}>"),
            Type::List(t) => write!(f, "一覧<{t}>"),
            Type::Option(t) => write!(f, "選択<{t}>"),
            Type::Result(t, e) => write!(f, "結果<{t}, {e}>"),
            Type::Record(fs) => {
                f.write_str("記録{")?;
                for (i, (n, t)) in fs.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{n}: {t}")?;
                }
                f.write_str("}")
            }
            Type::Data(n) | Type::Enum(n) => f.write_str(n),
            Type::Unit => f.write_str("なし"),
            Type::Error => f.write_str("<不明>"),
        }
    }
}

/// 通貨コードの形式検査(ISO 4217 形式の英大文字 3 文字)。
/// 実在通貨の一覧照合と金額精度(小数桁)は ADR-0106 で未決定。
pub fn is_currency_code(s: &str) -> bool {
    s.len() == 3 && s.chars().all(|c| c.is_ascii_uppercase())
}
