//! Surface AST(技術仕様書 §12 の「Surface AST」)。
//!
//! 構造の正式定義は `docs/spec/ast.md`。全ノードがソース位置(Span)を持つ。
//! 意図層の文(画面本体・約束・知識の値)は Surface AST では「生テキスト(RawText)」として保持し、
//! 意味付けは後段(knowledge / intent クレート)の有限パターン照合で行う。
//! これにより「日本語の文章をそのまま実行しない」ことを構造的に保証する。

use kotonoha_diag::{FileId, Span};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Ident {
    pub name: String,
    pub span: Span,
}

/// 意図層の生テキスト(コメント除去・前後空白除去済み)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RawText {
    pub text: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SourceFile {
    #[serde(skip)]
    pub file: FileId,
    pub items: Vec<Item>,
}

/// 注釈付き宣言。注釈は `@厳密` / `@柔軟(確認)` / `@柔軟(自動)` のみ有効(意味検査で確認)。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Item {
    pub annotations: Vec<Annotation>,
    pub decl: Decl,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Annotation {
    pub name: Ident,
    pub arg: Option<Ident>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind")]
pub enum Decl {
    App(AppDecl),
    TypeAlias(TypeAliasDecl),
    Enum(EnumDecl),
    Data(DataDecl),
    Adapter(AdapterDecl),
    Knowledge(KnowledgeDecl),
    Screen(ScreenDecl),
    Action(ActionDecl),
    Contract(ContractDecl),
    /// `関数 名前(引数) -> 型:` + 厳密層の本体(副作用なし)
    Function(FunctionDecl),
    /// `外部関数` / `分析` / `部品`(実装ファイルを `キー = 値` 行で指定する宣言)
    External(ExternalDecl),
    /// `実験 名前:` + `キー = 値` 行(A/B テスト)
    Experiment(ExperimentDecl),
    /// `役割 名前:` + `権限 = [...]`(役割ごとの権限。ADR-0126)
    Role(ExperimentDecl),
}

impl Decl {
    pub fn span(&self) -> Span {
        match self {
            Decl::App(d) => d.span,
            Decl::TypeAlias(d) => d.span,
            Decl::Enum(d) => d.span,
            Decl::Data(d) => d.span,
            Decl::Adapter(d) => d.span,
            Decl::Knowledge(d) => d.span,
            Decl::Screen(d) => d.span,
            Decl::Action(d) => d.span,
            Decl::Contract(d) => d.span,
            Decl::Function(d) => d.span,
            Decl::External(d) => d.span,
            Decl::Experiment(d) => d.span,
            Decl::Role(d) => d.span,
        }
    }
}

/// `アプリ「商店」`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AppDecl {
    pub name: String,
    pub span: Span,
}

/// `型 注文ID = 識別子`(透過的な別名。ADR-0102)
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TypeAliasDecl {
    pub name: Ident,
    pub ty: TypeExpr,
    pub span: Span,
}

/// `列挙 注文失敗:` + 1 行 1 値
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EnumDecl {
    pub name: Ident,
    pub variants: Vec<Ident>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DataDecl {
    pub name: Ident,
    pub fields: Vec<Field>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Field {
    pub name: Ident,
    pub ty: TypeExpr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind")]
pub enum TypeExpr {
    /// `整数`, `金額<JPY>`, `結果<注文ID, 注文失敗>`
    Named { name: Ident, args: Vec<TypeExpr>, span: Span },
    /// `記録{名前: 文字列, 価格: 金額<JPY>}`
    Record { fields: Vec<Field>, span: Span },
}

impl TypeExpr {
    pub fn span(&self) -> Span {
        match self {
            TypeExpr::Named { span, .. } | TypeExpr::Record { span, .. } => *span,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Param {
    pub name: Ident,
    pub ty: TypeExpr,
    pub span: Span,
}

/// データアダプタの操作種別(ADR-0104)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, PartialOrd, Ord)]
pub enum OpKind {
    /// 取得: ID で 1 件
    Get,
    /// 一覧
    List,
    /// 登録
    Create,
    /// 更新
    Update,
    /// 削除
    Delete,
}

impl OpKind {
    pub fn from_word(w: &str) -> Option<OpKind> {
        Some(match w {
            "取得" => OpKind::Get,
            "一覧" => OpKind::List,
            "登録" => OpKind::Create,
            "更新" => OpKind::Update,
            "削除" => OpKind::Delete,
            _ => return None,
        })
    }
    pub fn word(self) -> &'static str {
        match self {
            OpKind::Get => "取得",
            OpKind::List => "一覧",
            OpKind::Create => "登録",
            OpKind::Update => "更新",
            OpKind::Delete => "削除",
        }
    }
}

/// ```text
/// アダプタ 商品をIDで取得(ID: 識別子) -> 商品:
///     操作 = 取得(商品)
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AdapterDecl {
    pub name: Ident,
    pub params: Vec<Param>,
    pub ret: TypeExpr,
    pub op: OpKind,
    pub op_span: Span,
    pub target: Ident,
    pub span: Span,
}

/// `知識 売れ筋商品:` + `キー = 値` 行。値は knowledge クレートが解釈する。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KnowledgeDecl {
    pub name: Ident,
    pub entries: Vec<KvEntry>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KvEntry {
    pub key: Ident,
    pub value: RawText,
    pub span: Span,
}

/// `画面 商品一覧:` + 意図行
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScreenDecl {
    pub name: Ident,
    pub lines: Vec<RawText>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ActionDecl {
    pub name: Ident,
    pub params: Vec<Param>,
    pub ret: Option<TypeExpr>,
    pub items: Vec<ActionItem>,
    pub span: Span,
}

impl ActionDecl {
    pub fn requires(&self) -> Option<(&[CapRef], Span)> {
        self.items.iter().find_map(|i| match i {
            ActionItem::Requires { caps, span } => Some((caps.as_slice(), *span)),
            _ => None,
        })
    }
    pub fn effects(&self) -> Option<(&[EffectRef], Span)> {
        self.items.iter().find_map(|i| match i {
            ActionItem::Effects { effects, span } => Some((effects.as_slice(), *span)),
            _ => None,
        })
    }
    pub fn strict_blocks(&self) -> impl Iterator<Item = &Block> {
        self.items.iter().filter_map(|i| match i {
            ActionItem::Strict(b) => Some(b),
            _ => None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind")]
pub enum ActionItem {
    /// `必要 = [注文.作成, 商品.更新]`
    Requires { caps: Vec<CapRef>, span: Span },
    /// `効果 = [data.read]`(ADR-0105 提案)
    Effects { effects: Vec<EffectRef>, span: Span },
    /// `厳密:` ブロック
    Strict(Block),
    /// 動作本体の意図行(MVP では照合パターン未定義のため AMB001 になる)
    Intent(RawText),
}

/// `資源.動詞`(例 `商品.更新`)
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CapRef {
    pub resource: Ident,
    pub verb: Ident,
    pub span: Span,
}

/// `data.read` などのドット区切り効果名
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EffectRef {
    pub path: String,
    pub span: Span,
}

/// `約束:` + 意図行
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContractDecl {
    pub lines: Vec<RawText>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind")]
pub enum Stmt {
    /// `条件 を確認 または 失敗(値)`
    Check { cond: Expr, fail: Expr, span: Span },
    /// `値 を返す`
    Return { value: Expr, span: Span },
    /// `もし 条件:` … `そうでなければ もし 条件:` … `そうでなければ:`
    If {
        branches: Vec<(Expr, Block)>,
        else_block: Option<Block>,
        span: Span,
    },
    /// `繰り返す 変数 から 一覧:`(ADR-0102 提案構文)
    Loop { var: Ident, iter: Expr, body: Block, span: Span },
    /// `取引:`
    Transaction { body: Block, span: Span },
    /// `場所 = 式`
    Assign { target: Place, value: Expr, span: Span },
    /// 関数呼出のみの文
    Expr { expr: Expr, span: Span },
}

impl Stmt {
    pub fn span(&self) -> Span {
        match self {
            Stmt::Check { span, .. }
            | Stmt::Return { span, .. }
            | Stmt::If { span, .. }
            | Stmt::Loop { span, .. }
            | Stmt::Transaction { span, .. }
            | Stmt::Assign { span, .. }
            | Stmt::Expr { span, .. } => *span,
        }
    }
}

/// 代入先: `変数` または `変数.フィールド(.フィールド)*`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Place {
    pub root: Ident,
    pub fields: Vec<Ident>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", content = "value")]
pub enum ExprKind {
    Int(i64),
    /// 小数リテラルは字面のまま保持(decimal128 相当の精度方針は ADR-0106 で未決定)
    Decimal(String),
    Str(String),
    Bool(bool),
    /// `なし`(選択<T> の空)
    None,
    Ident(String),
    Field(Box<Expr>, Ident),
    Call {
        callee: Ident,
        args: Vec<Expr>,
    },
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
    },
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    List(Vec<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum UnaryOp {
    Neg,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum BinOp {
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Add,
    Sub,
    Mul,
    Div,
}

impl BinOp {
    pub fn symbol(self) -> &'static str {
        match self {
            BinOp::Or => "||",
            BinOp::And => "&&",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
        }
    }
    /// 優先順位(大きいほど強く結合)。docs/spec/grammar.md §4 と一致させること。
    pub fn precedence(self) -> u8 {
        match self {
            BinOp::Or => 1,
            BinOp::And => 2,
            BinOp::Eq | BinOp::Ne => 3,
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => 4,
            BinOp::Add | BinOp::Sub => 5,
            BinOp::Mul | BinOp::Div => 6,
        }
    }
    /// 比較演算子は非結合(`a < b < c` は構文エラー)。
    pub fn is_non_assoc(self) -> bool {
        self.precedence() == 3 || self.precedence() == 4
    }
}

/// `関数 名前(引数) -> 型:`(ADR-0124)
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FunctionDecl {
    pub name: Ident,
    pub params: Vec<Param>,
    pub ret: TypeExpr,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, PartialOrd, Ord)]
pub enum ExternalKind {
    /// 外部関数(TypeScript / Python。ADR-0124)
    Function,
    /// 分析(読み取り専用 SQL。ADR-0125)
    Analysis,
    /// 拡張部品(ADR-0123)
    Component,
}

impl ExternalKind {
    pub fn word(self) -> &'static str {
        match self {
            ExternalKind::Function => "外部関数",
            ExternalKind::Analysis => "分析",
            ExternalKind::Component => "部品",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExternalDecl {
    pub kind: ExternalKind,
    pub name: Ident,
    pub params: Vec<Param>,
    pub ret: Option<TypeExpr>,
    pub entries: Vec<KvEntry>,
    pub span: Span,
}

/// `実験 名前:`(ADR-0122)
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExperimentDecl {
    pub name: Ident,
    pub entries: Vec<KvEntry>,
    pub span: Span,
}
