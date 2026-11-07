# 文法仕様(EBNF 完全版)

状態: 提案(ADR-0101/0102/0104/0105) / 最終更新: 2026-11-02 / 上位文書: 技術仕様書 §3 / 実装: `crates/parser`

字句は [lexical.md](lexical.md)。`NEWLINE` は論理行の終わり、`INDENT`/`DEDENT` はインデント段数の増減を表す。
`RAW` は行末までの生テキスト(意図層)。`"…"` は予約語・記号の字面。

## 1. ファイル

```ebnf
file           = { item } ;
item           = { annotation NEWLINE } , declaration ;
annotation     = "@" , ident , [ "(" , ident , ")" ] ;
                 (* 有効なのは @厳密 / @柔軟(確認) / @柔軟(自動)。画面・動作・約束にのみ付けられる *)

declaration    = app_decl | type_decl | enum_decl | data_decl | adapter_decl
               | knowledge_decl | screen_decl | action_decl | contract_decl ;
```

## 2. 宣言

```ebnf
app_decl       = "アプリ" , string , NEWLINE ;                       (* プロジェクトに 1 つ *)
type_decl      = "型" , ident , "=" , type , NEWLINE ;               (* 透過的な別名 *)
enum_decl      = "列挙" , ident , ":" , NEWLINE , INDENT , ident , NEWLINE , { ident , NEWLINE } , DEDENT ;
data_decl      = "データ" , ident , ":" , NEWLINE , INDENT , field , { field } , DEDENT ;
field          = ident , ":" , type , NEWLINE ;

adapter_decl   = "アダプタ" , ident , params , "->" , type , ":" , NEWLINE ,
                 INDENT , "操作" , "=" , op_kind , "(" , ident , ")" , NEWLINE , DEDENT ;
op_kind        = "取得" | "一覧" | "登録" | "更新" | "削除" ;          (* 字面は識別子。意味は ADR-0104 *)

knowledge_decl = "知識" , ident , ":" , NEWLINE , INDENT , kv_line , { kv_line } , DEDENT ;
kv_line        = ident , "=" , RAW , NEWLINE ;                       (* "==" は区切りにならない。値の文法は knowledge.md *)

screen_decl    = "画面" , ident , ":" , NEWLINE , INDENT , intent_line , { intent_line } , DEDENT ;
contract_decl  = "約束" , ":" , NEWLINE , INDENT , intent_line , { intent_line } , DEDENT ;
intent_line    = RAW , NEWLINE ;                                     (* ":" で終わってはならない。文型は intent_patterns.md *)

action_decl    = "動作" , ident , params , [ "->" , type ] , ":" , NEWLINE ,
                 INDENT , action_item , { action_item } , DEDENT ;
action_item    = requires_line | effects_line | strict_block | intent_line ;
requires_line  = "必要" , "=" , "[" , [ cap_ref , { "," , cap_ref } ] , "]" , NEWLINE ;
cap_ref        = ident , "." , ident ;                               (* 資源.動詞: 読取/作成/更新/削除 *)
effects_line   = "効果" , "=" , "[" , [ effect_ref , { "," , effect_ref } ] , "]" , NEWLINE ;
effect_ref     = ident , { "." , ident } ;                           (* pure / data.read / data.write / … *)
strict_block   = "厳密" , ":" , NEWLINE , block ;                    (* 動作ごとに 1 つ *)

params         = "(" , [ param , { "," , param } ] , ")" ;
param          = ident , ":" , type ;

(* 拡張(ADR-0122〜0125) *)
function_decl  = "関数" , ident , params , "->" , type , ":" , NEWLINE , block ;   (* 本体は厳密層。副作用なし *)
external_decl  = "外部関数" , ident , params , "->" , type , ":" , NEWLINE , INDENT , kv_line , { kv_line } , DEDENT ;
                 (* キー: 実装 = "….ts" | "….py"、効果 = [ … ] *)
analysis_decl  = "分析" , ident , params , "->" , type , ":" , NEWLINE , INDENT , kv_line , { kv_line } , DEDENT ;
                 (* キー: 対象 = [データ, …]、SQL = "….sql"。戻り値は 一覧<記録{…}> *)
component_decl = "部品" , ident , params , ":" , NEWLINE , INDENT , kv_line , { kv_line } , DEDENT ;
                 (* キー: web = "….ts"、ios = "….swift"、android = "….kt"(1 つ以上) *)
experiment_decl= "実験" , ident , ":" , NEWLINE , INDENT , kv_line , { kv_line } , DEDENT ;
                 (* キー: 画面・変種・割合・成果(すべて必須) *)
```
関数・外部関数・分析・部品・動作・アダプタの名前は同じ名前空間(重複は REF002)。

## 3. 型

```ebnf
type           = named_type | record_type ;
named_type     = ident , [ "<" , type , { "," , type } , ">" ] ;
record_type    = "記録" , "{" , [ ident , ":" , type , { "," , ident , ":" , type } ] , "}" ;
```
型引数の数・意味は [type_system.md](type_system.md)。`金額<JPY>` の引数は通貨コード(英大文字 3 文字)。

## 4. 厳密層

```ebnf
block          = INDENT , stmt , { stmt } , DEDENT ;
stmt           = if_stmt | loop_stmt | tx_stmt | check_stmt | return_stmt | assign_stmt | call_stmt ;

if_stmt        = "もし" , expr , ":" , NEWLINE , block ,
                 { "そうでなければ" , "もし" , expr , ":" , NEWLINE , block } ,
                 [ "そうでなければ" , ":" , NEWLINE , block ] ;
loop_stmt      = "繰り返す" , ident , "から" , expr , ":" , NEWLINE , block ;
tx_stmt        = "取引" , ":" , NEWLINE , block ;                    (* 入れ子は TYP002 *)
check_stmt     = expr , "を確認" , "または" , "失敗" , "(" , expr , ")" , NEWLINE ;
return_stmt    = expr , "を返す" , NEWLINE ;
assign_stmt    = place , "=" , expr , NEWLINE ;
place          = ident , { "." , ident } ;
call_stmt      = call , NEWLINE ;
```

### 4.1 式と優先順位

| 優先 | 演算子 | 結合 |
|---|---|---|
| 1(弱) | `\|\|` | 左 |
| 2 | `&&` | 左 |
| 3 | `==` `!=` | **非結合**(`a == b == c` は SYN001) |
| 4 | `<` `<=` `>` `>=` | **非結合** |
| 5 | `+` `-` | 左 |
| 6 | `*` `/` | 左 |
| 7 | 単項 `-` `!` | 右 |
| 8(強) | フィールド参照 `.`、呼出 `f(…)` | 左 |

```ebnf
expr           = or_expr ;
or_expr        = and_expr , { "||" , and_expr } ;
and_expr       = eq_expr , { "&&" , eq_expr } ;
eq_expr        = rel_expr , [ ( "==" | "!=" ) , rel_expr ] ;
rel_expr       = add_expr , [ ( "<" | "<=" | ">" | ">=" ) , add_expr ] ;
add_expr       = mul_expr , { ( "+" | "-" ) , mul_expr } ;
mul_expr       = unary , { ( "*" | "/" ) , unary } ;
unary          = ( "-" | "!" ) , unary | postfix ;
postfix        = primary , { "." , ident } ;                        (* x.f(…) のメソッド呼出は SYN001 *)
primary        = integer | decimal | string | "真" | "偽" | "なし"
               | call | ident | "(" , expr , ")" | "[" , [ expr , { "," , expr } ] , "]" ;
call           = ( ident | "失敗" | "成功" ) , "(" , [ expr , { "," , expr } ] , ")" ;
```
`=` は代入、`==` は比較。

## 5. 正規形
`crates/parser/src/printer.rs` が正規形の印字器。性質: `parse(print(ast))` は span を除き `ast` と等しく、`print` は不動点(テスト `ast_roundtrip_is_fixpoint_on_examples`)。
正規形では文字列は `"…"`、優先順位上不要な括弧は付けない、インデントは 4 空白。

## 6. 技術仕様書 §3.3 抜粋との差分
| 抜粋 | 本仕様 | 理由 |
|---|---|---|
| `declaration = app | data | screen | action | knowledge | contract` | `type_decl` `enum_decl` `adapter_decl` を追加 | C-02 |
| `statement = assignment | strict_block | intent_line | contract_line` | 動作本体は `必要`/`効果`/`厳密:`/意図行。`contract_line` は `約束:` 宣言に集約 | C-08 |
| 条件・反復の構文なし | `もし` / `繰り返す … から` | U-10 |
