import sqlite3, os, json
DB = '/root/.opencrabs/opencrabs.db'
db = sqlite3.connect(DB)
db.row_factory = sqlite3.Row

def cols(t):
    try:
        return [c[1] for c in db.execute('pragma table_info(%s)' % t)]
    except Exception as e:
        return ['ERR %s' % e]

print('LEDGER_COLS', cols('usage_ledger'))
print('MSG_COLS', cols('messages'))
print('TOOL_COLS', cols('tool_executions'))

print('=== OPUS LEDGER ROWS (all) ===')
opus = [dict(r) for r in db.execute("select * from usage_ledger where model like '%opus%'")]
print('count', len(opus))
for r in opus:
    print(json.dumps(r)[:400])

sids = sorted(set(str(r.get('session_id', '')) for r in opus))
print('SIDS', sids)
for s in sids:
    try:
        ms = [dict(r) for r in db.execute('select * from messages where session_id=? order by rowid asc', (s,))]
        print('=== SESSION', s, 'msgs', len(ms))
        for m in ms[:8]:
            c = str(m.get('content', ''))
            meta = {k: m.get(k) for k in m if k not in ('content',)}
            print(meta, '|', c[:200].replace('\n', ' ~ '))
        if ms:
            first = next((m for m in ms if str(m.get('role', '')).lower() in ('user', 'human')), ms[0])
            open('/tmp/runprompt_%s.txt' % s[:8], 'w').write(str(first.get('content', '')))
            last = ms[-1]
            print('LAST_MSG_META', {k: last.get(k) for k in last if k not in ('content',)})
    except Exception as e:
        print('msg ERR', s, e)
    try:
        ts = [dict(r) for r in db.execute('select status, count(*) n from tool_executions where session_id=? group by status', (s,))]
        print('TOOLS', s, ts)
    except Exception as e:
        print('tool ERR', s, e)

print('=== FS /srv/bench/fps ===')
print(os.popen('ls -lt /srv/bench/fps/ | head -14').read())
print(os.popen('for d in /srv/bench/fps/*/; do echo "$d $(du -sh $d 2>/dev/null | cut -f1) $(find $d -type f | wc -l)"; done').read())
