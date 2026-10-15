#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""生成测试音：左/右交替脚步声（110Hz 短促脉冲）+ 周期枪声（宽带噪声突发）。
用于在无游戏环境下验证「采集 → 检测 → 覆盖层」全链路。
输出：test_sound.wav（48kHz 立体声 16bit）
"""
import math
import random
import struct
import wave

SR = 48000
SECONDS = 12

random.seed(1)
frames = bytearray()


def put(f: float):
    frames.extend(struct.pack("<h", int(max(-1.0, min(1.0, f)) * 32767)))


for i in range(SR * SECONDS):
    t = i / SR
    v = 0.0
    # 脚步：每 0.33 秒一步，左右交替
    step_i = int(t / 0.33)
    phase = t % 0.33
    pan = -0.9 if step_i % 2 == 0 else 0.9
    if phase < 0.09:
        env = math.exp(-phase * 40)
        v += math.sin(2 * math.pi * 110 * t) * env * 0.8
    # 枪声：每 3 秒一次（1.45s 处，错开脚步时间），右前方偏右
    g = t % 3.0
    if 1.45 <= g < 1.60:
        env = math.exp(-(g - 1.45) * 30)
        v += random.uniform(-1, 1) * env
        pan = 0.5
    # 写立体声（pan: -1=全左, +1=全右）
    left = v * (1 - pan) / 2
    right = v * (1 + pan) / 2
    put(left)
    put(right)

with wave.open("test_sound.wav", "wb") as w:
    w.setnchannels(2)
    w.setsampwidth(2)
    w.setframerate(SR)
    w.writeframes(bytes(frames))
print("OK: test_sound.wav")
