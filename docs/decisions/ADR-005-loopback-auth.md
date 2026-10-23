# ADR-005: Cổng động và token phiên cho các sidecar loopback

- **Status:** Accepted
- **Date:** 2026-10-23
- **Owners:** Tech Lead
- **Bổ sung:** ADR-001 (ranh giới runtime)

## Bối cảnh

Ba dịch vụ HTTP chạy loopback trong app đều **bind cổng cứng và không có xác thực**:

- llama-server: `let port: u16 = 8080;` (`runtime.rs`)
- RAG sidecar: `pub(crate) const RAG_PORT: u16 = 4733;`
- YouTube preview shim: `YT_PREVIEW_PORT = 14872` (CSP tĩnh cho phép)

Hai rủi ro:

1. **Đụng cổng**: 8080 là cổng phổ biến nhất mà proxy/dev tools chiếm. llama-server không
   lên được, hoặc tệ hơn — app nói chuyện nhầm với một server lạ trên 8080.
2. **Drive-by web page**: một trang web bất kỳ trong trình duyệt của người dùng có thể
   `fetch("http://127.0.0.1:4733/api/rag/playlists")` — đọc playlist, YouTube ranking,
   và POST ingest qua kết nối của người dùng. Trình duyệt chặn *đọc* response qua CORS
   nhưng request vẫn tới server; với preview shim, trang còn nhúng được iframe proxy
   YouTube qua kết nối người dùng.

## Quyết định

1. **Cổng động**: sidecar bind cổng ephemeral (`free_loopback_port()` — bind `:0`, đọc
   lại, đóng). Cửa sổ TOCTOU nhỏ được vòng chờ health biến thành lỗi timeout sạch thay
   vì nói chuyện nhầm với server lạ.
2. **Token phiên** (`session_token()`): 128-bit từ `RandomState` của std (seed OS) trộn
   clock + pid + địa chỉ ASLR — đủ chống đoán cho một trang web, không phải ranh giới
   chống local malware.
   - **llama-server**: spawn với `--api-key <token>`; endpoint ghi vào `runtime.env`
     (`LOCAL_LLM_BASE_URL`, `LOCAL_LLM_API_KEY`, `LOCAL_LLM_MANAGED=1`) nên Node
     helpers (helper_env) và RAG sidecar tự nhận cùng endpoint. Bootstrap writer giữ
     lại hai key này khi ghi lại runtime.env.
   - **RAG sidecar**: spawn với `LFAI_RAG_TOKEN`; mọi handler (GET/POST/DELETE) trả 401
     khi thiếu/sai bearer. `rag_server_url` giờ trả `{base, token}`; webview gửi
     `Authorization: Bearer` trên từng request qua plugin-http. Session file
     (`data/rag-session.json`, 0600) cho phép **tái gắn** sidecar còn sống sau app crash.
   - **Preview shim**: token trong query (`/preview?token=…`); `preview_origin` trả
     prefix kèm token, UI nối `&id=…&start=…&end=…`.
3. **Ngoại lệ có chủ đích — preview giữ cổng cố định**: CSP của webview tĩnh cho phép
   `frame-src http://localhost:14872`; cổng động đòi hỏi CSP động. Token trong URL là
   đủ chống drive-by (trang không đoán được URL).
4. **OAuth callback giữ cổng cố định**: redirect URI là ràng buộc bên ngoài (Google app
   registration); loopback dynamic port của RFC 8252 đòi hỏi đổi đăng ký app.
5. **Standalone/dev không đổi**: sidecar chỉ gate khi `LFAI_RAG_TOKEN` được set — chạy
   browser thường (không qua app) vẫn như cũ.

## Hệ quả

### Tích cực

- Hết đụng cổng: llama/RAG không thể chết vì một dev server khác chiếm 8080/4733.
- Drive-by web page nhận 401 từ sidecar và không build được URL preview.
- Llama chết vì crash app không còn là "server lạ chiếm 8080" ở lần mở sau.

### Tiêu cực / rủi ro

- Đường sidecar-FIXED-port cũ không còn tái sử dụng được cho RAG khi session file mất
  (xóa tay data dir): spawn một instance mới, instance cũ mồ côi tới khi chết.
- Preview token nằm trong URL — xuất hiện trong log trình duyệt webview nếu có; chấp
  nhận được cho threat model "trang web khác trên máy".
- `--api-key` của llama-server cần đúng phiên bản bundle (đã verify: b11160 hỗ trợ).

## Xác minh

- Live: sidecar build mới trả **401** không token, **401** sai token, **200** đúng token
  (đã chạy thật với `LFAI_RAG_TOKEN=testtok123`).
- `llama-server --help` của bundle chứa `--api-key`.
- Wiring test chặn hồi quy: `--api-key` khi spawn llama, `LFAI_RAG_TOKEN` khi spawn RAG,
  `authorized(request)` trong mọi handler, `free_loopback_port()`/`session_token()` được
  dùng, và const `RAG_PORT` không được phép quay lại.
- cargo test 49/49, npm test 32/32, svelte-check 0/0.

## Điều kiện xem xét lại

- Khi CSP webview hỗ trợ nguồn động: chuyển preview sang cổng động và bỏ token-URL.
- Khi OAuth app được đăng ký lại với loopback dynamic port (RFC 8252): chuyển callback
  sang cổng động.
