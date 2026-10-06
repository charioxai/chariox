"""MP-08 / MP-10 / MP-11: screenshot the real TUI's terminal emulator buffer."""
from pathlib import Path
import pyte
from PIL import Image, ImageDraw, ImageFont

def capture(log, output):
    class TerminalScreen(pyte.Screen):
        def report_device_status(self, mode, **kwargs):
            pass  # Device queries do not change the rendered buffer.
    screen = TerminalScreen(160, 40)
    stream = pyte.Stream(screen)
    stream.feed(Path(log).read_bytes().decode('utf-8',errors='replace'))
    font_path = next((p for root in ['/usr/share/fonts/truetype', '/usr/share/fonts/opentype'] for p in Path(root).rglob('*') if p.suffix in {'.otf','.ttf'} and ('Mono' in p.name or 'mono' in p.name)), None)
    font = ImageFont.truetype(str(font_path), 14) if font_path else ImageFont.load_default(size=14)
    canvas = Image.new('RGB',(160*9,40*18),'#111111')
    draw = ImageDraw.Draw(canvas)
    for row,text in enumerate(screen.display):
        draw.text((0,row*18),text,font=font,fill='#eeeeee')
    canvas.save(output)
