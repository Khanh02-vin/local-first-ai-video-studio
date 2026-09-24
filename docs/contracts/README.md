# Canonical contracts — Phase 0

## Mục đích

Local runner, cloud worker, AI pipeline và Svelte/CE.SDK editor dùng cùng contract. Contract là biên giới ổn định; implementation có thể thay đổi.

## Contract MVP

- `Transcript`: segments, words, timestamps, speaker IDs.
- `Highlight`: đoạn AI đề xuất, score, title/hook, word references.
- `Collection`: nhóm Highlight/RenderPlan/Artifact theo chủ đề, có thứ tự và trạng thái duyệt (P2, ADR-002).
- `RenderPlan`: ranges, crop/layout, aspect ratio, captions, hook và output profile.
- `Job`: type, state, execution target, idempotency, retry/error metadata.
- `Artifact`: immutable output, version, checksum, dimensions, duration, storage locator.
- `PublishTarget`: platform, account/profile, schedule và publish status.

## Quy tắc chung

- Mọi timestamp dùng giây số thực, `start >= 0`, `end > start`.
- Timestamp không vượt duration nguồn.
- Word timestamps phải tăng dần trong cùng transcript.
- Không dùng timestamp do LLM tự sinh nếu chưa match/validate với transcript.
- `RenderPlan` versioned; thay đổi caption/crop/layout không làm mất source artifact.
- Artifact immutable; chỉnh sửa tạo version mới.
- Job state transition được whitelist; client không tự đổi state.
- Mọi job có idempotency key.
- Provider-specific fields nằm trong phần mở rộng versioned, không làm hỏng core.
- Secret, token và credential không nằm trong contract gửi cho browser.

## State MVP

```text
created → queued → processing → review → rendering → completed
```

Nhánh lỗi:

```text
processing → failed
rendering  → failed
queued     → cancelled
processing → cancelled
rendering  → cancelled
failed     → retrying → queued
```

Transition phải idempotent; retry không tạo artifact trùng.

## Render semantics

- `9:16` là output MVP.
- `1:1` và `16:9` để sau MVP.
- CE.SDK dùng `RenderPlan` để preview.
- FFmpeg dùng cùng `RenderPlan` để final render.
- Nếu preview và final render khác nhau ngoài tolerance đã định nghĩa, QA phải báo lỗi parity.

## Validation bắt buộc

- Schema validation tại API boundary.
- MIME, file size, duration và resolution validation khi import.
- Highlight không overlap bất hợp lý.
- Collection: items phải tham chiếu đúng loại (highlight/renderPlan/artifact), position liên tục từ 0, không trùng ID/position/reference, title không rỗng, state ∈ {proposed, approved, rejected}, timestamps hợp lệ.
- Caption không vượt giới hạn đã chốt.
- Render output có duration, dimensions, codec và checksum.
- Artifact download kiểm tra quyền project/owner.

## Versioning

- Contract có version rõ ràng.
- Breaking change cần migration và ADR.
- Không đổi field bắt buộc mà không có compatibility plan.
- Fixture contract được lưu trong `tests/contracts`.

## Người sở hữu

- **Tech Lead:** định nghĩa và phê duyệt contract.
- **Backend:** serialize, persist, API validation.
- **Media/AI:** transcript/highlight semantics.
- **Browser Editor:** RenderPlan editing/serialization.
- **Worker/DevOps:** Job/artifact execution semantics.
- **QA:** contract và parity tests.

## Chưa làm trong Phase 0

- Chưa viết implementation.
- Chưa chọn provider AI cụ thể cho production.
- Chưa thêm UGC contract.
- Chưa thêm social publishing hàng loạt contract.
- Chưa đóng gói desktop installer.
