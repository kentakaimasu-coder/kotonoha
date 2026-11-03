// 生成物(koto build --target android)。手で編集しないでください。
package app.kotonoha.generated

import org.json.JSONObject

val ENUM__u6CE8_u6587_u5931_u6557: List<String> = listOf("数量不正", "在庫不足")

val ENUM__u6CE8_u6587_u72B6_u614B: List<String> = listOf("確定", "取消")

val ENUM__u767B_u9332_u5931_u6557: List<String> = listOf("在庫不正")

data class Entity__u5546_u54C1(
    val `ID`: String,
    val `名前`: String,
    val `価格`: Long,
    val `在庫`: Long,
)

fun parseEntity__u5546_u54C1(o: JSONObject): Entity__u5546_u54C1 = Entity__u5546_u54C1(
    `ID` = o.getString("ID"),
    `名前` = o.getString("名前"),
    `価格` = o.getLong("価格"),
    `在庫` = o.getLong("在庫"),
)

data class Entity__u6CE8_u6587(
    val `ID`: String,
    val `商品ID`: String,
    val `数量`: Long,
    val `注文者`: String,
)

fun parseEntity__u6CE8_u6587(o: JSONObject): Entity__u6CE8_u6587 = Entity__u6CE8_u6587(
    `ID` = o.getString("ID"),
    `商品ID` = o.getString("商品ID"),
    `数量` = o.getLong("数量"),
    `注文者` = o.getString("注文者"),
)

data class Entity__u6CE8_u6587_u660E_u7D30(
    val `ID`: String,
    val `商品ID`: String,
    val `売上金額`: Long,
    val `状態`: String,
    val `確定日時`: String,
)

fun parseEntity__u6CE8_u6587_u660E_u7D30(o: JSONObject): Entity__u6CE8_u6587_u660E_u7D30 = Entity__u6CE8_u6587_u660E_u7D30(
    `ID` = o.getString("ID"),
    `商品ID` = o.getString("商品ID"),
    `売上金額` = o.getLong("売上金額"),
    `状態` = o.getString("状態"),
    `確定日時` = o.getString("確定日時"),
)

data class Row__u58F2_u308C_u7B4B_u5546_u54C1(
    val `商品ID`: String,
    val `売上金額`: Long,
)

fun parseRow__u58F2_u308C_u7B4B_u5546_u54C1(o: JSONObject): Row__u58F2_u308C_u7B4B_u5546_u54C1 = Row__u58F2_u308C_u7B4B_u5546_u54C1(
    `商品ID` = o.getString("商品ID"),
    `売上金額` = o.getLong("売上金額"),
)

