"""Portable demonstration source; unbuffered writes, no hardware required."""
import math
import os
import time

index = 0
while True:
    data = {"temperature": round(24 + 3 * math.sin(index / 20), 3), "voltage": round(3.3 + 0.2 * math.cos(index / 15), 3)}
    line = f'temperature={data["temperature"]} voltage={data["voltage"]}\n'.encode()
    os.write(1, line)
    index += 1
    time.sleep(0.1)
