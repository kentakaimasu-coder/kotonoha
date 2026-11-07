// 拡張部品「数字評価」(iOS)
import SwiftUI

struct 数字評価: View {
    let 評価: Int64
    var body: some View { Text("評価 \(評価) / 5") }
}
