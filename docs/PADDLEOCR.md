# PaddleOCR Sidecar 部署

`scripts/paddleocr_server.py` 是「本地 OCR HTTP 服务」的实现：用官方
`paddleocr` 包起一个只监听本机的 HTTP 服务，供 Lingua 的
「OCR 引擎 → 本地 OCR HTTP 服务」选项调用。适合想要比 tesseract 更好的
中文识别效果、又不想把截图发给云端 AI 的场景。

## 安装与启动

```bash
pip install "paddleocr>=2.7" paddlepaddle flask
python3 scripts/paddleocr_server.py        # 监听 127.0.0.1:8433
```

* 首次调用会自动下载模型（几百 MB，缓存在 `~/.paddleocr`）；
* 默认 `lang="en"`，中英混排建议改成 `lang="ch"`（ch 模型同时支持英文），
  见脚本内注释；
* GPU 加速：把 `paddlepaddle` 换成 `paddlepaddle-gpu`（需要本机 CUDA
  环境匹配）；纯 CPU 也能跑，单张截图延迟在秒级。

## 在 Lingua 中使用

「设置 → OCR 引擎」选择**本地 OCR HTTP 服务**，服务地址保持默认
`http://127.0.0.1:8433/ocr`（脚本默认端口）。

## HTTP 契约

与 `lingua-ocr::HttpOcrProvider` 对应，自建服务照此实现即可：

```text
POST /ocr
{"imageBase64": "<PNG base64>", "languageHints": ["en"]}
→ {"blocks": [{"text", "bbox": [x, y, w, h], "confidence", "lines"?}]}
```

* `bbox` 为图片像素坐标，`confidence` 归一到 0..1；
* 空结果返回 `{"blocks": []}`，不要报错。

## 与 Vision 兜底的关系

PaddleOCR 是普通 OCR（快、本地、含 bbox/置信度）；Vision LLM 是
高级兜底（普通 OCR 空结果 / 低置信度时按路由派给视觉能力 Provider）。
两者叠加：PaddleOCR 识别不出 → 自动尝试 Vision。参见
`docs/ARCHITECTURE.md` §10。
