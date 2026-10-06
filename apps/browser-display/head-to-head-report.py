"""MP-08/MP-10/MP-11: report measured rows; missing/dirty data never wins."""
import json,sys
from pathlib import Path
ours,baselines,out=map(Path,sys.argv[1:4])
extra=list(zip(['ours OpenH264','ours VP8'],map(Path,sys.argv[4:])))
out.mkdir(parents=True,exist_ok=True)
rows=[]
for fixture in ['docs','canvas','video','scroll30','wheel30']:
    candidates=[('Selkies 2.0.0',baselines/('selkies2-'+fixture)),('Selkies legacy',baselines/('legacy-'+fixture)),('ours x264',ours/('local-'+fixture+'-8000000'))]+[(name,path/('local-'+fixture+'-8000000')) for name,path in extra]
    for backend,root in candidates:
        is_ours=backend.startswith('ours')
        p=root/'results.json';row={'fixture':fixture,'backend':backend,'receipt':str(p)};rows.append(row)
        if not p.exists():row['error']='missing receipt';continue
        d=json.loads(p.read_text());row.update(source=d['source'],source_dirty=d['source_dirty'],status=d['status'],error=d.get('error'))
        if d['source_dirty']:raise ValueError('MP-10 dirty source '+str(p))
        if is_ours and d.get('actual_encoders')!=[{'ours x264':'x264','ours OpenH264':'openh264','ours VP8':'vp8'}[backend]]:raise ValueError('MP-10 unproven encoder '+str(p))
        m=d.get('motion',{});idle=d.get('static_polling',{}) if is_ours else {}
        idle_cpu=idle.get('cpu',d.get('idle_cpu'));active=m.get('cpu',d.get('click_cpu'))
        row.update(click=d.get('latency'),typing=d.get('type_latency'),fps=m.get('effective_fps'),content_fps=m.get('effective_content_fps'),idle_cpu=idle_cpu,active_cpu=active,
            idle_video_mbps=idle.get('video_mbps',d.get('idle_video_mbps')),moving_video_mbps=m.get('event_mbps'),live_psnr_db=[x['psnr_db'] for x in m.get('live_pairs',[])])
        settled=m.get('settled_fidelity',d.get('settled',{}).get('fidelity',{}) if is_ours else d.get('settled',{}))
        row.update(settled_psnr_db=settled.get('psnr_db'),settled_exact=settled.get('lossless',False),cleanup=d.get('cleanup'))
        row['owner_target_pass']=bool(row['click'] and row['typing'] and active and row['click']['p95_ms']<50 and row['typing']['p95_ms']<50 and active['source_plus_pipeline_cores']<=1 and row['settled_exact'] and (fixture=='docs' or row['fps']>=30))
report={'item':'MP-08/MP-10/MP-11','rows':rows,'status':'RED_PERFORMANCE','gpu':'UNMEASURED: owner laptop required',
        'limits':['Same 1080p DPR1 fixtures and eight Mbps ceiling; local owned namespaces, MTU1500, offloads disabled.',
                  'Owner CPU target uses source plus pipeline, excluding the remote viewer. Source/pipeline classification can vary when a shell wrapper execs Chromium; their combined total is stable.',
                  'Canvas/rAF timing is a software presentation proxy. CPU is Linux task ticks, separated source/pipeline/viewer; sub100ms tasks can be missed.',
                  'Video-event Mbps includes Chariox encrypted frame envelopes, Selkies2 stripe headers, or legacy inbound RTP payload bytes; excludes requests, TCP/TLS/DTLS overhead and diagnostics.',
                  'Live PSNR compares latest source with a later viewer snapshot and includes temporal drift. Held-motion PSNR is separately labelled; neither is frame-aligned codec PSNR.',
                  'Content fps uses the same64x36 thumbnail threshold (>0.1% pixels differ by >8 RGB); repeated presentations are reported separately.']}
(out/'comparison.json').write_text(json.dumps(report,indent=2)+'\n')
def number(v):return '—' if v is None else f'{v:.2f}'
def quantile(r,k):return '/'.join(number((r.get(k) or {}).get('p'+q+'_ms')) for q in ['50','95'])
lines=['MP-08/MP-10/MP-11 — software1080p DPR1, 8 Mbps; RED_PERFORMANCE.',
 '|Fixture|Backend|Click P50/P95 ms|Type P50/P95 ms|FPS / content FPS|Pipeline idle/active cores|Source / viewer active cores|Source+pipeline cores|Video idle/moving Mbps|Live PSNR dB|Settled PSNR / exact|',
 '|---|---|---|---|---|---|---|---|---|---|---|']
for r in rows:
    a=(r.get('active_cpu') or {}).get('cores',{});i=(r.get('idle_cpu') or {}).get('cores',{});p=r.get('live_psnr_db',[])
    lines.append('|'+ '|'.join([r['fixture'],r['backend'],quantile(r,'click'),quantile(r,'typing'),number(r.get('fps'))+'/'+number(r.get('content_fps')),number(i.get('pipeline'))+'/'+number(a.get('pipeline')),number(a.get('source'))+'/'+number(a.get('viewer')),number((r.get('active_cpu') or {}).get('source_plus_pipeline_cores')),number(r.get('idle_video_mbps'))+'/'+number(r.get('moving_video_mbps')),('—' if not p else number(min(p))+'–'+number(max(p))),('exact' if r.get('settled_exact') else number(r.get('settled_psnr_db')))+'/'+str(r.get('settled_exact',False))])+'|')
lines+=['','MP-10 GPU/VAAPI: Selkies2, legacy and ours are unmeasured for all five fixtures. No MP item closes from this matrix.']
(out/'comparison.md').write_text('\n'.join(lines)+'\n')
print(json.dumps({'item':report['item'],'status':report['status'],'rows':len(rows),'owner_target_rows':sum(r.get('owner_target_pass',False) for r in rows),'output':str(out)}))
