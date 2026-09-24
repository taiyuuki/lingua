#!/usr/bin/env python3
"""Lingua OCR sidecar —— 用 PaddleOCR 实现本地 OCR HTTP 契约。

用法（首次需安装依赖）：

    pip install "paddleocr>=2.7" paddlepaddle flask
    python3 scripts/paddleocr_server.py            # 监听 127.0.0.1:8433

契约（与 lingua-ocr::HttpOcrProvider 对应）：

    POST /ocr
    {"imageBase64": "<PNG base64>", "languageHints": ["en"]}
    → {"blocks": [{"text", "bbox": [x,y,w,h], "confidence", "lines"?}]}
"""

import base64

import numpy as np
from flask import Flask, jsonify, request

try:
    from paddleocr import PaddleOCR
except ImportError as error:  # pragma: no cover
    raise SystemExit(
        "paddleocr is not installed: pip install 'paddleocr>=2.7' paddlepaddle flask"
    ) from error

app = Flask(__name__)
# 首次调用会自动下载模型；lang="en" 覆盖中英混排的英文侧，
# 需要更好中文效果可改 "ch"（PaddleOCR 的 ch 模型同时支持英文）。
ocr = PaddleOCR(lang="en", show_log=False)


@app.post("/ocr")
def ocr_endpoint():
    payload = request.get_json(silent=True) or {}
    image_b64 = payload.get("imageBase64", "")
    if not image_b64:
        return jsonify({"error": "imageBase64 is required"}), 400

    try:
        image_bytes = base64.b64decode(image_b64)
        buffer = np.frombuffer(image_bytes, dtype=np.uint8)
    except Exception:
        return jsonify({"error": "invalid base64 image"}), 400

    import cv2  # paddleocr 依赖里自带

    image = cv2.imdecode(buffer, cv2.IMREAD_COLOR)
    if image is None:
        return jsonify({"error": "cannot decode image"}), 400

    result = ocr.ocr(image, cls=True)
    blocks = []
    # paddleocr 返回: [ [ [box4pts, (text, conf)], ... ] ]（每页一个列表）
    for page in result or []:
        for detection in page or []:
            box, (text, confidence) = detection[0], detection[1]
            xs = [int(point[0]) for point in box]
            ys = [int(point[1]) for point in box]
            x, y = min(xs), min(ys)
            w, h = max(xs) - x, max(ys) - y
            blocks.append(
                {
                    "text": text,
                    "bbox": [x, y, w, h],
                    "confidence": round(float(confidence), 4),
                    "lines": [
                        {
                            "text": text,
                            "bbox": [x, y, w, h],
                            "confidence": round(float(confidence), 4),
                            "words": text.split(),
                        }
                    ],
                }
            )

    return jsonify({"blocks": blocks, "language": None})


if __name__ == "__main__":
    app.run(host="127.0.0.1", port=8433, debug=False)
