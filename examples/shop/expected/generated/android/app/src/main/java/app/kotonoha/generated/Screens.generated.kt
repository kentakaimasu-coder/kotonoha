// 生成物(koto build --target android)。手で編集しないでください。
package app.kotonoha.generated

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.navigation.NavController
import kotlinx.coroutines.launch
import org.json.JSONObject

@Composable
fun Screen0(api: KotonohaApi, nav: NavController) {
    val scope = rememberCoroutineScope()
    var message by remember { mutableStateOf<String?>(null) }
    var list0 by remember { mutableStateOf<List<Entity__u5546_u54C1>>(emptyList()) }
    suspend fun load() {
        try {
            list0 = api.getArray(listOf("api", "data", "商品")).map { parseEntity__u5546_u54C1(it) }
        } catch (e: Exception) { message = Kotonoha.describe(e) }
    }
    suspend fun act(op: suspend () -> Unit) {
        try { op(); message = "完了しました"; load() } catch (e: Exception) { message = Kotonoha.describe(e) }
    }
    LaunchedEffect(Unit) { load() }
    KotonohaUi.ScreenScaffold("商品一覧", nav) {
        LazyColumn(Modifier.fillMaxSize().padding(16.dp)) {
            item { Text("商品の一覧", style = MaterialTheme.typography.titleMedium) }
            items(list0) { row -> Column(Modifier.fillMaxWidth().clickable { nav.navigate("screen1/" + Kotonoha.enc(row.`ID`)) }.padding(vertical = 8.dp)) { Text(row.`名前`.toString()); Text(Kotonoha.yen(row.`価格`)); Text(row.`在庫`.toString()) }; HorizontalDivider() }
            message?.let { m -> item { Text(m, color = MaterialTheme.colorScheme.secondary) } }
        }
    }
}

@Composable
fun Screen1(api: KotonohaApi, nav: NavController, itemId: String) {
    val scope = rememberCoroutineScope()
    var message by remember { mutableStateOf<String?>(null) }
    var cur by remember { mutableStateOf<Entity__u5546_u54C1?>(null) }
    suspend fun load() {
        try {
            cur = parseEntity__u5546_u54C1(api.getObject(listOf("api", "data", "商品"), itemId))
        } catch (e: Exception) { message = Kotonoha.describe(e) }
    }
    suspend fun act(op: suspend () -> Unit) {
        try { op(); message = "完了しました"; load() } catch (e: Exception) { message = Kotonoha.describe(e) }
    }
    LaunchedEffect(Unit) { load() }
    KotonohaUi.ScreenScaffold("商品詳細", nav) {
        LazyColumn(Modifier.fillMaxSize().padding(16.dp)) {
            cur?.let { c -> item { Column { Text("名前" + ": " + c.`名前`.toString()); Text("価格" + ": " + Kotonoha.yen(c.`価格`)); Text("在庫" + ": " + c.`在庫`.toString()) } } }
            item { Button(onClick = { scope.launch { val c = cur ?: return@launch; act { api.post("注文する", mapOf("商品ID" to c.`ID`, "数量" to 1L)) } } }) { Text("購入") } }
            message?.let { m -> item { Text(m, color = MaterialTheme.colorScheme.secondary) } }
        }
    }
}

@Composable
fun Screen2(api: KotonohaApi, nav: NavController) {
    val scope = rememberCoroutineScope()
    var message by remember { mutableStateOf<String?>(null) }
    var f0__u540D_u524D by remember { mutableStateOf("") }
    var f0__u4FA1_u683C by remember { mutableStateOf("") }
    var f0__u5728_u5EAB by remember { mutableStateOf("") }
    var list1 by remember { mutableStateOf<List<Entity__u5546_u54C1>>(emptyList()) }
    suspend fun load() {
        try {
            list1 = api.getArray(listOf("api", "data", "商品")).map { parseEntity__u5546_u54C1(it) }
        } catch (e: Exception) { message = Kotonoha.describe(e) }
    }
    suspend fun act(op: suspend () -> Unit) {
        try { op(); message = "完了しました"; load() } catch (e: Exception) { message = Kotonoha.describe(e) }
    }
    LaunchedEffect(Unit) { load() }
    KotonohaUi.ScreenScaffold("商品管理", nav) {
        LazyColumn(Modifier.fillMaxSize().padding(16.dp)) {
            item { Column { Text("商品登録", style = MaterialTheme.typography.titleMedium); OutlinedTextField(value = f0__u540D_u524D, onValueChange = { f0__u540D_u524D = it }, label = { Text("名前") }); OutlinedTextField(value = f0__u4FA1_u683C, onValueChange = { f0__u4FA1_u683C = it }, label = { Text("価格") }, keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number)); OutlinedTextField(value = f0__u5728_u5EAB, onValueChange = { f0__u5728_u5EAB = it }, label = { Text("在庫") }, keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number)); Button(onClick = { scope.launch { act { api.post("商品登録", mapOf("名前" to f0__u540D_u524D, "価格" to Kotonoha.parseLong(f0__u4FA1_u683C, "価格"), "在庫" to Kotonoha.parseLong(f0__u5728_u5EAB, "在庫"))) } } }) { Text("送信") } } }
            item { Text("商品の一覧", style = MaterialTheme.typography.titleMedium) }
            items(list1) { row -> Column(Modifier.fillMaxWidth().clickable { nav.navigate("screen1/" + Kotonoha.enc(row.`ID`)) }.padding(vertical = 8.dp)) { Text(row.`名前`.toString()); Text(Kotonoha.yen(row.`価格`)); Text(row.`在庫`.toString()) }; HorizontalDivider() }
            message?.let { m -> item { Text(m, color = MaterialTheme.colorScheme.secondary) } }
        }
    }
}

@Composable
fun Screen3(api: KotonohaApi, nav: NavController) {
    val scope = rememberCoroutineScope()
    var message by remember { mutableStateOf<String?>(null) }
    var list0 by remember { mutableStateOf<List<Entity__u6CE8_u6587>>(emptyList()) }
    suspend fun load() {
        try {
            list0 = api.getArray(listOf("api", "data", "注文")).map { parseEntity__u6CE8_u6587(it) }
        } catch (e: Exception) { message = Kotonoha.describe(e) }
    }
    suspend fun act(op: suspend () -> Unit) {
        try { op(); message = "完了しました"; load() } catch (e: Exception) { message = Kotonoha.describe(e) }
    }
    LaunchedEffect(Unit) { load() }
    KotonohaUi.ScreenScaffold("注文履歴", nav) {
        LazyColumn(Modifier.fillMaxSize().padding(16.dp)) {
            item { Text("注文の一覧", style = MaterialTheme.typography.titleMedium) }
            items(list0) { row -> Column(Modifier.fillMaxWidth().padding(vertical = 8.dp)) { Text(row.`商品ID`.toString()); Text(row.`数量`.toString()); Text(row.`注文者`.toString()) }; HorizontalDivider() }
            message?.let { m -> item { Text(m, color = MaterialTheme.colorScheme.secondary) } }
        }
    }
}

@Composable
fun Screen4(api: KotonohaApi, nav: NavController) {
    val scope = rememberCoroutineScope()
    var message by remember { mutableStateOf<String?>(null) }
    var query0 by remember { mutableStateOf<List<Row__u58F2_u308C_u7B4B_u5546_u54C1>>(emptyList()) }
    suspend fun load() {
        try {
            query0 = api.getArray(listOf("api", "queries", "売れ筋商品"), mapOf("limit" to "20")).map { parseRow__u58F2_u308C_u7B4B_u5546_u54C1(it) }
        } catch (e: Exception) { message = Kotonoha.describe(e) }
    }
    suspend fun act(op: suspend () -> Unit) {
        try { op(); message = "完了しました"; load() } catch (e: Exception) { message = Kotonoha.describe(e) }
    }
    LaunchedEffect(Unit) { load() }
    KotonohaUi.ScreenScaffold("売れ筋", nav) {
        LazyColumn(Modifier.fillMaxSize().padding(16.dp)) {
            item { Text("売れ筋商品(上位20件)", style = MaterialTheme.typography.titleMedium) }
            items(query0) { row -> Row(Modifier.fillMaxWidth().padding(vertical = 8.dp), horizontalArrangement = Arrangement.SpaceBetween) { Text(row.`商品ID`.toString()); Text(Kotonoha.yen(row.`売上金額`)) } }
            message?.let { m -> item { Text(m, color = MaterialTheme.colorScheme.secondary) } }
        }
    }
}

