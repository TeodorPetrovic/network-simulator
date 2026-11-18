#!/usr/bin/env python3
"""Vezba 7 test harness (za odrzavanje, nije deo vezbe).

Pokrece tacno komande iz <pre data-dev="..."> blokova u task/resenje.frag.html, redom,
na Docker kontejnerima v7_sw1, v7_sw2, v7_pc1, v7_pc2 (vidi setup u test-log.md).
Za vreme Koraka 4 i 8 snima saobracaj tcpdump-om (kontejner sa tcpdump-om deli
mrezni namespace sa SW2 / PC1 / PC2) - to je ekvivalent Wireshark snimanja na kablu.
Na kraju ispisuje PASS/FAIL za svaku proveru.
"""
import html, re, subprocess, sys, time, pathlib

ROOT = pathlib.Path(__file__).resolve().parent.parent
SRC = (ROOT / 'task' / 'resenje.frag.html').read_text()
SHELL = {'sw1': 'bash', 'sw2': 'bash', 'pc1': 'sh', 'pc2': 'sh'}
CAPS = {'sw2eth0': ('v7_sw2', 'eth0'), 'pc1': ('v7_pc1', 'eth0'), 'pc2': ('v7_pc2', 'eth0')}
results, outputs = [], {}


def sh(cmd):
    p = subprocess.run(cmd, shell=isinstance(cmd, str), capture_output=True, text=True)
    return p.returncode, p.stdout + p.stderr


def run(dev, cmd):
    rc, out = sh(['docker', 'exec', f'v7_{dev}', SHELL[dev], '-c', cmd])
    print(f'[{dev}]$ {cmd}')
    if out.strip():
        print(out.rstrip())
    print(f'(rc={rc})')
    return rc, out


def check(name, ok, detail=''):
    results.append((name, ok))
    print(f'CHECK {"PASS" if ok else "FAIL"}: {name} {detail}')


def cap_start():
    for k, (tgt, ifc) in CAPS.items():
        subprocess.run(['docker', 'rm', '-f', f'v7_cap_{k}'], capture_output=True)
        subprocess.run(['docker', 'run', '-d', '--name', f'v7_cap_{k}', '--net', f'container:{tgt}',
                        'v7-tcpdump:local', 'tcpdump', '-l', '-U', '-e', '-n', '-v', '-i', ifc, 'not ip6'],
                       capture_output=True, check=True)
    time.sleep(1.5)


def cap_stop(tag):
    time.sleep(1)
    caps = {}
    for k in CAPS:
        name = f'v7_cap_{k}'
        subprocess.run(['docker', 'kill', '--signal=INT', name], capture_output=True)
        subprocess.run(['docker', 'wait', name], capture_output=True)
        _, out = sh(['docker', 'logs', name])
        subprocess.run(['docker', 'rm', '-f', name], capture_output=True)
        # spoji tcpdump -v nastavke (red koji pocinje razmakom) sa prethodnim redom
        lines = []
        for l in out.splitlines():
            if l.startswith((' ', '\t')) and lines:
                lines[-1] += ' | ' + l.strip()
            elif re.match(r'^\d\d:\d\d:', l):
                lines.append(l)
        caps[k] = lines
        print(f'----- capture {tag} {k} ({len(lines)} frames)')
        for l in lines:
            print('  ' + l)
    return caps


def flows_counts(out):
    return {m.group(2): int(m.group(1)) for m in re.finditer(r'n_packets=(\d+),.*?(priority=\d+,\S+)', out)}


def port_counts(out):
    rx = re.search(r'rx pkts=(\d+)', out); tx = re.search(r'tx pkts=(\d+)', out)
    return int(rx.group(1)), int(tx.group(1))


# ---- sekvencijalno izvrsavanje fragmenta ----
tokens = re.finditer(r'<h3>(.*?)</h3>|<pre data-dev="(\w+)"><code>(.*?)</code></pre>', SRC, re.S)
section, caps_running, k8 = '', False, []
sections = []
for t in tokens:
    if t.group(1):
        sections.append((html.unescape(t.group(1)), []))
    else:
        sections[-1][1].append((t.group(2), html.unescape(t.group(3))))

for title, blocks in sections:
    print(f'\n##### {title}')
    if title.startswith(('Korak 4', 'Korak 8')):
        cap_start()
    sec_out = []
    for dev, code in blocks:
        cmds, cur = [], ''
        for line in code.split('\n'):
            cur = cur + '\n' + line if cur else line
            if not line.endswith('\\'):
                cmds.append(cur); cur = ''
        for c in cmds:
            rc, out = run(dev, c)
            sec_out.append((dev, c, rc, out))
    outputs[title] = sec_out

    def outs(pred):
        return [o for d, c, r, o in sec_out if pred(d, c)]

    if title.startswith('Korak 3'):
        p = outs(lambda d, c: c.startswith('ping'))[0]
        check('K3 PC1->PC2 ping fails (no router, different VLAN)', '100% packet loss' in p)
        s = outs(lambda d, c: c == 'ovs-vsctl show')[-1]
        check('K3 tags/trunks set', 'tag: 10' in s and 'tag: 20' in s and 'trunks: [10, 20]' in s)
    if title.startswith('Korak 4'):
        caps = cap_stop('K4')
        s = outs(lambda d, c: c == 'ovs-vsctl show')[-1]
        check('K4 patch ports present, no error', 'peer=br2-br0' in s and 'peer=br0-br2' in s and 'error' not in s.lower())
        for p in outs(lambda d, c: c.startswith('ping')):
            check('K4 ping still fails', '100% packet loss' in p)
        t = caps['sw2eth0']
        check('K4 trunk: PC1 ARP arrives tagged vlan 10 (via patch)',
              any('vlan 10' in l and 'who-has 10.10.10.1 tell 10.10.10.11' in l for l in t))
        check('K4 trunk: PC2 ARP arrives tagged vlan 20',
              any('vlan 20' in l and 'who-has 10.10.20.1 tell 10.10.20.12' in l for l in t))
        check('K4 PC1 link: no 802.1Q frames', not any('802.1Q' in l for l in caps['pc1']))
        check('K4 PC2 link: no 802.1Q frames', not any('802.1Q' in l for l in caps['pc2']))
        check('K4 PC1 link: nothing from VLAN 20', not any('10.10.20.' in l or '02:00:00:00:20:' in l for l in caps['pc1']))
        check('K4 PC2 link: nothing from VLAN 10', not any('10.10.10.' in l or '02:00:00:00:10:' in l for l in caps['pc2']))
    if title.startswith('Korak 5'):
        o = outs(lambda d, c: c == 'ovs-ofctl dump-flows br0')
        check('K5 NORMAL deleted', 'actions=NORMAL' in o[0] and 'actions=' not in o[-1])
        check('K5 eth0 in br0', outs(lambda d, c: 'port-to-br' in c)[0].strip() == 'br0')
    if title.startswith('Korak 6'):
        n = [o for d, c, r, o in sec_out if 'neigh' in c]
        check('K6 PC1 learned GW10 MAC from OpenFlow', 'lladdr 02:00:00:00:10:01' in n[0])
        check('K6 PC2 learned GW20 MAC from OpenFlow', 'lladdr 02:00:00:00:20:01' in n[1])
        # harness-only negativni test: zahtev za 10.10.10.1 iz VLAN-a 20 ne dobija odgovor
        rc, o = run('pc2', 'arping -I eth0 -c 2 -w 3 10.10.10.1')
        check('K6 NEG: ARP for 10.10.10.1 asked from VLAN 20 is NOT answered', 'Received 0 response' in o)
        rc, o = run('pc1', 'arping -I eth0 -c 2 -w 3 10.10.20.1')
        check('K6 NEG: ARP for 10.10.20.1 asked from VLAN 10 is NOT answered', 'Received 0 response' in o)
        rc, o = run('pc1', 'ping -c 2 -W 2 10.10.20.12')
        check('K6 NEG: PC1->PC2 still fails without routing flows', '100% packet loss' in o)
    if title.startswith('Korak 7'):
        for p in outs(lambda d, c: c.startswith('ping')):
            check('K7 ping works, ttl=63', ' 0% packet loss' in p and 'ttl=63' in p and 'ttl=64' not in p)
    if title.startswith('Korak 8'):
        caps = cap_stop('K8')
        d = outs(lambda d, c: 'dump-ports' in c)
        f = outs(lambda d, c: 'dump-flows' in c)
        (rx0, tx0), (rx1, tx1) = port_counts(d[0]), port_counts(d[1])
        f0, f1 = flows_counts(f[0]), flows_counts(f[1])
        delta = {k: f1[k] - f0.get(k, 0) for k in f1}
        print('port eth0 delta rx', rx1 - rx0, 'tx', tx1 - tx0, '| flow deltas', delta)
        check('K8 eth0 rx and tx both grew by >= 6', rx1 - rx0 >= 6 and tx1 - tx0 >= 6, f'rx+{rx1-rx0} tx+{tx1-tx0}')
        r10 = [v for k, v in delta.items() if 'dl_vlan=10,nw_dst=10.10.20.0/24' in k]
        r20 = [v for k, v in delta.items() if 'dl_vlan=20,nw_dst=10.10.10.0/24' in k]
        check('K8 routing flow counters +3 each', r10 == [3] and r20 == [3], f'{r10} {r20}')
        check('K8 ping ok', ' 0% packet loss' in outs(lambda d, c: c.startswith('ping'))[0])
        t = caps['sw2eth0']
        req = [l for l in t if 'echo request' in l]
        ids = {}
        for l in req:
            i = re.search(r'id (\d+), offset', l).group(1)
            ids.setdefault(i, []).append(l)
        ok = bool(ids) and all(len(v) == 2 and
                               'vlan 10' in v[0] and 'ttl 64' in v[0] and '02:00:00:00:10:11 > 02:00:00:00:10:01' in v[0] and
                               'vlan 20' in v[1] and 'ttl 63' in v[1] and '02:00:00:00:20:01 > 02:00:00:00:20:12' in v[1]
                               for v in ids.values())
        check('K8 trunk: every request seen twice: vlan10/ttl64 in, vlan20/ttl63 out', ok, f'{len(ids)} ids')
        rep = [l for l in t if 'echo reply' in l]
        check('K8 trunk: replies vlan20/ttl64 in, vlan10/ttl63 out',
              any('vlan 20' in l and 'ttl 64' in l for l in rep) and any('vlan 10' in l and 'ttl 63' in l for l in rep))
        check('K8 PC1 link: no 802.1Q', not any('802.1Q' in l for l in caps['pc1']))
        check('K8 PC2 link: no 802.1Q', not any('802.1Q' in l for l in caps['pc2']))
        check('K8 PC2 sees request from GW20 MAC with ttl 63',
              any('02:00:00:00:20:01 > 02:00:00:00:20:12' in l and 'ttl 63' in l and 'echo request' in l for l in caps['pc2']))
        check('K8 PC1 sends request to GW10 MAC with ttl 64',
              any('02:00:00:00:10:11 > 02:00:00:00:10:01' in l and 'ttl 64' in l and 'echo request' in l for l in caps['pc1']))

print('\n===== SUMMARY')
for n, ok in results:
    print(('PASS ' if ok else 'FAIL ') + n)
bad = [n for n, ok in results if not ok]
print(f'{len(results) - len(bad)}/{len(results)} PASS')
sys.exit(1 if bad else 0)
