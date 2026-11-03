// 生成物(koto build --target ios)。手で編集しないでください。
import Foundation

enum Enum_注文失敗: String, Codable, CaseIterable, Hashable {
    case `数量不正` = "数量不正"
    case `在庫不足` = "在庫不足"
}

enum Enum_注文状態: String, Codable, CaseIterable, Hashable {
    case `確定` = "確定"
    case `取消` = "取消"
}

enum Enum_登録失敗: String, Codable, CaseIterable, Hashable {
    case `在庫不正` = "在庫不正"
}

struct Entity_商品: Codable, Hashable {
    let `ID`: String
    let `名前`: String
    let `価格`: Int64
    let `在庫`: Int64
}

struct Entity_注文: Codable, Hashable {
    let `ID`: String
    let `商品ID`: String
    let `数量`: Int64
    let `注文者`: String
}

struct Entity_注文明細: Codable, Hashable {
    let `ID`: String
    let `商品ID`: String
    let `売上金額`: Int64
    let `状態`: Enum_注文状態
    let `確定日時`: Date
}

struct Row_売れ筋商品: Codable, Hashable {
    let `商品ID`: String
    let `売上金額`: Int64
}

struct Req_商品登録: Encodable {
    let `名前`: String
    let `価格`: Int64
    let `在庫`: Int64
}

struct Req_注文する: Encodable {
    let `商品ID`: String
    let `数量`: Int64
}

