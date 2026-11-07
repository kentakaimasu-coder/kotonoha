# KOTONOHA 最終アーキテクチャ

状態: 提案(実装中) / 最終更新: 2026-11-03 / 上位文書: 要件定義書 §7、技術仕様書 §10〜12

## 1. 全体像

```
                         ┌──────────────── 開発時のみ ────────────────┐
 KOTONOHA ソース(.koto) │ resolver(AI 候補: 隔離・未信頼) → 人の承認 │
        │                └───────────────────────┬────────────────────┘
        ▼                                        │ 承認記録(正規文)
 lexer → parser → types → knowledge → intent ◀───┘
        → validator(効果/Capability)→ VAG(検証済み意味グラフ・ロック対象)
        │
        ▼
 App IR(全ターゲット共通): データモデル / 共通 API 契約 / UI IR / 動作本体
        │
        ├─▶ backend 生成 ─▶ Node.js + TypeScript + SQLite / PostgreSQL … 業務ロジック・認可・DB の唯一の実行場所
        ├─▶ web 生成     ─▶ React SSR + esbuild(同一オリジンで /api を backend にプロキシ)
        ├─▶ ios 生成     ─▶ SwiftUI(共通 API 契約のクライアント)
        └─▶ android 生成 ─▶ Kotlin + Jetpack Compose(共通 API 契約のクライアント)
```

設計の柱:
1. **意味は 1 回だけ確定する**: 曖昧さの解決はコンパイル前(承認・ロック)。生成物は AI に接続しない。
2. **業務ロジックはサーバーに 1 つ**: `厳密:` の処理と認可は backend だけが実行する。クライアント(Web/iOS/Android)は同じ API 契約を呼ぶだけなので、規則がプラットフォーム間でずれない(要件 §7「業務ロジックの共有」)。
3. **画面の意味と状態遷移は共有、表現はターゲット固有**: UI IR(一覧・詳細・集計・ボタン・フォーム・遷移)を各ターゲットの慣用的な UI に変換する。
4. **未対応は止める**: 生成器は対応表と型検査で未対応を TGT001 として報告し、黙って欠落させない。

## 2. コンパイラ層(実装済み)
docs/spec/compiler.md を参照。VAG までがターゲット非依存の「意味」、App IR 以降が「実現」。

## 3. App IR(`crates/codegen/src/appir.rs`)
| 要素 | 内容 | 由来 |
|---|---|---|
| entities / enums | データモデル | `データ` `列挙` |
| endpoints | 共通 API 契約(下表) | 画面の必要データ・知識・動作 |
| screens | UI IR: List / Detail / Query / Button / Form + 遷移先 | 意図文型 |
| actions | 引数・戻り型・必要 Capability・本体 AST | `動作` + validator |
| queries | 知識(ランキング)と画面で使う件数 | `知識` |
| policy | 公開読取・認証必須 | `約束` |
| entry_screen | 入力を持たない最初の画面 | |

### 共通 API 契約(`api.json`、全ターゲットに同梱)
| エンドポイント | 生成条件 | 認可 |
|---|---|---|
| `GET /api/data/{データ}?limit=&offset=` | どこかの画面が P-LIST で一覧表示する | 公開読取なら不要、本人限定(C-OWNER)なら認証済みで本人の行だけ、それ以外は `{データ}.read`。既定 100 件・上限 1000 件、総数は `X-Total-Count` |
| `GET /api/data/{データ}/{id}` | どこかの画面の入力がそのデータ | 同上 |
| `GET /api/queries/{知識}?limit=&base=` | 画面が知識を表示する。`limit` は画面で使う件数のみ | 対象データの read |
| `POST /api/actions/{動作}` | すべての動作 | `動作.{動作名}` の許可、または動作の必要 Capability すべて(推移閉包)(ADR-0118) |
| `GET /api/health` | 常に | 不要 |
画面が使わないデータは API を出さない(最小公開)。パスの日本語は percent-encode する。

## 4. ワイヤ形式(全クライアント共通)
| 型 | JSON |
|---|---|
| 真偽 | boolean |
| 整数・金額<JPY> | 整数の number(±2^53 を超える値は 422 `unsafe_integer`: K-03) |
| 文字列・識別子・URL・メール | string(入力時に長さ・形式を検証) |
| 日時 | RFC 3339 文字列(UTC で出力、入力はタイムゾーン必須) |
| 列挙 | 値名の string |
| 選択<T> | null または T |
| データ | フィールド名(日本語)をキーとするオブジェクト |
| 動作の応答 | 成功 `200 {"ok": 値}`、`失敗(e)` は `422 {"error":"failure","value":e}` |
| エラー | `400 invalid_input` / `401 unauthenticated` / `403 forbidden` / `404 not_found` / `413` / `415` / `422 runtime_error` / `500 internal`(本文 `{error, message}`) |
小数・日付・期間・バイト列・入れ子の記録は未定義(ADR-0106。生成時に TGT001)。

## 5. Backend(`runtime/backend/` + `crates/codegen/src/backend.rs`)
- Node.js 22.18+ の型除去で TypeScript を直接実行。SQLite だけなら **npm 依存なし**(SEC-07 の攻撃面を最小化)。DB は `KOTONOHA_DB` で選ぶ: SQLite(`node:sqlite`。Node 22 では実験的機能: K-04)または PostgreSQL(`postgres://…`、任意依存 `pg` 固定版)。生成コードは非同期で、方言差は `db.ts` が吸収する(ADR-003)。
- 厳密層 → TypeScript: 整数は BigInt で 64 ビット範囲を検査、`失敗` は例外、`取引:` は SQLite では `BEGIN IMMEDIATE` + 接続の排他、PostgreSQL では `SERIALIZABLE` + 直列化失敗の再試行(失敗・例外は `ROLLBACK`)。エンティティ更新は取引の終わりに変更フィールドだけ UPDATE。組込み値 `本人` は主体 ID。
- データアダプタ: 取得/一覧/登録/更新/削除をパラメータ化 SQL で実装(SEC-04)。ID は UUID v4。
- 認証(ADR-002 暫定): `Authorization: Bearer` の JWT。HS256(`KOTONOHA_JWT_SECRET`)と、OIDC の JWKS による RS256 / ES256(`KOTONOHA_JWKS_FILE` / `_JSON` / `_URL`)。`exp` 必須、`iss`/`aud` は任意設定。主体の Capability は claim `caps` と、ロール claim を `KOTONOHA_ROLE_MAP` で展開したものの和。
- 行レベル認可(ADR-0118): C-OWNER のデータは、取得・一覧・更新・削除を「所有者フィールド = 主体 ID」の行に限定し、登録時は所有者を主体 ID に固定する。
- レート制限: 書込み(POST)を主体または IP ごとに毎分 `KOTONOHA_RATE_LIMIT_PER_MINUTE`(既定 120)件まで。超過は 429。
- 認可: ルートごとに公開/必要 Capability を宣言し、サーバーで 401/403 を返す(SEC-03)。
- 入力検証: 型・長さ・列挙値・未知項目の拒否、本文 64KB 上限、`Content-Type: application/json` 必須。
- 可観測性: 1 リクエスト 1 行の JSON ログ(trace_id, method, path, status, ms, sub)、`X-Trace-Id` 応答ヘッダ(NFR-008)。
- スキーマ: 起動時に宣言と既存表を照合する差分マイグレーション。表の作成と選択型の列追加は自動。空の表の破壊的変更は作り直し、データのある表の破壊的変更(必須列追加・型変更・列削除)は起動を拒否する。適用履歴は `_kotonoha_migrations`。
- 付属ツール: `tools/dev-token.ts`(開発用トークン発行)、`tools/seed.ts`(ワイヤ形式で検証して初期データ投入)、`tests/contract.test.ts`(API 契約から導いた契約テスト)、`tests/actions.todo.test.ts`(業務規則のテスト雛形)。

## 6. Web(`runtime/web/` + `crates/codegen/src/web.rs`)
- React 19 の SSR(`renderToString`)→ ブラウザで `hydrateRoot`。クライアントは esbuild で 1 ファイルにバンドル。依存は react / react-dom / esbuild を固定版で指定。
- ルート: `/` → 入口画面、`/s/{画面}`、入力のある画面は `/s/{画面}/{id}`。`/login`、`/logout`。
- ログイン: OIDC 認可コードフロー(PKCE S256・state・nonce。`/login/oidc` → IdP → `/login/callback`)で得たトークンを HttpOnly・SameSite=Strict の Cookie に保存。OIDC 未設定時のみ開発用のトークン貼り付けログイン。
- `/api/*` は backend へのプロキシ。Cookie のトークンを `Authorization` に載せ替える。CSRF 対策として `X-CSRF: 1` を必須にし、`Origin` が異なる要求を拒否する。
- セキュリティヘッダ: CSP(`script-src 'self'` 等。インラインスクリプトなし)、`X-Frame-Options: DENY`、`nosniff`。初期データは `<script type="application/json">` に `<` `>` `&` をエスケープして埋め込む。React が全テキストをエスケープする(TC-008)。
- UI: 一覧(表 + 先頭列から詳細へのリンク)、詳細(定義リスト)、集計表、ボタン(実行 → 結果表示 → 画面データ再取得: FR-012)、フォーム(引数の型に応じた入力)。`lang="ja"`、ラベル・`aria-live`・フォーカス表示・44px のタップ領域(NFR-005 の土台。監査は未実施: K-06)。

## 7. iOS / Android
- どちらも共通 API 契約のクライアントで、画面ごとに実際の UI コード(SwiftUI View / Composable)を生成する。モデル型・API クライアント・ログイン画面はランタイム(`runtime/ios`, `runtime/android`)。
- iOS: SwiftUI(`NavigationStack`)、Codable モデル、URLSession(async/await)、トークンは Keychain(`WhenUnlockedThisDeviceOnly`)。XcodeGen の `project.yml`。
- Android: Kotlin + Compose(Navigation Compose)、org.json、HttpURLConnection(Dispatchers.IO)。平文 HTTP はエミュレータ→ホスト(10.0.2.2)だけ許可。
- Android のトークンは Android Keystore の AES-GCM 鍵で暗号化して保存する。
- JVM のクラス名(= ファイル名)は ASCII に符号化する(日本語名はプロパティ名に残す。ファイル名の文字コードに依存しないため)。
- 最低 OS(ADR-005)は案(iOS 16 / API 26)。この開発環境には Swift ツールチェーンも Android SDK も無い(配布元がネットワーク方針で遮断されている)。代わりに次を確認している:
  - 全 Swift / Kotlin ファイルの構文検査(tree-sitter、`tools/mobile-syntax/run.sh`)
  - Android の非 UI 層(モデル・API クライアント・Keystore)を本物の kotlinc と Android フレームワーク(Robolectric android-all)で `-Werror` コンパイルし、生成 backend の実際の応答を解析(`tools/verify_android_jvm.sh`)
  - Compose 画面のビルドと iOS のコンパイルは CI の `android-build` / `ios-build`(未実行: K-01)

## 8. ターゲット間の整合性
- `api.json` は全ターゲットで同一(テスト `all_targets_generate_and_share_one_api_contract`)。
- クライアントが使う動作・データ API は契約に含まれるものだけ。
- 認可は backend だけが判断する。クライアントは 401/403 を利用者向けの文言に変えるだけ。

## 8.1 拡張(ADR-0122〜0125)
| 宣言 | コンパイラ(`crates/ext`) | backend | Web | iOS / Android |
|---|---|---|---|---|
| 関数 | 型・純粋性の検査(types) | 同期関数として生成 | — | — |
| 外部関数 | 実装ファイルの読込・ハッシュ | TS は同一プロセス、Python は常駐ワーカー(`runtime/backend/pyworker.py`)。戻り値を型検査(`runtime/backend/ext.ts`) | — | — |
| 分析 | SQL の構文検査・`:名前` → `?N` | `GET /api/analyses/{名前}`。読み取り専用の取引・行数上限 | 表を SSR | 行モデル + 一覧 |
| 部品 | 実装ファイルの読込 | — | `src/ext/` に複製して import(`ExtBoundary` で囲む) | `Sources/Ext/` / `…/generated/ext/` に複製して呼ぶ |
| 実験 | 対象・変種・割合・成果の検査、割付関数 | `assign` / `results` API、露出と成果の表 | SSR 時に割付(匿名 ID の Cookie) | `api.variant`(端末の匿名 ID) |

## 9. 今後の拡張点
| 項目 | 方針 |
|---|---|
| PostgreSQL 本番 | DDL と接続アダプタ(`pg`)を backend に追加。SQL 低水準化は実装済み |
| デザイントークン(FR-015) | UI IR にトークンを持たせ、各ターゲットのテーマに変換 |
| LSP(FR-017) | VAG と診断を LSP で配信 |
| 組織・テナント単位の認可 | ADR-0118 の未決事項 |
