// KOTONOHA Android runtime: 共通 UI(Compose)
package app.kotonoha.generated

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.navigation.NavController

object KotonohaUi {
    @OptIn(ExperimentalMaterial3Api::class)
    @Composable
    fun ScreenScaffold(title: String, nav: NavController, content: @Composable () -> Unit) {
        Scaffold(topBar = { TopAppBar(title = { Text(title) }, navigationIcon = { TextButton(onClick = { nav.popBackStack() }) { Text("戻る") } }) }) { p ->
            Box(Modifier.padding(p)) { content() }
        }
    }

    @Composable
    fun Menu(appName: String, screens: List<Pair<String, String>>, nav: NavController) {
        ScreenScaffold(appName, nav) {
            LazyColumn(Modifier.fillMaxSize().padding(16.dp)) {
                items(screens) { (name, route) ->
                    Text(name, Modifier.fillMaxWidth().clickable { nav.navigate(route) }.padding(vertical = 12.dp))
                    HorizontalDivider()
                }
                item { Text("ログイン", Modifier.fillMaxWidth().clickable { nav.navigate("login") }.padding(vertical = 12.dp)) }
            }
        }
    }

    @Composable
    fun Login(api: KotonohaApi, nav: NavController) {
        var t by remember { mutableStateOf("") }
        ScreenScaffold("ログイン", nav) {
            Column(Modifier.padding(16.dp)) {
                Text("認証基盤(ADR-002)から発行されたトークン")
                OutlinedTextField(value = t, onValueChange = { t = it }, label = { Text("トークン") })
                Button(onClick = { api.token = t; nav.popBackStack() }) { Text("ログイン") }
                TextButton(onClick = { api.token = null }) { Text("ログアウト") }
            }
        }
    }

    @Composable
    fun EnumPicker(label: String, options: List<String>, selected: String, onSelect: (String) -> Unit) {
        Column {
            Text(label)
            options.forEach { o -> Row { RadioButton(selected = o == selected, onClick = { onSelect(o) }); Text(o) } }
        }
    }
}
