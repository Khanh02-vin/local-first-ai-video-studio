# KPI và ngưỡng nghiệm thu MVP

## Mục đích

Đo MVP theo kết quả người dùng và độ ổn định, không tối ưu số tính năng.

## KPI chính

| KPI | Cách đo | Ngưỡng MVP |
|---|---|---:|
| Job hoàn thành | Job `completed` / job bắt đầu | ≥ 95% trên fixture hợp lệ |
| Render thành công | Render thành công / render bắt đầu | ≥ 98% trên fixture hợp lệ |
| Clip đầu tiên | Upload hoàn tất → candidate đầu tiên | Ghi baseline; không đặt số trước benchmark |
| Highlight hợp lệ | Candidate đạt validation / tổng candidate | ≥ 95% |
| Highlight được chấp nhận | Candidate được người dùng chọn / candidate hiển thị | Đo baseline từ pilot |
| Download thành công | Download hoàn tất / artifact completed | ≥ 99% |
| Mất job sau restart | Job mất / job đang chạy | 0 |
| Artifact hỏng | Artifact fail validation / artifact tạo | 0 trên fixture release |
| Lộ secret | Secret trong browser/log/test output | 0 |
| Truy cập sai project | Request vượt quyền thành công | 0 |

## KPI kỹ thuật cần thu thập

- Thời lượng video nguồn.
- Thời gian transcription.
- Thời gian highlight detection.
- Thời gian render.
- CPU/RAM/GPU/disk peak.
- Queue wait time.
- Retry count.
- Provider latency/error.
- Chi phí provider/job.
- Storage usage.
- Browser memory trong editor.

## Quy tắc đo

- Tách local và cloud; không gộp số liệu.
- Báo p50 và p95 khi có đủ mẫu.
- Ghi provider/model, video duration và resolution.
- Không coi mock provider là số liệu production.
- QA lưu fixture, command và kết quả đo.

## Điều kiện release MVP

- Đạt mọi ngưỡng bắt buộc trong bảng.
- Không có lỗi security hoặc data-loss mức release-blocking.
- E2E đi qua upload → transcript → highlight → review → RenderPlan → FFmpeg → artifact → download.
- Product Owner nghiệm thu workflow.
- Tech Lead xác nhận contract/architecture.
- Security Owner xác nhận security baseline.
