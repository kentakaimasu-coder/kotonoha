// 拡張部品「数字評価」(web): 実験「評価の見せ方」の変種
import React from "react";

export function 数字評価(props: { 評価: number }) {
  return React.createElement("p", { className: "rating" }, `評価 ${props.評価} / 5`);
}
