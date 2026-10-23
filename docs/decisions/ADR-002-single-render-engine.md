# ADR-002: Một final renderer duy nhất (Rust render engine)

- **Status:** Accepted
- **Date:** 2026-10-22
- **Owners:** Tech Lead
- **Supersedes:** ADR-001 mục 3 và 4 (chi tiết hoá "Một final renderer")

## Bối cảnh

ADR-001 chốt "FFmpeg là final renderer authoritative" và "Một final renderer". Nhưng trước khi
ADR-002 này, repo có **hai renderer cùng làm việc**:

1. **Path A — Rust trong app**: `apps/desktop/src-tauri/src/render.rs` (`render_video` → `render_segments`).
   Đây là đường UI thực sự dùng: hai call site trong `Studio.svelte` gọi `render_video`. Nó
   plan-aware (captions, hook, crop keyframes, scene layout), báo progress qua
   `Channel<RenderProgress>`, và hủy được bằng process-group kill.

2. **Path B — Node**: `adapters/local/runner.ts` → `adapters/ffmpeg/index.ts` (`render()`),
   điều khiển bởi `scripts/local-render.ts` và ba test Node. Nó tự dựng argument FFmpeg:
   crop theo tỉ lệ khung hình, `libx264 -preset fast -crf 22`, không có captions, không có
   hook, không có crop keyframe, không có scene layout. `local-render.ts status` tự báo
   `renderedCaptions: false, renderedHooks: false`.

Hai đường này lệch nhau theo thời gian: preset, filter, cách xử lý audio, cách hủy. Không
có gì bảo đảm chúng cho cùng kết quả, và `packages/media-engine` (thư viện thuần) chỉ chứa
`generateAss` / `cropWindow` / `trackFacePath` — không phải renderer.

Ngoài ra, path B được Rust gọi ngược lại qua 5 command Tauri (`render_enqueue`, `render_list`,
`render_get`, `render_retry`, `render_drain`) — **không UI nào và không test nào dùng**, và
`render_enqueue` nhận `request: serde_json::Value` không kiểu, phải tự parse JSON ở tầng Node.
Đó là vòng đi vòng lại: app → Node → FFmpeg, trong khi app đã có renderer của riêng nó.

## Quyết định

1. **Tách renderer thành một crate Rust độc lập với Tauri**: `apps/desktop/src-tauri/crates/render-engine`.
   Crate này giữ toàn bộ logic dựng argument FFmpeg, ASS, crop, progress và cancel. Nó không
   biết Tauri là gì: caller truyền vào một "progress sink" và đọc cờ cancel.

2. **Hai front door, một engine**:
   - App: `src/render.rs` là lớp bọc mỏng, gọi `render_engine::render_segments` và chuyển
     progress lên webview channel.
   - CLI: `apps/desktop/src-tauri/crates/lfai-render` (`lfai-render`) — front door headless,
     nói cùng contract `RenderRequest`, in progress ra stderr dạng JSON.

3. **Node trở thành client, không phải renderer**: `adapters/local/runner.ts` giữ nguyên
   queue (enqueue / dedupe / retry / attempt budget / analysis gate) nhưng gọi `lfai-render`
   qua `adapters/local/engine.ts`. `adapters/ffmpeg/` — renderer trùng lặp — bị xoá.

4. **Contract ở crate engine**: các struct `#[typeshare]` (`ProbeMetadata`, `RenderProgress`,
   `RenderRange`, …) sống trong `render-engine/src/contracts.rs`. `npm run contracts:generate`
   và `npm run test:contracts-published` quét crate đó. App re-export từ đó, nên không có
   struct IPC nào bị định nghĩa hai lần.

5. **Bỏ 5 command `render_*` của path B**: không có consumer, nên xoá thay vì giữ làm
   đường render thứ ba. `scripts/local-render.ts` vẫn giữ lệnh queue (`enqueue`/`list`/`get`/
   `retry`/`drain`/`status`) vì test bridge dùng chúng — nhưng lệnh `drain` giờ đi qua engine.

## Hệ quả

### Tích cực

- Chỉ còn một chỗ quyết định cách clip được encode. Sửa preset hay filter sửa một chỗ,
  và app lẫn CLI đều hưởng.
- Node queue không còn tự dựng FFmpeg: `local-render.ts status` không còn phải tự báo
  `renderedCaptions: false` — engine render được captions/hook khi plan có chúng.
- Contract IPC có một nguồn: crate engine. Typeshare sinh TS từ đó, parity test canh drift.
- Engine không phụ thuộc Tauri nên test được bằng `cargo test` không cần webview.

### Tiêu cực

- Thêm một crate và một binary: build pipeline phải build `lfai-render` trước khi chạy test
  Node (CI đã thêm bước này).
- Engine phải nhận FFmpeg/ffprobe dạng tham số thay vì tự tìm trong resources của app;
  app truyền `ffmpeg_bin(&app)`, CLI nhận `--ffmpeg`/`--ffprobe` hoặc env.

### Rủi ro

- Node test phụ thuộc binary `lfai-render` đã build. Thiếu binary thì test fail với
  `spawn lfai-render ENOENT` — rõ ràng, nhưng CI phải build nó trước (đã làm).
- Render chạy qua một tiến trình con thay vì gọi FFmpeg trực tiếp: thêm một ranh giới
  lỗi. Engine trả mã lỗi theo chuẩn (`RENDER_CANCELLED`, `OUTPUT_MUST_DIFFER_FROM_INPUT`, …)
  nên hành vi quan sát được không đổi.

## Phương án không chọn

### Giữ hai renderer, thêm test parity

Không chọn: parity giữa hai bản sao vẫn là hai nguồn sự thật, và đường B đã thua đường A
(captions/hook/crop keyframe đều thiếu) nên parity sẽ phải chấp nhận khác biệt thay vì
loại bỏ nó.

### Cho Node gọi Tauri command `render_video` qua IPC

Không chọn: một tiến trình CLI không có `AppHandle` và không có webview channel. Điều này
buộc phải mở một cổng HTTP/IPC riêng — nhiều bề mặt hơn một crate thư viện.

### Dùng media-engine làm nguồn sự thật cho cả hai bên

Không chọn trong bước này: media-engine là thư viện thuần TS, còn renderer là Rust. Đồng bộ
hai ngôn ngữ qua một định dạng trung gian là thay đổi rủi ro cao hơn nhiều so với việc hợp
nhất hai bản sao của cùng một ngôn ngữ.

Bước 4b (ADR-003) đã xử lý phần này theo hướng khác: thay vì port thuật toán sang Rust, nó
xoá các bản sao TypeScript **không có call site production** và gộp đường crop về một nguồn
sự thật (Rust). Xem ADR-003.

## Điều kiện xem xét lại

Xem xải ADR này khi:

- Cloud worker cần dùng chung engine: khi đó contract `RenderRequest` phải trở thành wire
  format ổn định giữa Rust và worker, và engine cần một front door thứ ba (HTTP/RPC).
- Cần parity giữa preview CE.SDK và final render: khi đó `media-engine` có thể trở thành
  nguồn sự thật chung, và quyết định ở trên cần được mở lại (ADR-003 cũng ghi lại điều kiện
  này).
