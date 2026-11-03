# ADR-002 認証プロバイダー
状態: 未決定(暫定実装あり) / 日付: 2026-11-02 / 更新: 2026-11-03
## 背景
SEC-02「成熟した既存ライブラリ/ID 基盤」、SEC-03「認可は生成 API 側で強制」。AT-04 / TC-007(403)と UC-02 の前提。
## 選択肢
1. OIDC(外部 IdP: Auth0 / Cognito / Entra ID 等)をアダプタで抽象化し、生成 Backend は JWT 検証のみ
2. Backend にセッション認証を内蔵(成熟ライブラリ利用)
3. 両方をアダプタで選択可能
## 推奨
1 を既定にし、`adapters/auth` の契約(主体 ID・組織・ロールの取得)だけを先に固定する。行レベル認可(「本人だけ」)は主体 ID と資源の所有者フィールドの対応付けが必要で、言語側の構文(約束の文型)も合わせて設計する。
## 暫定実装(2026-11-03、同日拡張)
- 追加: OIDC 形式の **JWKS 検証(RS256 / ES256)**。`KOTONOHA_JWKS_FILE` / `KOTONOHA_JWKS_JSON` / `KOTONOHA_JWKS_URL`(定期再取得)。鍵と alg の組合せが一致しない署名・`crit` 付きヘッダは拒否
- 追加: ロール claim(`KOTONOHA_ROLES_CLAIM`、既定 `roles`)を `KOTONOHA_ROLE_MAP` で Capability / 動作許可に展開。Capability の claim 名は `KOTONOHA_CAPS_CLAIM`

全体開発を止めないため、推奨案 1 の「Backend は Bearer JWT を検証するだけ」の部分を実装した。
- HS256 のみ受理(alg=none・RS/HS 混同を拒否)、`exp` 必須、`iss`/`aud` は設定時のみ検査
- 秘密は `KOTONOHA_JWT_SECRET`(32 文字以上)。生成物に含めない
- 主体の Capability は claim `caps`
- 開発用に `tools/dev-token.ts` でトークンを発行。Web/iOS/Android のログイン画面はトークンを貼り付ける方式(**本番の利用者向けログインではない**)

- 追加: **Web の OIDC 認可コードフロー(PKCE S256、state・nonce 照合、クライアント秘密は任意)**。`KOTONOHA_OIDC_ISSUER` / `_CLIENT_ID` / `_CLIENT_SECRET` / `_REDIRECT_URI` / `_SCOPE` / `_TOKEN`。OIDC を設定すると開発用のトークン貼り付けログインは既定で無効(`KOTONOHA_DEV_LOGIN=1` で有効)。模擬 IdP で E2E 確認済み(tests/e2e/oidc.test.ts)

未決: IdP の選定、モバイルの OIDC(ASWebAuthenticationSession / Custom Tabs)、トークン更新(refresh token)。
