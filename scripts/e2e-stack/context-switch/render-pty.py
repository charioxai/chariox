# MP-08 / MP-10 / MP-11: replay captured real PTY bytes, never construct test UI text.
import re,sys
from pathlib import Path
from PIL import Image,ImageDraw,ImageFont
rows,cols=48,160
font=ImageFont.truetype('/usr/share/fonts/opentype/urw-base35/NimbusMonoPS-Regular.otf',16)
for arg in sys.argv[1:]:
 p=Path(arg);text=p.read_text(errors='replace');screen=[[' ']*cols for _ in range(rows)];r=c=0;saved=(0,0)
 tokens=re.findall(r'\x1b\[[0-?]*[ -/]*[@-~]|\x1b\][^\x07]*(?:\x07|\x1b\\)|\x1b.|[^\x1b]',text)
 for token in tokens:
  if token.startswith('\x1b['):
   op=token[-1];params=token[2:-1].lstrip('?');parts=[int(x) if x.isdigit() else 0 for x in params.split(';')];n=(parts[0] or 1) if parts else 1
   if op in 'Hf':r=max(0,min(rows-1,(parts[0] or 1)-1));c=max(0,min(cols-1,((parts[1] or 1) if len(parts)>1 else 1)-1))
   elif op=='A':r=max(0,r-n)
   elif op=='B':r=min(rows-1,r+n)
   elif op=='C':c=min(cols-1,c+n)
   elif op=='D':c=max(0,c-n)
   elif op=='G':c=max(0,min(cols-1,n-1))
   elif op=='d':r=max(0,min(rows-1,n-1))
   elif op=='J' and parts[0] in (2,3):screen=[[' ']*cols for _ in range(rows)]
   elif op=='K':
    if parts[0]==2:screen[r]=[' ']*cols
    elif parts[0]==1:screen[r][:c+1]=[' ']*(c+1)
    else:screen[r][c:]=[' ']*(cols-c)
   elif op=='s':saved=(r,c)
   elif op=='u':r,c=saved
  elif token.startswith('\x1b'):continue
  elif token=='\r':c=0
  elif token=='\n':
   r+=1
   if r>=rows:screen.pop(0);screen.append([' ']*cols);r=rows-1
  elif token=='\b':c=max(0,c-1)
  elif token=='\t':c=min(cols-1,(c//8+1)*8)
  elif token.isprintable():
   screen[r][c]=token;c+=1
   if c>=cols:c=0;r=min(rows-1,r+1)
 img=Image.new('RGB',(cols*10+24,rows*21+24),(12,15,20));draw=ImageDraw.Draw(img)
 for row,line in enumerate(screen):draw.text((12,row*21+12),''.join(line),font=font,fill=(220,227,235))
 img.save(p.with_suffix('.png'))
 print('MP-08 MP-10 MP-11 real PTY replay:',p.with_suffix('.png'))
