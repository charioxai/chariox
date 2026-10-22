#!/usr/bin/env python3
"""MP-08/MP-10/MP-11: compact measured results, never infer missing PASSes."""
import argparse,json,pathlib,statistics

def main():
 p=argparse.ArgumentParser();p.add_argument('--evidence',required=True);p.add_argument('--out',required=True);a=p.parse_args();ev=pathlib.Path(a.evidence)
 component=json.loads((ev/'component-results.json').read_text());native=json.loads((ev/'native-results.json').read_text());identities=json.loads((ev/'identities.json').read_text())
 reports=[]
 for doc in ['hello','paper','beamer']:
  runs=[r for r in component if r['document']==doc]
  row={'document':doc,'runs':len(runs),'readyMs':[r['report']['coldReadyMs'] for r in runs],'cgroupPeakMiB':[r.get('cgroupPeakBytes',0)/2**20 for r in runs]}
  for phase in ['cold','warm','edit']:
   cells=[c for r in runs for c in r['report']['cells'] if c['phase']==phase]
   row[phase]={'ms':[c['ms'] for c in cells],'ok':len(cells)==3 and all('Ok' in c['outcome'] for c in cells),'fetchedBytes':[c['fetchedBytes'] for c in cells],'peakRssMiB':[c['memory']['hwmKb']/1024 for c in cells]}
  row['cgroupOom']=any('oom_kill 0' not in r.get('cgroupEvents','') for r in runs);reports.append(row)
 comparisons=[]
 for engine,doc in [('tectonic','hello'),('tectonic','paper'),('tectonic','beamer'),('typst','typst')]:
  row={'engine':engine,'document':doc}
  for phase in ['cold','warm','edit']:
   cells=[r for r in native if r['engine']==engine and r['document']==doc and r['phase']==phase]
   row[phase]={'ms':[r['wallMs'] for r in cells],'exit':[r['exit'] for r in cells],'peakRssMiB':[r['peakRssKiB']/1024 for r in cells],'pdfBytes':[r['pdfBytes'] for r in cells]}
   if phase=='cold':row[phase]['downloadedFileBytes']=[r.get('downloadedFileBytes') for r in cells]
  comparisons.append(row)
 out={'mpItems':['MP-08','MP-10','MP-11'],'scope':'Compiler research only. No App UI/product acceptance. Apps remains PAUSED.','identities':identities,'precompile':json.loads((ev/'precompile.json').read_text()),'component':reports,'native':comparisons,'conditions':{'linearMemoryMiB':512,'processTreeCgroupMiB':512,'cpuSliceMs':30000,'network':'Real HTTPS archive range fetches through the existing spike HTTP import, broker and parent-side proxy; not the hosted Chariox relay.','coldCache':'Fresh private data preopen for each run, including format creation and the same curated union of package/font files. Native cold caches independently discover per-document files.','warm':'Same App instance, format and package cache, aux/bbl feedback; fresh native CLI process for the comparator.','coldReady':'AOT component loaded and tools registered; no format or TeX bundle loaded at ready.','nativeTypst':'Native CLI only; no new Typst component measured in this spike.','timing':'Shared b3. No quiet-host latency acceptance claim.','pdflatex':'Not installed. No comparison invented.'}}
 pathlib.Path(a.out).write_text(json.dumps(out,indent=2)+'\n')
 for r in reports:
  print(r['document'],'ready',round(statistics.median(r['readyMs']),2),'cold/warm/edit',*[round(statistics.median(r[k]['ms']),2) if r[k]['ms'] else None for k in ['cold','warm','edit']],'cgroupPeak',round(max(r['cgroupPeakMiB']),2))
 for r in comparisons:
  print('native',r['engine'],r['document'],*[round(statistics.median(r[k]['ms']),2) if r[k]['ms'] else None for k in ['cold','warm','edit']])
if __name__=='__main__':main()
