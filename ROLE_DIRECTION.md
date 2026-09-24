# Định hướng công việc: Product Owner và Tech Lead

## 1. Mục đích

Tài liệu này phân định rõ hai loại quyết định của dự án **Local-first AI Video Studio**:

- **Product Owner (PO): quyết định làm gì và vì sao.**
- **Tech Lead/Architect (TL): quyết định xây như thế nào và với giới hạn nào.**

Hai role phối hợp nhưng không thay thế nhau.

---

## 2. Ranh giới quyết định

| Vấn đề | Product Owner | Tech Lead |
|---|---|---|
| Người dùng mục tiêu | Quyết định | Tư vấn khả năng đáp ứng |
| Vấn đề cần giải quyết | Quyết định | Đánh giá tính khả thi |
| Tính năng cần làm | Quyết định ưu tiên | Thiết kế cách triển khai |
| Thứ tự phát hành | Quyết định theo giá trị | Ước lượng phụ thuộc kỹ thuật |
| KPI sản phẩm | Định nghĩa | Cung cấp số liệu kỹ thuật cần đo |
| Kiến trúc | Góp ý theo mục tiêu | Quyết định |
| Công nghệ và package | Nêu ràng buộc | Quyết định |
| API/data contract | Phê duyệt theo nhu cầu | Thiết kế và duy trì |
| Bảo mật kỹ thuật | Xác định mức chấp nhận | Thiết kế biện pháp kiểm soát |
| Chất lượng phát hành | Xác định tiêu chuẩn người dùng | Đảm bảo tiêu chuẩn kỹ thuật |
| Technical debt | Cân đối với roadmap | Đánh giá và đề xuất xử lý |
| Go/no-go release | Quyết định sản phẩm | Đưa đánh giá kỹ thuật và rủi ro |

**Quy tắc:** PO không áp đặt giải pháp kỹ thuật cụ thể. TL không tự mở rộng phạm vi sản phẩm ngoài backlog đã thống nhất.

---

# 3. Product Owner — quyết định làm gì

## 3.1. Trách nhiệm chính

### A. Xác định vấn đề và người dùng

- Xác định nhóm người dùng đầu tiên:
  - Creator.
  - Podcaster.
  - Editor.
  - Marketing team.
- Viết vấn đề người dùng đang gặp.
- Xác định kết quả người dùng cần đạt.
- Xác định phạm vi không làm trong giai đoạn hiện tại.
- Kiểm chứng nhu cầu bằng phỏng vấn, prototype hoặc dữ liệu sử dụng.

### B. Xác định phạm vi sản phẩm

MVP hiện tại:

1. Import/upload video dài.
2. Transcription.
3. AI đề xuất highlight.
4. Svelte browser editor.
5. Caption, hook, crop và layout.
6. Render MP4 local/cloud.
7. Download.
8. Project autosave.
9. Job retry/cancel/resume.

Chưa thuộc MVP:

- AI UGC.
- AI actor.
- Quảng cáo từ product URL.
- Public gallery.
- Collaboration nâng cao.
- Social publishing hàng loạt.
- Billing phức tạp.

PO có quyền thay đổi phạm vi, nhưng phải ghi rõ lý do, tác động và thay đổi ưu tiên.

### C. Quản lý backlog và ưu tiên

- Viết feature brief.
- Chuyển feature thành user story.
- Xác định acceptance criteria.
- Xếp hạng theo:
  - Giá trị người dùng.
  - Mức cấp thiết.
  - Rủi ro.
  - Chi phí.
  - Phụ thuộc.
- Chia feature lớn thành vertical slice có thể kiểm thử.
- Loại bỏ yêu cầu không tạo giá trị rõ ràng.

### D. Xác định KPI

KPI đề xuất:

- Thời gian từ upload đến clip đầu tiên.
- Tỷ lệ job hoàn thành.
- Tỷ lệ render thành công.
- Tỷ lệ người dùng chấp nhận highlight AI.
- Thời gian chỉnh sửa trung bình.
- Tỷ lệ export thành công.
- Chi phí trung bình mỗi job.
- Tỷ lệ người dùng quay lại.
- Tỷ lệ publish thành công khi tính năng publishing được phát hành.

PO xác định ngưỡng chấp nhận cho từng KPI trước khi release.

### E. Nghiệm thu sản phẩm

PO kiểm tra:

- Feature giải quyết đúng vấn đề chưa.
- Luồng người dùng có hoàn chỉnh không.
- Error message có dễ hiểu không.
- Có bước nào gây nhầm lẫn không.
- Kết quả có đủ dùng cho người dùng mục tiêu không.
- Feature có đúng acceptance criteria không.

PO không nghiệm thu bằng việc đọc code. PO nghiệm thu bằng workflow, output và giá trị sử dụng.

### F. Quản lý thay đổi

Mỗi yêu cầu mới cần ghi:

```text
Tên yêu cầu:
Vấn đề cần giải quyết:
Người dùng hưởng lợi:
Giá trị dự kiến:
Mức ưu tiên:
Phạm vi ảnh hưởng:
Phụ thuộc:
Tiêu chí nghiệm thu:
Lý do thêm hoặc thay đổi:
```

---

## 3.2. Đầu ra bắt buộc của Product Owner

- Product brief.
- User persona.
- Problem statement.
- Scope MVP.
- Roadmap.
- Prioritized backlog.
- User story.
- Acceptance criteria.
- KPI definition.
- Release decision.
- Feedback summary.

---

## 3.3. Mẫu user story

```text
## Tên feature

### Người dùng
Là [loại người dùng],

### Nhu cầu
Tôi muốn [hành động],

### Mục đích
Để [kết quả/giá trị].

### Acceptance criteria
- [ ] ...
- [ ] ...
- [ ] ...

### Không thuộc phạm vi
- ...

### KPI liên quan
- ...
```

---

# 4. Tech Lead — quyết định xây như thế nào

## 4.1. Trách nhiệm chính

### A. Thiết kế kiến trúc

- Xác định ranh giới giữa:
  - SvelteKit UI.
  - API.
  - Browser editor.
  - AI pipeline.
  - Media engine.
  - Queue.
  - Storage.
  - Publishing.
- Quyết định local, cloud và hybrid chạy qua adapter nào.
- Đảm bảo ba nguồn mã không bị trộn trực tiếp.
- Xác định module nào dùng chung và module nào giữ riêng.

Kiến trúc mặc định:

```text
SvelteKit UI
→ API
→ Job/Artifact services
→ local runner hoặc cloud worker
→ media engine/provider adapters
```

### B. Quyết định công nghệ

Tech Lead quyết định và ghi lại:

- Svelte/SvelteKit version.
- TypeScript conventions.
- Database và migration strategy.
- Queue và worker model.
- Storage abstraction.
- FFmpeg execution model.
- CE.SDK integration boundary.
- AI provider adapter boundary.
- Local credential storage.
- CI/CD và deployment model.

Không thêm dependency nếu standard library, native API hoặc dependency hiện có đã đủ.

### C. Thiết kế contract chung

TL sở hữu các contract:

- `Transcript`.
- `Highlight`.
- `RenderPlan`.
- `Job`.
- `Artifact`.
- `PublishTarget`.
- `ProviderCredential`.

Mỗi contract phải có:

- Schema.
- Validation.
- Version.
- Compatibility rule.
- Contract test.
- Migration path.

Ví dụ nguyên tắc:

- LLM không tự quyết định timestamp cuối cùng.
- `RenderPlan` là nguồn sự thật cho preview và render.
- Artifact immutable theo version.
- Job có idempotency key.

### D. Đảm bảo local/cloud parity

- Cùng input contract.
- Cùng `RenderPlan` semantics.
- Cùng output validation.
- Khác nhau chỉ ở execution adapter.
- Có test so sánh local và cloud.

FFmpeg là final-render authority. CE.SDK dùng cho preview/editor; khác biệt phải được đo và ghi nhận.

### E. Đảm bảo reliability

Thiết kế và review:

- Retry có giới hạn.
- Timeout.
- Cancel bằng AbortSignal hoặc cơ chế tương đương.
- Resume sau restart.
- Atomic artifact write.
- Cleanup file tạm.
- Duplicate-job protection.
- Dead-letter/error state.
- Graceful worker shutdown.
- Backup và recovery.

### F. Review kỹ thuật

TL review:

- Thay đổi kiến trúc.
- Thay đổi schema/contract.
- Dependency mới.
- Code xử lý file/media.
- Auth/authorization.
- Queue/storage.
- Provider integration.
- Performance-critical code.
- Release blocker.

TL không cần review từng dòng code nếu module đã có owner và test phù hợp.

### G. Quản lý technical debt

Mỗi technical debt cần ghi:

```text
Vị trí:
Vấn đề:
Tác động:
Mức độ:
Giải pháp ngắn hạn:
Giải pháp dài hạn:
Điều kiện xử lý:
```

TL phải phân biệt:

- Bug.
- Security issue.
- Reliability issue.
- Performance issue.
- Maintainability issue.
- Feature request.

### H. Đảm bảo khả năng vận hành

TL phối hợp với Worker/DevOps để xác định:

- Metrics.
- Logs.
- Traces.
- Alert.
- Runbook.
- Capacity limit.
- Cost limit.
- Rollback strategy.

---

## 4.2. Đầu ra bắt buộc của Tech Lead

- System architecture.
- Package boundaries.
- API/data contracts.
- Architecture Decision Record (ADR).
- Dependency policy.
- Security baseline kỹ thuật.
- Reliability checklist.
- Performance budget.
- Migration plan.
- Technical debt ledger.
- Code review decisions.

---

## 4.3. Mẫu Architecture Decision Record

```text
# ADR-XXX: [Tên quyết định]

## Bối cảnh
Vấn đề hoặc ràng buộc cần giải quyết.

## Quyết định
Chọn giải pháp nào.

## Lý do
Vì sao giải pháp này phù hợp.

## Phương án đã loại bỏ
Các phương án khác và lý do không chọn.

## Tác động
- Tích cực:
- Tiêu cực:
- Chi phí:
- Rủi ro:

## Điều kiện xem xét lại
Khi nào cần thay đổi quyết định.
```

---

# 5. Cách hai role phối hợp

## Trước khi bắt đầu feature

1. PO mô tả vấn đề, người dùng và kết quả cần đạt.
2. PO xác định phạm vi và acceptance criteria.
3. TL phân tích khả thi, phụ thuộc và rủi ro.
4. TL đề xuất một hoặc vài giải pháp kỹ thuật.
5. PO chốt giá trị và ưu tiên.
6. TL chốt thiết kế triển khai.
7. Hai bên thống nhất Definition of Ready.

## Trong khi phát triển

- PO giải đáp câu hỏi về nghiệp vụ và phạm vi.
- TL giải đáp câu hỏi về kiến trúc và kỹ thuật.
- Thay đổi scope phải qua PO.
- Thay đổi contract/architecture phải qua TL.
- Thay đổi ảnh hưởng cả scope và architecture cần cả hai phê duyệt.

## Trước khi phát hành

1. QA báo cáo lỗi và kết quả test.
2. TL xác nhận technical readiness.
3. Security Owner xác nhận security readiness.
4. PO xác nhận product acceptance.
5. PO quyết định go/no-go.
6. TL chuẩn bị rollback và monitoring.

---

# 6. Definition of Ready

Feature chỉ được bắt đầu khi có:

- Người dùng mục tiêu.
- Vấn đề rõ ràng.
- Phạm vi rõ ràng.
- Acceptance criteria.
- KPI hoặc cách đo.
- Phụ thuộc đã xác định.
- Rủi ro chính đã ghi nhận.
- Thiết kế kỹ thuật sơ bộ.
- Cách kiểm thử.
- Quyết định local/cloud nếu liên quan.

# 7. Definition of Done

Feature chỉ được xem là hoàn thành khi:

- Đúng acceptance criteria.
- Có test phù hợp.
- Có error handling.
- Không lộ secret.
- Có logging/metrics cần thiết.
- Có migration nếu thay đổi data.
- Có tài liệu vận hành nếu cần.
- Đã QA trên workflow thực tế.
- PO nghiệm thu.
- TL xác nhận không phá contract/architecture.

---

# 8. Quy tắc xử lý bất đồng

1. Ghi rõ quyết định cần đưa ra.
2. PO trình bày tác động đến người dùng, giá trị và deadline.
3. TL trình bày tác động đến kiến trúc, chất lượng, chi phí và rủi ro.
4. Ưu tiên giải pháp nhỏ nhất đáp ứng mục tiêu.
5. Nếu chưa thống nhất, ghi ADR và thử nghiệm nhỏ.
6. Không giải quyết bất đồng bằng cách âm thầm trộn hai giải pháp.
7. Quyết định cuối cùng phải ghi lại trong backlog hoặc ADR.

## Nguyên tắc cuối

- **PO bảo vệ giá trị sản phẩm và phạm vi.**
- **TL bảo vệ kiến trúc, chất lượng và khả năng vận hành.**
- **Cả hai cùng chịu trách nhiệm về trade-off, nhưng mỗi người quyết định trong phạm vi của mình.**
