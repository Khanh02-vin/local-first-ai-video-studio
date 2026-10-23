# ADR-008 (Draft): Re-segmentation chuẩn hoá cho highlight selector

- **Status:** Proposed — chưa triển khai; kích hoạt cùng trigger của E
  (onboard pilot user không kỹ thuật, hoặc báo cáo bootstrap-fail đầu tiên)
- **Date:** 2026-10-23
- **Owners:** Tech Lead
- **Điều kiện tiên quyết cho:** E2 (adapter whisper.cpp) — ADR-002/003/004/005/006/007
  không đổi

## Bối cảnh

Lớp 1 của E (benchmark `test:transcript-parity`, commit 7f96ec3) đo trên 3 slice thật
của fixture dialogue-180s (tiny vs tiny ggml):

- **WER 2.4–4.2%** — parity text xuất sắc.
- **Nhanh 3.5–5.8×** trên CPU (11–23s vs 39–67s mỗi clip 60s).
- **Nhưng selector chọn khác trên 1/3 clip** (2 vs 3 picks, IoU 0.548), boundary Δstart
  max tới 3.88s.

Nguyên nhân đã truy vết: `chooseHeuristicHighlights` **segment-driven hoàn toàn**
(`heuristic.ts`: word count × 5 + hook + completeness + pause-boundary từ *gap giữa các
segment* ≥ 0.7s). Mỗi engine có **chính sách chia câu riêng** — đã đo: ~90% boundary
khớp trong 0.5s, nhưng cú chia cục bộ lệch (một câu đuôi chia tại 51.76s so với 53.72s)
→ điểm số từng segment đổi → picks đổi. Không flag nào của whisper.cpp (-bs 1 greedy đã
thử) xoá được khác biệt này — nó nằm ở chính sách, không nằm ở độ chính xác timing.

Nghĩa là **E2 (adapter) không thể GO** với selector hiện tại: đổi engine sẽ đổi
highlight output trên một tỉ lệ clip, mà highlight boundary là thứ người dùng thấy.

## Quyết định (thiết kế cho lúc kích hoạt)

Thêm một lớp **canonical re-segmentation** giữa transcript và heuristic:

1. **Xây lại segment từ words** theo luật tất định, độc lập engine:
   - Cắt tại: dấu kết câu (`.` `?` `!`) theo sau bởi word gap, hoặc gap giữa words
     ≥ 0.7s (giữ đúng tín hiệu pause-boundary hiện có), hoặc chạm trần thời lượng
     (~15s) — số trần chốt khi chạy gate.
   - Mỗi canonical segment mang: text ghép từ words của nó, start/end từ word đầu/cuối,
     `segmentId` mới tất định.
2. **Heuristic chấm trên canonical segments** — mọi tín hiệu hiện có (word count, hook
   keywords, completeness, pause boundary) giữ nguyên công thức, chỉ đổi input.
3. Canonical re-segmentation chạy **cho mọi engine** (kể cả openai-whisper) — điểm mấu
   chốt: hai transcript khác policy nhưng cùng words sẽ ra **cùng canonical segments**
   → cùng picks.

## Hai gate kiểm thử (đều phải pass trước khi E2 mở)

1. **Regression trên engine hiện tại** — `npm run test:eval-highlights`: heuristic trên
   curated fixtures không được tụt so với baseline hiện tại (mean IoU 0.75, hit@5 83%;
   ngưỡng fail cứng đã có sẵn trong test: mean IoU > 0.2). Kỳ vọng:不变 hoặc tốt hơn —
   re-segmentation chỉ chuẩn hoá input, không đổi tín hiệu.
2. **Parity selector** — `npm run test:transcript-parity`: gate G3 phải pass trên corpus
   (same pick count, worst IoU ≥ 0.7 mọi clip) **mà không cần flag riêng của
   whisper.cpp**, và G2 (boundary Δ) được đánh giá lại trên canonical segments thay vì
   segments thô của engine.

## Hệ quả

### Tích cực
- Selector không còn phụ thuộc chính sách chia câu của engine — điều kiện cứng cho
  việc đổi engine transcript mà không đổi sản phẩm người dùng thấy.
- Caption timing và mọi consumer khác của segments có thể dùng canonical segments
  (follow-up, không bắt buộc trong bước này).

### Tiêu cực / rủi ro
- Thay đổi input của heuristic cho **engine hiện tại** — một số clip có thể đổi picks
  dù không đổi engine; gate 1 là hàng rào.
- Re-segmentation làm mất thông tin segment gốc của engine (id, thứ tự) — chấp nhận:
  contract không hứa segment id ổn định.
- Trần ~15s và luật cắt là tham số sản phẩm — chốt bằng gate 1, không chốt trên giấy.

## Phương án không chọn

- **Bắt adapter bắt chước segmentation của openai-whisper**: chính sách chia câu nằm
  trong nội bộ decoder; bắt chước bằng flag là đoán mò, đã thử (-bs 1) và không đủ.
- **Chấp nhận picks lệch theo engine**: người dùng thấy highlight boundary đổi khi đổi
  engine — vi phạm đúng thứ lớp 1 phải bảo vệ.
- **Đổi heuristic sang word-only (bỏ segment)**: viết lại signal hoàn chỉnh
  (pause/completeness đều đang sống trên segment) — lớn hơn nhiều và không có dữ liệu
  đo hậu quả; không trong phạm vi bước này.

## Điều kiện kích hoạt & xem xét lại

- Kích hoạt cùng trigger E; khi đó thực hiện theo thứ tự: canonical re-segmentation →
  gate 1 + gate 2 → E2 (adapter, ADR riêng) → E3 (bundle).
- Nếu gate 1 tụt dù đã thử trần/law cắt khác: dừng, cân nhắc giữ heuristic hiện tại và
  tách selector theo engine (phương án đắt hơn, cần đánh giá lại).
