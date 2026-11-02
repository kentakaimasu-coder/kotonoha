// KOTONOHA Android runtime: API クライアント・トークン保管・表示形式・共通 UI
// 共通 API 契約(api.json)のワイヤ形式に従う。認可は backend が強制する。
package app.kotonoha.generated

import android.content.Context
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.navigation.NavController
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject
import java.net.HttpURLConnection
import java.net.URL
import java.net.URLDecoder
import java.net.URLEncoder
import java.text.NumberFormat
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.util.Locale

class KotonohaHttpException(val status: Int, override val message: String) : Exception(message)
class KotonohaFailure(val value: String) : Exception(value)
class KotonohaInputException(override val message: String) : Exception(message)

class KotonohaApi(context: Context, private val base: String) {
    // 既知の課題 K-07: トークンは端末内の private SharedPreferences に保存(暗号化保管は未対応)
    private val prefs = context.getSharedPreferences("kotonoha", Context.MODE_PRIVATE)
    var token: String?
        get() = prefs.getString("token", null)
        set(v) { prefs.edit().apply { if (v == null) remove("token") else putString("token", v) }.apply() }

    private fun url(segments: List<String>, id: String?, query: Map<String, String>): URL {
        val path = (segments + listOfNotNull(id)).joinToString("/") { Kotonoha.enc(it) }
        val qs = query.entries.sortedBy { it.key }.joinToString("&") { Kotonoha.enc(it.key) + "=" + Kotonoha.enc(it.value) }
        return URL(base.trimEnd('/') + "/" + path + (if (qs.isEmpty()) "" else "?$qs"))
    }

    private suspend fun send(u: URL, method: String, body: String?): String = withContext(Dispatchers.IO) {
        val c = u.openConnection() as HttpURLConnection
        try {
            c.requestMethod = method
            c.connectTimeout = 10_000
            c.readTimeout = 15_000
            token?.let { c.setRequestProperty("Authorization", "Bearer $it") }
            if (body != null) {
                c.doOutput = true
                c.setRequestProperty("Content-Type", "application/json; charset=utf-8")
                c.outputStream.use { it.write(body.toByteArray(Charsets.UTF_8)) }
            }
            val status = c.responseCode
            val text = (if (status in 200..299) c.inputStream else c.errorStream)?.bufferedReader(Charsets.UTF_8)?.use { it.readText() } ?: ""
            if (status in 200..299) return@withContext text
            val err = runCatching { JSONObject(text) }.getOrNull()
            if (status == 422 && err?.optString("error") == "failure") throw KotonohaFailure(err.optString("value"))
            throw KotonohaHttpException(status, err?.optString("message")?.takeIf { it.isNotEmpty() } ?: "エラー($status)")
        } finally {
            c.disconnect()
        }
    }

    suspend fun getObject(segments: List<String>, id: String? = null, query: Map<String, String> = emptyMap()): JSONObject =
        JSONObject(send(url(segments, id, query), "GET", null))

    suspend fun getArray(segments: List<String>, query: Map<String, String> = emptyMap()): List<JSONObject> {
        val a = JSONArray(send(url(segments, null, query), "GET", null))
        return (0 until a.length()).map { a.getJSONObject(it) }
    }

    suspend fun post(action: String, body: Map<String, Any?>): JSONObject {
        val o = JSONObject()
        for ((k, v) in body) o.put(k, v ?: JSONObject.NULL)
        return JSONObject(send(url(listOf("api", "actions", action), null, emptyMap()), "POST", o.toString()))
    }
}

object Kotonoha {
    fun enc(s: String): String = URLEncoder.encode(s, "UTF-8").replace("+", "%20")
    fun dec(s: String): String = URLDecoder.decode(s, "UTF-8")
    fun yen(v: Long): String = "¥" + NumberFormat.getIntegerInstance(Locale.JAPAN).format(v)
    fun bool(v: Boolean): String = if (v) "はい" else "いいえ"
    fun datetime(v: String): String =
        DateTimeFormatter.ofPattern("yyyy/MM/dd HH:mm").withZone(ZoneId.systemDefault()).format(Instant.parse(v))
    fun parseLong(s: String, field: String): Long = s.trim().toLongOrNull() ?: throw KotonohaInputException("$field は整数で入力してください")
    fun describe(e: Exception): String = when (e) {
        is KotonohaHttpException -> when (e.status) { 401 -> "ログインが必要です"; 403 -> "権限がありません"; else -> e.message }
        is KotonohaFailure -> "失敗しました: ${e.value}"
        is KotonohaInputException -> e.message
        else -> e.message ?: e.toString()
    }

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
