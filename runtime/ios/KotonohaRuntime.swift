// KOTONOHA iOS runtime: API クライアント・トークン保管(Keychain)・表示形式
// 共通 API 契約(api.json)のワイヤ形式に従う。認可は backend が強制する。
import Foundation
import Security
import SwiftUI

enum KotonohaError: Error {
    case http(status: Int, message: String)
    case failure(String)
    case input(String)
}

struct OkResponse<T: Decodable>: Decodable { let ok: T }
struct ErrorBody: Decodable { let error: String; let message: String?; let value: String? }
struct Unit: Decodable {}

enum Keychain {
    static let service = "app.kotonoha.token"
    static func load() -> String? {
        let q: [String: Any] = [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: service, kSecReturnData as String: true]
        var out: AnyObject?
        guard SecItemCopyMatching(q as CFDictionary, &out) == errSecSuccess, let d = out as? Data else { return nil }
        return String(data: d, encoding: .utf8)
    }
    static func save(_ token: String?) {
        let base: [String: Any] = [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: service]
        SecItemDelete(base as CFDictionary)
        guard let token, let d = token.data(using: .utf8) else { return }
        var add = base
        add[kSecValueData as String] = d
        add[kSecAttrAccessible as String] = kSecAttrAccessibleWhenUnlockedThisDeviceOnly
        SecItemAdd(add as CFDictionary, nil)
    }
}

@MainActor
final class KotonohaAPI: ObservableObject {
    let base: URL
    @Published var token: String? = Keychain.load()

    init() {
        let s = Bundle.main.object(forInfoDictionaryKey: "KotonohaAPIBase") as? String ?? "http://127.0.0.1:8787"
        base = URL(string: s)!
    }

    static let decoder: JSONDecoder = {
        let d = JSONDecoder()
        d.dateDecodingStrategy = .custom { dec in
            let s = try dec.singleValueContainer().decode(String.self)
            let f = ISO8601DateFormatter()
            f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
            if let t = f.date(from: s) { return t }
            f.formatOptions = [.withInternetDateTime]
            if let t = f.date(from: s) { return t }
            throw DecodingError.dataCorrupted(.init(codingPath: dec.codingPath, debugDescription: "日時が不正です"))
        }
        return d
    }()
    static let encoder: JSONEncoder = {
        let e = JSONEncoder()
        e.dateEncodingStrategy = .iso8601
        return e
    }()

    private func url(_ segments: [String], id: String?, query: [String: String]) -> URL {
        var u = base
        for s in segments { u.appendPathComponent(s) }
        if let id { u.appendPathComponent(id) }
        if query.isEmpty { return u }
        var c = URLComponents(url: u, resolvingAgainstBaseURL: false)!
        c.queryItems = query.sorted { $0.key < $1.key }.map { URLQueryItem(name: $0.key, value: $0.value) }
        return c.url!
    }

    private func send(_ req: URLRequest) async throws -> Data {
        var r = req
        if let token { r.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization") }
        let (data, resp) = try await URLSession.shared.data(for: r)
        let status = (resp as? HTTPURLResponse)?.statusCode ?? 0
        if (200..<300).contains(status) { return data }
        let body = try? KotonohaAPI.decoder.decode(ErrorBody.self, from: data)
        if status == 422, body?.error == "failure" { throw KotonohaError.failure(body?.value ?? "") }
        throw KotonohaError.http(status: status, message: body?.message ?? "エラー(\(status))")
    }

    func get<T: Decodable>(_ segments: String..., id: String? = nil, query: [String: String] = [:]) async throws -> T {
        let data = try await send(URLRequest(url: url(segments, id: id, query: query)))
        return try KotonohaAPI.decoder.decode(T.self, from: data)
    }

    @discardableResult
    func post<B: Encodable>(_ action: String, body: B) async throws -> Data {
        var r = URLRequest(url: url(["api", "actions", action], id: nil, query: [:]))
        r.httpMethod = "POST"
        r.setValue("application/json", forHTTPHeaderField: "Content-Type")
        r.httpBody = try KotonohaAPI.encoder.encode(body)
        return try await send(r)
    }
}

enum Kotonoha {
    static func yen(_ v: Int64) -> String {
        let f = NumberFormatter()
        f.numberStyle = .decimal
        f.locale = Locale(identifier: "ja_JP")
        return "¥" + (f.string(from: NSNumber(value: v)) ?? String(v))
    }
    static func int(_ v: Int64) -> String { String(v) }
    static func bool(_ v: Bool) -> String { v ? "はい" : "いいえ" }
    static func datetime(_ v: Date) -> String { v.formatted(date: .abbreviated, time: .shortened) }
    /// 全角数字・全角マイナス・カンマ・空白を受け付ける(NR-01)
    static func parseInt(_ s: String, field: String) throws -> Int64 {
        var t = s.applyingTransform(.fullwidthToHalfwidth, reverse: false) ?? s
        for m in ["\u{2212}", "\u{30FC}", "\u{FF0D}"] { t = t.replacingOccurrences(of: m, with: "-") }
        t = t.components(separatedBy: CharacterSet(charactersIn: ",、 \u{3000}")).joined()
        guard let v = Int64(t) else { throw KotonohaError.input("\(field) は整数で入力してください") }
        return v
    }
    static func describe(_ e: Error) -> String {
        switch e {
        case KotonohaError.http(401, _): return "ログインが必要です"
        case KotonohaError.http(403, _): return "権限がありません"
        case KotonohaError.http(_, let m): return m
        case KotonohaError.failure(let v): return "失敗しました: \(v)"
        case KotonohaError.input(let m): return m
        default: return e.localizedDescription
        }
    }
}

struct LoginView: View {
    @EnvironmentObject var api: KotonohaAPI
    @State private var token = ""
    var body: some View {
        Form {
            Section("トークン(認証基盤から発行: ADR-002)") { SecureField("トークン", text: $token) }
            Button("ログイン") { Keychain.save(token); api.token = token }
            Button("ログアウト", role: .destructive) { Keychain.save(nil); api.token = nil }
        }
        .navigationTitle("ログイン")
    }
}
