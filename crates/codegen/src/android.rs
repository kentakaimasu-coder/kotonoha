//! Android 生成(Kotlin / Jetpack Compose / minSdk 26 案: ADR-005 未決定)。docs/architecture.md §7。
//! 共通 API 契約だけを使うクライアント。この環境に Android SDK が無いため**ビルド未検証**(K-01)。

use crate::appir::{api_contract, check_ir, AppIr, Component};
use crate::GeneratedFile;
use kotonoha_ast::{BinOp, Expr, ExprKind};
use kotonoha_diag::{Code, Diagnostic};
use kotonoha_types::Type;

pub const RUNTIME_API_KT: &str = include_str!("../../../runtime/android/KotonohaApi.kt");
pub const RUNTIME_UI_KT: &str = include_str!("../../../runtime/android/KotonohaUi.kt");
const PKG: &str = "app.kotonoha.generated";

fn bt(n: &str) -> String {
    format!("`{n}`")
}

fn kstr(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '$' => o.push_str("\\$"),
            '\n' => o.push_str("\\n"),
            c if c.is_control() => o.push_str(&format!("\\u{:04X}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// クラス名は JVM のクラスファイル名になるため ASCII に符号化する(ファイル名の文字コードに依存しないように: NFR-009)
fn tn(prefix: &str, n: &str) -> String {
    let mut s = String::from(prefix);
    for c in n.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            s.push(c);
        } else {
            s.push_str(&format!("_u{:04X}", c as u32));
        }
    }
    s
}

fn kt_type(t: &Type) -> String {
    match t {
        Type::Bool => "Boolean".into(),
        Type::Int | Type::Money(_) => "Long".into(),
        Type::Str | Type::Id | Type::Url | Type::Email | Type::DateTime => "String".into(),
        Type::Enum(_) => "String".into(),
        Type::Option(i) => format!("{}?", kt_type(i)),
        _ => "Nothing".into(),
    }
}

/// JSONObject から値を取り出す式
fn json_get(o: &str, n: &str, t: &Type) -> String {
    match t {
        Type::Option(i) => format!("(if ({o}.isNull({k})) null else {})", json_get(o, n, i), k = kstr(n)),
        Type::Bool => format!("{o}.getBoolean({})", kstr(n)),
        Type::Int | Type::Money(_) => format!("{o}.getLong({})", kstr(n)),
        _ => format!("{o}.getString({})", kstr(n)),
    }
}

fn display(v: &str, t: &Type) -> String {
    match t {
        Type::Money(c) if c == "JPY" => format!("Kotonoha.yen({v})"),
        Type::Bool => format!("Kotonoha.bool({v})"),
        Type::DateTime => format!("Kotonoha.datetime({v})"),
        Type::Option(i) => format!("({v}?.let {{ {} }} ?: \"—\")", display("it", i)),
        _ => format!("{v}.toString()"),
    }
}

fn arg_kt(e: &Expr, input: Option<&str>, errs: &mut Vec<Diagnostic>) -> String {
    match &e.kind {
        ExprKind::Int(v) => format!("{v}L"),
        ExprKind::Str(s) => kstr(s),
        ExprKind::Bool(b) => b.to_string(),
        ExprKind::None => "null".into(),
        ExprKind::Ident(n) if Some(n.as_str()) == input => "c".into(),
        ExprKind::Field(b, f) => format!("{}.{}", arg_kt(b, input, errs), bt(&f.name)),
        ExprKind::Binary {
            op: op @ (BinOp::Add | BinOp::Sub | BinOp::Mul),
            lhs,
            rhs,
        } => {
            format!("({} {} {})", arg_kt(lhs, input, errs), op.symbol(), arg_kt(rhs, input, errs))
        }
        _ => {
            errs.push(Diagnostic::error(Code::TGT001, e.span, "この画面引数式は Android 生成に未対応です"));
            "null".into()
        }
    }
}

pub fn generate(ir: &AppIr) -> Result<Vec<GeneratedFile>, Vec<Diagnostic>> {
    let mut errs = check_ir(ir);
    if !errs.is_empty() {
        return Err(errs);
    }
    let mut m = format!("// 生成物(koto build --target android)。手で編集しないでください。\npackage {PKG}\n\nimport org.json.JSONObject\n\n");
    for (e, vs) in &ir.enums {
        m.push_str(&format!(
            "val {}: List<String> = listOf({})\n\n",
            tn("ENUM_", e),
            vs.iter().map(|v| kstr(v)).collect::<Vec<_>>().join(", ")
        ));
    }
    let record = |name: String, fields: &[(String, Type)], m: &mut String| {
        m.push_str(&format!("data class {name}(\n"));
        for (n, t) in fields {
            m.push_str(&format!("    val {}: {},\n", bt(n), kt_type(t)));
        }
        m.push_str(")\n\n");
        m.push_str(&format!(
            "fun parse{name}(o: JSONObject): {name} = {name}(\n{}\n)\n\n",
            fields
                .iter()
                .map(|(n, t)| format!("    {} = {},", bt(n), json_get("o", n, t)))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    };
    for e in &ir.entities {
        record(tn("Entity_", &e.name), &e.fields, &mut m);
    }
    for (spec, _) in &ir.queries {
        if let Type::Record(cols) = spec.row_type() {
            record(tn("Row_", &spec.name), &cols, &mut m);
        }
    }

    let mut v = format!(
        "// 生成物(koto build --target android)。手で編集しないでください。\npackage {PKG}\n\nimport androidx.compose.foundation.clickable\nimport androidx.compose.foundation.layout.*\nimport androidx.compose.foundation.lazy.LazyColumn\nimport androidx.compose.foundation.lazy.items\nimport androidx.compose.foundation.text.KeyboardOptions\nimport androidx.compose.material3.*\nimport androidx.compose.runtime.*\nimport androidx.compose.ui.Modifier\nimport androidx.compose.ui.text.input.KeyboardType\nimport androidx.compose.ui.unit.dp\nimport androidx.navigation.NavController\nimport kotlinx.coroutines.launch\nimport org.json.JSONObject\n\n"
    );
    let index_of = |name: &str| ir.screens.iter().position(|s| s.name == name).unwrap_or(0);
    for (si, s) in ir.screens.iter().enumerate() {
        let mut state = Vec::new();
        let mut loads = Vec::new();
        let mut items = Vec::new();
        if let Some(d) = &s.input {
            state.push(format!("    var cur by remember {{ mutableStateOf<{}?>(null) }}", tn("Entity_", d)));
            loads.push(format!(
                "cur = parse{}(api.getObject(listOf(\"api\", \"data\", {}), itemId))",
                tn("Entity_", d),
                kstr(d)
            ));
        }
        for (ci, c) in s.components.iter().enumerate() {
            match c {
                Component::List { data, fields, link_to } => {
                    let var = format!("list{ci}");
                    let et = tn("Entity_", data);
                    state.push(format!("    var {var} by remember {{ mutableStateOf<List<{et}>>(emptyList()) }}"));
                    loads.push(format!(
                        "{var} = api.getArray(listOf(\"api\", \"data\", {})).map {{ parse{et}(it) }}",
                        kstr(data)
                    ));
                    let texts: Vec<String> = fields.iter().map(|(n, t)| format!("Text({})", display(&format!("row.{}", bt(n)), t))).collect();
                    let click = match link_to {
                        Some(t) => format!(".clickable {{ nav.navigate(\"screen{}/\" + Kotonoha.enc(row.`ID`)) }}", index_of(t)),
                        None => String::new(),
                    };
                    items.push(format!(
                        "item {{ Text({}, style = MaterialTheme.typography.titleMedium) }}",
                        kstr(&format!("{data}の一覧"))
                    ));
                    items.push(format!(
                        "items({var}) {{ row -> Column(Modifier.fillMaxWidth(){click}.padding(vertical = 8.dp)) {{ {} }}; HorizontalDivider() }}",
                        texts.join("; ")
                    ));
                }
                Component::Detail { fields, .. } => {
                    let rows: Vec<String> = fields
                        .iter()
                        .map(|(n, t)| format!("Text({} + \": \" + {})", kstr(n), display(&format!("c.{}", bt(n)), t)))
                        .collect();
                    items.push(format!("cur?.let {{ c -> item {{ Column {{ {} }} }} }}", rows.join("; ")));
                }
                Component::Query { knowledge, limit, columns } => {
                    let var = format!("query{ci}");
                    let rt = tn("Row_", knowledge);
                    state.push(format!("    var {var} by remember {{ mutableStateOf<List<{rt}>>(emptyList()) }}"));
                    loads.push(format!(
                        "{var} = api.getArray(listOf(\"api\", \"queries\", {}), mapOf(\"limit\" to \"{limit}\")).map {{ parse{rt}(it) }}",
                        kstr(knowledge)
                    ));
                    let texts: Vec<String> = columns
                        .iter()
                        .map(|(n, t)| format!("Text({})", display(&format!("row.{}", bt(n)), t)))
                        .collect();
                    items.push(format!(
                        "item {{ Text({}, style = MaterialTheme.typography.titleMedium) }}",
                        kstr(&format!("{knowledge}(上位{limit}件)"))
                    ));
                    items.push(format!("items({var}) {{ row -> Row(Modifier.fillMaxWidth().padding(vertical = 8.dp), horizontalArrangement = Arrangement.SpaceBetween) {{ {} }} }}", texts.join("; ")));
                }
                Component::Button { label, action, args } => {
                    let Some(a) = ir.actions.iter().find(|x| &x.name == action) else { continue };
                    let pairs: Vec<String> = a
                        .params
                        .iter()
                        .zip(args)
                        .map(|((n, _), e)| format!("{} to {}", kstr(n), arg_kt(e, s.input.as_deref(), &mut errs)))
                        .collect();
                    let pre = if s.input.is_some() { "val c = cur ?: return@launch; " } else { "" };
                    items.push(format!(
                        "item {{ Button(onClick = {{ scope.launch {{ {pre}act {{ api.post({}, mapOf({})) }} }} }}) {{ Text({}) }} }}",
                        kstr(action),
                        pairs.join(", "),
                        kstr(label)
                    ));
                }
                Component::Form { action, params } => {
                    let mut inputs = Vec::new();
                    let mut pairs = Vec::new();
                    for (n, t) in params {
                        let sv = tn(&format!("f{ci}_"), n);
                        let base = if let Type::Option(i) = t { i.as_ref() } else { t };
                        match base {
                            Type::Bool => {
                                state.push(format!("    var {sv} by remember {{ mutableStateOf(false) }}"));
                                inputs.push(format!(
                                    "Row {{ Checkbox(checked = {sv}, onCheckedChange = {{ {sv} = it }}); Text({}) }}",
                                    kstr(n)
                                ));
                                pairs.push(format!("{} to {sv}", kstr(n)));
                            }
                            Type::Enum(e) => {
                                state.push(format!("    var {sv} by remember {{ mutableStateOf({}.first()) }}", tn("ENUM_", e)));
                                inputs.push(format!("KotonohaUi.EnumPicker({}, {}, {sv}) {{ {sv} = it }}", kstr(n), tn("ENUM_", e)));
                                pairs.push(format!("{} to {sv}", kstr(n)));
                            }
                            Type::Int | Type::Money(_) => {
                                state.push(format!("    var {sv} by remember {{ mutableStateOf(\"\") }}"));
                                inputs.push(format!("OutlinedTextField(value = {sv}, onValueChange = {{ {sv} = it }}, label = {{ Text({}) }}, keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number))", kstr(n)));
                                pairs.push(format!("{} to Kotonoha.parseLong({sv}, {})", kstr(n), kstr(n)));
                            }
                            Type::Str | Type::Id | Type::Url | Type::Email => {
                                state.push(format!("    var {sv} by remember {{ mutableStateOf(\"\") }}"));
                                inputs.push(format!(
                                    "OutlinedTextField(value = {sv}, onValueChange = {{ {sv} = it }}, label = {{ Text({}) }})",
                                    kstr(n)
                                ));
                                pairs.push(format!("{} to {sv}", kstr(n)));
                            }
                            other => errs.push(Diagnostic::error(
                                Code::TGT001,
                                Default::default(),
                                format!("Android のフォーム入力は型 {other} に未対応です"),
                            )),
                        }
                    }
                    items.push(format!(
                        "item {{ Column {{ Text({}, style = MaterialTheme.typography.titleMedium); {}; Button(onClick = {{ scope.launch {{ act {{ api.post({}, mapOf({})) }} }} }}) {{ Text(\"送信\") }} }} }}",
                        kstr(action),
                        inputs.join("; "),
                        kstr(action),
                        pairs.join(", ")
                    ));
                }
            }
        }
        let params = if s.input.is_some() {
            "api: KotonohaApi, nav: NavController, itemId: String"
        } else {
            "api: KotonohaApi, nav: NavController"
        };
        v.push_str(&format!(
            "@Composable\nfun Screen{si}({params}) {{\n    val scope = rememberCoroutineScope()\n    var message by remember {{ mutableStateOf<String?>(null) }}\n{}\n    suspend fun load() {{\n        try {{\n            {}\n        }} catch (e: Exception) {{ message = Kotonoha.describe(e) }}\n    }}\n    suspend fun act(op: suspend () -> Unit) {{\n        try {{ op(); message = \"完了しました\"; load() }} catch (e: Exception) {{ message = Kotonoha.describe(e) }}\n    }}\n    LaunchedEffect(Unit) {{ load() }}\n    KotonohaUi.ScreenScaffold({}, nav) {{\n        LazyColumn(Modifier.fillMaxSize().padding(16.dp)) {{\n            {}\n            message?.let {{ m -> item {{ Text(m, color = MaterialTheme.colorScheme.secondary) }} }}\n        }}\n    }}\n}}\n\n",
            state.join("\n"),
            if loads.is_empty() { "Unit".to_string() } else { loads.join("\n            ") },
            kstr(&s.name),
            items.join("\n            ")
        ));
    }
    // ナビゲーション
    let mut routes = Vec::new();
    for (si, s) in ir.screens.iter().enumerate() {
        if s.input.is_some() {
            routes.push(format!(
                "composable(\"screen{si}/{{id}}\") {{ e -> Screen{si}(api, nav, Kotonoha.dec(e.arguments?.getString(\"id\") ?: \"\")) }}"
            ));
        } else {
            routes.push(format!("composable(\"screen{si}\") {{ Screen{si}(api, nav) }}"));
        }
    }
    let menu: Vec<String> = ir
        .screens
        .iter()
        .enumerate()
        .filter(|(_, s)| s.input.is_none())
        .map(|(i, s)| format!("{} to \"screen{i}\"", kstr(&s.name)))
        .collect();
    let main = format!(
        "// 生成物(koto build --target android)。手で編集しないでください。\npackage {PKG}\n\nimport android.os.Bundle\nimport androidx.activity.ComponentActivity\nimport androidx.activity.compose.setContent\nimport androidx.compose.material3.MaterialTheme\nimport androidx.navigation.compose.NavHost\nimport androidx.navigation.compose.composable\nimport androidx.navigation.compose.rememberNavController\n\nclass MainActivity : ComponentActivity() {{\n    override fun onCreate(savedInstanceState: Bundle?) {{\n        super.onCreate(savedInstanceState)\n        val api = KotonohaApi(applicationContext, BuildConfig.KOTONOHA_API_BASE)\n        setContent {{\n            MaterialTheme {{\n                val nav = rememberNavController()\n                NavHost(navController = nav, startDestination = \"menu\") {{\n                    composable(\"menu\") {{ KotonohaUi.Menu({}, listOf({}), nav) }}\n                    composable(\"login\") {{ KotonohaUi.Login(api, nav) }}\n                    {}\n                }}\n            }}\n        }}\n    }}\n}}\n",
        kstr(&ir.name),
        menu.join(", "),
        routes.join("\n                    ")
    );
    if !errs.is_empty() {
        return Err(errs);
    }
    let settings = "pluginManagement {\n    repositories { google(); mavenCentral(); gradlePluginPortal() }\n}\ndependencyResolutionManagement {\n    repositories { google(); mavenCentral() }\n}\nrootProject.name = \"KotonohaApp\"\ninclude(\":app\")\n";
    let root_build = "plugins {\n    id(\"com.android.application\") version \"8.7.3\" apply false\n    id(\"org.jetbrains.kotlin.android\") version \"2.0.21\" apply false\n    id(\"org.jetbrains.kotlin.plugin.compose\") version \"2.0.21\" apply false\n}\n";
    let app_build = format!(
        "plugins {{\n    id(\"com.android.application\")\n    id(\"org.jetbrains.kotlin.android\")\n    id(\"org.jetbrains.kotlin.plugin.compose\")\n}}\n\nandroid {{\n    namespace = \"{PKG}\"\n    compileSdk = 35\n    defaultConfig {{\n        applicationId = \"{PKG}\"\n        minSdk = 26   // ADR-005 未決定(案)\n        targetSdk = 35\n        versionCode = 1\n        versionName = \"0.1.0\"\n        buildConfigField(\"String\", \"KOTONOHA_API_BASE\", \"\\\"http://10.0.2.2:8787\\\"\")\n    }}\n    buildFeatures {{ compose = true; buildConfig = true }}\n    compileOptions {{ sourceCompatibility = JavaVersion.VERSION_17; targetCompatibility = JavaVersion.VERSION_17 }}\n    kotlinOptions {{ jvmTarget = \"17\" }}\n}}\n\ndependencies {{\n    implementation(platform(\"androidx.compose:compose-bom:2024.10.01\"))\n    implementation(\"androidx.compose.material3:material3\")\n    implementation(\"androidx.compose.ui:ui\")\n    implementation(\"androidx.activity:activity-compose:1.9.3\")\n    implementation(\"androidx.navigation:navigation-compose:2.8.4\")\n    implementation(\"org.jetbrains.kotlinx:kotlinx-coroutines-android:1.9.0\")\n}}\n"
    );
    let manifest = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<manifest xmlns:android=\"http://schemas.android.com/apk/res/android\">\n    <uses-permission android:name=\"android.permission.INTERNET\" />\n    <application android:label=\"KOTONOHA\" android:networkSecurityConfig=\"@xml/network_security_config\" android:allowBackup=\"false\">\n        <activity android:name=\".MainActivity\" android:exported=\"true\">\n            <intent-filter>\n                <action android:name=\"android.intent.action.MAIN\" />\n                <category android:name=\"android.intent.category.LAUNCHER\" />\n            </intent-filter>\n        </activity>\n    </application>\n</manifest>\n";
    let nsc = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!-- 平文 HTTP はエミュレータからホストの開発サーバー(10.0.2.2)への接続だけ許可する -->\n<network-security-config>\n    <base-config cleartextTrafficPermitted=\"false\" />\n    <domain-config cleartextTrafficPermitted=\"true\">\n        <domain includeSubdomains=\"false\">10.0.2.2</domain>\n    </domain-config>\n</network-security-config>\n";
    let src = "app/src/main/java/app/kotonoha/generated";
    Ok(vec![
        GeneratedFile::text("settings.gradle.kts", settings.into()),
        GeneratedFile::text(
            "sbom.cdx.json",
            crate::sbom(
                &ir.name,
                "android",
                &[
                    ("maven", "androidx.compose:compose-bom", "2024.10.01"),
                    ("maven", "androidx.activity:activity-compose", "1.9.3"),
                    ("maven", "androidx.navigation:navigation-compose", "2.8.4"),
                    ("maven", "org.jetbrains.kotlinx:kotlinx-coroutines-android", "1.9.0"),
                ],
            ),
        ),
        GeneratedFile::text("build.gradle.kts", root_build.into()),
        GeneratedFile::text("gradle.properties", "android.useAndroidX=true\nkotlin.code.style=official\n".into()),
        GeneratedFile::text("app/build.gradle.kts", app_build),
        GeneratedFile::text("app/src/main/AndroidManifest.xml", manifest.into()),
        GeneratedFile::text("app/src/main/res/xml/network_security_config.xml", nsc.into()),
        GeneratedFile::text("api.json", serde_json::to_string_pretty(&api_contract(ir)).unwrap() + "\n"),
        GeneratedFile::text(&format!("{src}/Models.generated.kt"), m),
        GeneratedFile::text(&format!("{src}/Screens.generated.kt"), v),
        GeneratedFile::text(&format!("{src}/MainActivity.kt"), main),
        GeneratedFile::text(&format!("{src}/KotonohaApi.kt"), RUNTIME_API_KT.to_string()),
        GeneratedFile::text(&format!("{src}/KotonohaUi.kt"), RUNTIME_UI_KT.to_string()),
        GeneratedFile::text(
            "README.md",
            format!("# {} Android(KOTONOHA 生成物)\n\nKotlin / Jetpack Compose / minSdk 26(案)。Android Studio で開いてビルド。API の接続先は app/build.gradle.kts の KOTONOHA_API_BASE。\n\n**注意**: 生成環境に Android SDK が無く、ビルドは未検証です。\n", ir.name),
        ),
    ])
}
