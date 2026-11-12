"""Historical mechanical-boundary audit, not a proof of Rust program equivalence."""
import hashlib, json, re, subprocess, sys
from pathlib import Path
root=Path(sys.argv[1]).resolve() if len(sys.argv)>1 else Path(__file__).resolve().parents[3]
baseline='8fae23346a35fe83164f12b24d84bc4500e84c3d'
files=['crates/engine/src/engine.rs','crates/engine/src/engine/source_attempts.rs','crates/engine/src/engine/source_cleanup.rs']
def unwrap(source):
    lines=source.splitlines(keepends=True);out=[];i=0;count=0
    while i<len(lines):
        match=re.fullmatch(r'( +)(self|engine)\.store\.(read|update)\(\|db\| \{\n',lines[i])
        if not match:out.append(lines[i]);i+=1;continue
        indent=len(match[1]);end=i+1
        while end<len(lines) and lines[end]!=' '*(indent-4)+'}\n':end+=1
        assert end<len(lines) and lines[end-1]==' '*indent+'})\n'
        out.extend(lines[i+1:end-1]);out.append(lines[end]);i=end+1;count+=1
    return ''.join(out),count
def normalize(source):
    source=source.replace('mod state_access;\n','')
    source=re.sub(r' +let (?:mut )?db = (?:self|engine)\.store\.lock\(\)\?;\n','',source)
    source=re.sub(r'&(?:mut\s+)?db(?=\s*[,);])','db',source)
    # Preserve quoted strings exactly. Rustfmt also changes trailing commas
    # when callback indentation wraps calls. This lexical audit is not a Rust
    # equivalence proof (e.g. tuple commas need semantic review and tests).
    parts=re.split(r'("(?:\\.|[^"\\])*")',source)
    return ''.join(part if i%2 else re.sub(r',(?=[)\]])','',re.sub(r'\s+','',part)) for i,part in enumerate(parts))
report={'schema':'celetixo/state-access-mechanical-audit/v1','baseline':baseline,'scope':'Callback wrapping, mutex acquisition removal, db reborrows, rustfmt trailing commas and module declaration only. Does not prove runtime/semantic equivalence; paired with real tests. Evidence gate is separately reviewed/tested.','files':{}}
for name in files:
    old=subprocess.check_output(['git','show',baseline+':'+name],cwd=root).decode();new=(root/name).read_text();unwrapped,count=unwrap(new)
    if normalize(old)!=normalize(unwrapped):raise SystemExit('Unexpected source change beyond the reviewed mechanical boundary in '+name)
    report['files'][name]={'baseline_sha256':hashlib.sha256(old.encode()).hexdigest(),'current_sha256':hashlib.sha256(new.encode()).hexdigest(),'wrapped_operations':count}
print(json.dumps(report,indent=2))
