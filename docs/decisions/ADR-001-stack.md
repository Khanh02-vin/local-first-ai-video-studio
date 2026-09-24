# ADR-001: Stack và ranh giới runtime MVP

- **Status:** Accepted
- **Date:** 2026-09-16
- **Owners:** Tech Lead

## Bối cảnh

Dự án hợp nhất lấy nền tảng xử lý từ LTS, AI clip pipeline từ OpenShorts và browser editor từ VideoClipper. Ba nguồn có runtime khác nhau. MVP chỉ cần biến video dài thành short; không triển khai UGC.

## Quyết định

1. **Svelte/SvelteKit** là UI production duy nhất.
2. **TypeScript** dùng cho UI, contracts và media/editor model.
3. **FFmpeg** là final renderer authoritative.
4. **CE.SDK** chỉ dùng cho browser preview/editor; không thay final renderer.
5. Local và cloud dùng cùng `Transcript`, `Highlight`, `RenderPlan`, `Job`, `Artifact`, `PublishTarget` contracts.
6. Local runtime dùng Tauri/Svelte, SQLite queue, filesystem artifact và FFmpeg.
7. Cloud runtime dùng API, PostgreSQL, Redis/BullMQ, S3/R2 và FFmpeg workers.
8. AI providers nằm sau adapter; browser không chứa API secret.
9. Không copy nguyên ba codebase; tích hợp bằng packages, services và adapters.
10. MVP chỉ gồm video dài → highlight → editor → render → download; YouTube export/publish cơ bản là phần phụ trợ, không mở rộng sang UGC.

## Ranh giới

```text
SvelteKit UI
→ API/contracts
→ local runner hoặc cloud worker
→ media/provider adapters
→ artifacts
```

- UI không truy cập trực tiếp DB, queue, storage private hoặc provider secret.
- Core contracts không import framework, DB, Redis, S3 hoặc provider SDK.
- Editor tạo/cập nhật `RenderPlan`.
- Worker thực thi job và tạo artifact.

## Lý do

- Svelte phù hợp quyết định UI đã chốt và loại bỏ hai dashboard React khác nhau.
- FFmpeg giữ output cuối nhất quán giữa local/cloud.
- CE.SDK phù hợp preview/edit tương tác nhưng không nên là chuẩn render duy nhất.
- Contract chung ngăn local, cloud và editor tạo semantics khác nhau.
- Adapter giữ khả năng đổi provider mà không sửa core.
- Phạm vi MVP nhỏ đủ để xác nhận workflow trước khi mở rộng.

## Phương án không chọn

### Dùng React làm UI chính

Không chọn vì quyết định sản phẩm yêu cầu Svelte/SvelteKit và LTS đã có nền Svelte.

### Dùng CE.SDK làm final renderer duy nhất

Không chọn vì preview/browser export không đảm bảo parity, throughput và khả năng xử lý batch/cloud.

### Gộp nguyên ba repository

Không chọn vì khác framework, runtime, storage, queue và dependency; dễ tạo code trùng và contract lệch.

### Xây UGC ngay trong MVP

Không chọn vì ngoài phạm vi; làm tăng provider, chi phí và rủi ro trước khi chứng minh clip workflow.

## Tác động

### Tích cực

- Phạm vi MVP rõ.
- Một UI production.
- Một final renderer.
- Local/cloud có thể thay thế nhau.
- Dễ test và migration từng phần.

### Tiêu cực

- Cần viết adapter giữa các nguồn.
- Cần tách logic VideoClipper khỏi `App.tsx`.
- Cần duy trì CE.SDK preview và FFmpeg parity tests.
- Một số tính năng OpenShorts bị hoãn.

### Rủi ro

- Khác biệt crop/timebase giữa CE.SDK và FFmpeg.
- Provider AI lỗi hoặc thay đổi API.
- Local toolchain/FFmpeg packaging.
- Chi phí cloud và GPU chưa có baseline.

## Điều kiện xem xét lại

Xem xét ADR khi:

- CE.SDK/FFmpeg parity không đạt ngưỡng KPI sau benchmark.
- Svelte không đáp ứng accessibility/performance đã chốt.
- Local/cloud contract cần breaking change.
- MVP có nhu cầu bắt buộc về batch rendering hoặc provider khác.

Mọi thay đổi phải tạo ADR mới hoặc cập nhật ADR này kèm migration plan.
