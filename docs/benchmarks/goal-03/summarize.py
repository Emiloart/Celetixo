import pathlib,json,statistics,hashlib,sys
def require(condition,message):
 if not condition:raise ValueError(message)
root=pathlib.Path(sys.argv[1]);metadata=json.loads((root/'run.json').read_text())
require(bool(metadata['commands']) and all(c['exit']==0 for c in metadata['commands']) and metadata['clean_worktree_before'] and metadata['clean_worktree_after'],'measurement commands must succeed from a clean checkout')
def stats(values):
 require(len(values)>=3,'at least three samples are required per metric')
 return {'samples':len(values),'minimum':min(values),'median':statistics.median(values),'maximum':max(values)}
summary={'schema':'celetixo/goal03-benchmark-summary/v1','source_commit':metadata['source_commit'],'timing_unit':'microseconds unless explicitly named otherwise','scope':'observed local distributions; no production or cold-cache claim','tracking_target_method':'provisional per-case ceiling equals observed maximum; throughput floor equals observed minimum; review future deviations with new measurements','t0':{},'t1':{},'capacity_and_recovery':{}}
for name in ['celetixo','xxhash']:
 data=json.loads((root/(name+'-t0.json')).read_text());samples=data['samples']
 require(len({s['base_snapshot'] for s in samples})==1,'T0 samples must share an exact base snapshot')
 cases={}
 for sample in samples:
  for case in sample['cases']:cases.setdefault(case['case'],[]).append(case)
 groups={}
 for case,values in cases.items():
  metrics={key:stats([v[key] for v in values]) for key in values[0] if key.endswith('_us')}
  groups[case]={'metrics_us':metrics,'source_files':values[0]['source_files'],'copied_source_bytes':values[0]['source_content_bytes_copied'],'throughput_bytes_per_second':stats([v['source_copy_bytes_per_second'] for v in values]),'host_current_rss_bytes':stats([v['host_memory']['current_rss_bytes'] for v in values]),'host_process_lifetime_high_water_rss_bytes':stats([v['host_memory']['process_lifetime_high_water_rss_bytes'] for v in values]),'provisional_timing_ceilings_us':{key:value['maximum'] for key,value in metrics.items()}}
 summary['t0'][name]={'commit':samples[0]['git_commit'],'files':samples[0]['base_source_files'],'content_bytes':samples[0]['base_source_bytes'],'capture_us':stats([s['capture_us'] for s in samples]),'cases':groups}
data=json.loads((root/'generated-t1.json').read_text());groups={}
for sample in data['samples']:groups.setdefault(sample['case'],[]).append(sample)
for case,values in groups.items():
 require(len({v['base_snapshot'] for v in values})==1,'T1 case samples must share an exact base snapshot')
 require(all(not v['cache_reuse_enabled'] for v in values),'T1 measurements must disable cache reuse')
 keys=['patch_apply_us','patch_replay_validation_us','manifest_canonical_and_hash_us','materialization_us','checks_including_backend_setup_us','source_cleanup_us','persistence_us','engine_evidence_admission_us','total_run_us','receipt_query_us','envelope_read_us','cache_key_us']
 metrics={key:stats([v[key] for v in values]) for key in keys}
 phases={key:stats([v['raw_execution_and_effective_isolation']['execution']['phase_timings_us'][key] for v in values]) for key in ['preparation','startup','command_supervision','runtime_cleanup']}
 summary['t1'][case]={'materialized_files':values[0]['materialized_files'],'copied_source_bytes':values[0]['copied_source_content_bytes'],'metrics_us':metrics,'backend_phase_metrics_us':phases,'guest_observed_wall_ms':stats([v['recorded_wall_time_ms'] for v in values]),'guest_observed_cpu_us':stats([v['recorded_cpu_time_us'] for v in values]),'guest_observed_peak_memory_bytes':stats([v['recorded_peak_memory_bytes'] for v in values]),'host_current_rss_bytes':stats([v['host_memory']['current_rss_bytes'] for v in values]),'host_process_lifetime_high_water_rss_bytes':stats([v['host_memory']['process_lifetime_high_water_rss_bytes'] for v in values]),'provisional_timing_ceilings_us':{key:value['maximum'] for key,value in metrics.items()}}
values=data['capacity_and_recovery']
require(all(v['termination_confirmed'] and v['owned_source_removed'] and not v['check_result_recovered'] and v['queued_attempts']==0 for v in values),'capacity/recovery samples must confirm termination and owned cleanup without recovered results or queued attempts')
for key in ['busy_rejection_us','preparation_and_controlled_failure_us','owned_orphan_recovery_us','idempotent_repeated_recovery_us']:summary['capacity_and_recovery'][key]=stats([v[key] for v in values])
summary['input_sha256']={name:hashlib.sha256((root/name).read_bytes()).hexdigest() for name in ['run.json','celetixo-t0.json','xxhash-t0.json','generated-t1.json']}
(root/'summary.json').write_text(json.dumps(summary,indent=2)+'\n')
print(json.dumps({'source_commit':summary['source_commit'],'t0_cases':sum(len(v['cases']) for v in summary['t0'].values()),'t1_cases':len(summary['t1']),'all_sample_counts_at_least_three':True}),flush=True)
