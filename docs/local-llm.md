# Local LLM (offline highlight, không cần API key)

"Local LLM" là chiến lược highlight thứ ba (bên cạnh `heuristic` mặc định và
`semantic-gemini` BYOK): đọc transcript bằng **Qwen2.5-3B chạy local** qua
`llama.cpp`, không gọi mạng, không tốn tiền.

## Thành phần

| Thành phần | Vị trí | Git? |
|---|---|---|
| `llama-server` (b11160, linux x64, ~41MB) | `apps/desktop/src-tauri/resources/llama/` | Có (nhỏ) |
| Model `qwen2.5-3b-instruct-q4_k_m.gguf` (~2.1GB) | `resources/models/llama/` + `~/.cache/local-first-ai-video-studio/models/llama/` | **Không** (vượt giới hạn 100MB của GitHub) |
| Download script + SHA-256 | `scripts/download-llm-model.sh` | Có |
| Provider (OpenAI-compatible client) | `services/ai-pipeline/providers.ts` → `LlamaCppHighlightProvider` | Có |
| Spawn/stop server | `apps/desktop/src-tauri/src/main.rs` → `start_local_llm` / `stop_local_llm` | Có |

## Luồng

1. User chọn "Local LLM (offline)" trong Settings → `set_highlight_strategy("semantic-local", …)`
   ghi `HIGHLIGHT_STRATEGY=semantic-local` + `LOCAL_LLM_BASE_URL` + `LOCAL_LLM_MODEL` vào `runtime.env`.
2. Worker (`scripts/analyze-video.ts`) thấy strategy `semantic-local` → tạo
   `LlamaCppHighlightProvider` (không cần key).
3. Tauri `start_local_llm` spawn `llama-server -m <gguf> --port 8080 --ctx-size 4096`,
   chờ `/health` (≤30s), lưu pid vào `LlamaState`.
4. Provider resolve `modelId` qua `GET /v1/models`, gọi `POST /v1/chat/completions`
   theo từng cửa sổ 20 phút (map-reduce), sửa JSON bị cắt cụt nếu model bị `max_tokens`.
5. App thoát → hook `RunEvent::Exit` kill pid llama-server.

## Tải model (2.1GB, on-demand)

```bash
LOCAL_FIRST_STATE_DIR=~/.cache/local-first-ai-video-studio scripts/download-llm-model.sh
# verify SHA-256: 626b4a6678b86442240e33df819e00132d3ba7dddfe1cdc4fbb18e0a9615c62d
```

## Đã xác minh (e2e thật trên máy này)

```text
llama-server b11160 + qwen2.5-3b-instruct-q4_k_m.gguf (2.1GB)
→ LlamaCppHighlightProvider.choose(transcript podcast 40 từ)
→ 5 highlights trong 199s (CPU đang tải, ~5 token/s):
   [0-10s]   score=95 "Welcome to the show."
   [10-23s]  score=90 "The key insight is that a good hook..."
   [23-31s]  score=85 "We also cover a common mistake..."
   [31-32s]  score=80 "Finally, we share a free tool..."
   (JSON bị cắt cụt bởi max_tokens được repair → item score chuẩn hóa)
npm test: 10/10 suite ok (gồm json-repair + long-video map-reduce)
```

Model nhỏ (3B) có thể trả score ngoài 0–100 hoặc JSON bị cắt cụt — provider đã
chuẩn hóa score (0.95→95, clamp 0..100) và `repairTruncatedJsonArray` đóng ngoặc
theo đúng thứ tự nesting trước khi `JSON.parse`.

## Giới hạn

- Chỉ test trên **linux x64** (binary llama.cpp b11160). mac/win cần binary
  tương ứng (llama.cpp có release cho từng OS) — thêm vào resources khi build
  trên runner đúng OS.
- Tốc độ CPU: ~5 token/s trên máy tải (≈80–120s/cửa sổ 20 phút transcript).
  GPU/CUDA build nhanh hơn nhiều.
- Prompt/JSON output bằng model nhỏ (3B): chất lượng highlight thấp hơn Gemini,
  nhưng offline và miễn phí.
