# Runbook: Code Signing & Notarization

Trạng thái: **chuẩn bị sẵn, chưa chạy thật** — phần sign thật cần macOS + Apple Developer ID + cert.

## macOS (Developer ID Application)

1. Tạo cert trong Keychain Access: `Certificates → Signing → Developer ID Application`.
2. Export PKCS12: Keychain Access → key `Developer ID Application: ...` → Export `.p12` (đặt mật khẩu).
3. Config secrets GitHub:
   - `APPLE_IDENTITY` = `base64 signing.p12`
   - `APPLE_IDENTITY_PASSWORD` = mật khẩu p12
   - `APPLE_TEAM_ID`, `APPLE_ID` (Apple ID), `APPLE_PASSWORD` (App-specific password).
4. Push tag `v*` → workflow `release.yml` (job `macos`) build + sign + notarize, upload `.app`.

## Windows (Tauri updater signing)

Tauri 2 dùng key updater. Tạo bằng `npx tauri signer generate` rồi đặt:
- `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.

Job `windows` build MSI/NSIS và sign (nếu có key).

## Linux

Không sign (định dạng deb/rpm/AppImage). Job `linux` chỉ build + upload.

## Kiểm chứng nhanh

- macOS: `codesign --verify --deep --strict --verbose=2 app.app`
- Notarization: kết quả `notarytool` trong log CI.
- Windows: `signtool verify /pa app.exe`.
