# Backlog MVP

## Phạm vi đã chốt

**MVP chỉ biến video dài thành video ngắn.** Không triển khai UGC, AI actor, quảng cáo từ product URL, talking-head, B-roll quảng cáo, public gallery hoặc collaboration nâng cao.

## Ưu tiên

- **P0:** bắt buộc để MVP chạy end-to-end.
- **P1:** cần cho MVP usable/reliable.
- **P2:** sau MVP.
- **Out:** ngoài phạm vi hiện tại.

## P0 — Vertical slice

| ID | Hạng mục | Kết quả cần đạt | Owner |
|---|---|---|---|
| MVP-001 | Upload/import | Nhận video local; kiểm tra MIME, size, duration | Backend |
| MVP-002 | Probe/quality gate | Biết duration, resolution, codec; từ chối input không hợp lệ | Media/AI |
| MVP-003 | Transcription | Transcript có segment và word timestamps | Media/AI |
| MVP-004 | Highlight candidates | AI đề xuất 3–5 đoạn hợp lệ, không overlap sai | Media/AI |
| MVP-005 | Candidate review | Người dùng xem, chọn/bỏ candidate | Svelte |
| MVP-006 | RenderPlan | Lưu range, aspect ratio, crop, caption, hook | Tech Lead |
| MVP-007 | Final render | FFmpeg tạo MP4 9:16 | Worker/DevOps |
| MVP-008 | Download artifact | Kiểm tra output và tải MP4 | Backend |
| MVP-009 | Job lifecycle | `queued/processing/review/rendering/completed/failed` | Backend + Worker |
| MVP-010 | Svelte dashboard | Project, upload, job status, result | Svelte |

## P1 — Độ tin cậy và trải nghiệm

| ID | Hạng mục | Kết quả cần đạt | Owner |
|---|---|---|---|
| MVP-011 | Browser editor | Preview, timeline, trim, caption, hook, crop | Editor |
| MVP-012 | CE.SDK adapter | CE.SDK chỉ preview/editor; không thay final renderer | Editor |
| MVP-013 | Local runner | SQLite/filesystem/FFmpeg chạy không upload media | Worker/DevOps |
| MVP-014 | Cloud runner | PostgreSQL/Redis/S3/BullMQ chạy cùng contract | Backend + Worker |
| MVP-015 | Retry/cancel/resume | Job không mất sau lỗi, restart hoặc cancel | Worker/DevOps |
| MVP-016 | Autosave/versioning | Không mất RenderPlan và chỉnh sửa | Editor + Backend |
| MVP-017 | Auth cơ bản | User chỉ truy cập project/artifact được cấp quyền | Backend + Security |
| MVP-018 | E2E test | Video mẫu đi hết upload → render → download | QA |
| MVP-019 | Security baseline | Secret, signed URL, traversal, SSRF được kiểm soát | Security |

## P2 — Sau MVP

- Aspect ratio 1:1 và 16:9.
- Speaker tracking nâng cao.
- Solo/picture-in-picture/side-by-side nâng cao.
- YouTube title, thumbnail, description, chapters.
- YouTube publish/scheduling.
- TikTok/Instagram Reels publishing.
- Batch processing.
- Team workspace.
- Quota/billing nâng cao.
- MCP/API.
- Translation/dubbing.
- Analytics.
- Local model management.
- Provider fallback nâng cao.

## Out — Không làm trong MVP/roadmap hiện tại

- AI UGC.
- AI actor.
- Quảng cáo từ URL sản phẩm.
- Talking-head generation.
- B-roll generation cho quảng cáo.
- Public gallery.
- Collaboration nâng cao.
- Social publishing hàng loạt.

## Quy tắc backlog

- Mọi item phải có user story, acceptance criteria và owner.
- Không bắt đầu item nếu chưa có contract/phụ thuộc cần thiết.
- Thay đổi phạm vi phải được ghi trong quyết định sản phẩm.
- P0 phải chạy được end-to-end trước khi mở P1/P2.
