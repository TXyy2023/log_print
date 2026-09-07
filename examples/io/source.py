import os
import time

for n in range(20):
    os.write(1, f"sample={n} temp={20+n/10:.1f} source_ns={time.time_ns()}\n".encode())
    if n % 5 == 0:
        os.write(2, f"diagnostic={n}\n".encode())
    time.sleep(0.05)
