//! 診断(エラー/警告)の共通定義。
//!
//! 技術仕様書 §15 のエラーコードと §14 の終了コードを機械可読に固定する。
//! 仕様書に無いコード(SYN002 等)は `docs/spec/errors.md` に「提案」として記載している。

use serde::Serialize;
use std::fmt;

/// ソースファイルの識別子(SourceMap 上のインデックス)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Default)]
pub struct FileId(pub u32);

/// ソース位置。line/column は 1 始まり、column は Unicode スカラー値単位。
/// start/end はファイル先頭からのバイトオフセット(半開区間)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Default)]
pub struct Span {
    #[serde(skip)]
    pub file: FileId,
    pub line: u32,
    pub column: u32,
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(file: FileId, line: u32, column: u32, start: u32, end: u32) -> Self {
        Span {
            file,
            line,
            column,
            start,
            end,
        }
    }
    /// 2 つの位置を覆う範囲(先頭は self)。
    pub fn to(self, other: Span) -> Span {
        Span {
            end: other.end.max(self.end),
            ..self
        }
    }
}

/// エラーコード。仕様書 §15 + 提案コード。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, PartialOrd, Ord)]
pub enum Code {
    /// 不正字句/インデント
    SYN001,
    /// [提案] ソースが NFC 正規化されていない
    SYN002,
    /// 型不一致
    TYP001,
    /// [提案] 代入規則違反(不変値・取引外のエンティティ更新)
    TYP002,
    /// [提案] 戻り値の欠落
    TYP003,
    /// 未定義参照
    REF001,
    /// [提案] 重複定義
    REF002,
    /// 未解決意図
    AMB001,
    /// 複数解釈
    AMB002,
    /// 未承認
    APP001,
    /// ロック不整合
    LOCK001,
    /// 効果違反
    SEC001,
    /// Capability 不足
    SEC002,
    /// ターゲット未対応
    TGT001,
    /// 生成失敗
    GEN001,
    /// [提案] 内部障害・入出力障害
    INT001,
}

impl Code {
    pub fn as_str(self) -> &'static str {
        match self {
            Code::SYN001 => "SYN001",
            Code::SYN002 => "SYN002",
            Code::TYP001 => "TYP001",
            Code::TYP002 => "TYP002",
            Code::TYP003 => "TYP003",
            Code::REF001 => "REF001",
            Code::REF002 => "REF002",
            Code::AMB001 => "AMB001",
            Code::AMB002 => "AMB002",
            Code::APP001 => "APP001",
            Code::LOCK001 => "LOCK001",
            Code::SEC001 => "SEC001",
            Code::SEC002 => "SEC002",
            Code::TGT001 => "TGT001",
            Code::GEN001 => "GEN001",
            Code::INT001 => "INT001",
        }
    }

    /// 技術仕様書 §14 の終了コード。
    /// 0=成功、2=構文/型、3=曖昧、4=権限、5=ロック、6=ターゲット非対応、7=内部障害。
    /// APP001(未承認)は「曖昧さが確定していない」ものとして 3 に割り当てる(ADR-0103)。
    /// REF001/REF002 は構文/型と同じ 2。GEN001 は 7。
    pub fn exit_code(self) -> i32 {
        match self {
            Code::SYN001 | Code::SYN002 | Code::TYP001 | Code::TYP002 | Code::TYP003 => 2,
            Code::REF001 | Code::REF002 => 2,
            Code::AMB001 | Code::AMB002 | Code::APP001 => 3,
            Code::SEC001 | Code::SEC002 => 4,
            Code::LOCK001 => 5,
            Code::TGT001 => 6,
            Code::GEN001 | Code::INT001 => 7,
        }
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

/// 診断。§15「全診断に code, severity, message_ja, file, span, hints[] を含める」。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Diagnostic {
    pub code: Code,
    pub severity: Severity,
    pub message_ja: String,
    /// ファイルパス。driver が FileId から解決して埋める。
    pub file: String,
    pub span: Span,
    pub hints: Vec<String>,
}

impl Diagnostic {
    pub fn error(code: Code, span: Span, message: impl Into<String>) -> Self {
        Diagnostic {
            code,
            severity: Severity::Error,
            message_ja: message.into(),
            file: String::new(),
            span,
            hints: Vec::new(),
        }
    }
    pub fn warning(code: Code, span: Span, message: impl Into<String>) -> Self {
        Diagnostic {
            severity: Severity::Warning,
            ..Diagnostic::error(code, span, message)
        }
    }
    pub fn hint(mut self, h: impl Into<String>) -> Self {
        self.hints.push(h.into());
        self
    }
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sev = match self.severity {
            Severity::Error => "エラー",
            Severity::Warning => "警告",
            Severity::Info => "情報",
        };
        write!(
            f,
            "{}:{}:{}: {}[{}] {}",
            if self.file.is_empty() { "<input>" } else { &self.file },
            self.span.line,
            self.span.column,
            sev,
            self.code,
            self.message_ja
        )?;
        for h in &self.hints {
            write!(f, "\n    ヒント: {h}")?;
        }
        Ok(())
    }
}

/// 診断の集合。エラーの有無と終了コードを決める。
#[derive(Debug, Default, Clone)]
pub struct Diagnostics {
    pub items: Vec<Diagnostic>,
}

impl Diagnostics {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn push(&mut self, d: Diagnostic) {
        self.items.push(d);
    }
    pub fn extend(&mut self, it: impl IntoIterator<Item = Diagnostic>) {
        self.items.extend(it);
    }
    pub fn has_errors(&self) -> bool {
        self.items.iter().any(|d| d.is_error())
    }
    pub fn errors(&self) -> impl Iterator<Item = &Diagnostic> {
        self.items.iter().filter(|d| d.is_error())
    }
    pub fn codes(&self) -> Vec<Code> {
        self.errors().map(|d| d.code).collect()
    }
    /// 終了コード: エラーのうち「最も早い段階」のカテゴリを優先する。
    /// 優先順: 構文/型(2) > 曖昧(3) > 権限(4) > ロック(5) > ターゲット(6) > 内部(7)。
    /// 0 はエラーなし。
    pub fn exit_code(&self) -> i32 {
        self.errors().map(|d| d.code.exit_code()).min().unwrap_or(0)
    }
    /// 出力順を決定的にする(ファイル→位置→コード)。
    pub fn sort(&mut self) {
        self.items.sort_by(|a, b| {
            (a.file.as_str(), a.span.start, a.code, a.message_ja.as_str()).cmp(&(b.file.as_str(), b.span.start, b.code, b.message_ja.as_str()))
        });
        self.items.dedup();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_code_prefers_earliest_category() {
        let mut d = Diagnostics::new();
        d.push(Diagnostic::error(Code::SEC001, Span::default(), "x"));
        d.push(Diagnostic::error(Code::TYP001, Span::default(), "y"));
        assert_eq!(d.exit_code(), 2);
        let mut d2 = Diagnostics::new();
        d2.push(Diagnostic::warning(Code::TYP001, Span::default(), "w"));
        assert_eq!(d2.exit_code(), 0);
    }

    #[test]
    fn exit_codes_match_spec_section_14() {
        assert_eq!(Code::SYN001.exit_code(), 2);
        assert_eq!(Code::AMB001.exit_code(), 3);
        assert_eq!(Code::SEC002.exit_code(), 4);
        assert_eq!(Code::LOCK001.exit_code(), 5);
        assert_eq!(Code::TGT001.exit_code(), 6);
        assert_eq!(Code::GEN001.exit_code(), 7);
    }
}
