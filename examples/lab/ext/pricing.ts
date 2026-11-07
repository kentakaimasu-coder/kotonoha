// 外部関数「割引額」: 価格(円)から値引きする金額(円)を返す。引数・戻り値は API と同じ JSON 値。
export function 割引額(価格: number): number {
  if (価格 >= 10000) return Math.floor(価格 / 10);
  if (価格 >= 3000) return Math.floor(価格 / 20);
  return 0;
}
