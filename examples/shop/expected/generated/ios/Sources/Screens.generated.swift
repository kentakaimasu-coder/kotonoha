// 生成物(koto build --target ios)。手で編集しないでください。
import SwiftUI

struct Screen_商品一覧: View {
    @EnvironmentObject var api: KotonohaAPI
    @State private var list0: [Entity_商品] = []
    @State private var message: String? = nil

    var body: some View {
        List {
            Section("商品の一覧") { ForEach(list0, id: \.self) { row in NavigationLink { Screen_商品詳細(itemID: row.`ID`) } label: { VStack(alignment: .leading) { Text(String(describing: row.`名前`)).font(.body); Text(Kotonoha.yen(row.`価格`)).font(.body); Text(Kotonoha.int(row.`在庫`)).font(.body) } } } }
            if let message { Section { Text(message).accessibilityAddTraits(.updatesFrequently) } }
        }
        .navigationTitle("商品一覧")
        .task { await load() }
        .refreshable { await load() }
    }

    private func load() async {
        do {
            list0 = try await api.get("api", "data", "商品")
        } catch { message = Kotonoha.describe(error) }
    }

    private func run(_ op: () async throws -> Void) async {
        do { try await op(); message = "完了しました"; await load() } catch { message = Kotonoha.describe(error) }
    }
}

struct Screen_商品詳細: View {
    @EnvironmentObject var api: KotonohaAPI
    let itemID: String
    @State private var cur: Entity_商品? = nil
    @State private var message: String? = nil

    var body: some View {
        List {
            if let cur { Section { LabeledContent("名前", value: String(describing: cur.`名前`)); LabeledContent("価格", value: Kotonoha.yen(cur.`価格`)); LabeledContent("在庫", value: Kotonoha.int(cur.`在庫`)) } }
            Section { Button("購入") { Task { guard let cur else { return }; await run { _ = try await api.post("注文する", body: Req_注文する(`商品ID`: cur.`ID`, `数量`: Int64(1))) } } } }
            if let message { Section { Text(message).accessibilityAddTraits(.updatesFrequently) } }
        }
        .navigationTitle("商品詳細")
        .task { await load() }
        .refreshable { await load() }
    }

    private func load() async {
        do {
            cur = try await api.get("api", "data", "商品", id: itemID)
        } catch { message = Kotonoha.describe(error) }
    }

    private func run(_ op: () async throws -> Void) async {
        do { try await op(); message = "完了しました"; await load() } catch { message = Kotonoha.describe(error) }
    }
}

struct Screen_商品管理: View {
    @EnvironmentObject var api: KotonohaAPI
    @State private var f0_名前 = ""
    @State private var f0_価格 = ""
    @State private var f0_在庫 = ""
    @State private var list1: [Entity_商品] = []
    @State private var message: String? = nil

    var body: some View {
        List {
            Section("商品登録") { TextField("名前", text: $f0_名前); TextField("価格", text: $f0_価格).keyboardType(.numberPad); TextField("在庫", text: $f0_在庫).keyboardType(.numberPad); Button("送信") { Task { await run { _ = try await api.post("商品登録", body: Req_商品登録(`名前`: f0_名前, `価格`: try Kotonoha.parseInt(f0_価格, field: "価格"), `在庫`: try Kotonoha.parseInt(f0_在庫, field: "在庫"))) } } } }
            Section("商品の一覧") { ForEach(list1, id: \.self) { row in NavigationLink { Screen_商品詳細(itemID: row.`ID`) } label: { VStack(alignment: .leading) { Text(String(describing: row.`名前`)).font(.body); Text(Kotonoha.yen(row.`価格`)).font(.body); Text(Kotonoha.int(row.`在庫`)).font(.body) } } } }
            if let message { Section { Text(message).accessibilityAddTraits(.updatesFrequently) } }
        }
        .navigationTitle("商品管理")
        .task { await load() }
        .refreshable { await load() }
    }

    private func load() async {
        do {
            list1 = try await api.get("api", "data", "商品")
        } catch { message = Kotonoha.describe(error) }
    }

    private func run(_ op: () async throws -> Void) async {
        do { try await op(); message = "完了しました"; await load() } catch { message = Kotonoha.describe(error) }
    }
}

struct Screen_注文履歴: View {
    @EnvironmentObject var api: KotonohaAPI
    @State private var list0: [Entity_注文] = []
    @State private var message: String? = nil

    var body: some View {
        List {
            Section("注文の一覧") { ForEach(list0, id: \.self) { row in VStack(alignment: .leading) { Text(String(describing: row.`商品ID`)).font(.body); Text(Kotonoha.int(row.`数量`)).font(.body); Text(String(describing: row.`注文者`)).font(.body) } } }
            if let message { Section { Text(message).accessibilityAddTraits(.updatesFrequently) } }
        }
        .navigationTitle("注文履歴")
        .task { await load() }
        .refreshable { await load() }
    }

    private func load() async {
        do {
            list0 = try await api.get("api", "data", "注文")
        } catch { message = Kotonoha.describe(error) }
    }

    private func run(_ op: () async throws -> Void) async {
        do { try await op(); message = "完了しました"; await load() } catch { message = Kotonoha.describe(error) }
    }
}

struct Screen_売れ筋: View {
    @EnvironmentObject var api: KotonohaAPI
    @State private var query0: [Row_売れ筋商品] = []
    @State private var message: String? = nil

    var body: some View {
        List {
            Section("売れ筋商品(上位20件)") { ForEach(query0, id: \.self) { row in HStack { Text(String(describing: row.`商品ID`)); Spacer(); Text(Kotonoha.yen(row.`売上金額`)) } } }
            if let message { Section { Text(message).accessibilityAddTraits(.updatesFrequently) } }
        }
        .navigationTitle("売れ筋")
        .task { await load() }
        .refreshable { await load() }
    }

    private func load() async {
        do {
            query0 = try await api.get("api", "queries", "売れ筋商品", query: ["limit": "20"])
        } catch { message = Kotonoha.describe(error) }
    }

    private func run(_ op: () async throws -> Void) async {
        do { try await op(); message = "完了しました"; await load() } catch { message = Kotonoha.describe(error) }
    }
}

struct RootView: View {
    var body: some View {
        NavigationStack {
            List {
                NavigationLink("商品一覧") { Screen_商品一覧() }
                NavigationLink("商品管理") { Screen_商品管理() }
                NavigationLink("注文履歴") { Screen_注文履歴() }
                NavigationLink("売れ筋") { Screen_売れ筋() }
                NavigationLink("ログイン") { LoginView() }
            }
            .navigationTitle("商店")
        }
    }
}

@main
struct GeneratedApp: App {
    @StateObject private var api = KotonohaAPI()
    var body: some Scene { WindowGroup { RootView().environmentObject(api) } }
}
