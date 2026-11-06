# 実装状況報告

状態: Web / Backend の生成と実行、PostgreSQL、OIDC、行レベル認可、影響分析、契約テストまで実装。iOS / Android は生成と部分検証 / 最終更新: 2026-11-03
全体構成は [architecture.md](architecture.md)。

## 1. 検証結果(2026-11-03)
| 検査 | 結果 |
|---|---|
| `cargo fmt --check` / `cargo clippy --all-targets -D warnings` | ✅ |
| `cargo test`(コンパイラ・生成器・CLI・生成コードのゴールデン) | ✅ 98 件 |
| `tools/validate_schemas.py`(VAG・候補・承認・ロックの実出力) | ✅ |
| `koto test`(見本アプリの API 契約から導いた契約テスト) | ✅ 13 件 + 業務規則の雛形 5 件(todo) |
| `tests/e2e/backend.test.ts`(生成 backend を起動) | ✅ 9 件 |
| `tests/e2e/backend_security.test.ts`(許可・行レベル認可・JWKS・ページング・レート制限・マイグレーション・同時実行) | ✅ 7 件 |
| `tests/e2e/postgres.test.ts`(生成 backend を PostgreSQL 16 で起動) | ✅ 5 件 |
| `tests/e2e/web.test.ts`(Chromium で操作・axe-core 監査) | ✅ 9 件 |
| `tests/e2e/oidc.test.ts`(模擬 IdP での OIDC ログイン → 購入) | ✅ 2 件 |
| `tools/mobile-syntax/run.sh`(生成 Swift 3 / Kotlin 5 ファイルの構文検査) | ✅ 8 ファイル |
| `tools/verify_android_jvm.sh`(Android 非 UI 層を kotlinc `-Werror` でコンパイルし、実 API 応答を解析) | ✅ |
| iOS のコンパイル・Compose 画面のビルド | ✅ GitHub Actions の `ios-build`(macOS 15 / Xcode、iOS シミュレータ向け)と `android-build`(`gradle :app:assembleDebug`)で成功(2026-11-06) |

## 2. 実装済み
- **コンパイラ**: 字句 → 構文 → 型 → 知識 → 意図(承認適用)→ 効果/Capability → VAG → ロック → App IR → 4 ターゲット生成。`koto init/fmt/check/resolve/approve/graph/build/test/impact/doctor`
- **言語**(Phase 0 以降の追加。いずれも ADR「提案」): P-FORM(フォーム)、C-OWNER(本人だけ閲覧)、組込み値 `本人`、登録アダプタの必須フィールド検査
- **Backend**: Node.js + TypeScript。SQLite / PostgreSQL、厳密層の実行(BigInt 64 ビット検査・取引・ロールバック・同時実行の正しさ)、差分マイグレーション、JWT(HS256 / JWKS RS256・ES256)、ロール → Capability、動作単位の許可、行レベル認可、入力検証、ページング、レート制限、構造化ログ、契約テストと業務規則の雛形
- **Web**: React SSR + hydration、一覧・詳細・集計・ボタン・フォーム、OIDC ログイン(PKCE)、API プロキシ、CSRF 対策、CSP、axe-core で WCAG 2.2 A/AA 違反ゼロ
- **iOS**: SwiftUI の画面・モデル・API クライアント・Keychain・ログイン・XcodeGen 定義(構文検査のみ)
- **Android**: Compose の画面・モデル・API クライアント・Keystore 暗号化トークン・Gradle 定義(構文検査 + 非 UI 層のコンパイルと実応答の解析)
- **影響分析**: `koto impact <名前>` / `koto impact --base <旧 VAG>`(AT-08)

### 受入テストの状況
| ID | 状況 |
|---|---|
| AT-01〜AT-08, AT-10 | ✅ 自動テスト |
| AT-09 iOS/Android で同じ商品一覧・API 通信 | △ 両 OS のビルド成功(CI)と Android 非 UI 層の実 API 解析まで。シミュレータ上の操作は未確認(K-17) |
| TC-001〜TC-013 | ✅ |
| TC-014 | ⚠ AT-09 と同じ |

## 3. 未実装
| 機能 | 要件 |
|---|---|
| LSP / VS Code | FR-017 |
| 生成コードとソース行の対応表 | FR-018(生成コードに宣言名のコメントのみ) |
| デザイントークン | FR-015 |
| 定期処理・外部 SDK・オフライン同期 | FR-021〜023(P2) |
| モバイルの OIDC ログイン・トークン更新 | ADR-002 の未決 |
| 承認の署名検証 | ADR-007(承認入りの `--locked` ビルドは拒否のまま) |
| 実 AI プロバイダとの接続 | ADR-006 |
| 組織・テナント単位の認可、本人限定データの集計 | ADR-0118 の未決 |
| 分岐網羅率の計測 | NFR-003 |

## 4. 既知の不具合・制約
| ID | 重要度 | 状態 | 内容 |
|---|---|---|---|
| K-01 | 高 | **解消** | iOS のコンパイルと Android(Compose 画面を含む)のビルドを CI で確認。実機/シミュレータ上での画面操作の自動テストは未実施(K-17) |
| K-02 | 高 | **解消** | 動作単位の許可(`動作.名前`)と行レベル認可を実装(ADR-0118) |
| K-03 | 中 | 制約(仕様) | ±2^53 を超える整数は JSON 応答で 422(ADR-0117 の設計判断) |
| K-04 | 中 | 制約 | `node:sqlite` は Node 22 で実験的機能。本番は PostgreSQL を推奨 |
| K-05 | 中 | **解消** | 差分マイグレーション(SQLite / PostgreSQL) |
| K-06 | 中 | 一部解消 | axe-core 自動監査で違反ゼロ。手動監査(スクリーンリーダー等)は未実施 |
| K-07 | 中 | **解消** | Android のトークンは Keystore の AES-GCM で暗号化 |
| K-08 | 中 | 一部解消 | Web は OIDC ログイン実装済み。モバイルはトークン貼り付けのまま |
| K-09 | 低 | **解消** | 一覧のページング(limit / offset / X-Total-Count)。Web は先頭 100 件を表示(続きの表示 UI は未実装) |
| K-10 | 低 | **解消** | 書込みのレート制限(429) |
| K-11 | 低 | 制約 | 日時の表示時刻帯は環境変数で固定(既定 Asia/Tokyo) |
| K-12 | 低 | **解消** | 生成コードのゴールデン(4 ターゲット) |
| K-13 | 低 | **解消** | 動作の結果値(単純な値)を表示 |
| K-14 | 中 | 新規 | フィールド単位の影響分析は、動作本体のフィールド参照を VAG に持たないため、資源を使う動作を保守的に含める |
| K-15 | 低 | 新規 | レート制限はプロセス内のメモリで数える(複数インスタンス間では共有しない) |
| K-16 | 低 | 新規 | Web の一覧は 101 件目以降を表示する UI が無い |
| K-17 | 中 | 新規 | iOS/Android は CI でビルドまで。シミュレータ/エミュレータ上の UI テスト(XCUITest / Compose UI テスト)は未実装 |
| K-18 | 中 | **解消** | Node 22.18 の node:sqlite が番号付きプレースホルダ(`?1`)で失敗する問題。SQLite 接続では位置指定の `?` に変換して渡す |

## 5. 次の作業
1. iOS/Android のシミュレータ上の UI テスト(K-17)
2. モバイルの OIDC ログイン(ASWebAuthenticationSession / Custom Tabs)
3. 承認署名(ADR-007)、LSP(FR-017)、生成コードの行対応(FR-018)
4. 未承認の ADR(0101〜0119)のレビューと確定
