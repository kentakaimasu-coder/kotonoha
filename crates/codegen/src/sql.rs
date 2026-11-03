//! 知識(集計ランキング)→ パラメータ化 SQL(技術仕様書 §8、SEC-04)。
//!
//! - 値はすべてバインド変数。SQL 文字列に入るのは引用符付き識別子と固定の構文だけ
//! - 物理マッピング(表名・列名)は MVP では「宣言名をそのまま引用符付きで使う」恒等写像(ADR-0110 提案)
//! - SQLite の格納形式(ADR-0110 提案): 金額<JPY>=INTEGER(円)、日時=INTEGER(UTC エポックミリ秒)、列挙/識別子/文字列=TEXT

use kotonoha_ast::{BinOp, Expr, ExprKind, UnaryOp};
use kotonoha_diag::{Code, Diagnostic};
use kotonoha_knowledge::{Agg, Dir, RankingSpec};
use kotonoha_types::{DataInfo, Type};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    Sqlite,
    Postgres,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SqlParam {
    Text(String),
    Int(i64),
    Bool(bool),
    /// 実行時に計算: 基準日時 - N 日(ミリ秒)
    WindowStart,
    /// 実行時に計算: 基準日時(ミリ秒)
    WindowEnd,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LoweredQuery {
    pub sql: String,
    pub params: Vec<SqlParam>,
}

pub fn quote_ident(s: &str) -> Result<String, Diagnostic> {
    if s.is_empty() || s.contains('"') || s.chars().any(|c| c.is_control()) {
        return Err(Diagnostic::error(
            Code::GEN001,
            Default::default(),
            format!("識別子 `{s}` は SQL に安全に埋め込めません"),
        ));
    }
    Ok(format!("\"{s}\""))
}

struct Builder {
    dialect: Dialect,
    params: Vec<SqlParam>,
}

impl Builder {
    fn bind(&mut self, p: SqlParam) -> String {
        self.params.push(p);
        match self.dialect {
            Dialect::Sqlite => format!("?{}", self.params.len()),
            Dialect::Postgres => format!("${}", self.params.len()),
        }
    }

    fn filter(&mut self, e: &Expr) -> Result<String, Diagnostic> {
        let unsupported = || {
            Diagnostic::error(
                Code::GEN001,
                e.span,
                "この条件式は SQL に変換できません(MVP は比較・論理演算・リテラル・列参照のみ)",
            )
        };
        Ok(match &e.kind {
            ExprKind::Ident(n) => quote_ident(n)?,
            ExprKind::Str(s) => self.bind(SqlParam::Text(s.clone())),
            ExprKind::Int(v) => self.bind(SqlParam::Int(*v)),
            ExprKind::Bool(b) => self.bind(SqlParam::Bool(*b)),
            ExprKind::Unary { op: UnaryOp::Not, expr } => format!("(NOT {})", self.filter(expr)?),
            ExprKind::Unary { op: UnaryOp::Neg, expr } => format!("(-{})", self.filter(expr)?),
            ExprKind::Binary { op, lhs, rhs } => {
                let o = match op {
                    BinOp::Or => "OR",
                    BinOp::And => "AND",
                    BinOp::Eq => "=",
                    BinOp::Ne => "<>",
                    BinOp::Lt => "<",
                    BinOp::Le => "<=",
                    BinOp::Gt => ">",
                    BinOp::Ge => ">=",
                    _ => return Err(unsupported()),
                };
                format!("({} {o} {})", self.filter(lhs)?, self.filter(rhs)?)
            }
            _ => return Err(unsupported()),
        })
    }
}

/// 集計ランキングを SQL にする。
pub fn lower_ranking(spec: &RankingSpec, dialect: Dialect) -> Result<LoweredQuery, Diagnostic> {
    let mut b = Builder { dialect, params: Vec::new() };
    let table = quote_ident(&spec.source)?;
    let unit = quote_ident(&spec.unit)?;
    let out = quote_ident(&spec.metric.output)?;
    let agg = match (spec.metric.agg, &spec.metric.field) {
        (Agg::Sum, Some(f)) => format!("SUM({})", quote_ident(f)?),
        (Agg::Count, _) => "COUNT(*)".to_string(),
        (Agg::Sum, None) => return Err(Diagnostic::error(Code::GEN001, spec.span, "合計対象の列がありません")),
    };
    let time = quote_ident(&spec.time_field)?;
    let mut wheres = Vec::new();
    if let Some(f) = &spec.filter {
        wheres.push(b.filter(f)?);
    }
    let ws = b.bind(SqlParam::WindowStart);
    let we = b.bind(SqlParam::WindowEnd);
    wheres.push(format!("{time} >= {ws}"));
    wheres.push(format!("{time} < {we}"));
    let order: Vec<String> = spec
        .order
        .iter()
        .map(|o| Ok(format!("{} {}", quote_ident(&o.column)?, if o.dir == Dir::Desc { "DESC" } else { "ASC" })))
        .collect::<Result<_, Diagnostic>>()?;
    let limit = b.bind(SqlParam::Int(spec.limit as i64));
    let sql = format!(
        "SELECT {unit}, {agg} AS {out}\nFROM {table}\nWHERE {}\nGROUP BY {unit}\nORDER BY {}\nLIMIT {limit};",
        wheres.join("\n  AND "),
        order.join(", ")
    );
    Ok(LoweredQuery { sql, params: b.params })
}

/// 区間 [基準 - N 日, 基準)(ミリ秒)。固定オフセットの時刻帯(Asia/Tokyo, UTC)では
/// 「N 暦日前の同時刻」と「N×24 時間前」が一致する。夏時間のある時刻帯は未対応(ADR-0108)。
pub fn window_ms(spec: &RankingSpec, base_ms: i64) -> Option<(i64, i64)> {
    let span = (spec.window_days as i64).checked_mul(86_400_000)?;
    Some((base_ms.checked_sub(span)?, base_ms))
}

/// SQLite 用の DDL(テストフィクスチャと将来の Backend 生成で使う)。
pub fn sqlite_ddl(data: &DataInfo, env: &kotonoha_types::Env) -> Result<String, Diagnostic> {
    sqlite_ddl_for(&data.name, &data.fields, &env.enums)
}

/// 列定義(マイグレーション照合用)。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ColumnDef {
    pub name: String,
    pub sql_type: String,
    /// PostgreSQL の型(information_schema.data_type の大文字表記)
    pub pg_type: String,
    pub notnull: bool,
    pub pk: bool,
    /// `"列" 型 [NOT NULL] [PRIMARY KEY] [CHECK]`
    pub def: String,
    pub pg_def: String,
}

pub fn sqlite_columns(fields: &[(String, Type)], enums: &std::collections::BTreeMap<String, Vec<String>>) -> Result<Vec<ColumnDef>, Diagnostic> {
    let mut cols = Vec::new();
    for (n, t) in fields {
        let (base, nullable) = match t {
            Type::Option(inner) => (inner.as_ref(), true),
            other => (other, false),
        };
        let ty = match base {
            Type::Int | Type::Bool | Type::DateTime => "INTEGER",
            Type::Money(c) if c == "JPY" => "INTEGER",
            Type::Money(c) => {
                return Err(Diagnostic::error(
                    Code::GEN001,
                    Default::default(),
                    format!("金額<{c}> の格納精度は未決定です(ADR-0106)"),
                ))
            }
            Type::Str | Type::Id | Type::Url | Type::Email | Type::Enum(_) => "TEXT",
            other => {
                return Err(Diagnostic::error(
                    Code::GEN001,
                    Default::default(),
                    format!("型 {other} の SQLite 格納形式は未定義です"),
                ))
            }
        };
        let pg_ty = if ty == "INTEGER" { "BIGINT" } else { "TEXT" };
        let pk = n == "ID";
        let mut tail = String::new();
        if !nullable {
            tail.push_str(" NOT NULL");
        }
        if pk {
            tail.push_str(" PRIMARY KEY");
        }
        if let Type::Enum(e) = base {
            let vs: Vec<String> = enums[e].iter().map(|v| format!("'{}'", v.replace('\'', "''"))).collect();
            tail.push_str(&format!(" CHECK ({} IN ({}))", quote_ident(n)?, vs.join(", ")));
        }
        let def = format!("{} {ty}{tail}", quote_ident(n)?);
        let pg_def = format!("{} {pg_ty}{tail}", quote_ident(n)?);
        cols.push(ColumnDef {
            name: n.clone(),
            sql_type: ty.into(),
            pg_type: pg_ty.into(),
            notnull: !nullable,
            pk,
            def,
            pg_def,
        });
    }
    Ok(cols)
}

/// `CREATE TABLE IF NOT EXISTS`(既存表との差分は backend ランタイムの migrate が照合する)
pub fn sqlite_ddl_for(name: &str, fields: &[(String, Type)], enums: &std::collections::BTreeMap<String, Vec<String>>) -> Result<String, Diagnostic> {
    let cols = sqlite_columns(fields, enums)?;
    Ok(format!(
        "CREATE TABLE IF NOT EXISTS {} (\n    {}\n);",
        quote_ident(name)?,
        cols.iter().map(|c| c.def.as_str()).collect::<Vec<_>>().join(",\n    ")
    ))
}

/// PostgreSQL 用の `CREATE TABLE IF NOT EXISTS`
pub fn pg_ddl_for(name: &str, fields: &[(String, Type)], enums: &std::collections::BTreeMap<String, Vec<String>>) -> Result<String, Diagnostic> {
    let cols = sqlite_columns(fields, enums)?;
    Ok(format!(
        "CREATE TABLE IF NOT EXISTS {} (\n    {}\n);",
        quote_ident(name)?,
        cols.iter().map(|c| c.pg_def.as_str()).collect::<Vec<_>>().join(",\n    ")
    ))
}
