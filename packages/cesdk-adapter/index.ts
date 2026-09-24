export type CesdkEngine = { dispose?: () => void | Promise<void>; [key: string]: unknown };
export type CesdkConfig = { license?: string; baseURL?: string };

export async function createEngine(config: CesdkConfig): Promise<CesdkEngine> {
  if (!config.baseURL && !config.license) throw new Error("CESDK_CONFIG_REQUIRED");
  // The commercial CE.SDK package is intentionally peer-loaded by the web app.
  // This adapter keeps the media core independent and fails clearly when not installed/configured.
  const loader = (globalThis as { __CREATE_CESDK_ENGINE__?: (config: CesdkConfig) => Promise<CesdkEngine> }).__CREATE_CESDK_ENGINE__;
  if (!loader) throw new Error("CESDK_RUNTIME_NOT_CONFIGURED");
  return loader(config);
}

export async function disposeEngine(engine: CesdkEngine | undefined): Promise<void> { await engine?.dispose?.(); }
