//! iOS 生成(SwiftUI / iOS 16+ 案: ADR-005 未決定)。docs/architecture.md §7。
//! 共通 API 契約だけを使うクライアント。この環境に Swift ツールチェーンが無いため**コンパイル未検証**(K-01)。

use crate::appir::{api_contract, check_ir, AppIr, Component, EndpointKind};
use crate::GeneratedFile;
use kotonoha_ast::{BinOp, Expr, ExprKind};
use kotonoha_diag::{Code, Diagnostic};
use kotonoha_types::Type;

pub const RUNTIME_SWIFT: &str = include_str!("../../../runtime/ios/KotonohaRuntime.swift");

fn bt(n: &str) -> String {
    format!("`{n}`")
}

fn swift_str(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            c if c.is_control() => o.push_str(&format!("\\u{{{:X}}}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn type_name(prefix: &str, n: &str) -> String {
    crate::ts::mangle(prefix, n)
}

fn swift_type(t: &Type) -> String {
    match t {
        Type::Bool => "Bool".into(),
        Type::Int | Type::Money(_) => "Int64".into(),
        Type::Str | Type::Id | Type::Url | Type::Email => "String".into(),
        Type::DateTime => "Date".into(),
        Type::Enum(e) => type_name("Enum_", e),
        Type::Data(d) => type_name("Entity_", d),
        Type::Option(i) => format!("{}?", swift_type(i)),
        Type::List(i) | Type::Array(i) => format!("[{}]", swift_type(i)),
        _ => "Never".into(),
    }
}

fn display_expr(v: &str, t: &Type) -> String {
    match t {
        Type::Money(c) if c == "JPY" => format!("Kotonoha.yen({v})"),
        Type::Int => format!("Kotonoha.int({v})"),
        Type::Bool => format!("Kotonoha.bool({v})"),
        Type::DateTime => format!("Kotonoha.datetime({v})"),
        Type::Enum(_) => format!("{v}.rawValue"),
        Type::Option(i) => format!("({v}.map {{ {} }} ?? \"—\")", display_expr("$0", i)),
        _ => format!("String(describing: {v})"),
    }
}

fn arg_swift(e: &Expr, input: Option<&str>, errs: &mut Vec<Diagnostic>) -> String {
    match &e.kind {
        ExprKind::Int(v) => format!("Int64({v})"),
        ExprKind::Str(s) => swift_str(s),
        ExprKind::Bool(b) => b.to_string(),
        ExprKind::None => "nil".into(),
        ExprKind::Ident(n) if Some(n.as_str()) == input => "cur".into(),
        ExprKind::Field(b, f) => format!("{}.{}", arg_swift(b, input, errs), bt(&f.name)),
        ExprKind::Binary {
            op: op @ (BinOp::Add | BinOp::Sub | BinOp::Mul),
            lhs,
            rhs,
        } => {
            format!("({} {} {})", arg_swift(lhs, input, errs), op.symbol(), arg_swift(rhs, input, errs))
        }
        _ => {
            errs.push(Diagnostic::error(Code::TGT001, e.span, "この画面引数式は iOS 生成に未対応です"));
            "nil".into()
        }
    }
}

pub fn generate(ir: &AppIr) -> Result<Vec<GeneratedFile>, Vec<Diagnostic>> {
    let mut errs = check_ir(ir);
    if !errs.is_empty() {
        return Err(errs);
    }
    let mut m = String::from("// 生成物(koto build --target ios)。手で編集しないでください。\nimport Foundation\n\n");
    for (e, vs) in &ir.enums {
        m.push_str(&format!("enum {}: String, Codable, CaseIterable, Hashable {{\n", type_name("Enum_", e)));
        for v in vs {
            m.push_str(&format!("    case {} = {}\n", bt(v), swift_str(v)));
        }
        m.push_str("}\n\n");
    }
    for e in &ir.entities {
        m.push_str(&format!("struct {}: Codable, Hashable {{\n", type_name("Entity_", &e.name)));
        for (n, t) in &e.fields {
            m.push_str(&format!("    let {}: {}\n", bt(n), swift_type(t)));
        }
        m.push_str("}\n\n");
    }
    for (spec, _) in &ir.queries {
        let Type::Record(cols) = spec.row_type() else { continue };
        m.push_str(&format!("struct {}: Codable, Hashable {{\n", type_name("Row_", &spec.name)));
        for (n, t) in &cols {
            m.push_str(&format!("    let {}: {}\n", bt(n), swift_type(t)));
        }
        m.push_str("}\n\n");
    }
    for a in &ir.actions {
        m.push_str(&format!("struct {}: Encodable {{\n", type_name("Req_", &a.name)));
        for (n, t) in &a.params {
            m.push_str(&format!("    let {}: {}\n", bt(n), swift_type(t)));
        }
        m.push_str("}\n\n");
    }

    // 画面
    let mut v = String::from("// 生成物(koto build --target ios)。手で編集しないでください。\nimport SwiftUI\n\n");
    for s in &ir.screens {
        let sname = type_name("Screen_", &s.name);
        let mut state = Vec::new();
        let mut loads = Vec::new();
        let mut body = Vec::new();
        let mut props = String::new();
        let mut init = String::new();
        if let Some(d) = &s.input {
            props.push_str("    let itemID: String\n");
            init = "itemID: String".into();
            state.push(format!("    @State private var cur: {}? = nil", type_name("Entity_", d)));
            loads.push(format!("cur = try await api.get(\"api\", \"data\", {}, id: itemID)", swift_str(d)));
        }
        let _ = init;
        for (ci, c) in s.components.iter().enumerate() {
            match c {
                Component::List { data, fields, link_to } => {
                    let var = format!("list{ci}");
                    state.push(format!("    @State private var {var}: [{}] = []", type_name("Entity_", data)));
                    loads.push(format!("{var} = try await api.get(\"api\", \"data\", {})", swift_str(data)));
                    let cells: Vec<String> = fields
                        .iter()
                        .map(|(n, t)| format!("Text({}).font(.body)", display_expr(&format!("row.{}", bt(n)), t)))
                        .collect();
                    let row = format!("VStack(alignment: .leading) {{ {} }}", cells.join("; "));
                    let item = match link_to {
                        Some(t) => format!("NavigationLink {{ {}(itemID: row.`ID`) }} label: {{ {row} }}", type_name("Screen_", t)),
                        None => row,
                    };
                    body.push(format!(
                        "Section({}) {{ ForEach({var}, id: \\.self) {{ row in {item} }} }}",
                        swift_str(&format!("{data}の一覧"))
                    ));
                }
                Component::Detail { fields, .. } => {
                    let rows: Vec<String> = fields
                        .iter()
                        .map(|(n, t)| format!("LabeledContent({}, value: {})", swift_str(n), display_expr(&format!("cur.{}", bt(n)), t)))
                        .collect();
                    body.push(format!("if let cur {{ Section {{ {} }} }}", rows.join("; ")));
                }
                Component::Query { knowledge, limit, columns } => {
                    let var = format!("query{ci}");
                    state.push(format!("    @State private var {var}: [{}] = []", type_name("Row_", knowledge)));
                    loads.push(format!(
                        "{var} = try await api.get(\"api\", \"queries\", {}, query: [\"limit\": \"{limit}\"])",
                        swift_str(knowledge)
                    ));
                    let cells: Vec<String> = columns
                        .iter()
                        .map(|(n, t)| format!("Text({})", display_expr(&format!("row.{}", bt(n)), t)))
                        .collect();
                    body.push(format!(
                        "Section({}) {{ ForEach({var}, id: \\.self) {{ row in HStack {{ {} }} }} }}",
                        swift_str(&format!("{knowledge}(上位{limit}件)")),
                        cells.join("; Spacer(); ")
                    ));
                }
                Component::Button { label, action, args } => {
                    let Some(a) = ir.actions.iter().find(|x| &x.name == action) else { continue };
                    let fields: Vec<String> = a
                        .params
                        .iter()
                        .zip(args)
                        .map(|((n, _), e)| format!("{}: {}", bt(n), arg_swift(e, s.input.as_deref(), &mut errs)))
                        .collect();
                    let guard = if s.input.is_some() { "guard let cur else { return }; " } else { "" };
                    body.push(format!(
                        "Section {{ Button({}) {{ Task {{ {guard}await run {{ _ = try await api.post({}, body: {}({})) }} }} }} }}",
                        swift_str(label),
                        swift_str(action),
                        type_name("Req_", action),
                        fields.join(", ")
                    ));
                }
                Component::Form { action, params } => {
                    let mut submit_fields = Vec::new();
                    let mut inputs = Vec::new();
                    for (n, t) in params {
                        let sv = format!("f{ci}_{}", crate::ts::mangle("", n));
                        let base = if let Type::Option(i) = t { i.as_ref() } else { t };
                        match base {
                            Type::Bool => {
                                state.push(format!("    @State private var {sv} = false"));
                                inputs.push(format!("Toggle({}, isOn: ${sv})", swift_str(n)));
                                submit_fields.push(format!("{}: {sv}", bt(n)));
                            }
                            Type::Enum(e) => {
                                let et = type_name("Enum_", e);
                                state.push(format!("    @State private var {sv}: {et} = {et}.allCases[0]"));
                                inputs.push(format!(
                                    "Picker({}, selection: ${sv}) {{ ForEach({et}.allCases, id: \\.self) {{ Text($0.rawValue).tag($0) }} }}",
                                    swift_str(n)
                                ));
                                submit_fields.push(format!("{}: {sv}", bt(n)));
                            }
                            Type::Int | Type::Money(_) => {
                                state.push(format!("    @State private var {sv} = \"\""));
                                inputs.push(format!("TextField({}, text: ${sv}).keyboardType(.numberPad)", swift_str(n)));
                                submit_fields.push(format!("{}: try Kotonoha.parseInt({sv}, field: {})", bt(n), swift_str(n)));
                            }
                            Type::Str | Type::Id | Type::Url | Type::Email => {
                                state.push(format!("    @State private var {sv} = \"\""));
                                inputs.push(format!("TextField({}, text: ${sv})", swift_str(n)));
                                submit_fields.push(format!("{}: {sv}", bt(n)));
                            }
                            other => errs.push(Diagnostic::error(
                                Code::TGT001,
                                a_span(ir, action),
                                format!("iOS のフォーム入力は型 {other} に未対応です"),
                            )),
                        }
                    }
                    body.push(format!(
                        "Section({}) {{ {}; Button(\"送信\") {{ Task {{ await run {{ _ = try await api.post({}, body: {}({})) }} }} }} }}",
                        swift_str(action),
                        inputs.join("; "),
                        swift_str(action),
                        type_name("Req_", action),
                        submit_fields.join(", ")
                    ));
                }
            }
        }
        v.push_str(&format!(
            "struct {sname}: View {{\n    @EnvironmentObject var api: KotonohaAPI\n{props}{}\n    @State private var message: String? = nil\n\n    var body: some View {{\n        List {{\n            {}\n            if let message {{ Section {{ Text(message).accessibilityAddTraits(.updatesFrequently) }} }}\n        }}\n        .navigationTitle({})\n        .task {{ await load() }}\n        .refreshable {{ await load() }}\n    }}\n\n    private func load() async {{\n        do {{\n            {}\n        }} catch {{ message = Kotonoha.describe(error) }}\n    }}\n\n    private func run(_ op: () async throws -> Void) async {{\n        do {{ try await op(); message = \"完了しました\"; await load() }} catch {{ message = Kotonoha.describe(error) }}\n    }}\n}}\n\n",
            state.join("\n"),
            if body.is_empty() { "EmptyView()".to_string() } else { body.join("\n            ") },
            swift_str(&s.name),
            if loads.is_empty() { "_ = 0".to_string() } else { loads.join("\n            ") },
        ));
    }
    let menu: Vec<String> = ir
        .screens
        .iter()
        .filter(|s| s.input.is_none())
        .map(|s| format!("NavigationLink({}) {{ {}() }}", swift_str(&s.name), type_name("Screen_", &s.name)))
        .collect();
    v.push_str(&format!(
        "struct RootView: View {{\n    var body: some View {{\n        NavigationStack {{\n            List {{\n                {}\n                NavigationLink(\"ログイン\") {{ LoginView() }}\n            }}\n            .navigationTitle({})\n        }}\n    }}\n}}\n\n@main\nstruct GeneratedApp: App {{\n    @StateObject private var api = KotonohaAPI()\n    var body: some Scene {{ WindowGroup {{ RootView().environmentObject(api) }} }}\n}}\n",
        menu.join("\n                "),
        swift_str(&ir.name)
    ));

    let _ = ir.endpoints.iter().filter(|e| matches!(e.kind, EndpointKind::Action { .. })).count();
    if !errs.is_empty() {
        return Err(errs);
    }
    let project = format!(
        "# XcodeGen 定義(xcodegen generate で .xcodeproj を作る)\nname: KotonohaApp\noptions:\n  bundleIdPrefix: app.kotonoha.generated\n  deploymentTarget:\n    iOS: \"16.0\"   # ADR-005 未決定(案)\ntargets:\n  KotonohaApp:\n    type: application\n    platform: iOS\n    sources: [Sources]\n    info:\n      path: Sources/Info.plist\n      properties:\n        CFBundleDisplayName: {}\n        KotonohaAPIBase: http://127.0.0.1:8787\n        UILaunchScreen: {{}}\n",
        swift_str(&ir.name)
    );
    Ok(vec![
        GeneratedFile::text("project.yml", project),
        GeneratedFile::text("api.json", serde_json::to_string_pretty(&api_contract(ir)).unwrap() + "\n"),
        GeneratedFile::text("Sources/Models.generated.swift", m),
        GeneratedFile::text("Sources/Screens.generated.swift", v),
        GeneratedFile::text("Sources/Runtime/KotonohaRuntime.swift", RUNTIME_SWIFT.to_string()),
        GeneratedFile::text(
            "README.md",
            format!("# {} iOS(KOTONOHA 生成物)\n\nSwiftUI / iOS 16+(案)。`xcodegen generate` → Xcode でビルド。API の接続先は Info.plist の KotonohaAPIBase。\n\n**注意**: 生成環境に Swift ツールチェーンが無く、コンパイルは未検証です。\n", ir.name),
        ),
    ])
}

fn a_span(ir: &AppIr, action: &str) -> kotonoha_diag::Span {
    ir.actions.iter().find(|a| a.name == action).map(|a| a.span).unwrap_or_default()
}
