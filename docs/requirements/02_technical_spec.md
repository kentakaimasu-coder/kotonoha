# KOTONOHA 技術仕様書
版: 1.0-draft / 2026-11-03 / 実装可能なMVP仕様（P1以降は契約と拡張点を規定）

## 1. 設計原則
Intent First / Meaning Once / Verified Execution / Strict Escape Hatch / AI-free Runtime。曖昧さを認めるのは開発時だけ。実行成果物は意味が確定していること。

## 2. 最小の言語サンプル
```kotonoha
アプリ「商店」

データ 商品:
    ID: 識別子
    名前: 文字列
    価格: 金額<JPY>
    在庫: 整数

画面 商品一覧:
    商品を一覧表示
    商品を選ぶと 商品詳細 へ

画面 商品詳細:
    商品の名前と価格を表示
    「購入」を押すと 注文する(商品.ID, 1)

動作 注文する(商品ID: 識別子, 数量: 整数) -> 結果<注文ID, 注文失敗>:
    必要 = [注文.作成, 商品.更新]
    厳密:
        数量 > 0 を確認 または 失敗("数量不正")
        取引:
            商品 = 商品をIDで取得(商品ID)
            商品.在庫 >= 数量 を確認 または 失敗("在庫不足")
            商品.在庫 = 商品.在庫 - 数量
            注文ID = 注文を登録(商品ID, 数量)
            注文IDを返す

約束:
    個人情報は本人だけ閲覧
    未認証なら注文不可
```
注: `商品をIDで取得`、`注文を登録` は標準提供ではなく、型付きデータアダプタが定義する操作。初期MVPでは明示的なアダプタ定義が必要。

## 3. 文法の二層化
### 3.1 意図層
定義済みパターンを優先し、未登録の日本語は `@柔軟` 宣言またはプロジェクト設定時だけ解釈候補へ送る。未承認の候補はコンパイル不可。`画面 商品一覧:` 等の宣言は決定的に解析。
### 3.2 厳密層
式、代入、関数呼出、分岐、繰り返し、取引、エラー処理を静的に解析する。AIを呼び出さない。識別子は日本語/ASCII、演算子は `+ - * / == != < <= > >= && ||` を採用。`=` は代入、`==` は比較。インデントは4スペース、タブ禁止。UTF-8、NFC正規化。全角記号の自動同一視はしない。

### 3.3 EBNF抜粋
```ebnf
program      = { declaration } ;
declaration  = app | data | screen | action | knowledge | contract ;
app          = "アプリ", string, newline ;
data         = "データ", identifier, ":", newline, indent, { field }, dedent ;
field        = identifier, ":", type, newline ;
screen       = "画面", identifier, ":", newline, indent, { intent_line }, dedent ;
action       = "動作", identifier, "(", [ params ], ")", [ "->", type ], ":", newline, indent, { statement }, dedent ;
statement    = assignment | strict_block | intent_line | contract_line ;
strict_block = "厳密:", newline, indent, { strict_stmt }, dedent ;
assignment   = identifier, "=", expression, newline ;
intent_line  = japanese_sentence, newline ;
```
`japanese_sentence` は任意実行を意味しない。既知パターン照合に失敗したら `UNRESOLVED_INTENT` とする。正確な字句規則と演算子優先順位はPhase 0で機械可読仕様として固定。

## 4. 型システム
プリミティブ: 真偽, 整数(i64), 小数(decimal128相当), 文字列, 日時, 日付, 識別子, バイト列。ドメイン型: 金額<通貨>, 期間, URL, メールアドレス。合成型: 配列<T>, 一覧<T>, 選択<T>, 結果<T,E>, 記録{...}。NULLは `選択<T>` のみ。暗黙の数値→文字列変換禁止。金額の通貨違いは演算不可。日時はタイムゾーンを必須にする。数値オーバーフローは検査し例外/結果へ変換。型推論は局所式に限定し公開APIは型明示。

## 5. 意図と厳密処理の接続
意図ノードは `画面/動作/問い合わせ/表示/遷移/検証` の有限集合から開始。各ノードは `inputs, outputs, effects, capabilities, source_span, knowledge_refs` を持つ。`厳密:` 内の操作は型付き関数呼出として同じGraphへ接続。曖昧な意図を厳密な関数に接続する際は引数型・結果型・副作用を照合する。

## 6. Verified Application Graph (VAG) v1
```json
{
  "schema_version":"1.0",
  "application":{"name":"商店"},
  "nodes":[
    {"id":"screen.products","kind":"screen","title":"商品一覧","inputs":[],"outputs":["商品[]"],"effects":["data.read"],"capabilities":["商品.read"],"source_span":{"file":"app.koto","line":8}},
    {"id":"action.order","kind":"action","inputs":["識別子","整数"],"outputs":["結果<注文ID,注文失敗>"],"effects":["data.read","data.write"],"capabilities":["注文.create","商品.update"],"source_span":{"file":"app.koto","line":15}}
  ],
  "edges":[{"from":"screen.products","to":"action.order","kind":"invokes"}],
  "status":"validated"
}
```
ノードIDは宣言の安定IDを用いる。ハッシュは正規化されたJSONで計算。グラフの `validated` は承認・型・効果・権限・参照整合性の全検証を通過したときだけ付与。

## 7. 曖昧性と承認
モード: `@厳密`（既定）、`@柔軟(確認)`、`@柔軟(自動)`。柔軟自動は承認済みの知識と別名だけを使用し、未登録の意味をAIが無断確定しない。AIは解釈候補をJSON Schema準拠で返す。候補には `intent_id, interpretations, assumptions, unresolved, required_effects, evidence_refs` を必須とする。未確定フィールドがあれば承認不可。承認者は解釈差分と権限を確認。承認後はソース・知識・スキーマ・IR・ターゲットのハッシュをロック。プロダクションビルドは `--locked` 必須。

## 8. 売れ筋集計の確定例
確定事項: 最近=直近30日、売れ筋=売上金額、件数=20。
```kotonoha
知識 売れ筋商品:
    対象 = 注文明細
    期間 = 直近30日
    指標 = 売上金額の合計
    単位 = 商品ID
    並び = 売上金額の降順, 商品IDの昇順
    件数 = 20

画面 売れ筋:
    売れ筋商品を表示
```
追加で必須の業務決定: 注文状態、取消/返品、税込/税抜、基準日時、時刻帯、NULL、データソースの物理マッピング。例示のテストフィクスチャでは `状態=確定`、取消除外、税込JPY、Asia/Tokyo、区間 `[基準日時-30日, 基準日時)`、NULL金額拒否と仮定し、正式採用前に承認する。

SQLバックエンドの参考出力（PostgreSQL、識別子はマッピング済み）:
```sql
SELECT product_id, SUM(amount_jpy) AS revenue_jpy
FROM order_items
WHERE status = 'confirmed'
  AND confirmed_at >= :window_start
  AND confirmed_at < :window_end
GROUP BY product_id
ORDER BY revenue_jpy DESC, product_id ASC
LIMIT 20;
```
`:window_start`, `:window_end` は実際のドライバのパラメータ記法へ変換する。返品を別テーブルで管理する場合は承認済みの調整ロジックを追加する。単純なSQL例を業務仕様の確定と混同しない。

## 9. セキュリティ実行モデル
Effects: pure, data.read, data.write, network.send, filesystem.read/write, device.camera/location/notifications。Capabilitiesは対象資源に紐づく（例 `商品.read`）。静的検査は呼出グラフ上の必要権限を集約。実行時にもサーバーで本人・組織・リソース単位の認可を実施。画面の権限表示だけでは保護としない。DBアクセスはパラメータ化。HTML/URL/JS文脈に応じて出力をエスケープ。秘密情報はSecrets Provider参照のみ。AI出力は一切コードとして直接評価しない。

## 10. UI/状態/イベント
UI IR: `Page, Layout, Text, Image, Form, List, Button, Navigation, Dialog`。State IR: `local, derived, server`。Event IR: `tap, submit, change, load`。遷移は明示イベントと型付きAction参照。レイアウトはデザイントークン、ブレークポイント、アクセシビリティ属性を含む。曖昧な「高級感」はデザイントークン候補へ変換し承認固定。画像素材は自動生成せず、明示アセット参照またはプレースホルダー。

## 11. ターゲット別バックエンド契約
| Target | 初期生成 | 共通仕様 | 固有仕様 |
|---|---|---|---|
| web | React/TypeScript, SSR可能 | UI IR, State, Actions | SEO, browser navigation |
| backend | Node.js/TypeScript | Types, Effects, Auth | DB adapter, HTTP routing |
| ios | SwiftUI/Swift | UI IR, State, Actions | OS permissions, navigation |
| android | Kotlin/Compose | UI IR, State, Actions | OS permissions, navigation |

各生成器は `supports(node_kind, feature)` を返し、未対応機能を黙って欠落させず `TARGET_UNSUPPORTED` で停止。iOS/AndroidはMVP段階でスタブと契約テストのみ、P1で本生成を実装。

## 12. コンパイラパイプライン
UTF-8 source → Lexer → Parser → Surface AST → Knowledge Resolver → (optional AI candidate) → Approval/Lock → Typed VAG → Effect/Capability Validator → Target Lowering → Generated Project → Test/Build。各フェーズは不変データ構造を入出力とし、診断にソース位置を付与。AIなしで同一入力から同一VAGを生成できること。

## 13. ロックファイル
ファイル `kotonoha.lock.json`、正規化JSON、SHA-256。最低フィールド: `lock_version, compiler_version, source_hash, knowledge_hash, schema_hash, graph_hash, target, approved_interpretations[], approval_reference`。承認の真正性は署名済み承認レコードまたは保護された監査サービスで検証し、自己申告の `approved=true` だけを信用しない。ロックと実際の入力が一致しない場合 `LOCK_STALE`。

## 14. CLI契約
`koto init`, `koto fmt`, `koto check`, `koto resolve`, `koto approve`, `koto graph`, `koto build --target web|backend|ios|android --locked`, `koto test`, `koto doctor`。終了コード 0=成功、2=構文/型、3=曖昧、4=権限、5=ロック、6=ターゲット非対応、7=内部障害。`--json` で機械可読診断を出力。

## 15. エラー仕様
SYN001 不正字句/インデント、TYP001 型不一致、REF001 未定義参照、AMB001 未解決意図、AMB002 複数解釈、APP001 未承認、LOCK001 ロック不整合、SEC001 効果違反、SEC002 Capability不足、TGT001 ターゲット未対応、GEN001 生成失敗。全診断に `code, severity, message_ja, file, span, hints[]` を含める。

## 16. テスト仕様
Unit: Lexer/Parser/型/効果/正規化。Property: AST roundtrip、識別子のUnicode正規化。Golden: 意図→VAG→生成コード。Integration: SQLite CRUD、認証拒否、Webフォーム。E2E: Playwright Web; iOS XCTest/Android instrumentation（P1）。Security: injection/XSS/IDOR/secret leakage/権限昇格。Mutation: ソース・知識・スキーマを変えてロック失効。AI offline: 承認済みビルドはネットワークなしで成功。

## 17. 実装モジュール
```text
kotonoha/
  crates/{lexer,parser,ast,types,knowledge,intent,resolver,approval,lock,graph,validator,codegen,cli}/
  adapters/{ai,db,auth}/
  targets/{web,backend,ios,android}/
  runtime/{web,backend,mobile}/
  stdlib/{ui,data,network,security}/
  examples/{shop,ranking,admin}/
  tests/{unit,golden,integration,e2e,security}/
  schemas/{intent-candidate.schema.json,vag.schema.json,lock.schema.json}/
  docs/{requirements.md,technical-spec.md,adr/}
  CLAUDE.md
```

## 18. Claude Codeの作業分割とDoD
P0-01 リポジトリとRust workspace、CI、formatter。P0-02 Lexer/Parser + snapshot tests。P0-03 厳密式と型検査。P0-04 Knowledge registry。P0-05 Intent GraphとJSON Schema。P0-06 承認・ロックとCLI。P0-07 Effects/Capabilities。P0-08 Web generator。P0-09 Backend generator + DB。P0-10 E2E/セキュリティテスト。各PRは仕様参照、テスト、変更影響、既知制約、生成物の差分を含む。テスト不合格のまま次工程へ進まない。

## 19. 受入テストケース
TC-001 `売れ筋商品を表示` → 30日・売上金額・20件。TC-002 `売れ筋商品を5件表示` → 件数だけ5件。TC-003 未定義「人気」→確認要求。TC-004 未承認→本番ビルド拒否。TC-005 型不一致→TYP001。TC-006 data.readしかないActionで更新→SEC001。TC-007 APIへの権限なし更新→HTTP 403。TC-008 HTMLへ不正入力→XSS非成立。TC-009 同一ロックでAIオフライン→同一正規化VAG。TC-010 知識変更→LOCK001。TC-011 Web商品一覧→ブラウザ表示とクリック遷移。TC-012 API+DB→注文登録と在庫の原子的更新。TC-013 ターゲット未対応→TGT001。TC-014 iOS/Android商品一覧→共通API通信（P1）。

## 20. 未決定の仕様と実装禁止事項
未決定: 言語の完全文法、金額精度、返品調整、時刻境界、認証実装、モバイル最小OS、UIの詳細トークン、署名基盤。これらはADRで決定し、決定前に推測で本番実装しない。禁止: AI応答の直接eval、承認の偽装、生成コードの手修正を正本にすること、未対応機能の黙殺、認可をフロントエンドだけで完結させること。
