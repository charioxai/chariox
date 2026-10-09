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
try:
    from Xlib import X, display
    from Xlib.ext import xtest
except ModuleNotFoundError as error:
    if error.name != 'Xlib': raise
    # MP-08 / MP-11: use the same installed XTEST backend as the keyboard
    # helper when the selected interpreter exposes the vendored package.
    from selkies.Xlib import X, display
    from selkies.Xlib.ext import xtest
from PIL import Image


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace('-', '_'), Path(__file__).with_name(name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


keyboard = load('slice-keyboard')
finder = load('slice-text-finder')


def capture(mask, uncovered=()):
    connection = load('native-x11').open_display(display)
    try:
        screen = connection.screen()
        if mask: return Image.new('RGB', (screen.width_in_pixels, screen.height_in_pixels), 'black')
        raw = screen.root.get_image(0, 0, screen.width_in_pixels, screen.height_in_pixels, X.ZPixmap, 0xffffffff)
        if raw.depth != 24: raise ValueError('unsupported display depth')
        image = Image.frombytes('RGB', (screen.width_in_pixels, screen.height_in_pixels), raw.data, 'raw', 'BGRX')
        for x, y, width, height in uncovered: image.paste((0, 0, 0), (x, y, x+width, y+height))
        return image
    finally: connection.close()


def input_action(action, processes=None):
    kind = action['kind']
    guard=None
    if processes is not None and (kind in ('hold','pointer_hold','drag','clipboard_write','keycode') or action.get('button')==2):
        raise ValueError('native paste/repeat requires human or Vault input')
    if processes is not None and kind in ('text','key','click'):
        # MP-11: any agent key, text or click may reach a Paste control
        # (chord, menu mnemonic, focused button). Gate on clipboard owner.
        accessibility=load('native-accessibility')
        admit_clipboard=load('native-clipboard').input_admission(processes,accessibility)
        admit_focus=accessibility.input_guard(processes) if kind!='click' else lambda:None
        def guard():
            admit_focus()
            admit_clipboard()
    if kind == 'text': keyboard.type_text(action['text'],before_press=guard); return
    if kind=='pointer_hold':
        keyboard.hold_input('button',{1:'left',2:'middle',3:'right'}[action['button']],action['duration_ms'],action['x'],action['y']);return
    if kind in ('key', 'hold'):
        keyboard.hold_input('key', action['key'].replace('Enter', 'Return'), action.get('duration_ms', 1),before_press=guard); return
    if kind == 'clipboard_write': raise ValueError('clipboard lifetime belongs to placement adapter')
    connection = load('native-x11').open_display(display)
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
                if guard is not None:guard()
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
        return accessibility.snapshot(request['processes'],request.get('browser_processes')) if op=='accessibility' else accessibility.act(request)
    if op == 'input': input_action(request['input'],request.get('processes',[]) if request.get('agent_input') else None); return {'applied':True}
    if op == 'release':
        connection=load('native-x11').open_display(display)
        try:
            for code in request['codes']:
                if not isinstance(code,int) or not 8 <= code <= 255: raise ValueError('invalid owned release')
                xtest.fake_input(connection,X.KeyRelease,code)
            connection.sync()
        finally: connection.close()
        return {'released':True}
    if op == 'clipboard_read':
        accessibility=load('native-accessibility')
        value=load('native-clipboard').public_clipboard(request.get('processes',[]),accessibility,request['mask'],request.get('browser_processes'))
        return {'text':'[protected]' if value is None else value[0]}
    accessibility=load('native-accessibility')
    browser_protection=request.get('browser_protection')
    values=request.get('values') or []
    before=accessibility.snapshot(request.get('processes',[]),request.get('browser_processes'),browser_protection,values)
    mask=request['mask'] or not before['available'] or not before['complete'] or before['protected']
    image=capture(mask,before.get('masks',before.get('uncovered',())))
    after=accessibility.snapshot(request.get('processes',[]),request.get('browser_processes'),browser_protection,values)
    if before!=after:
        image.close()
        raise ValueError('native protection changed during capture')
    request['mask']=mask
    try:
        if op == 'screenshot':
            encoded=io.BytesIO();image.save(encoded,format='PNG')
            return {'mime_type':'image/png','data_base64':base64.b64encode(encoded.getvalue()).decode('ascii'),'width':image.width,'height':image.height,'protected':request['mask'],'browser_withheld':before.get('browser_withheld',0)}
        if op == 'ocr':
            if request['mask']: return {'text':'[protected]','targets':[]}
            with tempfile.TemporaryDirectory(prefix='chariox-native-ocr-') as root:
                name=str(Path(root)/'protected.png');image.save(name)
                output=io.StringIO()
                from contextlib import redirect_stdout
                with redirect_stdout(output): finder.recognize_image(name,request.get('query'))
                value=output.getvalue().strip()
                result={'targets':[json.loads(line) for line in value.splitlines() if line!='null']} if request.get('query') else {'text':value[:65536]}
                return {**result,'browser_withheld':before.get('browser_withheld',0)}
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
