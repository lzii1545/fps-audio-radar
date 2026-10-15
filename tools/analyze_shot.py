# -*- coding: utf-8 -*-
"""决定性检验：雷达环是否渲染（低阈值采样 r=240 圆环）"""
import math
from PIL import Image

im = Image.open("shot.png").convert("RGB")
W, H = im.size
cx, cy = W // 2, H // 2
px = im.load()

# 以屏幕中心为圆心，r=240 处精确采样（±3px 内找局部最亮）
hits = []
for deg in range(0, 360, 5):
    x = int(cx + 240 * math.sin(math.radians(deg)))
    y = int(cy - 240 * math.cos(math.radians(deg)))
    best = 0
    for dx in range(-3, 4):
        for dy in range(-3, 4):
            r, g, b = px[x + dx, y + dy]
            best = max(best, r + g + b)
    hits.append(best)
lit = [h for h in hits if h > 200]
print(f"r=240 采样 {len(hits)} 点, 亮于 200 的有 {len(lit)} 点, 最亮 {max(hits)}")

# 背景参照：屏幕四角
for (nx, ny, label) in [(50, 50, "TL"), (W - 50, 50, "TR"), (50, H - 50, "BL"), (W - 50, H - 50, "BR")]:
    r, g, b = px[nx, ny]
    print(f"corner {label}: ({r},{g},{b})")

# 中心 3x3
for dy in range(-1, 2):
    row = [px[cx + dx, cy + dy] for dx in range(-1, 2)]
    print("center row:", row)
