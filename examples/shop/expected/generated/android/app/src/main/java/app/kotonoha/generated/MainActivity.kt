// 生成物(koto build --target android)。手で編集しないでください。
package app.kotonoha.generated

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.material3.MaterialTheme
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val api = KotonohaApi(applicationContext, BuildConfig.KOTONOHA_API_BASE)
        setContent {
            MaterialTheme {
                val nav = rememberNavController()
                NavHost(navController = nav, startDestination = "menu") {
                    composable("menu") { KotonohaUi.Menu("商店", listOf("商品一覧" to "screen0", "商品管理" to "screen2", "注文履歴" to "screen3", "売れ筋" to "screen4"), nav) }
                    composable("login") { KotonohaUi.Login(api, nav) }
                    composable("screen0") { Screen0(api, nav) }
                    composable("screen1/{id}") { e -> Screen1(api, nav, Kotonoha.dec(e.arguments?.getString("id") ?: "")) }
                    composable("screen2") { Screen2(api, nav) }
                    composable("screen3") { Screen3(api, nav) }
                    composable("screen4") { Screen4(api, nav) }
                }
            }
        }
    }
}
