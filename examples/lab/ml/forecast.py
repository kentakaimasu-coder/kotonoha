# 外部関数「需要予測」: 機械学習モデルの代わりの簡単な線形モデル(実運用では学習済みモデルを読み込む)
WEIGHTS = {"bias": 120.0, "price": -0.004}


def 需要予測(商品ID: str, 価格: int) -> int:
    score = WEIGHTS["bias"] + WEIGHTS["price"] * 価格 + (len(商品ID) % 7)
    return max(0, int(round(score)))
