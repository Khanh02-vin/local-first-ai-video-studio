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
    cascade_path = cv2.data.haarcascades + "haarcascade_frontalface_default.xml"
    detector = cv2.CascadeClassifier(cascade_path)
    if detector.empty():
        raise RuntimeError("FACE_DETECTOR_UNAVAILABLE")
    faces = detector.detectMultiScale(gray, scaleFactor=1.1, minNeighbors=5, minSize=(48, 48))
    height, width = image.shape[:2]
    result = []
    for x, y, w, h in faces[:4]:
        pad_x, pad_y = int(w * 0.18), int(h * 0.18)
        left, top = max(0, x - pad_x), max(0, y - pad_y)
        right, bottom = min(width, x + w + pad_x), min(height, y + h + pad_y)
        ok, encoded = cv2.imencode(".jpg", image[top:bottom, left:right], [cv2.IMWRITE_JPEG_QUALITY, 82])
        if ok:
            result.append({"x": int(x), "y": int(y), "width": int(w), "height": int(h), "thumbnail": encoded.tobytes().hex()})
    print(json.dumps(result, separators=(",", ":")))


if __name__ == "__main__":
    main()
