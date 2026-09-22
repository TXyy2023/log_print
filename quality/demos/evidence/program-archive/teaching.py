import os,time
os.write(1,b'INFO demo=program step=boot\n')
time.sleep(0.2)
os.write(2,b'WARN demo=program queue=3\n')
time.sleep(0.2)
os.write(1,b'INFO demo=program step=done\n')
