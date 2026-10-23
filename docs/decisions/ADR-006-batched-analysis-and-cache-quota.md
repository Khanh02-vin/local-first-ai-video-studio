# ADR-006: Batch hoá phân tích frame và quota cho preview cache

- **Status:** Accepted
- **Date:** 2026-10-23
- **Owners:** Tech Lead
- **Bổ sung:** ADR-003 (một nguồn sự thật cho crop), ADR-004 (state layout)

## Bối cảnh

Kiểm toán production (mục A và H, đã đối chiếu với code) còn lại hai vấn đề:

1. **Crop/scene analysis spawn process theo từng mẫu**: mỗi mẫu = 1 ffmpeg (extract 1
   frame bằng seek) + 1 python (import cv2 + load cascade + detect 1 frame). Video 180s
   = 72 process; video 30 phút = 720 process, trong đó chi phí import cv2 (~1s) trả đi
   từng mẫu. Benchmark thật trên fixture 180s: **15.4s → 5.4s** khi gộp.
2. **`preview-cache/` không có eviction**: mỗi video import thêm một transcode gần bằng
   kích thước nguồn, và không có gì xoá bao giờ — rò đĩa chậm nhưng chắc trên máy nhỏ.

## Quyết định

### A — Hai process thay vì hàng trăm

- **FFmpeg**: một lệnh duy nhất với filter `fps=1/<interval>,scale=…` và `-frames:v <cap>`
  xuất toàn bộ mẫu (crop: 1/5s, cap 360; scene: 1/2s, cap 900 = 1800s/2s). Decode tuần
  tự thay cho hàng trăm seek — thời gian CPU tương đương, nhưng một process với decoder
  ấm thay vì spawn storm.
- **Python**: `detect-face-boxes.py` nhận **danh sách frame**, import cv2 và load
  cascade đúng một lần, xuất mảng kết quả theo thứ tự; frame đọc lỗi trả `null` thay vì
  làm chết cả đợt (caller bỏ mẫu đó).
- **Ngắt quãng giữ nguyên**: crop vẫn cap 360 mẫu/5s; scene vẫn dừng ở 360 *scenes*
  (khác crop — cap cũ là theo scene, không theo frame) và thời điểm mẫu vẫn
  `index × interval`, chặn tại `t ≥ duration` (`sample_times`, có test).
- **`sample_speaker_faces` giữ nguyên**: ≤8 mẫu, không nằm trong bài toán spawn.

### H — Quota LRU cho preview cache

- Trần **5 GiB**; mỗi lần publish preview mới, dọn file `*.mp4` cũ nhất theo mtime tới
  khi đủ trần. File mới nhất không bao giờ bị dọn. File đang phát trên Windows không
  xoá được → bỏ qua, các file khác vẫn dọn (best-effort).
- Chỉ đụng `*.mp4` trong cache dir — dị vật khác không bị xoá nhầm.

## Hệ quả

### Tích cực

- Video 30 phút: 720 spawn → 2; thời gian phân tích crop giảm ~2.8× (đo trên 180s:
  15.4s → 5.4s, phần lớn còn lại là decode + detect thật).
- Một frame hỏng không còn giết cả đợt phân tích (trước: một frame đọc lỗi = toàn bộ
  command trả lỗi).
- Preview cache có trần đĩa; bài toán "ổ đầy chậm nhưng chắc" được đóng.

### Tiêu cực / rủi ro

- Decode tuần tự cả video thay vì seek lẻ tẻ: với video rất dài nhưng chỉ cần mẫu đầu,
  cách cũ có thể nhanh hơn; trong giới hạn 30 phút của lệnh, tổng thời gian vẫn thấp
  hơn (đã đo).
- Scene với nhiều biến động có thể detect nhiều hơn 360 frame cần (extraction không dừng
  sớm theo scene nữa) — chênh vài giây, đổi lấy một lệnh ffmpeg.
- Quota 5 GiB là hằng số, chưa cấu hình qua Settings — làm khi có nhu cầu thật.

## Xác minh

- cargo test 51/51 (thêm `sample_times` và `preview_cache_quota_evicts_oldest_first` —
  pin trật tự LRU, dưới trần không đụng, file khác `*.mp4` không bị đụng).
- Benchmark A/B trên fixture thật (cùng binary, cùng fixture): 15.4s → 5.4s.
- E2E crop-path (detector + tracker + render thật) vẫn xanh trên app build mới.

## Điều kiện xem xét lại

- Nếu phân tích video dài cần nhanh hơn nữa: chuyển detector sang Rust (`ort`/ONNX) —
  loại bỏ cả process Python; khi đó helper này thành một trait trong engine.
- Nếu người dùng phàn nàn preview cache chiếm 5 GiB: thêm cấu hình dung lượng qua
  Settings.
