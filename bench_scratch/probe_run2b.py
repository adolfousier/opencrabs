import os
base = '/srv/bench/harness/opencrabs/opus-5-2nd-round'
print('EXISTS', os.path.isdir(base))
n = 0
sz = 0
for root, dirs, files in os.walk(base):
    for f in files:
        p = os.path.join(root, f)
        n += 1
        try:
            sz += os.path.getsize(p)
        except Exception:
            pass
print('FILES', n, 'BYTES', sz)
print(os.popen('ls -la %s | head -20' % base).read())
print('=== webroot landing ===')
print(os.popen('ls -la /srv/html/opencrabs-bench-landing/ 2>/dev/null | head -20').read())
print('=== webroot harness dir ===')
print(os.popen('ls -la /srv/html/opencrabs-bench-landing/harness/ 2>/dev/null').read())
print('=== nginx harness refs ===')
print(os.popen("grep -rn 'harness' /etc/nginx/sites-enabled/ 2>/dev/null | head -20").read())
print('=== runs-harness tree ===')
print(os.popen('ls -laR /srv/bench/fps/runs-harness/').read())
print('=== run2 prompt markers ===')
c = open('/tmp/runprompt_1df8b3a8.txt').read()
print('LEN', len(c))
print('HAS_VISOPT', 'Visual Options' in c)
print('HAS_IMMED_TYPO', 'immediatelly' in c)
print('FIRST150', repr(c[:150]))
print('LAST250', repr(c[-250:]))
