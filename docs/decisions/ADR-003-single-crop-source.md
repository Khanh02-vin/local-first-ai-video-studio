# ADR-003: Một nguồn sự thật cho đường crop

- **Status:** Accepted
- **Date:** 2026-10-22
- **Owners:** Tech Lead
- **Bổ sung:** ADR-002 (một final renderer)

## Bối cảnh

ADR-002 đã hợp nhất hai renderer FFmpeg thành một crate Rust (`render-engine`). Nhưng còn
một đường trùng lặp thứ hai, nhỏ hơn và **nằm trên đường thật**: đường crop.

Đường crop chạy qua hai tầng:

1. Rust `analyze_crop_path` (`src/diarization.rs`): lấy frame, chạy detector mặt, chọn mặt
   của speaker, **smooth đường đi** (alpha 0.35), gán `sceneId`, phát ra `centerX`/`centerY`.
2. Editor (`apps/web/src/lib/pages/Editor.svelte`): nhận output đó, biến mỗi điểm thành hộp
   giả 0.1×0.1, rồi chạy lại `trackFacePath` của `packages/media-engine/reframe` — smooth
   **lần thứ hai** với alpha 0.35, và cắt cảnh theo **luật khác** (xem dưới).

Renderer đọc `plan.cropKeyframes`, tức là nó đọc kết quả của **cả hai** tầng.

### Ba lệch đo được

**a) Smooth hai lần → crop trôi khỏi chủ thể.** Với chủ thể nhảy từ `x=0.2` sang `x=0.8`:

| | Kết quả |
|---|---|
| Chủ thể thật | 0.8000 |
| Rust một mình (smooth 1 lần) | 0.7304 |
| Rust + TS (smooth 2 lần) | **0.6086** |

Tầng TS kéo crop lệch thêm ~0.12 so với crop đúng — và renderer bám theo giá trị sai đó.

**b) Luật cắt cảnh khác nhau.** TS cắt cảnh khi *diện tích mặt* co ≤80% hoặc phồng ≥125%
(kể cả khi mặt không di chuyển); Rust chỉ cắt khi *dịch chuyển* ≥0.35 hoặc nhảy thời gian
>10s, không có luật diện tích. Mặt co 50% mà không di chuyển: TS cắt, Rust không.

**c) Tầng TS bịa dữ liệu cho frame không có mặt.** Rust bỏ hẳn frame không detect được mặt.
TS cũ tạo *fallback point* cho chúng — vị trí suy ra từ frame trước, không phải quan sát
thật. Plan vì thế chứa keyframe cho những vị trí chưa từng được đo.

### Phần không cần đụng tới

`packages/media-engine` còn `generateAss` (TS) và `cropWindow` (TS). Đối chiếu trực tiếp:

- `generateAss` (TS) gộp **4 từ/cue**; `ass_dialogue` (Rust) tạo **1 caption/cue**. Hai bên
  khác semantics, không chỉ khác cách viết. Cả hai đều **không có call site production**:
  `generateAss` chỉ được `tests/engine-parity` gọi; renderer Rust tự sinh ASS.
- `cropWindow` (TS) tính đúng bằng crop filter của Rust (đã đối chiếu: 1920×1080 → cùng
  `width=607.5, x=656.25`), nhưng cũng **0 call site production**.
- `ffmpegCropCenterExpression` (TS) lặp lại `crop_for_range` của Rust, lệch nhẹ ở chỗ
  clamp thời điểm âm (Rust `-0.500`, TS `0.000`) — cũng 0 call site production.

Port chúng sang Rust như đề xuất ban đầu sẽ tạo ra **bản sao thứ hai trong Rust** cho
những hàm không ai gọi, và buộc phải chọn semantics ASS nào là chuẩn — một quyết định
sản phẩm chưa ai cần.

## Quyết định

1. **Rust `analyze_crop_path` là nguồn sự thật duy nhất của đường crop.** Editor dùng thẳng
   output của nó.

2. **Tách phần map sang contract ra module riêng**: `packages/media-engine/reframe/crop-keyframes.ts`
   (`cropKeyframesFromPath`). Module này chỉ *map* — không smooth, không re-track, không bịa
   fallback. Nhờ đó hành vi đó test được độc lập, thay vì nằm trong một `.svelte` dài.

3. **Xoá các hàm TypeScript không có call site production**: `generateAss`, `cropWindow`,
   `ffmpegCropCenterExpression`, `trackFacePath`, `smoothFacePath`, `isSceneCut`,
   `choosePrimaryFace`, `sceneLayoutAt`, `aspectTarget`, `validateEngineRenderPlan`
   (cùng thư mục `packages/media-engine/subtitles/` và `packages/media-engine/render-plan/`).

4. **Giữ `cropCenterAt`**: nó chỉ phục vụ hiển thị giá trị slider trong Editor (view-only),
   không vào render path.

5. **Không port thuật toán sang Rust trong bước này.** Xem "Điều kiện xem xét lại".

## Hệ quả

### Tích cực

- Crop theo đúng vị trí tracker tính ra. Với chủ thể nhảy từ `x=0.2` sang `x=0.8`, điểm ngay
  sau nhảy lưu `0.41` (smooth một lần: `0.2 + 0.35 × (0.8 − 0.2)`), thay vì `0.27` như khi
  smooth hai lần — và các điểm sau đó hội tụ về `0.73` thay vì dừng ở `0.61`.
- Plan không còn chứa keyframe bịa cho frame không detect được mặt.
- Luật cắt cảnh chỉ có một: của Rust. `sceneId` trong plan đã là của Rust từ trước, giờ
  không còn tầng nào định nghĩa lại nó.
- `packages/media-engine` thuần và nhỏ: còn lại validation + probe + view helper.
- `tests/engine-parity/media-engine.test.ts` giờ chặn được hồi quy: nó assert keyframe lưu
  vào plan **bằng đúng** giá trị tracker tính, và mapping chưa xác nhận thì confidence phải
  dưới ngưỡng 0.5 của renderer.

### Tiêu cực

- Editor không còn tự điều chỉnh crop khi tracker bỏ sót frame: trước đây tầng TS lấp đầy
  bằng fallback, giờ plan có lỗ hổng ở các frame đó. Đây là hành vi đúng hơn (không bịa
  dữ liệu) nhưng nhìn thấy rõ hơn trong timeline.
- `packages/media-engine/subtitles` và `packages/media-engine/render-plan` bị xoá; ai import
  bằng đường dẫn con (`media-engine/subtitles`) sẽ phải đổi sang import từ barrel.

### Rủi ro

- Crop trôi lệch trong các project đã lưu: project cũ đã có `cropKeyframes` bị lệch sẽ giữ
  nguyên giá trị cũ cho tới khi người dùng phân tích lại. Đây là dữ liệu đã persist, không
  được sửa ngầm.

## Phương án không chọn

### Port `generateAss` / `cropWindow` / `ffmpegCropCenterExpression` sang Rust

Không chọy: cả ba có 0 call site production. Port chúng tạo bản sao thứ hai trong Rust cho
việc không ai gọi, và bắt buộc phải quyết định semantics ASS nào là chuẩn khi hai bên đang
khác nhau (4 từ/cue so với 1 caption/cue). Đó là quyết định sản phẩm, không phải refactor.

### Giữ `trackFacePath` nhưng bỏ smooth trong đó

Không chọn: nếu giữ hàm thì vẫn còn hai định nghĩa "crop path" trong repo, và tầng TS vẫn
có thể bịa fallback cho frame Rust bỏ. Xoá hẳn để một nơi duy nhất định nghĩa.

### Sửa tầng TS cho khớp Rust (thêm luật diện tích, bỏ fallback)

Không chọy: tầng TS vẫn là bản sao của thuật toán tracking, chỉ là đã sửa cho khớp. Bỏ
tầng đó rẻ hơn và bớt một nơi phải đồng bộ.

## Điều kiện xem xét lại

Xem lại ADR này khi:

- Cần parity giữa preview CE.SDK và final render: khi đó `media-engine` có thể phải trở
  thành nguồn sự thật chung, và quyết định ở trên cần mở lại.
- Cloud worker cần cùng thuật toán crop: khi đó thuật toán phải được port sang Rust hoặc
  thành service, và khi đó port code chết thành code sống — lúc đó bước port có nghĩa.
- Cần một chuẩn ASS dùng chung giữa preview và export: lúc đó phải chọn giữa "4 từ/cue"
  và "1 caption/cue", và đó là một ADR riêng.

## Ghi chú về command blocking

Khi thêm E2E cho đường crop, phát hiện `analyze_crop_path` — và mọi command media khác
(`diarize_video`, `sample_speaker_faces`, `analyze_scene_layouts`,
`generate_transcript_variants`) — chạy **đồng bộ trên main thread** của Tauri. Mỗi lệnh
spawn FFmpeg rồi Python cho từng mẫu frame, tức là chờ hàng giây: trong thời gian đó toàn
bộ webview đóng băng (WebDriver mất session, `TypeError: fetch failed`).

Cả năm command đã được chuyển sang `async` + `tauri::async_runtime::spawn_blocking`, cùng
pattern mà `probe_video` và các lệnh project đã dùng. `tests/wiring/ipc-persistence.test.ts`
giờ assert mỗi lệnh đều chạy trên blocking worker, nên lớp bug này không thể quay lại vô
hình: reverting một command về dạng sync làm test fail với tên command.

## Ghi chú về đánh số ADR

ADR-002 (một final renderer) được viết ở bước 4a. Bước 4b này là ADR-003. ADR-001
(ADR-001-stack.md) không nhắc tới hai ADR sau vì nó chốt ranh giới tổng thể; hai ADR sau
chi tiết hoá "Một final renderer" và "một nguồn sự thật cho crop".
