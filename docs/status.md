# 実装状況報告

状態: M2(Web/Backend 生成)の主要機能を実装、M4(iOS/Android 生成)のコード生成を実装 / 最終更新: 2026-11-03
全体構成は [architecture.md](architecture.md)。

## 1. 検証結果(2026-11-03)
| 検査 | 結果 |
|---|---|
| `cargo fmt --check` / `cargo clippy --all-targets -D warnings` | ✅ |
| `cargo test`(コンパイラ・生成器・CLI) | ✅ 94 件 |
| `tools/validate_schemas.py`(VAG・候補・承認・ロックの実出力) | ✅ |
| `tests/e2e/backend.test.ts`(生成 backend を起動して API 検証) | ✅ 9 件 |
| `tests/e2e/web.test.ts`(生成 web + backend を起動し Chromium で操作) | ✅ 7 件 |
| iOS / Android 生成物のコンパイル | ⚠ **未検証**(Swift・Android SDK がこの環境で取得できない。CI ジョブを用意済み) |

## 2. 実装済み
- **コンパイラ**: 字句 → 構文 → 型 → 知識 → 意図(承認適用)→ 効果/Capability → VAG → ロック → App IR → 4 ターゲット生成
- **言語**: Phase 0 の文法に P-FORM(フォーム)を追加
- **App IR / 共通 API 契約**: データ・API・UI IR・動作。全ターゲットで同一の `api.json`
- **Backend**: Node.js + TypeScript(依存なし)+ SQLite。厳密層の実行(BigInt の 64 ビット検査・取引・ロールバック)、データアダプタ、JWT 認証、サーバー側の認可(401/403)、入力検証、構造化ログ、初期データ投入、開発用トークン発行
- **Web**: React SSR + hydration、一覧・詳細・集計表・ボタン・フォーム、画面遷移、ログイン(Cookie)、API プロキシ、CSRF 対策、CSP
- **iOS**: SwiftUI の画面・モデル・API クライアント・Keychain・ログイン・XcodeGen 定義(未コンパイル)
- **Android**: Compose の画面・モデル・API クライアント・ナビゲーション・Gradle 定義(未ビルド)

### 受入テストの状況
| ID | 状況 |
|---|---|
| AT-01 商品一覧 Web | ✅ E2E |
| AT-02 同じ定義から API | ✅ E2E |
| AT-03 商品追加が Web に反映 | ✅ E2E(API とフォームの両方) |
| AT-04 権限なし更新をサーバーで拒否 | ✅ E2E(401/403) |
| AT-05 / AT-06 / AT-07 | ✅ |
| AT-08 意味変更の影響列挙 | ❌ 未実装 |
| AT-09 iOS/Android 同じ商品一覧・API 通信 | ⚠ コード生成のみ(未ビルド) |
| AT-10 認証情報を生成フロントに含めない | ✅ E2E |
| TC-007 / TC-008 / TC-011 / TC-012 | ✅ E2E |
| TC-014 | ⚠ 生成のみ |

## 3. 未実装
| 機能 | 要件 |
|---|---|
| 影響分析コマンド | FR-016, AT-08 |
| LSP / VS Code | FR-017 |
| 生成コードとソース行の対応表 | FR-018(生成コードに宣言名のコメントのみ) |
| `koto test`・テスト雛形生成 | FR-019 |
| 定期処理・外部 SDK・オフライン同期 | FR-021〜023(P2) |
| デザイントークン | FR-015 |
| 本番 IdP 連携(OIDC/JWKS)、承認署名 | ADR-002 / ADR-007 |
| PostgreSQL 接続(SQL は生成済み) | ADR-003 |
| 差分マイグレーション | FR-011 の一部 |

## 4. 既知の不具合・制約(後続の集中デバッグで対応)
| ID | 重要度 | 内容 |
|---|---|---|
| K-01 | 高 | iOS/Android 生成物のコンパイル未確認。CI の `ios-build` / `android-build` で確認する |
| K-02 | 高(設計) | Capability が資源単位のため、購入者に `商品.update` を与えると他の更新系動作も呼べる。仕様どおりの実装だが権限設計の判断が必要(ADR-0118) |
| K-03 | 中 | ±2^53 を超える整数は JSON 応答で 422(`unsafe_integer`) |
| K-04 | 中 | `node:sqlite` は Node 22 で実験的機能 |
| K-05 | 中 | スキーマ変更は `CREATE TABLE IF NOT EXISTS` のみ。既存表の列追加・変更は反映されない |
| K-06 | 中 | WCAG 2.2 AA の監査は未実施(ラベル・aria-live・フォーカス表示・タップ領域のみ対応) |
| K-07 | 中 | Android のトークン保存は private SharedPreferences(暗号化なし) |
| K-08 | 中 | ログイン画面はトークン貼り付け方式(開発用)。利用者向けログインは IdP 決定後 |
| K-09 | 低 | 一覧 API にページングが無い |
| K-10 | 低 | API のレート制限が無い |
| K-11 | 低 | 日時の表示時刻帯は環境変数で固定(既定 Asia/Tokyo) |
| K-12 | 低 | 生成コードのスナップショット(ゴールデン)テストは backend の SQL と VAG のみ。他は E2E と構造テストで確認 |
| K-13 | 低 | 動作の結果値は画面に表示しない(「完了しました」のみ) |

## 5. 次の作業
1. CI を実行できるリモートリポジトリの用意 → K-01(iOS/Android ビルド)の確認と修正
2. ADR-0118(権限の粒度)と ADR-002(IdP)の決定 → 動作単位の許可・OIDC 検証
3. 差分マイグレーション(K-05)と PostgreSQL 接続
4. 影響分析(FR-016)・`koto test`(FR-019)
5. 集中デバッグ: K-03〜K-13
