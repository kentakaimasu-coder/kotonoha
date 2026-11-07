# KOTONOHA はじめてのアプリ(チュートリアル)

この文書では、商品を一覧表示して注文できる小さなお店アプリを作り、Web とバックエンドを実際に動かします。
所要時間は 20 分ほどです。

## 0. 準備
- Rust(`cargo`)と Node.js 22.18 以上
- KOTONOHA のコンパイラ `koto` をビルドします

```bash
cargo build --release
export PATH="$PWD/target/release:$PATH"
koto doctor          # 実装状況を表示
```

## 1. プロジェクトを作る
```bash
koto init お店 --name お店
cd お店
koto check           # 「検査に合格しました」と出れば OK
```

`src/app.koto` が KOTONOHA のソースです。日本語と最小限の記号で書きます。

## 2. データを決める
```kotonoha
アプリ「お店」

データ 商品:
    ID: 識別子
    名前: 文字列
    価格: 金額<JPY>
    在庫: 整数
```
- インデントは**半角空白 4 個**です(タブは使えません)
- `:` `=` `(` などの記号は**半角**です。全角で書いてしまったら `koto fmt --zenkaku` でまとめて直せます

## 3. 画面を書く(日本語の文で書く)
```kotonoha
画面 商品一覧:
    商品を一覧表示
    商品を選ぶと 商品詳細 へ

画面 商品詳細:
    商品の名前と価格と在庫を表示
    「購入」を押すと 注文する(商品.ID, 1)
```
画面の文は、決まった**文型**で書きます(一覧表・詳細・ボタン・フォーム・遷移など。`docs/spec/intent_patterns.md`)。
文型に当てはまらない文は、コンパイラが推測せずに**質問**を返します:

```
src/app.koto:9:5: エラー[AMB001] 「人気商品を表示」を解釈できません
    ヒント: 質問: 「人気商品」は知識・データとして定義されていません。何を表示しますか
```

## 4. 処理を書く(厳密に書く)
お金や在庫のように間違えてはいけない処理は `厳密:` の中に書きます。

```kotonoha
型 注文ID = 識別子

列挙 注文失敗:
    数量不正
    在庫不足

データ 注文:
    ID: 注文ID
    商品ID: 識別子
    数量: 整数
    注文者: 識別子

アダプタ 商品をIDで取得(ID: 識別子) -> 商品:
    操作 = 取得(商品)

アダプタ 注文を登録(商品ID: 識別子, 数量: 整数, 注文者: 識別子) -> 注文ID:
    操作 = 登録(注文)

動作 注文する(商品ID: 識別子, 数量: 整数) -> 結果<注文ID, 注文失敗>:
    必要 = [注文.作成, 商品.読取, 商品.更新]
    厳密:
        数量 > 0 を確認 または 失敗("数量不正")
        取引:
            商品 = 商品をIDで取得(商品ID)
            商品.在庫 >= 数量 を確認 または 失敗("在庫不足")
            商品.在庫 = 商品.在庫 - 数量
            注文ID = 注文を登録(商品ID, 数量, 本人)
            注文ID を返す
```
- `必要 = [...]` に、その処理が使う権限を書きます。書き忘れると SEC002 で止まります(**既定ですべて禁止**)
- `取引:` の中は全部成功するか、全部取り消されるかのどちらかです(在庫だけ減って注文が無い、は起きません)
- `本人` はログインしている人の ID です

## 5. 誰が見てよいかを書く(約束)
```kotonoha
約束:
    商品は誰でも閲覧可
    注文は注文者が本人のものだけ閲覧可
```

## 6. 検査して、生成して、動かす
```bash
koto check
koto build --target backend --out ../out/backend
koto build --target web --out ../out/web

# バックエンド(秘密は環境変数で渡す。ソースや生成物には入れない)
cd ../out/backend
export KOTONOHA_JWT_SECRET=$(openssl rand -hex 32)
npm start                                   # http://127.0.0.1:8787

# Web(別の端末で)
cd ../out/web && npm install && npm run build
KOTONOHA_BACKEND_URL=http://127.0.0.1:8787 npm start   # http://127.0.0.1:3000
```
開発中のログインは、`npm run token -- --sub 山田 --caps 動作.注文する` で作ったトークンを `/login` に貼り付けます。
本番では OIDC(社内の認証基盤)でログインします(`KOTONOHA_OIDC_ISSUER` など。docs/architecture.md)。

## 7. 困ったとき
| やりたいこと | コマンド |
|---|---|
| エラーの意味と直し方を見る | `koto explain AMB001` |
| 全角の記号をまとめて半角にする | `koto fmt --zenkaku` |
| 変更がどこに影響するか調べる | `koto impact 商品` |
| API の権限が正しいか試す | `koto test` |
| iPhone / Android アプリも作る | `koto build --target ios` / `--target android` |

## 8. 本番に出す前に
- `koto approve --write-lock` で意味をロックし、`koto build --locked` でビルドします(ロック後にソースが変わると LOCK001 で止まります)
- バックエンドと Web は `KOTONOHA_ENV=production` で起動します。安全でない設定(弱い秘密、HTTPS でない Cookie、開発用ログイン)だと起動しません

## 9. 言語を広げる: 関数・外部関数・分析・部品・実験
完全な例は `examples/lab` にあります(`koto -C examples/lab check`)。

### 9.1 関数(副作用なし)
```kotonoha
関数 値引き後(価格: 金額<JPY>, 値引き: 金額<JPY>) -> 金額<JPY>:
    もし 値引き > 価格:
        価格 - 価格 を返す
    価格 - 値引き を返す
```
関数の中ではデータの読み書き・`本人`・`現在日時()` は使えません(同じ入力なら必ず同じ結果にするため)。
組込み関数: `長さ` `件数` `絶対値` `最大` `最小` `文字列化` `含む` `現在日時`(動作の中だけ)。

### 9.2 外部関数(TypeScript / Python)
既にある TypeScript のコードや、Python の機械学習モデルを呼べます。
```kotonoha
外部関数 割引額(価格: 金額<JPY>) -> 金額<JPY>:
    実装 = "ext/pricing.ts"

外部関数 需要予測(商品ID: 識別子, 価格: 金額<JPY>) -> 整数:
    実装 = "ml/forecast.py"
```
```python
# ml/forecast.py(関数名は宣言と同じ)
def 需要予測(商品ID: str, 価格: int) -> int:
    return max(0, 120 - 価格 // 250)
```
- 引数と戻り値は API と同じ JSON の値(整数・金額は数値、日時は ISO 8601 の文字列)
- 戻り値は宣言した型で検査されます。違えば 422(`external_type`)
- 実装ファイルの中身を変えるとグラフのハッシュが変わり、本番ビルド(`--locked`)は承認し直すまで止まります
- Python は backend の起動ユーザーの `python3`(`KOTONOHA_PYTHON` で変更可)。必要なパッケージは各自の環境に入れてください

### 9.3 分析(SQL)
```kotonoha
分析 商品別売上() -> 一覧<記録{商品ID: 識別子, 売上: 金額<JPY>, 件数: 整数}>:
    対象 = [注文]
    SQL = "analytics/sales_by_product.sql"
```
```sql
SELECT "商品ID" AS "商品ID", SUM("金額") AS "売上", COUNT(*) AS "件数"
FROM "注文" GROUP BY "商品ID" ORDER BY "売上" DESC
```
- 書けるのは SELECT 1 文だけ。読める表は `対象` に書いたデータだけ。引数は `:開始` のように書きます
- 画面に `商品別売上を表示` と書くと表になります。API は `GET /api/analyses/商品別売上`
- 列名は戻り値の記録の名前と同じにします(`AS "売上"`)

### 9.4 部品(手書きの UI)
```kotonoha
部品 星評価(評価: 整数):
    web = "ext/StarRating.ts"
    ios = "ext/StarRating.swift"
    android = "ext/StarRating.kt"

画面 商品詳細:
    星評価(商品.評価)を表示
```
- Web は `React.createElement` で書きます(JSX は使えません)。関数名は部品名と同じにします
- 実装の無いターゲットでその部品を使うと、そのターゲットの生成が TGT001 で止まります

### 9.5 実験(A/B テスト)
```kotonoha
実験 購入ボタン文言:
    画面 = 商品詳細
    変種 = [購入, 今すぐ購入]     # 先頭が今の表示(対照群)
    割合 = [50, 50]
    成果 = 購入する               # この動作が成功したら成果
```
- 同じ端末には常に同じ変種が出ます(Web は Cookie、スマホは端末内の ID)
- 結果は `GET /api/experiments/購入ボタン文言/results`(許可 `実験.結果` が必要)。変種ごとの表示数・成果数・成果率と p 値が返ります
- 途中で何度も結果を見て、良さそうなところで止めると誤った結論になりやすくなります。先に「何人に見せたら判断するか」を決めてください
