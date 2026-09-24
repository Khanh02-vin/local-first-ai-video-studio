export type MediaProbe = {
  duration: number;
  width: number;
  height: number;
  codec?: string;
  format?: string;
};

export function validateProbe(probe: MediaProbe): void {
  if (!Number.isFinite(probe.duration) || probe.duration <= 0) throw new Error("Invalid media duration");
  if (!Number.isInteger(probe.width) || probe.width <= 0 || !Number.isInteger(probe.height) || probe.height <= 0) {
    throw new Error("Invalid media dimensions");
  }
}

// Host adapters execute ffprobe; the core only validates its result.
