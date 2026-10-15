import json
import os
import sys

from pyannote.audio import Pipeline


def main():
    if len(sys.argv) != 2:
        raise RuntimeError("DIARIZATION_ARGUMENTS")
    token = os.environ.get("HF_TOKEN", "").strip()
    if not token:
        raise RuntimeError("DIARIZATION_HF_TOKEN_REQUIRED")
    pipeline = Pipeline.from_pretrained("pyannote/speaker-diarization-3.1", use_auth_token=token)
    result = pipeline(sys.argv[1])
    intervals = []
    for segment, _, speaker in result.itertracks(yield_label=True):
        intervals.append({"start": float(segment.start), "end": float(segment.end), "speaker": str(speaker)})
    print(json.dumps(intervals, separators=(",", ":")))


if __name__ == "__main__":
    main()
