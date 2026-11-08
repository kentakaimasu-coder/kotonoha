# 実装状況報告

状態: Web / Backend の生成と実行、PostgreSQL、OIDC、行レベル認可、影響分析、契約テスト、承認署名、関数・外部関数(TS/Python)・分析(SQL)・拡張部品・A/B 実験まで実装。iOS / Android は生成と CI ビルド / 最終更新: 2026-11-07
全体構成は [architecture.md](architecture.md)。追補要件(NR-01〜08)は [03_requirements_addendum.md](requirements/03_requirements_addendum.md)。

## 1. 検証結果(2026-11-07)
| 検査 | 結果 |
|---|---|
| `cargo fmt --check` / `cargo clippy --all-targets -D warnings` | ✅ |
| `cargo test`(コンパイラ・生成器・CLI・生成コードのゴールデン・拡張の負例・乱数入力の堅牢性) | ✅ 116 件 |
| `tools/validate_schemas.py`(VAG・候補・承認・ロックの実出力。拡張を含む examples/lab も) | ✅ |
| `koto test`(examples/shop・examples/lab の API 契約から導いた契約テスト) | ✅ 13 件 / 14 件 + 業務規則の雛形(todo) |
| `tests/e2e/*.test.ts`(backend・セキュリティ・PostgreSQL 16・web(Chromium・axe-core)・OIDC・拡張) | ✅ 42 件(PostgreSQL ありで実行) |
| `tests/e2e/extensions.test.ts`(TS/Python 外部関数・戻り値の型検証・分析・実験・Web の変種表示・PostgreSQL) | ✅ 7 件 |
| `tools/mobile-syntax/run.sh`(shop・lab の生成 Swift / Kotlin と拡張部品の構文検査) | ✅ |
| `tools/verify_android_jvm.sh`(Android 非 UI 層を kotlinc `-Werror` でコンパイルし、実 API 応答を解析。lab の分析行モデル・実験 API も) | ✅ |
| iOS のコンパイル・Compose 画面のビルド | CI の `ios-build` / `android-build`(shop・lab のマトリクス)で確認。この開発環境では Swift / Android SDK が無いため未検証 |

## 2. 実装済み
- **コンパイラ**: 字句 → 構文 → 型 → 知識 → 意図(承認適用)→ 効果/Capability → VAG → ロック → App IR → 4 ターゲット生成。`koto init/fmt/check/resolve/approve/graph/build/test/impact/doctor`
- **言語**(Phase 0 以降の追加。いずれも ADR「提案」): P-FORM(フォーム)、C-OWNER(本人だけ閲覧)、組込み値 `本人`、登録アダプタの必須フィールド検査
- **Backend**: Node.js + TypeScript。SQLite / PostgreSQL、厳密層の実行(BigInt 64 ビット検査・取引・ロールバック・同時実行の正しさ)、差分マイグレーション、JWT(HS256 / JWKS RS256・ES256)、ロール → Capability、動作単位の許可、行レベル認可、入力検証、ページング、レート制限、構造化ログ、契約テストと業務規則の雛形
- **Web**: React SSR + hydration、一覧・詳細・集計・ボタン・フォーム、OIDC ログイン(PKCE)、API プロキシ、CSRF 対策、CSP、axe-core で WCAG 2.2 A/AA 違反ゼロ
- **iOS**: SwiftUI の画面・モデル・API クライアント・Keychain・ログイン・XcodeGen 定義(構文検査のみ)
- **Android**: Compose の画面・モデル・API クライアント・Keystore 暗号化トークン・Gradle 定義(構文検査 + 非 UI 層のコンパイルと実応答の解析)
- **影響分析**: `koto impact <名前>` / `koto impact --base <旧 VAG>`(AT-08)
- **言語の拡張(NR-04〜08)**: `関数`(副作用なし)・組込み関数、`外部関数`(TypeScript / Python。戻り値は実行時にも型検査、宣言効果は動作の効果検査へ)、`分析`(読み取り専用 SQL。コンパイル時の構文検査と実行時の読み取り専用取引)、`部品`(Web / iOS / Android の手書き UI)、`実験`(A/B テスト: 決定的割付・露出・成果・z 検定の結果 API)。実装ファイルは内容のハッシュで VAG とロックに固定(ADR-0122〜0125)
- **セキュリティ(NR-02/03)**: 痕跡を出さない応答、セキュリティヘッダ、認証失敗の遮断、slowloris 対策、本番モードの設定検査、監査ログ、承認の Ed25519 署名と信頼鍵、SBOM、cargo audit / npm audit / CodeQL
- **一覧の並べ替え・絞り込みと日本語の集計(ADR-0128)**: 「注文のうち状態が調理中のものを注文日時順に一覧表示」「注文を品名ごとに集計」など。Web / iOS / Android は固定のクエリ・固定の API を呼ぶだけ
- **手元のオープンモデルと学習記録(ADR-0127)**: `koto resolve --adapter local`(Ollama など OpenAI 互換。ループバックだけに接続)と `--adapter memory`(過去の承認だけ)、`koto learn init/status/collect/export/eval`(承認から SFT / DPO データを書き出し、過去の承認の的中率でモデルを評価)。模擬サーバーでの自動テストのみ。実モデルでは未検証(K-30)
- **日本語での使いやすさ(NR-01)**: 診断の行抜粋、`koto explain`、`koto fmt --zenkaku`、IME の Enter 誤送信防止、全角数字の受付、日本語チュートリアル

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
| 外部関数のサンドボックス(OS レベルの隔離) | ADR-0124 の未決(現状は信頼したコードをハッシュで固定) |
| 分析 SQL の方言差の吸収・非同期実行 | ADR-0125 の未決 |
| 実験の逐次検定・多重比較の補正、終了手順 | ADR-0122 の未決 |
| 外部の AI サービスへの送信 | ADR-006(手元のモデルは ADR-0127 で実装) |
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
| K-13 | 低 | **解消(改訂)** | 動作の結果値(数値・短い文字列)を表示。自動採番の ID(UUID)は利用者に意味が無いため出さない(実アプリ examples/event の試用で判明) |
| K-14 | 中 | 新規 | フィールド単位の影響分析は、動作本体のフィールド参照を VAG に持たないため、資源を使う動作を保守的に含める |
| K-15 | 低 | 新規 | レート制限はプロセス内のメモリで数える(複数インスタンス間では共有しない) |
| K-16 | 低 | 新規 | Web の一覧は 101 件目以降を表示する UI が無い |
| K-17 | 中 | 新規 | iOS/Android は CI でビルドまで。シミュレータ/エミュレータ上の UI テスト(XCUITest / Compose UI テスト)は未実装 |
| K-19 | 低 | 新規 | 実験の結果 API の p 値は固定標本の z 検定。途中で何度も見て止めると誤判定が増える(結果に注意書きを付ける) |
| K-20 | 低 | 新規 | 拡張部品の iOS / Android 実装は CI のビルドでのみ確認(この環境では未コンパイル) |
| K-21 | 低 | 制約 | 分析の SQL は SQLite と PostgreSQL の両方で動く書き方が必要(識別子は二重引用符) |
| K-22 | 中 | **一部解消** | Web では、フィールド「XID」を関連データ X の最初の文字列項目(名前など)で表示する(X の一覧 API があり、読む権限があるとき)。iOS / Android は ID のまま |
| K-23 | 低 | 新規 | 状態に応じたボタンの無効化(満席なら押せない等)の文型が無い |
| K-24 | 低 | 新規 | 登録アダプタに既定値(例: 予約数 0)を書けない(ペルソナ評価) |
| K-25 | 低 | 新規 | 範囲付きの型(0 以上の金額など)が無く、入力の範囲は動作で書く必要がある |
| K-26 | 低 | 新規 | 自動で入る安全対策が利用者から見えない(一覧を示す手段が無い) |
| K-31 | 低 | 制約 | 日本語の集計(ADR-0128)は backend で行を読んでから数えるため、数十万行を超えるデータでは遅い。大量データは `分析`(SQL)を使う |
| K-30 | 中 | 制約 | `local` アダプタと `koto learn` は模擬の OpenAI 互換サーバーでのみ検証。実際のオープンモデル(Ollama 等)での候補の質・追加学習の効果は未検証(この環境からモデルを取得できない) |
| K-18 | 中 | **解消** | Node 22.18 の node:sqlite が番号付きプレースホルダ(`?1`)で失敗する問題。SQLite 接続では位置指定の `?` に変換して渡す |

## 5. 次の作業
0. **使いやすさ 300%**: 第 2 回ペルソナ評価(60 人、AI によるシミュレーション)で、非エンジニアの SUS は KOTONOHA 68.6 / 普段の手段 27.2(2.5 倍、30 人全員が KOTONOHA を高評価)、自分のアプリは 60 人全員が完成。300% には未達(docs/evaluation/persona/results_round2.md)。残る要望(集計の文型・絞り込み・二重登録防止・日時の表示)の対応と、実在の利用者評価が必要
1. iOS/Android のシミュレータ上の UI テスト(K-17)
2. モバイルの OIDC ログイン(ASWebAuthenticationSession / Custom Tabs)
3. LSP(FR-017)、生成コードの行対応(FR-018)、Web 一覧の続き表示(K-16)
4. 外部関数の隔離方式・分析の方言差・実験の逐次検定(ADR-0122/0124/0125 の未決)
5. 未承認の ADR(0101〜0125)のレビューと確定
