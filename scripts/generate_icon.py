#!/usr/bin/env python3
"""生成 Lingua 应用图标母版（1024x1024 RGBA PNG）。

只依赖 Python 标准库。生成后用 Tauri CLI 展开成各平台图标：

    python3 scripts/generate_icon.py scripts/icon-source.png
    pnpm tauri icon scripts/icon-source.png
"""

import struct
import sys
import zlib

SIZE = 1024
CORNER = 180  # 圆角半径

TOP = (0x5B, 0x76, 0xF7)     # 靛蓝
BOTTOM = (0x8A, 0x4F, 0xED)  # 紫色


def pixel(x: int, y: int) -> tuple[int, int, int, int]:
    t = y / (SIZE - 1)
    r = round(TOP[0] + (BOTTOM[0] - TOP[0]) * t)
    g = round(TOP[1] + (BOTTOM[1] - TOP[1]) * t)
    b = round(TOP[2] + (BOTTOM[2] - TOP[2]) * t)

    dx = min(x, SIZE - 1 - x)
    dy = min(y, SIZE - 1 - y)
    if dx < CORNER and dy < CORNER:
        ex = CORNER - dx
        ey = CORNER - dy
        if ex * ex + ey * ey > CORNER * CORNER:
            return (0, 0, 0, 0)
    return (r, g, b, 255)


def chunk(tag: bytes, data: bytes) -> bytes:
    return (
        struct.pack(">I", len(data))
        + tag
        + data
        + struct.pack(">I", zlib.crc32(tag + data))
    )


def write_png(path: str) -> None:
    raw = bytearray()
    for y in range(SIZE):
        raw.append(0)  # filter: None
        for x in range(SIZE):
            raw.extend(pixel(x, y))

    ihdr = struct.pack(">IIBBBBB", SIZE, SIZE, 8, 6, 0, 0, 0)
    png = (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", ihdr)
        + chunk(b"IDAT", zlib.compress(bytes(raw), 9))
        + chunk(b"IEND", b"")
    )
    with open(path, "wb") as file:
        file.write(png)
    print(f"written: {path}")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit("usage: generate_icon.py <output.png>")
    write_png(sys.argv[1])
