// KOTONOHA Android runtime: API クライアント・トークン保管(Keystore)・表示形式(Compose に依存しない)
// 共通 API 契約(api.json)のワイヤ形式に従う。認可は backend が強制する。
package app.kotonoha.generated

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject
import java.net.HttpURLConnection
import java.net.URI
import java.net.URL
import java.net.URLDecoder
import java.net.URLEncoder
import java.text.NumberFormat
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.util.Locale

class KotonohaHttpException(val status: Int, override val message: String) : Exception(message)
class KotonohaFailure(val value: String) : Exception(value)
class KotonohaInputException(override val message: String) : Exception(message)

/** トークンを Android Keystore の AES-GCM 鍵で暗号化して保存する(鍵は端末外へ出ない)。 */
class TokenStore(context: Context) {
    private val prefs = context.getSharedPreferences("kotonoha", Context.MODE_PRIVATE)
    private val alias = "kotonoha.token.key"

    private fun key(): SecretKey {
        val ks = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (ks.getKey(alias, null) as? SecretKey)?.let { return it }
        val gen = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
        gen.init(
            KeyGenParameterSpec.Builder(alias, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .build()
        )
        return gen.generateKey()
    }

    fun load(): String? {
        val data = prefs.getString("token.enc", null) ?: return null
        return runCatching {
            val raw = Base64.decode(data, Base64.NO_WRAP)
            val c = Cipher.getInstance("AES/GCM/NoPadding")
            c.init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, raw.copyOfRange(0, 12)))
            String(c.doFinal(raw.copyOfRange(12, raw.size)), Charsets.UTF_8)
        }.getOrNull()
    }

    fun save(token: String?) {
        if (token == null) {
            prefs.edit().remove("token.enc").apply()
            return
        }
        val c = Cipher.getInstance("AES/GCM/NoPadding")
        c.init(Cipher.ENCRYPT_MODE, key())
        val out = c.iv + c.doFinal(token.toByteArray(Charsets.UTF_8))
        prefs.edit().putString("token.enc", Base64.encodeToString(out, Base64.NO_WRAP)).apply()
    }
}

class KotonohaApi(context: Context, private val base: String) {
    private val store = TokenStore(context)
    /** 実験(A/B テスト)の匿名の対象者 ID。端末ごとに 1 つ(個人情報は含めない。ADR-0122) */
    private val subject: String = context.getSharedPreferences("kotonoha", Context.MODE_PRIVATE).let { p ->
        p.getString("subject", null) ?: java.util.UUID.randomUUID().toString().also { p.edit().putString("subject", it).apply() }
    }
    var token: String?
        get() = store.load()
        set(v) = store.save(v)

    private fun url(segments: List<String>, id: String?, query: Map<String, String>): URL {
        val path = (segments + listOfNotNull(id)).joinToString("/") { Kotonoha.enc(it) }
        val qs = query.entries.sortedBy { it.key }.joinToString("&") { Kotonoha.enc(it.key) + "=" + Kotonoha.enc(it.value) }
        return URI(base.trimEnd('/') + "/" + path + (if (qs.isEmpty()) "" else "?$qs")).toURL()
    }

    private suspend fun send(u: URL, method: String, body: String?): String = withContext(Dispatchers.IO) {
        val c = u.openConnection() as HttpURLConnection
        try {
            c.requestMethod = method
            c.connectTimeout = 10_000
            c.readTimeout = 15_000
            token?.let { c.setRequestProperty("Authorization", "Bearer $it") }
            c.setRequestProperty("X-Koto-Subject", subject)
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

    /** 実験の変種を受け取る(失敗したら対照群 = 先頭。露出は記録されないので結果は偏らない) */
    suspend fun variant(experiment: String, variants: List<String>): String = try {
        val v = JSONObject(send(url(listOf("api", "experiments", experiment, "assign"), null, emptyMap()), "POST", "{}")).optString("variant")
        if (v in variants) v else variants[0]
    } catch (e: Exception) {
        variants[0]
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
        DateTimeFormatter.ofPattern("yyyy/MM/dd HH:mm").withZone(ZoneId.systemDefault()).format(java.time.OffsetDateTime.parse(v).toInstant())
    /** 全角数字・全角マイナス・カンマ・空白を受け付ける(NR-01) */
    fun parseLong(s: String, field: String): Long =
        java.text.Normalizer.normalize(s, java.text.Normalizer.Form.NFKC)
            .replace(Regex("[\u2212\u30FC\uFF0D]"), "-")
            .replace(Regex("[,、\\s\u3000]"), "")
            .toLongOrNull() ?: throw KotonohaInputException("$field は整数で入力してください")
    /** 暦日 "2026-12-05" → 2026年12月5日(土) */
    fun date(v: String): String =
        runCatching { java.time.LocalDate.parse(v).format(DateTimeFormatter.ofPattern("yyyy年M月d日(E)", Locale.JAPAN)) }.getOrDefault(v)
    /** 日付の入力。「2026-12-05」「2026/12/5」、全角数字も受け付ける(NR-01) */
    fun parseDate(s: String, field: String): String {
        val t = java.text.Normalizer.normalize(s, java.text.Normalizer.Form.NFKC).trim().replace('/', '-')
        return runCatching { java.time.LocalDate.parse(t, DateTimeFormatter.ofPattern("yyyy-M-d")).toString() }.getOrNull()
            ?: throw KotonohaInputException("$field は「2026-12-05」の形で入力してください")
    }
    /** 日時の入力(端末の時刻帯)。「2026-12-05 19:00」「2026/12/05 19:00」、全角数字も受け付ける(NR-01) */
    fun parseDateTime(s: String, field: String): String {
        val t = java.text.Normalizer.normalize(s, java.text.Normalizer.Form.NFKC).trim().replace('/', '-').replace(Regex("\\s+"), " ")
        val local = runCatching { java.time.LocalDateTime.parse(t, DateTimeFormatter.ofPattern("yyyy-M-d H:mm")) }.getOrNull()
            ?: throw KotonohaInputException("$field は「2026-12-05 19:00」の形で入力してください")
        return local.atZone(ZoneId.systemDefault()).toInstant().toString()
    }
    fun describe(e: Exception): String = when (e) {
        is KotonohaHttpException -> when (e.status) { 401 -> "ログインが必要です"; 403 -> "権限がありません"; 429 -> "要求が多すぎます"; else -> e.message }
        is KotonohaFailure -> "失敗しました: ${e.value}"
        is KotonohaInputException -> e.message
        else -> e.message ?: e.toString()
    }
}
