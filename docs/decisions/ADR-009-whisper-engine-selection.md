# ADR-009: Whisper engine selection (Python openai-whisper vs whisper.cpp)

- **Status:** Accepted (with migration path)
- **Date:** 2026-10-25
- **Owners:** Tech Lead

## Bối cảnh

Pipeline xử lý video dài → short hiện sử dụng `openai-whisper` (Python venv, qua `adapters/local/whisper.ts`) để sinh transcript, sau đó `chooseHeuristicHighlights` (services/ai-pipeline/heuristic.ts) chọn các highlight từ transcript.

`openai-whisper` có nhược điểm trên máy người dùng cuối:
- Tạo Python `venv` và chạy `pip install openai-whisper` → tải PyTorch (vài GB), tỷ lệ lỗi cao trên Windows/macOS (thiếu Visual C++ Redistributable, xung đột PATH, tường lửa/AV chặn).
- Tốc độ chậm trên CPU (~39–67s mỗi clip 60s).
- Không có binary độc lập; phụ thuộc vào environment Python của máy host.

`whisper.cpp` (C++, có binary CLI + model `.ggml`) là thay thế tiềm năng:
- Binary độc lập, không cần Python venv hay PyTorch.
- Tốc độ nhanh hơn 3.5–5.8× trên CPU.
- Có model `tiny` (ggml-tiny.bin) đã có benchmark trong `tests/ai-pipeline/transcript-parity.ts`.

Tuy nhiên, benchmark layer-1 (2026-10-23, dialogue-180s × 3 slices, tiny vs tiny ggml) cho thấy:
- Text parity tốt: WER 2.4–4.2%.
- Nhưng **segmentation policy khác cấu trúc**: ~90% matched segment starts đồng nhất trong 0.5s, nhưng sentence splits khác nhau cục bộ (ví dụ tail sentence split 51.76s vs 53.72s → Δstart 3.88s).
- Vì `chooseHeuristicHighlights` là **segment-driven** (segment gaps, segment word counts, segment text), pick sets khác nhau trên 1/3 clip (2 vs 3 picks, worst IoU 0.548).
- → **Verdict: NO-GO** cho việc swap adapter trực tiếp với selector hiện tại.

## Quyết định

1. **Giữ `openai-whisper` làm engine mặc định** trong `adapters/local/whisper.ts`. Không xóa, không disable.
2. **Thêm `whisper.cpp` làm engine tuỳ chọn** qua biến môi trường `WHISPER_ENGINE=cpp` (default `python`). Khi bật, adapter tương ứng sẽ được sử dụng thay vì Python venv.
3. **Làm `chooseHeuristicHighlights` segmentation-robust**: chuyển từ segment-driven sang **word-level sliding window**, vì segment boundaries từ hai engine khác nhau, còn word timings và pause giữa các từ ổn định hơn. Selector phải cho kết quả giống nhau (IoU ≥ 0.7) bất kể engine nào sinh transcript.
4. **Đến khi G3 (selector parity) đạt trên mọi clip**, giữ `openai-whisper` làm mặc định; `whisper.cpp` chỉ dùng cho đánh giá/parity test.
5. **Sau khi G3 pass**, có thể đổi mặc định sang `whisper.cpp` và deprecate Python venv theo ADR riêng.

## Ranh giới

```text
WHISPER_ENGINE=python (default)
  → adapters/local/whisper.ts → openai-whisper CLI (Python venv)
  → Transcript contract
  → chooseHeuristicHighlights (word-level sliding window)

WHISPER_ENGINE=cpp
  → adapters/local/whisper-cpp.ts (mới) → whisper.cpp CLI (binary độc lập)
  → Transcript contract (giống transcript-parity.ts:transcriptFromWhisperCpp)
  → chooseHeuristicHighlights (cùng hàm, segmentation-robust)
```

- `whisper.cpp` chỉ được bật khi `LF_WHISPERCPP_BIN` và `LF_WHISPERCPP_MODEL` tồn tại (kiểm tra trong System Doctor).
- Contract `Transcript`/`Highlight` không thay đổi: chỉ source của transcript khác nhau.

## Lý do

- `openai-whisper` là rào cản lớn nhất đối với người dùng cuối (Python venv, pip, PyTorch, vários GB).
- `whisper.cpp` loại bỏ hoàn toàn Python khỏi bản phát hành, đóng gói thành binary độc lập qua Tauri `externalBin`.
- Selector segmentation-robust là điều kiện tiên quyết: nếu highlight set thay đổi giữa hai engine, người dùng sẽ thấy kết quả khác nhau tuỳ theo engine đang bật — không chấp nhận được đối với sản phẩm.
- Giữ cả hai engine trong khoảng thời gian Übergang cho phép benchmark thực tế trước khi đổi mặc định.

## Phương án không chọn

### Swap trực tiếp `openai-whisper` bằng `whisper.cpp` ngay bây giờ

Không chọn vì `chooseHeuristicHighlights` segment-driven sẽ cho kết quả khác nhau giữa hai engine (G3 fail). Cần làm selector segmentation-robust trước.

### Xóa Python venv ngay trong ADR này

Không chọn vì `openai-whisper` vẫn là engine mặc định; Python venv vẫn cần thiết cho chế độ mặc định và cho backward compatibility.

### Dùng whisper.cpp làm thư viện Rust (whisper-rs) thay vì binary CLI

Không chọn vì thêm crate Rust phức tạp, build time dài, và binary CLI đã có sẵn, dễ đóng gói qua `externalBin`.

## Tác động

### Tích cực

- Loại bỏ rào cản Python/PyTorch đối với người dùng cuối.
- Tốc độ transcribe nhanh hơn hơn 3× trên CPU.
- Selector segmentation-robust cũng giúp kết quả ổn định hơn giữa các phiên bản model và các nguồn transcript khác.

### Tiêu cực

- Cần duy trì hai adapter trong khoảng thời gian Übergang.
- `whisper.cpp` binary + model `.ggml` cần bundle vào Tauri `externalBin` (tăng kích thước cài đặt).
- Cần benchmark thực tế trên nhiều clip đa ngôn ngữ trước khi đổi mặc định.

### Rủi ro

- Nếu selector chưa đủ robust, swap engine sẽ làm thay đổi highlight set mà người dùng thấy — ảnh hưởng trải nghiệm.
- `whisper.cpp` có thể không hỗ trợ tất cả các tính năng của `openai-whisper` (ví dụ một số ngôn ngữ ít phổ biến).

## Điều kiện xem xét lại

Xem xét ADR khi:

- `chooseHeuristicHighlights` đã segmentation-robust và G3 pass trên mọi clip test.
- Có benchmark thực tế trên clip đa ngôn ngữ, nhiều độ dài.
- Người dùng yêu cầu tốc độ transcribe nhanh hơn hoặc không thể cài Python venv.
- Có nhu cầu bundle `whisper.cpp` vào bản cài đặt `.deb`/`.exe`/`.dmg`.

Mọi thay đổi phải tạo ADR mới hoặc cập nhật ADR này kèm migration plan.

## Cập nhật 2026-10-28 — chi phí đã đo của sliding-window selector

Sliding-window (10s / step 5s) đã triển khai xong, `test:heuristic-segmentation-robust`
pass (cùng picks qua các segmentation khác nhau — G3 về mặt parity engine đạt).
Nhưng eval harness trên bộ fixture chuẩn (6 cases / 8 highlights do người chọn,
`npm run test:eval-highlights`) cho thấy chi phí so với selector segment-driven cũ:

| Selector | IoU | hit@5 |
|---|---|---|
| Segment-driven (trước, HEAD `0725a94`) | 0.75 | 83% |
| Sliding-window (hiện tại) | 0.39 | 67% |

Tức là parity giữa hai engine được mua bằng việc giảm chất lượng picks cho **cả hai**
engine so với chuẩn người chọn. Nguyên nhân: cửa sổ 10s cắt giữa câu, lệch khỏi
ranh giới câu mà highlights thật thường bám theo. Chưa chốt: chấp nhận trade-off này
(ghi vào đây), hay chuyển sang re-segmentation theo ADR-008 (Proposed) để lấy lại
chất lượng mà vẫn giữ parity. Quyết định pending trước khi đổi engine mặc định.
