import json
import sys

import cv2


def main():
    # Batch interface: one process detects faces in every sampled frame, so the
    # per-frame Python + cv2 import cost (~1s) is paid once instead of per sample.
    # stdout: JSON array with one element per argument — the face boxes for that
    # frame, or null when the frame is unreadable (the caller skips the sample;
    # one truncated frame must not kill a long scan).
    paths = sys.argv[1:]
    if not paths:
        raise RuntimeError("FACE_DETECTION_ARGUMENTS")
    detector = cv2.CascadeClassifier(cv2.data.haarcascades + "haarcascade_frontalface_default.xml")
    if detector.empty():
        raise RuntimeError("FACE_DETECTOR_UNAVAILABLE")
    results = []
    for path in paths:
        image = cv2.imread(path)
        if image is None:
            results.append(None)
            continue
        gray = cv2.cvtColor(image, cv2.COLOR_BGR2GRAY)
        faces = detector.detectMultiScale(gray, scaleFactor=1.1, minNeighbors=5, minSize=(48, 48))
        height, width = image.shape[:2]
        results.append([
            {"x": x / width, "y": y / height, "width": w / width, "height": h / height, "confidence": min(1.0, 0.5 + w / width)}
            for x, y, w, h in faces[:4]
        ])
    print(json.dumps(results, separators=(",", ":")))


if __name__ == "__main__":
    main()
