import json
import sys

import cv2


def main():
    if len(sys.argv) != 2:
        raise RuntimeError("FACE_DETECTION_ARGUMENTS")
    image = cv2.imread(sys.argv[1])
    if image is None:
        raise RuntimeError("FACE_FRAME_UNREADABLE")
    gray = cv2.cvtColor(image, cv2.COLOR_BGR2GRAY)
    detector = cv2.CascadeClassifier(cv2.data.haarcascades + "haarcascade_frontalface_default.xml")
    if detector.empty():
        raise RuntimeError("FACE_DETECTOR_UNAVAILABLE")
    faces = detector.detectMultiScale(gray, scaleFactor=1.1, minNeighbors=5, minSize=(48, 48))
    height, width = image.shape[:2]
    print(json.dumps([
        {"x": x / width, "y": y / height, "width": w / width, "height": h / height, "confidence": min(1.0, 0.5 + w / width)}
        for x, y, w, h in faces[:4]
    ], separators=(",", ":")))


if __name__ == "__main__":
    main()
