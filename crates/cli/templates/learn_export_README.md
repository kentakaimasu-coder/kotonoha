# KOTONOHA 学習データ(koto learn export)

このフォルダは、あなたが `koto approve` で承認した「書いた文 → 正規文」の記録から作った、
オープンモデルの追加学習用データです。中身はあなたの承認だけで、ソース本文やデータの中身は含みません。

| ファイル | 形式 | 使い道 |
|---|---|---|
| `sft.jsonl` | `{"messages": [system, user, assistant]}` | 教師ありの追加学習(SFT / LoRA) |
| `dpo.jsonl` | `{"prompt": [...], "chosen": [...], "rejected": [...]}` | 選ばれた解釈を選ばれなかった解釈より好むように調整(DPO) |

どちらも Hugging Face TRL などの会話形式(chat 形式)のデータセットとしてそのまま読み込めます。

## 育て方の流れ

1. 今のモデルの成績を測る: `koto learn eval --model qwen2.5:7b`
2. このデータで追加学習する(例: TRL / Unsloth / Axolotl で LoRA を作る)
3. 学習したモデルを手元のサーバーに載せる(例: GGUF に変換して Ollama の `Modelfile` で `FROM` と `ADAPTER` を指定し、`ollama create kotonoha-ja -f Modelfile`)
4. 新しいモデルを同じ方法で測る: `koto learn eval --model kotonoha-ja --min 0.8`
5. 前回より良く、基準を満たしたときだけ使う: `koto resolve --adapter local --model kotonoha-ja`

`koto learn status` で評価の推移を確認できます。

## 注意

- モデルが出した解釈は、どのモデルでも「候補」にすぎません。KOTONOHA は形式の検査と、既知の文型で
  成立するかの再照合を行い、人が承認したものだけを使います。学習したモデルでも、この順番は変わりません。
- 承認は人の判断の記録です。間違った承認を学習すると間違いも覚えます。おかしな記録は
  学習記録のフォルダの `memory.jsonl` から該当の行を消してから書き出してください。
- この手順の 2〜3(学習とモデルの作成)は KOTONOHA の外の道具で行います。KOTONOHA の開発環境では
  実際のモデルでの学習・評価は確認していません(模擬サーバーで動作を確認しています)。
