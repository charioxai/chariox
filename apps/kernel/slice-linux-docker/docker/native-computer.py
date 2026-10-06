"""MP-08 / MP-11: shared host/slice native operations; no lifecycle authority."""
import base64
import importlib.util
import io
import json
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
from Xlib import X, display
from Xlib.ext import xtest
from PIL import Image


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace('-', '_'), Path(__file__).with_name(name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


keyboard = load('slice-keyboard')
finder = load('slice-text-finder')


def capture(mask):
    connection = display.Display()
    try:
        screen = connection.screen()
        if mask: return Image.new('RGB', (screen.width_in_pixels, screen.height_in_pixels), 'black')
        raw = screen.root.get_image(0, 0, screen.width_in_pixels, screen.height_in_pixels, X.ZPixmap, 0xffffffff)
        if raw.depth != 24: raise ValueError('unsupported display depth')
        return Image.frombytes('RGB', (screen.width_in_pixels, screen.height_in_pixels), raw.data, 'raw', 'BGRX')
    finally: connection.close()


def input_action(action):
    kind = action['kind']
    if kind == 'text': keyboard.type_text(action['text']); return
    if kind=='pointer_hold':
        keyboard.hold_input('button',{1:'left',2:'middle',3:'right'}[action['button']],action['duration_ms'],action['x'],action['y']);return
    if kind in ('key', 'hold'):
        key = action['key'].replace('Enter', 'Return')
        keyboard.hold_input('key', key, action.get('duration_ms', 1)); return
    if kind == 'clipboard_write': raise ValueError('clipboard lifetime belongs to placement adapter')
    connection = display.Display()
    try:
        screen = connection.screen()
        if kind == 'keycode':
            code = action['keycode']
            if not 8 <= code <= 255 or action['state'] not in ('down','up'): raise ValueError('invalid keycode')
            xtest.fake_input(connection, X.KeyPress if action['state'] == 'down' else X.KeyRelease, code)
        else:
            x, y = action['x'], action['y']
            if not 0 <= x < screen.width_in_pixels or not 0 <= y < screen.height_in_pixels: raise ValueError('outside viewport')
            xtest.fake_input(connection, X.MotionNotify, x=x, y=y)
            if kind in ('click','drag'):
                button = action.get('button',1)
                if button not in (1,2,3): raise ValueError('invalid button')
                try:
                    xtest.fake_input(connection, X.ButtonPress, button)
                    if kind == 'drag':
                        tx, ty = action['to_x'], action['to_y']
                        if not 0 <= tx < screen.width_in_pixels or not 0 <= ty < screen.height_in_pixels: raise ValueError('outside viewport')
                        xtest.fake_input(connection, X.MotionNotify, x=tx, y=ty)
                finally: xtest.fake_input(connection, X.ButtonRelease, button)
            elif kind == 'scroll':
                steps = action['steps']
                if not 1 <= abs(steps) <= 100: raise ValueError('invalid scroll')
                for _ in range(abs(steps)):
                    xtest.fake_input(connection, X.ButtonPress, 5 if steps>0 else 4)
                    xtest.fake_input(connection, X.ButtonRelease, 5 if steps>0 else 4)
            elif kind != 'move': raise ValueError('unsupported native input')
        connection.sync()
    finally: connection.close()


def main(request):
    op = request['op']
    if op in ('accessibility','accessibility_action'):
        accessibility=load('native-accessibility')
        return accessibility.snapshot(request['processes']) if op=='accessibility' else accessibility.act(request)
    if op == 'input': input_action(request['input']); return {'applied':True}
    if op == 'release':
        connection=display.Display()
        try:
            for code in request['codes']:
                if not isinstance(code,int) or not 8 <= code <= 255: raise ValueError('invalid owned release')
                xtest.fake_input(connection,X.KeyRelease,code)
            connection.sync()
        finally: connection.close()
        return {'released':True}
    if op == 'clipboard_read':
        accessibility=load('native-accessibility')
        coverage=accessibility.snapshot(request.get('processes',[]))
        if request['mask'] or not coverage['available'] or not coverage['complete'] or coverage['protected']: return {'text':'[protected]'}
        result=subprocess.run(['xclip','-selection','clipboard','-o'],check=True,capture_output=True,timeout=2)
        if len(result.stdout)>65536: raise ValueError('clipboard too large')
        return {'text':result.stdout.decode('utf-8')}
    accessibility=load('native-accessibility')
    before=accessibility.snapshot(request.get('processes',[]))
    mask=request['mask'] or not before['available'] or not before['complete'] or before['protected']
    image=capture(mask)
    after=accessibility.snapshot(request.get('processes',[]))
    if before!=after:
        image.close()
        raise ValueError('native protection changed during capture')
    request['mask']=mask
    try:
        if op == 'screenshot':
            encoded=io.BytesIO();image.save(encoded,format='PNG')
            return {'mime_type':'image/png','data_base64':base64.b64encode(encoded.getvalue()).decode('ascii'),'width':image.width,'height':image.height,'protected':request['mask']}
        if op == 'ocr':
            if request['mask']: return {'text':'[protected]','targets':[]}
            with tempfile.TemporaryDirectory(prefix='chariox-native-ocr-') as root:
                name=str(Path(root)/'protected.png');image.save(name)
                output=io.StringIO()
                from contextlib import redirect_stdout
                with redirect_stdout(output): finder.recognize_image(name,request.get('query'))
                value=output.getvalue().strip()
                return {'targets':[json.loads(line) for line in value.splitlines() if line!='null']} if request.get('query') else {'text':value[:65536]}
        raise ValueError('unsupported observation')
    finally: image.close()


def keyboard_channel():
    held=set()
    print(json.dumps({'ready':True}),flush=True)
    try:
        for line in sys.stdin:
            if len(line)>2048:raise ValueError('oversized physical event')
            request=json.loads(line)
            if request['op']=='input' and request['input']['kind']=='keycode':
                action=request['input'];code=action['keycode']
                if action['state']=='down':held.add(code)
                result=main(request)
                if action['state']=='up':held.discard(code)
            elif request['op']=='release':
                result=main(request);held.difference_update(request['codes'])
            else:raise ValueError('unsupported physical channel operation')
            print(json.dumps({'id':request['id'],'ok':True,'result':result}),flush=True)
    finally:
        if held:
            try:main({'op':'release','codes':list(held)})
            except Exception:pass  # Dead display; desktop teardown remains mandatory.


if __name__=='__main__':
    def terminate(number,_): raise SystemExit(128+number)
    for number in (signal.SIGTERM,signal.SIGINT): signal.signal(number,terminate)
    try:
        if sys.argv[1:]==['--keyboard-channel']:
            keyboard_channel();sys.exit(0)
        request=json.loads(sys.stdin.buffer.read(131073))
        print(json.dumps(main(request),ensure_ascii=False,separators=(',',':')))
    except Exception as error:
        # Exception class only: never expose native text, paths or request bytes.
        print('MP-08: native operation '+type(error).__name__,file=sys.stderr)
        sys.exit(1)
