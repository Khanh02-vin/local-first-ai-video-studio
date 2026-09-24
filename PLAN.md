# Local-first AI Video Studio — Local MVP

## Mục tiêu

Ứng dụng desktop biến video dài thành video ngắn, không upload media:

```text
File local
→ SQLite job
→ Faster-Whisper local
→ heuristic highlight
→ Svelte editor
→ FFmpeg
→ MP4 local
```

## Kiến trúc đã chốt

```text
Svelte/SvelteKit UI
→ một Node process/Tauri
→ SQLite metadata queue
→ filesystem artifacts
→ Faster-Whisper CLI/process
→ FFmpeg
→ MP4
```

### Dùng trong MVP

- Svelte/SvelteKit.
- Tauri desktop shell.
- SQLite.
- Filesystem local.
- FFmpeg/ffprobe.
- Faster-Whisper local.
- Heuristic highlight selection.
- HTML video/timeline/CSS editor.
- `RenderPlan` dùng chung.

### Hoãn

- PostgreSQL.
- Redis/BullMQ.
- S3/R2.
- Cloud worker.
- JWT/session/tenant.
- Network rate limit.
- CE.SDK.
- Gemini/OpenAI cloud.
- OAuth/publishing tự động.
- Billing/team workspace.

## Chức năng MVP

1. Chọn video local.
2. Kiểm tra MIME, dung lượng, thời lượng và codec.
3. Probe bằng `ffprobe`.
4. Tách audio bằng FFmpeg.
5. Transcribe bằng Faster-Whisper local.
6. Chọn 3–5 highlight bằng câu đủ ý, keyword và khoảng lặng.
7. Hiển thị transcript và timeline.
8. Người dùng chọn/chỉnh range.
9. Tạo caption.
10. Crop 9:16 bằng FFmpeg.
11. Render MP4.
12. Lưu job/recovery trong SQLite.
13. Tải MP4 thủ công.

## Bảo mật và an toàn

- Video không rời máy.
- Không lưu API key trong MVP.
- Không ghi đè source.
- Giới hạn file size/duration.
- Validate content trước xử lý.
- Dọn file tạm sau job.
- Không log nội dung video/transcript đầy đủ.
- Model local phải có version/checksum/license.
- Output dùng artifact version và checksum.

## Cấu trúc bật

```text
apps/desktop
apps/web
packages/contracts
packages/media-engine
packages/editor-core
adapters/local
adapters/ffmpeg
services/ai-pipeline
```

Cloud/publishing/provider directories giữ lại làm boundary, nhưng không nằm trong MVP runtime.

## Lộ trình

### Phase A — Local pipeline

- SQLite queue.
- Probe.
- Audio extraction.
- Faster-Whisper CLI adapter.
- Heuristic highlights.
- FFmpeg render.
- Recovery/retry/cancel.

### Phase B — Editor

- HTML video preview.
- Transcript word mapping.
- Timeline/range editing.
- Caption/hook.
- Crop/layout 9:16.
- Autosave `RenderPlan`.

### Phase C — Desktop

- Tauri shell.
- Native file picker.
- Local runner command boundary.
- Progress/cancel/retry.
- Secure filesystem paths.
- Installer chỉ sau clean-machine check.

## Tiêu chí hoàn thành MVP

- Một video hợp lệ đi từ import → transcript → highlight → edit → render → MP4.
- Không cần network hoặc credential.
- Job khôi phục sau restart.
- Output không ghi đè input.
- `npm test` và Svelte check/build xanh.
- Có fixture test cho video, transcript, highlight và render.

## Điều kiện mở rộng

Chỉ thêm dependency khi có bằng chứng:

- Gemini/OpenAI: heuristic không đạt highlight acceptance.
- CE.SDK: editor HTML/CSS không đủ.
- PostgreSQL/S3: cần cloud/team/sync.
- Redis: có nhiều worker.
- OAuth: người dùng cần publish tự động.

## Đánh đổi

Ưu điểm: ít dependency, privacy, chi phí thấp, dễ debug, dễ phát hành desktop.

Hạn chế: không collaboration, cloud sync, batch đa máy hoặc publish tự động; tốc độ phụ thuộc máy người dùng.
