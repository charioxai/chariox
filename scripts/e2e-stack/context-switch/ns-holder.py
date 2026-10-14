# MP-08 / MP-10 / MP-11: exact owned namespace holder.
import time,os,sys
from pathlib import Path
(Path(sys.argv[1])/'holder-ready').write_text(str(os.getpid()))
while True: time.sleep(1)
