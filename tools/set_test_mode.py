# -*- coding: utf-8 -*-
"""把应用切换到测试回放模式（写入 settings.json，UTF-8 无 BOM）"""
import json
import os

p = os.path.join(os.environ["APPDATA"], "FPSAudioRadar", "settings.json")
os.makedirs(os.path.dirname(p), exist_ok=True)
with open(p, "w", encoding="utf-8") as f:
    json.dump({"device_name": "（测试）WAV 回放"}, f, ensure_ascii=False)
print("test mode set:", p)
