import json, statistics as st, os, glob
from collections import Counter
E2="ROUND2_DIR"
E3="ROUND3_DIR"
g2={}
for f in glob.glob(E2+"/grade_part*.json"):
    for r in json.load(open(f)): g2[(r["persona"],r["variant"])]=r
g3={}
for f in glob.glob(E3+"/grade_part*.json"):
    try:
        for r in json.load(open(f)): g3[(r["persona"],r["variant"])]=r
    except Exception as e: print("bad",f,e)
mapN={"N01":"Q01","N02":"Q04","N03":"Q06","N04":"Q07","N05":"Q08","N06":"Q10"}
mapE={"E01":"Q02","E02":"Q03","E03":"Q05","E04":"Q09"}
def sus(a): return sum((x-1) if i%2==0 else (5-x) for i,x in enumerate(a))*2.5
def rep(E,p):
    f=f"{E}/{p}/report.fixed.json"
    f=f if os.path.exists(f) else f"{E}/{p}/report.json"
    return json.load(open(f)) if os.path.exists(f) else None
groups={"N 慣れていない人・非エンジニア":[mapN.get(f"N{i:02d}",f"N{i:02d}") for i in range(1,31)],"E いろいろな業種のエンジニア":[mapE.get(f"E{i:02d}",f"E{i:02d}") for i in range(1,31)]}
out={}
for k,ids in groups.items():
    rows=[]
    for p in ids:
        r3=rep(E3,p); r2=rep(E2,p)
        if r3 is None: continue
        def s(r,v):
            try: return sus(r[v]["sus"])
            except Exception: return None
        A3=g3.get((p,"A")); C3=g3.get((p,"C")); A2=g2.get((p,"A")); B2=g2.get((p,"B"))
        rows.append(dict(p=p,susA3=s(r3,"A"),susC3=s(r3,"C"),susA2=s(r2,"A"),susB2=s(r2,"B"),susC2=s(r2,"C"),
            sa3=A3["probe"]["score"] if A3 else None, la3=A3["lines"] if A3 else None, lb2=B2["lines"] if B2 else None, sb2=B2["probe"]["score"] if B2 else None,
            c3=(bool(C3["check"] and C3["contract_test"]) if C3 else None), ac=r3.get("A",{}).get("completed"), cc=r3.get("C",{}).get("completed"),
            ta3=r3.get("A",{}).get("tool_calls_approx")))
    m=lambda key: round(st.mean([r[key] for r in rows if isinstance(r[key],(int,float))]),1) if any(isinstance(r[key],(int,float)) for r in rows) else None
    print(f"== {k}({len(rows)}人)")
    print(f" SUS 第3回 A {m('susA3')} / C {m('susC3')}   第2回 A {m('susA2')} / B {m('susB2')} / C {m('susC2')}")
    a3,b2=m('susA3'),m('susB2')
    if a3 and b2: print(f" 第3回 A / 第2回 B = {round(a3/b2,2)} 倍   (第2回 A/B = {round(m('susA2')/b2,2)})")
    print(f" A>B(第3回A vs 第2回B) {sum(1 for r in rows if r['susA3'] is not None and r['susB2'] is not None and r['susA3']>r['susB2'])}人")
    print(f" 品質 A {m('sa3')} (満点 {sum(1 for r in rows if r['sa3']==19)}人/採点済 {sum(1 for r in rows if r['sa3'] is not None)}) | B(第2回) {m('sb2')}")
    print(f" 記述量 A {m('la3')} / B {m('lb2')}  比 {round(st.mean(r['lb2']/r['la3'] for r in rows if r['la3'] and r['lb2']),1) if any(r['la3'] for r in rows) else '-'}")
    print(f" 完成(自己申告) A {sum(1 for r in rows if r['ac'])}/{len(rows)}  C {sum(1 for r in rows if r['cc'])}/{len(rows)}  C の検査+契約テスト合格 {sum(1 for r in rows if r['c3'])}/{sum(1 for r in rows if r['c3'] is not None)}")
