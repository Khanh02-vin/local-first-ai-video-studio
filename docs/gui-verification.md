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
