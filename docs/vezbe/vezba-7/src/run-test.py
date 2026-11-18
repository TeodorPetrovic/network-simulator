#!/usr/bin/env python3
"""Vezba 7 test harness (za odrzavanje, nije deo vezbe).

Pokrece tacno komande iz <pre data-dev="..."> blokova u task/resenje.frag.html, redom:
sw1/sw2 kroz `docker exec v7_swN bash -c`, a vpc1/vpc2 kroz konzolu pravog VPCS-a
(vpcs -p 5000 u kontejneru v7_vpcN, kao telnet konzola u GNS3; vidi test-setup.sh).
MAC adrese VPCS-a zavise od -m (u GNS3/simulatoru od ID-a cvora). Primer u resenju koristi
00:50:79:66:68:1c i 00:50:79:66:68:a7 (-m 28 i -m 167); ako je setup pokrenut sa drugim -m,
harness procita stvarne MAC adrese iz `show` (Korak 2) i njima zameni primer u kasnijim komandama
- isto ono sto student radi kad upise svoje MAC adrese u pravila.
Za vreme Koraka 4 i 8 snima saobracaj tcpdump-om (kontejner sa tcpdump-om deli mrezni namespace
sa SW2 / VPC1 / VPC2) - ekvivalent Wireshark snimanja na kablu. Na kraju PASS/FAIL za svaku proveru.
"""
import html, os, re, select, subprocess, sys, time, pathlib

ROOT = pathlib.Path(__file__).resolve().parent.parent
SRC = (ROOT / 'task' / 'resenje.frag.html').read_text()
EX = {'vpc1': '00:50:79:66:68:1c', 'vpc2': '00:50:79:66:68:a7'}
MAC = dict(EX)
CAPS = {'sw2eth0': ('v7_sw2', 'eth0'), 'vpc1': ('v7_vpc1', 'eth0'), 'vpc2': ('v7_vpc2', 'eth0')}
results, outputs = [], {}


class VCon:
    """Telnet konzola VPCS-a (vpcs -p 5000) preko nc u kontejneru."""
    def __init__(self, ctr):
        self.p = subprocess.Popen(['docker', 'exec', '-i', ctr, 'nc', '127.0.0.1', '5000'],
                                  stdin=subprocess.PIPE, stdout=subprocess.PIPE, bufsize=0)
        self._read(5)
        self.cmd('')

    def _read(self, timeout):
        buf, end = b'', time.time() + timeout
        while time.time() < end:
            r, _, _ = select.select([self.p.stdout], [], [], 0.2)
            if r:
                d = os.read(self.p.stdout.fileno(), 65536)
                if not d:
                    break
                buf += d
                if re.search(rb'\n[\w-]+> $', buf.replace(b'\r', b'')):
                    break
        return buf

    def cmd(self, c, timeout=90):
        self.p.stdin.write((c + '\n').encode())
        out = self._read(timeout)
        out = re.sub(rb'\xff[\xfb-\xfe].', b'', out).decode('utf8', 'replace').replace('\r', '')
        out = re.sub(r'\x1b\[[0-9;]*m', '', out)
        lines = out.split('\n')
        if lines and lines[0].strip() == c.strip():
            lines = lines[1:]
        if lines and re.match(r'^[\w-]+> $', lines[-1]):
            lines = lines[:-1]
        return '\n'.join(lines).strip('\n')


CON = {}


def sh(cmd):
    p = subprocess.run(cmd, shell=isinstance(cmd, str), capture_output=True, text=True)
    return p.returncode, p.stdout + p.stderr


def subst(cmd):
    for k in EX:
        cmd = cmd.replace(EX[k], MAC[k])
    return cmd


def run(dev, cmd):
    cmd = subst(cmd)
    if dev.startswith('vpc'):
        if dev not in CON:
            CON[dev] = VCon(f'v7_{dev}')
        rc, out = 0, CON[dev].cmd(cmd)
        print(f'[{dev.upper()}> {cmd}')
    else:
        rc, out = sh(['docker', 'exec', f'v7_{dev}', 'bash', '-c', cmd])
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


def has_tag(l):
    return '802.1Q' in l


tokens = re.finditer(r'<h3>(.*?)</h3>|<pre data-dev="(\w+)"><code>(.*?)</code></pre>', SRC, re.S)
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

    m1, m2 = MAC['vpc1'], MAC['vpc2']
    if title.startswith('Korak 2'):
        for dev, ip in (('vpc1', '10.10.10.11/24'), ('vpc2', '10.10.20.12/24')):
            s = outs(lambda d, c, dev=dev: d == dev and c == 'show')[-1]
            m = re.search(r'^VPC\d\s+(\S+)\s+(\S+)\s+([0-9a-f:]{17})', s, re.M)
            check(f'K2 {dev} show: IP, gateway, MAC', bool(m) and m.group(1) == ip, m.group(0) if m else s)
            if m:
                MAC[dev] = m.group(3)
        print('MAC adrese VPC-ova (iz show):', MAC, '| primer u resenju:', EX)
        m1, m2 = MAC['vpc1'], MAC['vpc2']
    if title.startswith('Korak 3'):
        p = outs(lambda d, c: c.startswith('ping'))[0]
        check('K3 VPC1->VPC2 ping fails: gateway not reachable (no router)', 'host (10.10.10.1) not reachable' in p)
        s = outs(lambda d, c: c == 'ovs-vsctl show')[-1]
        check('K3 tags/trunks set', 'tag: 10' in s and 'tag: 20' in s and 'trunks: [10, 20]' in s)
    if title.startswith('Korak 4'):
        caps = cap_stop('K4')
        s = outs(lambda d, c: c == 'ovs-vsctl show')[-1]
        check('K4 patch ports present, no error', 'peer=br2-br0' in s and 'peer=br0-br2' in s and 'error' not in s.lower())
        p = outs(lambda d, c: c.startswith('ping'))
        check('K4 VPC1 ping still fails', 'host (10.10.10.1) not reachable' in p[0])
        check('K4 VPC2 ping still fails', 'host (10.10.20.1) not reachable' in p[1])
        t = caps['sw2eth0']
        check('K4 trunk: VPC1 ARP arrives tagged vlan 10 (via patch)',
              any('vlan 10' in l and f'{m1} > ff:ff:ff:ff:ff:ff' in l and re.search(r'who-has 10\.10\.10\.1 (\(ff:ff:ff:ff:ff:ff\) )?tell 10\.10\.10\.11', l) for l in t))
        check('K4 trunk: VPC2 ARP arrives tagged vlan 20',
              any('vlan 20' in l and f'{m2} > ff:ff:ff:ff:ff:ff' in l and re.search(r'who-has 10\.10\.20\.1 (\(ff:ff:ff:ff:ff:ff\) )?tell 10\.10\.20\.12', l) for l in t))
        check('K4 VPC1 link: ARP seen untagged', any('who-has 10.10.10.1' in l and not has_tag(l) for l in caps['vpc1']))
        check('K4 VPC1 link: no 802.1Q frames', not any(has_tag(l) for l in caps['vpc1']))
        check('K4 VPC2 link: no 802.1Q frames', not any(has_tag(l) for l in caps['vpc2']))
        check('K4 VPC1 link: nothing from VLAN 20', not any('10.10.20.' in l or m2 in l for l in caps['vpc1']))
        check('K4 VPC2 link: nothing from VLAN 10', not any('10.10.10.' in l or m1 in l for l in caps['vpc2']))
    if title.startswith('Korak 5'):
        o = outs(lambda d, c: c == 'ovs-ofctl dump-flows br0')
        check('K5 NORMAL deleted', 'actions=NORMAL' in o[0] and 'actions=' not in o[-1])
        check('K5 eth0 in br0', outs(lambda d, c: 'port-to-br' in c)[0].strip() == 'br0')
    if title.startswith('Korak 6'):
        p = outs(lambda d, c: c.startswith('ping'))
        check('K6 VPC1 gateway ping: ARP ok, ICMP timeout', 'timeout' in p[0] and 'not reachable' not in p[0])
        check('K6 VPC2 gateway ping: ARP ok, ICMP timeout', 'timeout' in p[1] and 'not reachable' not in p[1])
        n = outs(lambda d, c: c == 'show arp')
        check('K6 VPC1 learned GW10 MAC from OpenFlow', re.search(r'02:00:00:00:10:01\s+10\.10\.10\.1 ', n[0]) is not None)
        check('K6 VPC2 learned GW20 MAC from OpenFlow', re.search(r'02:00:00:00:20:01\s+10\.10\.20\.1 ', n[1]) is not None)
        # harness-only negativni testovi: VPC privremeno dobija adresu iz tudje podmreze i trazi tudji gateway
        run('vpc2', 'ip 10.10.10.99/24 10.10.10.1')
        rc, o = run('vpc2', 'ping 10.10.10.1 -c 1')
        check('K6 NEG: ARP for 10.10.10.1 asked from VLAN 20 is NOT answered', 'host (10.10.10.1) not reachable' in o)
        run('vpc2', 'ip 10.10.20.12/24 10.10.20.1')
        run('vpc1', 'ip 10.10.20.98/24 10.10.20.1')
        rc, o = run('vpc1', 'ping 10.10.20.1 -c 1')
        check('K6 NEG: ARP for 10.10.20.1 asked from VLAN 10 is NOT answered', 'host (10.10.20.1) not reachable' in o)
        run('vpc1', 'ip 10.10.10.11/24 10.10.10.1')
        rc, o = run('vpc1', 'ping 10.10.20.12 -c 2')
        check('K6 NEG: VPC1->VPC2 still fails without routing flows', 'timeout' in o and 'bytes from' not in o)
    if title.startswith('Korak 7'):
        p = outs(lambda d, c: c.startswith('ping'))
        for x in p:
            check('K7 ping works, ttl=63', re.search(r'bytes from .* ttl=63', x) is not None
                  and 'timeout' not in x and 'ttl=64' not in x)
        tr = outs(lambda d, c: c.startswith('trace'))[0]
        check('K7 trace: hop 1 = router (no ICMP time exceeded), hop 2 = VPC2',
              re.search(r'^ 1\s+\*\s+\*\s+\*', tr, re.M) is not None and re.search(r'^ 2\s+10\.10\.20\.12 ', tr, re.M) is not None)
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
        pp = outs(lambda d, c: c.startswith('ping'))[0]
        check('K8 ping ok (3 replies ttl=63)', len(re.findall(r'bytes from 10\.10\.20\.12 .*ttl=63', pp)) == 3)
        t = caps['sw2eth0']
        seqs = {}
        for l in t:
            m = re.search(r'ICMP echo request, id (\d+), seq (\d+)', l)
            if m:
                seqs.setdefault(m.groups(), []).append(l)
        ok = len(seqs) == 3 and all(len(v) == 2 and
                                    'vlan 10' in v[0] and 'ttl 64' in v[0] and f'{m1} > 02:00:00:00:10:01' in v[0] and
                                    'vlan 20' in v[1] and 'ttl 63' in v[1] and f'02:00:00:00:20:01 > {m2}' in v[1]
                                    for v in seqs.values())
        check('K8 trunk: every request seen twice: vlan10/ttl64 in, vlan20/ttl63 out', ok, f'{len(seqs)} requests')
        rep = [l for l in t if 'echo reply' in l]
        check('K8 trunk: replies vlan20/ttl64 in, vlan10/ttl63 out',
              any('vlan 20' in l and 'ttl 64' in l and f'{m2} > 02:00:00:00:20:01' in l for l in rep) and
              any('vlan 10' in l and 'ttl 63' in l and f'02:00:00:00:10:01 > {m1}' in l for l in rep))
        check('K8 VPC1 link: no 802.1Q', not any(has_tag(l) for l in caps['vpc1']))
        check('K8 VPC2 link: no 802.1Q', not any(has_tag(l) for l in caps['vpc2']))
        check('K8 VPC2 sees request from GW20 MAC with ttl 63',
              any(f'02:00:00:00:20:01 > {m2}' in l and 'ttl 63' in l and 'echo request' in l for l in caps['vpc2']))
        check('K8 VPC1 sends request to GW10 MAC with ttl 64',
              any(f'{m1} > 02:00:00:00:10:01' in l and 'ttl 64' in l and 'echo request' in l for l in caps['vpc1']))

print('\n===== SUMMARY')
for n, ok in results:
    print(('PASS ' if ok else 'FAIL ') + n)
bad = [n for n, ok in results if not ok]
print(f'{len(results) - len(bad)}/{len(results)} PASS')
sys.exit(1 if bad else 0)
