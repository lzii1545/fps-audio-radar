# -*- coding: utf-8 -*-
"""差分 quiet/sound 截图，定位事件渲染位置"""
from PIL import Image

a = Image.open("shot_quiet.png").convert("RGB")
b = Image.open("shot_sound.png").convert("RGB")
W, H = a.size
pa, pb = a.load(), b.load()

changed = []
for y in range(0, H, 2):
    for x in range(0, W, 2):
        ra, ga, ba = pa[x, y]
        rb, gb, bb = pb[x, y]
        if abs(ra - rb) + abs(ga - gb) + abs(ba - bb) > 60:
            changed.append((x, y))

print(f"changed pixels (sampled): {len(changed)}")
if changed:
    xs = [p[0] for p in changed]
    ys = [p[1] for p in changed]
    print(f"changed x range: [{min(xs)},{max(xs)}] y range: [{min(ys)},{max(ys)}]")

    # 聚类：按 80px 网格统计
    from collections import Counter
    grid = Counter((x // 80, y // 80) for x, y in changed)
    print("changed clusters (grid80):")
    for (gx, gy), n in sorted(grid.items(), key=lambda kv: -kv[1])[:12]:
        print(f"  grid({gx * 80}-{gx * 80 + 80}, {gy * 80}-{gy * 80 + 80}): {n}")

# 设置窗口内 vs 外
win_rect = (208, 208, 1164, 967)
inside = sum(1 for x, y in changed if win_rect[0] <= x <= win_rect[2] and win_rect[1] <= y <= win_rect[3])
print(f"inside settings window: {inside}, outside: {len(changed) - inside}")

# 关键点位 quiet vs sound 直接对比
cx, cy = W // 2, H // 2
for (x, y, label) in [(700, 540, "dir-left"), (1220, 540, "dir-right"),
                      (cx, cy, "center"), (cx - 240, cy, "ring-left"), (cx + 240, cy, "ring-right")]:
    print(f"{label}: quiet={pa[x, y]} sound={pb[x, y]}")
