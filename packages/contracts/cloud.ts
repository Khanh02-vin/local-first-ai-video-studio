export type AuthContext = { userId: string; tenantId: string; projectIds?: string[] };

export function assertProjectAccess(auth: AuthContext, tenantId: string, projectId: string, allowedProjectIds?: string[]): void {
  if (auth.tenantId !== tenantId) throw new Error("TENANT_ACCESS_DENIED");
  if (allowedProjectIds && !allowedProjectIds.includes(projectId)) throw new Error("PROJECT_ACCESS_DENIED");
}

export type UploadDescriptor = { tenantId: string; projectId: string; artifactId: string; contentType: string; size: number; checksum?: string };
export type SignedUrl = { url: string; expiresAt: string };

export function validateUploadDescriptor(upload: UploadDescriptor, maxBytes: number): void {
  if (!upload.tenantId || !upload.projectId || !upload.artifactId) throw new Error("UPLOAD_SCOPE_REQUIRED");
  if (!upload.contentType.startsWith("video/")) throw new Error("UNSUPPORTED_MEDIA_TYPE");
  if (!Number.isInteger(upload.size) || upload.size <= 0 || upload.size > maxBytes) throw new Error("UPLOAD_SIZE_LIMIT");
}
