# MiRelay HTTP API v1

本文档定义上传端向中心服务器暂存文件，以及 Linux CLI 拉取并确认送达所需的最小协议。仓库中的 `mirelay-server` 是参考实现；`mirelay-upload` 是独立的 Linux 测试发送器，[Android 开发版](../android/README.md) 通过 JNI 复用同一个 Rust 上传核心。独立 Folder、权限凭证与配对握手见 [Folder pairing](folder-pairing.md)；完整多用户账户管理尚未实现。

## 基本约定

- 客户端在配置的 `base_url` 后追加 `/api/v1/deliveries`。`base_url` 可以带路径前缀，但不能包含查询参数、fragment 或用户凭据。
- 生产环境必须使用 HTTPS。CLI 只有在显式配置 `allow_insecure_http = true` 时才接受 HTTP，供可信本地开发使用。
- 每个受保护请求都发送 `Authorization: Bearer <token>`。令牌代表且唯一确定设备身份，服务端不能信任客户端另行声明的 `device_id`。
- 投递列表、下载和 ACK 请求发送 `mirelay-protocol-version: 1`；tus 请求按 tus 1.0 发送 `Tus-Resumable: 1.0.0`。
- 服务端必须确认被访问的投递属于当前令牌对应的设备，并避免记录 `Authorization` 请求头。
- JSON 使用 UTF-8；时间为 Unix 秒；SHA-256 为 64 位小写十六进制字符串。

服务端应在敏感响应中返回 `Cache-Control: no-store`。协议版本不受支持时建议返回 `426 Upgrade Required`。

## tus 可恢复上传

上传接口实现 [tus 1.0 Core Protocol 与 Creation extension](https://tus.io/protocols/resumable-upload)。当前不声明 concatenation、creation-with-upload、checksum、expiration 或 termination 扩展。

能力发现无需鉴权：

```http
OPTIONS /api/v1/uploads
```

响应包含：

```http
HTTP/1.1 204 No Content
Tus-Version: 1.0.0
Tus-Extension: creation
Tus-Max-Size: 104857600
```

创建上传时必须预先声明非零长度，并提供三个 base64 编码的元数据值：

```http
POST /api/v1/uploads
Authorization: Bearer <token>
Tus-Resumable: 1.0.0
Upload-Length: 1843920
Upload-Metadata: filename aWxsdXN0cmF0aW9uLnBuZw==,media_type aW1hZ2UvcG5n,sha256 MDEyMzQ1Njc4OWFiY2RlZg...
```

`filename` 解码后必须是单个 UTF-8 文件名，`media_type` 必须是下面列出的规范类型之一，`sha256` 必须是 64 位小写十六进制摘要。成功响应为 `201 Created`，`Location` 可为相对 URL；客户端必须持久化解析后的上传 URL 后再发送数据。

恢复时先读取服务端权威偏移：

```http
HEAD /api/v1/uploads/{upload_id}
Authorization: Bearer <token>
Tus-Resumable: 1.0.0
```

`200 OK` 响应包含 `Upload-Offset`、`Upload-Length` 和 `Upload-Metadata`。继续上传：

```http
PATCH /api/v1/uploads/{upload_id}
Authorization: Bearer <token>
Tus-Resumable: 1.0.0
Content-Type: application/offset+octet-stream
Upload-Offset: 4096

<raw bytes beginning at offset 4096>
```

服务端只在请求体完整缓存后修改上传，成功返回 `204 No Content` 和新 `Upload-Offset`。偏移不一致返回 `409 Conflict` 与当前 `Upload-Offset`，请求体超过剩余长度返回 `413 Payload Too Large`，两种情况都不修改已提交数据。

达到 `Upload-Length` 后，服务端核对声明的 SHA-256、实际大小及按内容识别的媒体类型。全部一致才原子加入 pending 队列，并在响应中附加 `MiRelay-Delivery-Id`。若摘要或媒体类型不匹配，服务端返回 `422 Unprocessable Entity` 并丢弃该无效上传；发送器随后也会删除本地续传状态。Linux 测试发送器依赖该扩展头确认 MiRelay 入队，但上传偏移状态机仍遵循 tus。当前可恢复的是上传方向；Linux 下载端在瞬时故障时会从头重试完整对象，尚未实现 HTTP Range 下载续传。

规范媒体类型包括：

- 图片：`image/png`、`image/jpeg`、`image/gif`、`image/webp`、`image/avif`
- 文档和归档：`application/pdf`、`application/zip`、`application/gzip`、`application/x-7z-compressed`、`application/vnd.rar`
- 音视频：`audio/mpeg`、`audio/wav`、`audio/flac`、`application/ogg`、`video/mp4`、`video/webm`
- 纯文本和回退类型：`text/plain`、`application/octet-stream`

客户端和服务端必须使用相同的内容识别规则。不能识别为上述签名格式、且不是不含二进制控制字符的合法 UTF-8 文本时，类型必须为 `application/octet-stream`；原文件扩展名不参与判定。

## 获取待处理投递

```http
GET /api/v1/deliveries?status=pending&limit=50&cursor=<opaque>
Accept: application/json
Authorization: Bearer <token>
mirelay-protocol-version: 1
```

第一次请求不带 `cursor`。成功响应：

```http
HTTP/1.1 200 OK
Content-Type: application/json
Cache-Control: no-store

{
  "schema_version": 1,
  "items": [
    {
      "delivery_id": "01JEXAMPLE",
      "original_name": "illustration.png",
      "size_bytes": 1843920,
      "sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
      "media_type": "image/png",
      "created_at_unix": 1788451200
    }
  ],
  "next_cursor": null
}
```

约束：

- `limit` 必须在 1–100；返回的 `items` 数量不能超过它。
- `next_cursor` 是不透明字符串。非空时客户端将原样用于下一页；最后一页必须为 `null` 或省略。
- 分页顺序必须稳定，推荐按 `(created_at_unix, delivery_id)` 升序。一次扫描内不能重复游标或投递 ID。
- `delivery_id` 只能包含 1–128 个 ASCII 字母、数字、`-` 或 `_`。
- `original_name` 必须是单个 UTF-8 文件名，不能包含路径或控制字符，最长 255 字节。
- `size_bytes` 必须大于零；`media_type` 必须是“tus 可恢复上传”一节列出的规范类型之一。
- `created_at_unix` 可省略。响应对象中新增未知字段必须保持向后兼容，客户端会忽略它们。

CLI 单次扫描最多读取 100 页、10,000 个有效投递，索引响应单页上限为 4 MiB。单个非法条目会被隔离并报告，页面或分页协议错误会中止本次扫描。

## 下载内容

```http
GET /api/v1/deliveries/{delivery_id}/content
Accept: image/png
Accept-Encoding: identity
If-Match: "sha256:<hex>"
Authorization: Bearer <token>
mirelay-protocol-version: 1
```

成功响应必须是未经内容编码的原始文件字节：

```http
HTTP/1.1 200 OK
Content-Type: image/png
Content-Length: 1843920
ETag: "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
Cache-Control: no-store
```

`Content-Type`、`Content-Length` 和强 `ETag` 必须与索引元数据一致；`Content-Encoding` 必须省略或为 `identity`。若 `If-Match` 已过期或不匹配，返回 `412 Precondition Failed`，不要返回另一份内容。

CLI 流式读取响应，同时限制大小、计算 SHA-256 并根据文件 magic 再次识别媒体类型；只有三项全部匹配才会持久化。

## 确认送达

```http
PUT /api/v1/deliveries/{delivery_id}/ack
Content-Type: application/json
Authorization: Bearer <token>
mirelay-protocol-version: 1

{
  "sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
}
```

规范成功响应为 `204 No Content`。客户端为兼容性也接受空的 `200 OK`。

ACK 必须以“认证设备 + `delivery_id`”为作用域并保持幂等：第一次以正确摘要确认后，相同请求始终成功；同一投递使用不同摘要必须返回 `409 Conflict`。服务端只有在成功 ACK 后才可将任务从该设备的 pending 队列移除。

## 错误响应

错误推荐使用 `application/problem+json`，至少提供稳定的机器码和可诊断的请求 ID：

```json
{
  "type": "https://mirelay.example/problems/digest-conflict",
  "title": "Delivery digest conflict",
  "status": 409,
  "code": "digest_conflict",
  "detail": "The supplied digest does not match this delivery.",
  "request_id": "req_01JEXAMPLE"
}
```

建议状态码：`400` 参数错误、`401` 令牌无效、`404` 投递不存在或不属于当前设备、`409` ACK 摘要冲突、`412` 下载前置条件失败、`413` 请求或对象过大、`429` 限流、`500`/`503` 临时服务端错误。`429` 和 `503` 可附带 `Retry-After`。

CLI 仅自动重试连接/超时、响应体中断、`408`、`429`、`500`、`502`、`503` 和 `504`。重试使用有上限的指数退避与抖动，并解析秒数或 HTTP 日期形式的 `Retry-After`；服务端给出的等待时间仍受客户端最大退避配置约束。鉴权、参数、协议、ETag、摘要和媒体格式错误不会自动重试。

错误正文超过 8 KiB 时客户端只读取前 8 KiB。`detail` 和 `request_id` 仅用于诊断，不能影响客户端状态机。

## 可靠性不变量

```text
索引元数据
→ 带 If-Match 下载
→ 校验长度、摘要、真实媒体类型
→ 原子持久化文件
→ 持久化 ack_pending
→ 幂等 ACK
→ 持久化 acknowledged
→ 图片进入壁纸工作流，其他文件标记为 not_applicable
```

CLI 可能因为超时或崩溃重复发送 ACK，因此服务端不能把重复 ACK 当作错误。若 ACK 失败，CLI 保留已校验的本地文件，下次同步先验证本地文件并重试 ACK，不重新下载。若该 `ack_pending` 文件随后损坏，而服务端仍返回对应 pending 投递，CLI 会隔离本地对象并从头重新下载。
