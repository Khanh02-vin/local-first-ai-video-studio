# GUI verification status

## Đã xác minh (thật, trên máy này)

```text
Build:  npx tauri build --no-bundle  → release binary (Rust compile sạch, 53s)
Launch: DISPLAY=:0 ./target/release/local-first-ai-video-studio
Result: process sống, KHÔNG panic, WebKitWebProcess + WebKitNetworkProcess load
        (bằng chứng UI khởi động được), log rỗng.
```

**Bug tìm ra và sửa nhờ lần chạy GUI đầu tiên:**

```text
thread 'main' panicked: state for type 'Arc<Mutex<Option<BootstrapProgress>>>'
is already being managed
```

`BootstrapState` và `ModelDownloadState` là hai type alias **trùng kiểu**, mà Tauri
định danh state theo *type* → manage lần 2 panic. Đã tách thành 2 newtype riêng
(`struct BootstrapState(...)`, `struct ModelDownloadState(...)`). Không có lần chạy
app thật thì lỗi này chỉ lộ ra khi khách mở app.

## Logic của 2 nút bấm đã xác minh bằng unit test thật (cargo test)

Click pixel không được thì test chính code mà nút bấm gọi — `#[cfg(test)]` trong `main.rs`
chạy cùng binary, qua `tauri::test::mock_app()`:

```text
test write_runtime_env_preserves_strategy_key_and_endpoints
    → runtime.env ghi mới KHÔNG mất HIGHLIGHT_STRATEGY / GEMINI_API_KEY / LOCAL_LLM_*
test bootstrap_fast_path_adopts_existing_whisper_without_installs
    → nút Setup Whisper nhận whisper sẵn có, KHÔNG chạy pip, báo progress done,
      vẫn giữ lựa chọn strategy cũ
test download_and_verify_accepts_good_checksum_and_rejects_bad_one
    → file:// URL (không tải 2.1GB trong test): sha đúng → vào place, xóa .part;
      sha sai → lỗi MODEL_CHECKSUM_MISMATCH, không để lại file nào
```

**Wiring test** (`tests/wiring/commands.test.ts`, vào `npm test`): quét mọi
`invoke("...")` trong UI (18 lệnh) và đối chiếu với `generate_handler![...]` —
bấm sai tên lệnh = UI chết âm thầm, test này bắt được.

```text
cargo test               → 3 passed
npm test (11 suite)      → all ok, gồm command wiring (18/18 registered)
```

## Bug tìm ra nhờ đo lường (đã sửa)

| Bug | Cách phát hiện |
|---|---|
| `state ... is already being managed` panic khi mở app | Launch app thật lần đầu |
| CI `working-directory: local-first-ai-video-studio` trỏ nhầm (repo root = project) → **CI fail từ đầu** | Đọc lại ci.yml khi thêm cargo test |
| `whisper_command()` tìm `apps/desktop/.venv` (sai 1 cấp so với repo root `../../../.venv`) | Kiểm tra path khi viết fast-path test |
| `runtime.env` chứa `WHISPER_COMMAND=whisper` (bare, không có trong PATH) bị tin dùng → analysis chết với ENOENT | Đối chiếu runtime.env thật với code |
| fast-path nuốt lỗi ghi `runtime.env` (báo thành công nhưng env chưa ghi) | Compiler warning khi viết test |

## Chưa xác minh được (giới hạn môi trường, không phải lỗi app)

Phiên desktop này là **Wayland** (`WAYLAND_DISPLAY=wayland-0`), mọi đường chụp màn hình
đều bị chặn:

| Cách | Kết quả |
|---|---|
| `xwd -root` | `BadMatch` (app chạy qua XWayland, không map vào X tree) |
| GNOME Shell Screenshot (D-Bus) | `AccessDenied: Screenshot is not allowed` |
| xdg-desktop-portal Screenshot | request treo, không trả `Response` signal |
| AT-SPI (accessibility tree) | registry trống (`GetRegisteredEvents` → `[]`) |

→ **Chưa bấm được nút thật qua UI.** Cần một trong: phiên X11 thật, quyền
portal/gnome-screenshot, hoặc chạy trên máy có màn hình vật lý.

## Cách tự kiểm chứng trong 2 phút (khách/dev)

Mở app rồi lần lượt:

1. **Settings → Whisper (transcription)** → bấm *Setup Whisper*.
   Máy đã có whisper (như máy dev) → hiện "Whisper found on this machine."
   Máy sạch → tạo venv + `pip install openai-whisper` (vài GB, một lần).
2. **Settings → Local LLM model** → bấm *Download model (2.1 GB)*.
   Chờ progress → "Model ready." (bỏ qua nếu đã có).
3. **Settings → Highlight strategy** → chọn *Local LLM (offline)* → Save.
4. **Studio** → kéo video vào → *Start analysis* → chờ candidate → chọn 1 → *Render*.
5. Mở MP4 đầu ra, kiểm tra 9:16.

Mọi bước trên đều có đường CLI tương đương đã test thật
(`scripts/run-local-demo.ts`, e2e local-LLM), nên nếu UI lệch thì lỗi nằm ở tầng wiring UI.
