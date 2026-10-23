# ADR-004: Tách thư mục dữ liệu theo chuẩn OS và kill cây tiến trình đa nền tảng

- **Status:** Accepted
- **Date:** 2026-10-23
- **Owners:** Tech Lead
- **Bổ sung:** ADR-001 (ranh giới runtime), ADR-002/003 (một renderer / một nguồn crop)

## Bối cảnh

Kiểm toán production-readiness chỉ ra hai blocker:

1. **Toàn bộ state nằm dưới `~/.cache/local-first-ai-video-studio`.** `projects.sqlite`,
   `analysis.sqlite`, `runtime.env` (chứa token) cùng ổ với preview-cache. Các công cụ dọn
   dẹp (Storage Sense, CleanMyMac, systemd-tmpfiles) coi thư mục cache là dữ liệu xóa được
   — một đợt dọn máy mất toàn bộ project. Trên Windows còn tệ: code fallback theo `HOME`,
   mà Windows không set `HOME` → DB rơi vào `%TEMP%`, thứ bị dọn aggressive nhất.

2. **Hủy analysis worker chỉ chạy trên POSIX.** `kill_analysis_worker` gọi lệnh `kill`
   với `-PGID`; trên Windows lệnh `kill` không tồn tại và `process_alive` trả `false` cứng
   (`analysis.rs`, nhánh `#[cfg(not(unix))]`) — hủy effectively no-op trong khi CI vẫn
   build Windows MSI + portable zip.

## Quyết định

### B — Hai thư mục gốc, theo chuẩn OS

- **`data_dir()`** — dữ liệu bền: `projects.sqlite`, `analysis.sqlite`, `runtime.env`,
  `license.lic`, `providers.json`, `youtube-tokens.json`, `youtube-client.json`,
  `youtube/` (RAG playlists), `panic.log`.
  - Windows: `%APPDATA%\local-first-ai-video-studio` (fallback `%LOCALAPPDATA%`)
  - macOS: `~/Library/Application Support/local-first-ai-video-studio`
  - Linux: `$XDG_DATA_HOME` hoặc `~/.local/share/local-first-ai-video-studio`
- **`cache_dir()`** — tạo lại được: `preview-cache/`, `llm-cache/`, cache chunk phân tích,
  `llama/` (runtime + model GGUF), `models/`, `.venv` (whisper + opencv).
  - Windows: `%LOCALAPPDATA%\local-first-ai-video-studio\cache`
  - macOS: `~/Library/Caches/local-first-ai-video-studio`
  - Linux: `$XDG_CACHE_HOME` hoặc `~/.cache/local-first-ai-video-studio`

Chọn nhánh theo `std::env::consts::OS` tại **runtime** (không `cfg!`) để cả ba nhánh
unit-test được trên mọi host; `LOCAL_FIRST_STATE_DIR` vẫn ghi đè cả hai gốc về một thư
mục (single-root mode) — đúng những gì test và E2E dựa vào. Node workers nhận cả hai gốc
qua env (`LOCAL_FIRST_STATE_DIR` = data, `LOCAL_FIRST_CACHE_DIR` = cache); `llm-cache.ts`
và `analysis-cache.ts` ưu tiên biến cache mới.

**Migration copy-only** chạy một lần lúc mở app (`migrate_legacy_state`): các file bền
được copy từ gốc cũ sang `data_dir` (gốc cũ giữ nguyên làm backup), `youtube/` và
`llm-cache/` copy recursive, còn `llama/`, `models/`, `.venv` được **rename** (instant
trên cùng volume; fail cross-device thì app tự tải lại). File đã tồn tại ở đích không
bao giờ bị ghi đè. Idempotent.

### C — Kill cây tiến trình bằng `command-group`

Thêm crate `command-group` (Windows: Job Object với `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`;
Unix: process group). Analysis worker giờ spawn qua `group_spawn()` **mọi nền tảng** —
không còn nhánh `#[cfg(unix)] process_group(0)` riêng:

- Hủy (mọi nền tảng): Unix giữ nguyên đường `kill -TERM -PGID` + KILL trì hoãn 3s (đã
  được unit test với fixture bash + E2E `analysis-cancel` chứng minh không sót mồ côi);
  Windows dùng `taskkill /F /T /PID` (diệt cả cây whisper/ffmpeg con).
- App crash: trên Windows, Job Object bị đóng khi tiến trình app chết → **cây còn lại
  bị hệ điều hành giết** — lớp chống mồ côi không phụ thuộc vào đường hủy nào chạy hay
  không. Unix đã có process group + registry dọn lúc exit.
- `GroupChild` phải sống đến khi worker kết thúc (`wait_with_output` của crate giữ
  handle Job suốt); `process_alive` trên Windows tra `tasklist /FI "PID eq …"`.

## Hệ quả

### Tích cực

- Dọn cache của OS không thể mất project nữa — rủi ro mất trắng lớn nhất được đóng.
- Windows có đường hủy thật (cả chủ động lẫn lúc crash) thay vì no-op.
- Fallback TEMP biến mất: mọi nhánh đều rơi vào vị trí chuẩn của OS đó.
- Migration copy-only nghĩa là rollback an toàn: xóa thư mục data mới, app quay lại
  đọc bản gốc trong cache cũ.

### Tiêu cực

- Người dùng hiện có thấy dữ liệu "di chuyển": vị trí mới khác `~/.cache`. Migration
  tự động che phần lớn; bản gốc vẫn nằm lại (chiếm thêm dung lượng một bản nhỏ DB).
- Thêm một crate (`command-group`) và một biến env (`LOCAL_FIRST_CACHE_DIR`).
- Đường Windows (`taskkill`, `tasklist`) không chạy được trên máy dev Linux — chỉ
  review được mã nguồn; trừ khi có runner Windows, đây là điểm cần một lần chạy thử.

### Rủi ro

- `APPDATA` (Roaming) trên Windows doanh nghiệp sync profile — nếu DB phình to, cân
  nhắc chuyển DB sang `%LOCALAPPDATA%` (Local) trong một ADR khác.
- Rename `llama/` cross-device bỏ qua lặng lẽ: lần chạy semantic-local sau sẽ tải lại
  model (~2 GB) — chấp nhận được vì hiếm khi hai volume cùng dính legacy.

## Điều kiện xem xét lại

- Khi Windows runner khả dụng: chạy một lần `cargo test` + smoke test hủy analysis
  trên Windows thật, rồi gỡ nhãn "chưa chạy thử" khỏi quyết định này.
- Khi cần sync profile hoặc DB lớn: tách DB sang `%LOCALAPPDATA%` (đổi `data_dir`
  Windows sang Local và thêm migration lần hai).
