# User stories và acceptance criteria

## US-001 — Import video dài

**Là** creator, **tôi muốn** import video dài, **để** tạo clip ngắn.

- [ ] Nhận file local được hỗ trợ.
- [ ] Hiển thị tên, dung lượng và trạng thái upload.
- [ ] Từ chối MIME, kích thước hoặc duration vượt giới hạn.
- [ ] Không ghi đè source artifact.

## US-002 — Chọn chế độ xử lý

**Là** người dùng, **tôi muốn** chọn local hoặc cloud, **để** cân bằng privacy và tốc độ.

- [ ] Có chế độ local và cloud.
- [ ] Local không upload media.
- [ ] Hai chế độ dùng cùng contract.
- [ ] Hiển thị giới hạn và chi phí dự kiến nếu có.

## US-003 — Tạo transcript

**Là** creator, **tôi muốn** transcript có word timestamps, **để** cắt đúng nội dung.

- [ ] Có segment và word `start/end/text`.
- [ ] Timestamp tăng dần và nằm trong duration.
- [ ] Hiển thị lỗi provider rõ ràng.
- [ ] Có thể dùng provider mock trong test.

## US-004 — Nhận highlight đề xuất

**Là** creator, **tôi muốn** AI đề xuất 3–5 đoạn nổi bật, **để** không phải xem lại toàn bộ video.

- [ ] Mỗi candidate có start, end, title/hook và score.
- [ ] Candidate tham chiếu transcript.
- [ ] Không cắt giữa từ.
- [ ] Không có overlap bất hợp lý.
- [ ] Người dùng có thể bỏ candidate không phù hợp.

## US-005 — Duyệt candidate

**Là** creator, **tôi muốn** preview và chọn candidate, **để** kiểm soát nội dung trước render.

- [ ] Preview candidate.
- [ ] Chọn/bỏ từng candidate.
- [ ] Hiển thị lý do/score nếu có.
- [ ] Không render trước khi người dùng xác nhận.

## US-006 — Chỉnh sửa clip

**Là** creator, **tôi muốn** chỉnh timeline, caption, hook và crop, **để** clip phù hợp mục tiêu.

- [ ] Có preview và timeline.
- [ ] Có trim/cut.
- [ ] Có caption và hook.
- [ ] Có crop/reframe.
- [ ] Thay đổi không phá source artifact.
- [ ] Lưu thành `RenderPlan`.

## US-007 — Render clip

**Là** creator, **tôi muốn** render MP4 9:16, **để** tải và đăng lên nền tảng short video.

- [ ] FFmpeg là final renderer.
- [ ] Output được kiểm tra duration, resolution, codec và checksum.
- [ ] Job có progress/trạng thái.
- [ ] Lỗi render có thể retry.

## US-008 — Quản lý job

**Là** creator, **tôi muốn** retry, cancel và resume job, **để** không mất công khi có lỗi.

- [ ] Có trạng thái `queued`, `processing`, `review`, `rendering`, `completed`, `failed`, `cancelled`.
- [ ] Job có idempotency key.
- [ ] Restart không tạo artifact trùng hoặc mất job.
- [ ] Cancel không để lại output giả.

## US-009 — Lưu project

**Là** creator, **tôi muốn** project tự lưu, **để** tiếp tục sau khi reload.

- [ ] Source, transcript, candidates và RenderPlan được lưu.
- [ ] Có version artifact.
- [ ] Có thể mở lại project.
- [ ] Không mất thay đổi đã xác nhận.

## US-010 — Tải output

**Là** creator, **tôi muốn** tải MP4 hợp lệ, **để** sử dụng ngoài hệ thống.

- [ ] Chỉ tải artifact đã hoàn tất.
- [ ] Download URL có thời hạn nếu chạy cloud.
- [ ] Artifact có checksum và metadata.
- [ ] Không cho truy cập artifact của project khác.

## Ngoài phạm vi

- AI UGC, AI actor, product URL ads, talking-head, advertising B-roll.
- Social publishing hàng loạt.
- Public gallery và collaboration nâng cao.
