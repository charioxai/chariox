"""MD-DISPLAY-02: real encoder packet budget and dependency regression."""
import base64
import io
import json
from pathlib import Path
import subprocess
import sys
import unittest
from PIL import Image, ImageChops, ImageDraw, ImageFont


class EncoderBudget(unittest.TestCase):
    def test_dense_scroll_packet_budget_and_deltas(self):
        image = Image.new('RGB', (1280, 800), 'white')
        draw = ImageDraw.Draw(image)
        font = ImageFont.load_default(size=16)
        for y in range(24, 800, 24):
            draw.text((28, y), 'Kernel authority and encrypted pixels: ' * 3, font=font, fill='#182331')
        requests = []
        for index in range(60):
            data = io.BytesIO()
            ImageChops.offset(image, 0, -index * 20).save(data, format='PNG')
            requests.append(json.dumps({'png': base64.b64encode(data.getvalue()).decode(),
                                        'bitrate': 2_000_000, 'reset': index == 0}))
        child = subprocess.Popen([sys.executable, '-u', str(Path(__file__).with_name('kernel-browser-encoder.py'))],
                                 stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        try:
            output, _ = child.communicate('\n'.join(requests) + '\n', timeout=30)
        finally:
            if child.poll() is None:
                if not isinstance(child.pid, int) or child.pid <= 1:
                    raise RuntimeError('MD-DISPLAY: refuse unsafe child PID')
                child.kill()
                child.wait(timeout=5)
        self.assertEqual(child.returncode, 0)
        packets = [json.loads(line) for line in output.splitlines()]
        self.assertEqual(len(packets), 60)
        self.assertTrue(packets[0]['key'])
        self.assertTrue(any(not packet['key'] for packet in packets[1:]))
        sizes = [len(base64.b64decode(packet['data_base64'])) for packet in packets]
        self.assertLess(max(sizes), 64 * 1024, 'dense keyframe with outer base64 must fit half a second of budget')
        self.assertLess(sum(sizes) * 8 / 2, 1_000_000, '60 frames at the 30fps timebase must fit the reserved media budget')


if __name__ == '__main__':
    unittest.main()
