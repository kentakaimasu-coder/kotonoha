# Surface AST 仕様

状態: 提案 / 最終更新: 2026-11-02 / 上位文書: 技術仕様書 §12 / 実装: `crates/ast`(`koto ast <file>` で JSON 出力)

全ノードは `Span { line, column, start, end }`(+ 内部用 FileId)を持つ。AST は不変データとして後段に渡す。

```text
SourceFile { items: [Item] }
Item       { annotations: [Annotation], decl: Decl }
Annotation { name: Ident, arg: Ident?, span }

Decl =
  | App        { name: String }
  | TypeAlias  { name: Ident, ty: TypeExpr }
  | Enum       { name: Ident, variants: [Ident] }
  | Data       { name: Ident, fields: [Field] }
  | Adapter    { name: Ident, params: [Param], ret: TypeExpr, op: OpKind, target: Ident }
  | Knowledge  { name: Ident, entries: [KvEntry { key: Ident, value: RawText }] }
  | Screen     { name: Ident, lines: [RawText] }
  | Action     { name: Ident, params: [Param], ret: TypeExpr?, items: [ActionItem] }
  | Contract   { lines: [RawText] }

Field / Param { name: Ident, ty: TypeExpr }
TypeExpr  = Named { name: Ident, args: [TypeExpr] } | Record { fields: [Field] }
OpKind    = Get(取得) | List(一覧) | Create(登録) | Update(更新) | Delete(削除)

ActionItem =
  | Requires { caps: [CapRef { resource: Ident, verb: Ident }] }
  | Effects  { effects: [EffectRef { path: String }] }
  | Strict(Block)
  | Intent(RawText)

Block { stmts: [Stmt] }
Stmt =
  | Check       { cond: Expr, fail: Expr(=Call 失敗(x)) }
  | Return      { value: Expr }
  | If          { branches: [(Expr, Block)], else_block: Block? }
  | Loop        { var: Ident, iter: Expr, body: Block }
  | Transaction { body: Block }
  | Assign      { target: Place { root: Ident, fields: [Ident] }, value: Expr }
  | Expr        { expr: Expr(Call) }

Expr { kind, span }
ExprKind =
  | Int(i64) | Decimal(String) | Str(String) | Bool | None
  | Ident(String)
  | Field(Expr, Ident)
  | Call { callee: Ident, args: [Expr] }          (* 失敗/成功 も Call *)
  | Unary { op: Neg | Not, expr }
  | Binary { op: Or|And|Eq|Ne|Lt|Le|Gt|Ge|Add|Sub|Mul|Div, lhs, rhs }
  | List([Expr])

RawText { text: String, span }   (* 意図層の生テキスト。この段階では意味を持たない *)
```

## 設計判断
- 意図層の文を AST の段階で解釈しないことで、「日本語文が直接実行される」経路を構造的に持たない(要件 §1)。
- コメントは保持しない(ADR-0107)。
- `Decimal` は字面を保持(精度方針 ADR-0106 が決まるまで数値化しない)。
- 後段の型付き表現は AST を変更せず、`types::Env`(宣言の型)、`knowledge::Registry`、`intent::IntentModel`、`validator::Validation` を別に構築する。
