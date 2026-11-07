#!/usr/bin/env bash
# Android 生成物の部分検証(Android SDK が無い環境向け: K-01)
# 生成した Kotlin のうち Compose に依存しない層(モデル・API クライアント・Keystore トークン保管)を、
# 本物の kotlinc と Android フレームワーク(Robolectric android-all)に対して -Werror でコンパイルし、
# 生成 backend が返す実際の JSON を生成モデルで解析できることを確認する。
# Compose を使う画面(Screens.generated.kt / KotonohaUi.kt / MainActivity.kt)は対象外(CI の android-build で検証)。
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
WORK=${WORK:-$ROOT/target/android-jvm}
mkdir -p "$WORK" && cd "$WORK"
if [ ! -d lib ]; then
  cat > pom.xml <<'POM'
<project xmlns="http://maven.apache.org/POM/4.0.0"><modelVersion>4.0.0</modelVersion>
<groupId>k</groupId><artifactId>k</artifactId><version>1</version>
<dependencies>
<dependency><groupId>org.jetbrains.kotlin</groupId><artifactId>kotlin-compiler-embeddable</artifactId><version>2.0.21</version></dependency>
<dependency><groupId>org.robolectric</groupId><artifactId>android-all</artifactId><version>14-robolectric-10818077</version></dependency>
</dependencies></project>
POM
  mvn -q -B dependency:copy-dependencies -DoutputDirectory=lib
fi
CP=$(ls lib/*.jar | grep -v android-all | tr '\n' ':')
ANDROID=lib/android-all-14-robolectric-10818077.jar
STDLIB=$(ls lib/kotlin-stdlib-2*.jar)
CORO=$(ls lib/kotlinx-coroutines-core-jvm-*.jar)
KOTLINC="java -Dfile.encoding=UTF-8 -Dsun.jnu.encoding=UTF-8 -cp $CP org.jetbrains.kotlin.cli.jvm.K2JVMCompiler -no-stdlib"
rm -rf out classes vclasses lab lab-classes
"$ROOT/target/debug/koto" -C "$ROOT/examples/shop" build --target android --out "$WORK/out" >/dev/null
G=out/app/src/main/java/app/kotonoha/generated
$KOTLINC -Werror -classpath "$STDLIB:$ANDROID:$CORO" -d classes $G/Models.generated.kt $G/KotonohaApi.kt
echo "OK  生成 Kotlin(モデル + API クライアント)を -Werror でコンパイル"
"$ROOT/target/debug/koto" -C "$ROOT/examples/lab" build --target android --out "$WORK/lab" >/dev/null
L=lab/app/src/main/java/app/kotonoha/generated
$KOTLINC -Werror -classpath "$STDLIB:$ANDROID:$CORO" -d lab-classes $L/Models.generated.kt $L/KotonohaApi.kt
echo "OK  拡張の例(分析の行モデル・実験の割付 API)を -Werror でコンパイル"

# 実際の backend の応答を生成モデルで解析する
node --no-warnings "$ROOT/tools/android_jvm_fetch.mjs" "$WORK"
cat > Verify.kt <<'KT'
import app.kotonoha.generated.*
import org.json.JSONArray
import java.io.File
fun main(args: Array<String>) {
    val p = JSONArray(File(args[0]).readText())
    val items = (0 until p.length()).map { parseEntity__u5546_u54C1(p.getJSONObject(it)) }
    check(items.map { it.`名前` } == listOf("緑茶", "抹茶")) { "名前: $items" }
    check(items[0].`価格` == 1200L && items[1].`在庫` == 0L) { "数値: $items" }
    check(Kotonoha.yen(items[1].`価格`) == "¥98,000") { Kotonoha.yen(items[1].`価格`) }
    val r = JSONArray(File(args[1]).readText())
    val rows = (0 until r.length()).map { parseRow__u58F2_u308C_u7B4B_u5546_u54C1(r.getJSONObject(it)) }
    check(rows.size == 1 && rows[0].`商品ID` == "P1" && rows[0].`売上金額` == 2400L) { "集計: $rows" }
    println("OK  backend の JSON を生成 Kotlin モデルで解析: ${items.size} 商品, ${rows.size} 集計行")
}
KT
$KOTLINC -classpath "$STDLIB:$ANDROID:classes" -d vclasses Verify.kt
java -Dstdout.encoding=UTF-8 -cp "vclasses:classes:$STDLIB:$ANDROID" VerifyKt products.json ranking.json
