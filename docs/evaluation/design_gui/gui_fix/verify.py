import json,sys,re,os
d=sys.argv[1]
src=open(os.path.join(d,'src/app.koto'),encoding='utf-8').read()
cfg=json.load(open(os.path.join(d,'kotonoha.json'),encoding='utf-8'))
body=re.sub(r'(?m)^\s*#.*$','',src)
def screen(name):
    m=re.search(r'(?m)^画面 '+re.escape(name)+r':\n((?:    .*\n?)*)',body)
    return m.group(1) if m else ''
first=re.search(r'(?m)^画面 (\S+):',body)
listscr=[n for n in re.findall(r'(?m)^画面 (\S+):',body) if re.search(r'イベント(のうち.*のもの)?を.*一覧表示',screen(n))]
ls=''.join(screen(n) for n in listscr)
det=''.join(screen(n) for n in re.findall(r'(?m)^画面 (\S+):',body) if re.search(r'イベントの\S+を表示',screen(n)))
r={
 1: str((cfg.get('theme') or {}).get('color','')) in ('緑','green','#2e7d32') or '緑' in json.dumps(cfg,ensure_ascii=False),
 2: 'アプリ「市民講座の申込」' in body,
 3: '「申し込む」を押すと' in body and '「1名で予約」' not in body,
 4: '定員に達しました' in body and '"満席"' not in body,
 5: bool(re.search(r'イベント(のうち.*のもの)?を開始日時(の(古い|早い|小さい|昇))?順に一覧表示',ls)),
 6: bool(re.search(r'イベントのうち開始日時が今日以降',ls)),
 7: bool(re.search(r'(?m)^    持ち物: 選択<文字列>',body)) and '持ち物' in det,
 8: '「2名で予約」' not in body,
 9: bool(re.search(r'(?m)^画面 マイ予約:',body)) and not re.search(r'(?m)^画面 自分の予約:',body),
 10: bool(re.search(r'(?m)^    参加費: 選択<金額>',body)),
 11: bool(re.search(r'この画面は主催者だけ',screen('管理'))),
 12: bool(first) and first.group(1) in ('マイ予約','自分の予約'),
}
print(json.dumps({k:v for k,v in r.items()},ensure_ascii=False)); print('合計',sum(r.values()),'/ 12')
