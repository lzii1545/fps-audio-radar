# -*- coding: utf-8 -*-
"""分析回放截图：ULW 覆盖层是否渲染出雷达环与方向图标"""
import math
from PIL import Image

im = Image.open("shot_replay.png").convert("RGB")
W, H = im.size
cx, cy = W // 2, H // 2
px = im.load()
print(f"size {W}x{H}")

green = []
orange = []
for y in range(0, H, 2):
    for x in range(0, W, 2):
        r, g, b = px[x, y]
        if g > 130 and r < 140 and b < 180:
            green.append((x, y))
        elif r > 190 and 40 < g < 180 and b < 120:
            orange.append((x, y))
print(f"green px: {len(green)}, orange px: {len(orange)}")
if green:
    xs = [p[0] for p in green]; ys = [p[1] for p in green]
    print(f"green range x[{min(xs)},{max(xs)}] y[{min(ys)},{max(ys)}]")
if orange:
    xs = [p[0] for p in orange]; ys = [p[1] for p in orange]
    print(f"orange range x[{min(xs)},{max(xs)}] y[{min(ys)},{max(ys)}]")


def probe(x, y, r=40, label=""):
    g = 0; o = 0; b = 0
    for dy in range(-r, r + 1, 2):
        for dx in range(-r, r + 1, 2):
            rr, gg, bb = px[x + dx, y + dy]
            if gg > 130 and rr < 140 and bb < 180:
                g += 1
            elif rr > 190 and 40 < gg < 180 and bb < 120:
                o += 1
            elif rr > 100 and gg > 100 and bb > 100:
                b += 1
    print(f"  ({x},{y}) {label}: green={g} orange={o} bright={b}")


print("== 方向图标位（dir_radius=260, y=540）==")
probe(cx - 260, cy, label="left-icon")
probe(cx + 260, cy, label="right-icon")
print("== 雷达环（r=240）==")
probe(cx - 240, cy, label="ring-left")
probe(cx + 240, cy, label="ring-right")
probe(cx, cy - 240, label="ring-top")
probe(cx, cy, label="center")
print("== 雷达事件点（r≈29, 左右）==")
probe(cx - 29, cy, label="dot-left")
probe(cx + 29, cy, label="dot-right")
