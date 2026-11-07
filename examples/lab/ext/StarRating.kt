// 拡張部品「星評価」(Android)。名前は KOTONOHA の部品名と同じにする(日本語名は `…` で囲む)
package app.kotonoha.generated

import androidx.compose.material3.Text
import androidx.compose.runtime.Composable

@Composable
fun `星評価`(`評価`: Long) {
    val n = `評価`.coerceIn(0, 5).toInt()
    Text("★".repeat(n) + "☆".repeat(5 - n))
}
