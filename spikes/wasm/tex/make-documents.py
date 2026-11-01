#!/usr/bin/env python3
"""MP-08/MP-10: authored compiler corpus, not a claim of UI acceptance."""
from pathlib import Path
import struct,zlib,sys
out=Path(sys.argv[1]);out.mkdir(parents=True,exist_ok=True)
def chunk(t,data): return struct.pack('>I',len(data))+t+data+struct.pack('>I',zlib.crc32(t+data)&0xffffffff)
w,h=256,128
raw=b''.join(b'\x00'+b''.join(bytes((x%256,y*2%256,(x+y)%256)) for x in range(w)) for y in range(h))
png=b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',w,h,8,2,0,0,0))+chunk(b'IDAT',zlib.compress(raw))+chunk(b'IEND',b'')
hello=r'\documentclass{article}\begin{document}Hello, world! $E=mc^2$.\end{document}'
preamble=r'''\documentclass{article}
\usepackage{amsmath,graphicx,tikz,hyperref}
\title{A reproducible typesetting experiment}\author{Chariox}\date{}
\begin{document}\maketitle
'''
paragraph='We compare a portable compiler with the native engine using the same sources, fonts, figures and bibliography. The experiment records cache state and preserves the document structure. '
paper=preamble
for n in range(1,11):
 if n>1:paper+='\n\\newpage\n'
 paper+=f'\\section{{Experiment {n}}}\\label{{sec:{n}}}\n'
 paper+=(paragraph*4+'\n\n')*2
 paper+=r'\begin{align} E &= mc^2, & \int_0^1 x^2\,dx &= \frac13.\end{align}'+'\n'
 paper+=r'Following Knuth~\cite{knuth1984}; see Section~\ref{sec:1}.'+'\n'
 if n==2:paper+=r'\begin{figure}[h]\centering\includegraphics[width=.6\linewidth]{figure.png}\caption{A generated PNG.}\label{fig:png}\end{figure}'+'\n'
 if n==3:paper+=r'\begin{figure}[h]\centering\begin{tikzpicture}\draw[->](0,0)--(3,0)node[right]{$x$};\draw[->](0,0)--(0,2)node[above]{$y$};\draw[blue,thick](0,0)parabola(2,1);\end{tikzpicture}\caption{A TikZ diagram.}\end{figure}'+'\n'
paper+=r'\bibliographystyle{plain}\bibliography{references}\end{document}'
bib='@book{knuth1984, author={Donald E. Knuth}, title={The TeXbook}, year={1984}, publisher={Addison-Wesley}}\n'
deck=r'\documentclass{beamer}\usepackage{amsmath,graphicx}\title{Portable typesetting}\begin{document}\frame{\titlepage}'
for n in range(1,7):deck+=f'\\begin{{frame}}{{Slide {n}}}'+r'$E=mc^2$\par\includegraphics[width=.6\linewidth]{figure.png}\end{frame}'
deck+=r'\end{document}'
for name,text in [('hello',hello),('paper',paper),('beamer',deck),('broken',r'\documentclass{article}'+'\n'+r'\begin{document}'+'\n'+r'\undefinedcommand{Broken}'+'\n'+r'\end{document}')]:
 d=out/name;d.mkdir(exist_ok=True);(d/'main.tex').write_text(text+'\n');(d/'figure.png').write_bytes(png);(d/'references.bib').write_text(bib)
d=out/'typst';d.mkdir(exist_ok=True);(d/'figure.png').write_bytes(png);(d/'references.bib').write_text(bib)
typ='#set page(paper: "a4", margin: 24mm)\n#set text(size: 10pt)\n= A reproducible typesetting experiment\n'
for n in range(1,11):
 if n>1:typ+='\n#pagebreak()\n'
 typ+=f'== Experiment {n}\n'+(paragraph*4+'\n\n')*2+'\n$ E = m c^2 quad integral_0^1 x^2 dif x = 1/3 $\nFollowing @knuth1984.\n'
 if n==2:typ+='\n#figure(image("figure.png", width: 60%), caption: [A generated PNG.])\n'
 if n==3:typ+='\n#figure(box(width: 65mm, height: 25mm, stroke: blue)[A vector diagram $x arrow y$], caption: [A native vector diagram; not TikZ.])\n'
typ+='\n#bibliography("references.bib")\n';(d/'main.typ').write_text(typ)
print(out)
