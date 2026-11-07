// 拡張部品「星評価」(iOS)
import SwiftUI

struct 星評価: View {
    let 評価: Int64
    var body: some View {
        let n = Int(max(0, min(5, 評価)))
        Text(String(repeating: "★", count: n) + String(repeating: "☆", count: 5 - n))
            .accessibilityLabel("評価 5 段階中 \(n)")
    }
}
