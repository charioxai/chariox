# MP-10/MP-11: capture the real CLI in a PTY; render its captured VT screen.
import os, sys, pty, subprocess, fcntl, termios, struct, select, re

def render(source, output):
    from PIL import Image, ImageDraw, ImageFont
    rows, cols = 42, 132
    screen = [[' '] * cols for _ in range(rows)]
    x = y = 0
    text = open(source, 'rb').read().decode('utf8', errors='replace')
    tokens = re.findall(r'\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b\[[0-?]*[ -/]*[@-~]|\x1b.|[^\x1b]', text, re.DOTALL)
    for token in tokens:
        if token.startswith('\x1b['):
            command = token[-1]
            params = token[2:-1]
            nums = [int(n) if n.isdigit() else 0 for n in params.lstrip('?').split(';')]
            n = nums[0] or 1
            if command in 'Hf': y, x = (nums[0] or 1)-1, ((nums[1] if len(nums)>1 else 1) or 1)-1
            elif command == 'A': y -= n
            elif command == 'B': y += n
            elif command == 'C': x += n
            elif command == 'D': x -= n
            elif command == 'G': x = n-1
            elif command == 'J' and nums[0] in [2,3]: screen = [[' '] * cols for _ in range(rows)]
            elif command == 'K':
                a, b = (0, cols) if nums[0]==2 else ((0,x+1) if nums[0]==1 else (x,cols))
                screen[max(0,min(rows-1,y))][a:b] = [' '] * (b-a)
            x, y = max(0,min(cols-1,x)), max(0,min(rows-1,y))
        elif token.startswith('\x1b'): continue
        elif token == '\r': x = 0
        elif token == '\n': y = min(rows-1,y+1)
        elif token == '\b': x = max(0,x-1)
        elif token >= ' ':
            screen[y][x] = token
            x = min(cols-1,x+1)
    try: font = ImageFont.truetype('DejaVuSansMono.ttf', 15)
    except OSError: font = ImageFont.load_default(size=15)
    image = Image.new('RGB', (cols*9+24,rows*19+24), '#15191f')
    draw = ImageDraw.Draw(image)
    for i,line in enumerate(screen): draw.text((12,12+i*19), ''.join(line), fill='#dce2ea', font=font)
    image.save(output)

if sys.argv[1] == '--render':
    render(sys.argv[2], sys.argv[3])
else:
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH',42,132,0,0))
    child = subprocess.Popen(sys.argv[1:], stdin=slave, stdout=slave, stderr=slave)
    os.close(slave)
    while child.poll() is None:
        if select.select([master],[],[],0.1)[0]:
            try: data = os.read(master, 65536)
            except OSError: break
            if not data: break
            sys.stdout.buffer.write(data); sys.stdout.buffer.flush()
    os.close(master)
    sys.exit(child.wait())
