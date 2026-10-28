#!/usr/bin/env python3
"""MP-08/MP-10 research measurements; no product acceptance claims."""
import argparse,json,pathlib,os,shutil,subprocess,time

def main():
 p=argparse.ArgumentParser();p.add_argument('--state',required=True);p.add_argument('--evidence',required=True);p.add_argument('--tectonic',required=True);p.add_argument('--typst',required=True);a=p.parse_args()
 state=pathlib.Path(a.state);ev=pathlib.Path(a.evidence);rows=[]
 def run(command,label,env):
  log=ev/(label+'.log');start=time.perf_counter()
  with log.open('w') as output:r=subprocess.run(['/usr/bin/time','-v',*command],env=env,stdout=output,stderr=subprocess.STDOUT,timeout=300)
  text=log.read_text();rss=next((int(s.split(':')[-1]) for s in text.splitlines() if 'Maximum resident set size' in s),None)
  return {'label':label,'command':command,'exit':r.returncode,'wallMs':(time.perf_counter()-start)*1000,'peakRssKiB':rss,'log':str(log)}
 for engine in ['tectonic','typst']:
  for doc in (['hello','paper','beamer'] if engine=='tectonic' else ['typst']):
   for n in range(1,4):
    work=state/f'bench-{engine}-{doc}-{n}';shutil.copytree(state/'documents'/doc,work,dirs_exist_ok=True)
    for f in work.iterdir():
     if f.suffix not in ['.tex','.typ','.png','.bib']:f.unlink()
    cache=state/f'cache-{engine}-{doc}-{n}';env={**os.environ,'CHARIOX_HOME':str(state),'XDG_CACHE_HOME':str(cache),'XDG_CONFIG_HOME':str(state/'config'),'TMPDIR':str(state/'tmp')}
    source=work/('main.tex' if engine=='tectonic' else 'main.typ')
    command=[a.tectonic,'--keep-logs','--keep-intermediates',str(source)] if engine=='tectonic' else [a.typst,'compile',str(source),str(work/'main.pdf')]
    for phase in ['cold','warm','edit']:
     if phase=='edit':
      text=source.read_text();source.write_text(text.replace('\\end{document}','One line edited.\n\\end{document}') if engine=='tectonic' else text+'\nOne line edited.\n')
     label=f'native-{engine}-{doc}-{n}-{phase}';row=run(command,label,env);row.update(engine=engine,document=doc,run=n,phase=phase)
     row['pdfBytes']=(work/'main.pdf').stat().st_size if (work/'main.pdf').exists() else None
     if phase=='cold' and engine=='tectonic':
      bundle=cache/'tectonic/bundles/data';row['downloadedFileBytes']=sum(f.stat().st_size for d in bundle.iterdir() if d.is_dir() for f in d.iterdir() if f.is_file()) if bundle.exists() else 0
     rows.append(row);(ev/'native-results.json').write_text(json.dumps(rows,indent=2)+'\n')
     print(label,row['exit'],round(row['wallMs']),rss if (rss:=row['peakRssKiB']) else '',flush=True)
    shutil.rmtree(cache,ignore_errors=True)
if __name__=='__main__':main()
