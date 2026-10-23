# ADR-007: Bước join dùng stream copy có validate và fallback

- **Status:** Accepted
- **Date:** 2026-10-23
- **Owners:** Tech Lead
- **Bổ sung:** ADR-001 ("Một final renderer"), dựa trên benchmark `bench:concat-ab` (commit 197a1fa)

## Bối cảnh

Bước join của renderer nối các segment đã encode bằng **concat filter + re-encode toàn
bộ** — tức là encode lần hai trên toàn bộ thời lượng output (crf22 thêm một thế hệ).
Benchmark A/B (`tests/local/concat-ab.ts`, baseline commit 197a1fa) đo trên fixture 24s:
filter concat **16.3s**, concat demuxer `-c copy` **0.09s**; drift audio 18ms so với 23ms
(cả hai trong ngưỡng nghe được); SSIM(filter, copy) = **0.971** — re-encode đo được là
mất chất lượng so với copy; 8-segment check xác nhận drift copy **không tích luỹ** qua
boundary (~23ms hằng số = một frame AAC). Bốn gate đều pass → GO.

Rủi ro của copy — đúng thứ ADR-001 cảnh báo — là output lệch shape/thời lượng khi
segment không đồng nhất. Quyết định này xử lý rủi ro đó bằng validate + fallback, không
bằng niềm tin.

## Quyết định

1. **Bước join thành orchestrator** (`join_segments` trong render-engine):
   - **Fast path** — `join_segments_copy`: concat demuxer `-c copy -movflags +faststart`;
     khi nguồn không có audio (`!has_audio`), thêm `-an` để drop track anullsrc mà
     segment vẫn mang — output giữ đúng shape cũ (không bao giờ xuất hiện stream mà
     đường filter không có).
   - **Validate** (`validate_joined_output`) bằng ffprobe trên file copy: video stream
     tồn tại với đúng kích thước kỳ vọng (từ aspect ratio), video duration khớp
     `total_duration` trong tolerance `0.25s + 0.05s × số segment` (độ trôi frame-grid
     tối đa), audio tồn tại khi `has_audio` và lệch video ≤ 0.1s, vắng mặt khi
     `!has_audio`.
   - **Fallback** — `join_segments_filter`: code filter + re-encode cũ, nguyên vẹn.
     Bất kỳ lỗi nào (ffmpeg copy lỗi, probe lỗi, validate reject) đều rơi về đây; nếu
     fallback cũng lỗi, lỗi gốc của cả hai đường được báo cùng lúc.
2. **Không đổi gì khác trong pipeline**: segment vẫn encode như cũ; chỉ bước join đổi.
3. **Benchmark `bench:concat-ab` là baseline** để đối chiếu sau khi đổi.

## Hệ quả

### Tích cực

- Thời gian bước join: 16.3s → 0.09s trên fixture 24s — với export dài, phần re-encode
  thứ hai biến mất khỏi thời gian render.
- Chất lượng output: bỏ một thế hệ crf22 (SSIM filter-vs-copy 0.971 chứng minh mất
  mát cũ là có thật).
- Hành vi đầu ra có sàn cứng: mọi lệch shape/duration đều rơi về đường đã được E2E
  chứng minh suốt nhiều tháng.

### Tiêu cực / rủi ro

- Validate thêm một lần ffprobe (~vài chục ms) — không đáng kể so với encode.
- Copy nhị phân bit-identical với segment: nếu một segment hỏng ngầm (đĩa), cả hai
  đường đều sản xuất hỏng như nhau — fallback không cứu dữ liệu vào đã hỏng, chỉ cứu
  các trường hợp copy không giữ được stream.
- Tolerance duration dựa trên giả định frame-grid của `-r 30`; nếu preset/fps đổi,
  công thức tolerance cần xem lại (đã ghi trong code).

## Xác minh

- Ba test mới: validate reject shape/duration/audio-presence sai; `-an` copy giữ đúng
  shape không-audio; fallback qua **mixed codec** (segment h264 + mpeg4) vẫn sản xuất
  output h264 đúng duration qua đường filter. Integration test render cũ pass trên
  đường copy mới; một assertion framemd5 fragile trong test (lấy row cuối — có thể là
  row audio) đã được thay bằng so sánh frame video tại 0.8s qua `-map 0:v:0`.
- cargo test 54/54, npm test 32/32, E2E render-success trên app build mới.

## Điều kiện xem xét lại

- Nếu đổi codec/preset segment: chạy lại `bench:concat-ab` và đọc lại tolerance.
- Nếu cần hỗ trợ format container khác mp4: kiểm tra hành vi edit-list của demuxer.
