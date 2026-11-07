// 拡張部品「星評価」(web)。React.createElement で書く(JSX は使わない: SSR を Node の型除去で動かすため)
import React from "react";

export function 星評価(props: { 評価: number }) {
  const n = Math.max(0, Math.min(5, props.評価));
  return React.createElement("p", { className: "rating", "aria-label": `評価 5 段階中 ${n}` }, "★".repeat(n) + "☆".repeat(5 - n));
}
