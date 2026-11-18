#!/usr/bin/env python3
"""Vezba 7 test harness (za odrzavanje, nije deo vezbe).

Pokrece tacno komande iz <pre data-dev="..."> blokova u task/resenje.frag.html, redom:
sw1/sw2 kroz `docker exec v7_swN bash -c`, a vpc1/vpc2 kroz konzolu pravog VPCS-a
(vpcs -p 5000 u kontejneru v7_vpcN, kao telnet konzola u GNS3; vidi test-setup.sh).
Konzola VPCS-a ostaje otvorena do kraja: zatvaranje konzole resetuje VPCS (gubi se IP adresa).

MikroTik NIJE dostupan: blok data-dev="mikrotik" (RouterOS) se NE izvrsava. Umesto njega se u
kontejneru v7_router (Linux, alpine + iproute2, ip_forward=1) izvrsavaju ekvivalentne komande iz
ROUTER_EMU (VLAN podinterfejsi eth0.10/eth0.20 sa istim adresama). RouterOS sintaksa je neproverena.

Tokom Koraka 6 tcpdump (kontejner sa tcpdump-om deli mrezni namespace) snima kablove
SW1 eth2-SW2 eth0, SW2 eth1-ruter, VPC1 i VPC2 - ekvivalent Wireshark snimanja u GNS3.
Na kraju harness-only negativni test: VLAN 30 (podinterfejsi na SW1 eth2 i na ruteru) ne prolazi kroz SW2.
"""
import html, os, re, select, subprocess, sys, time, pathlib

ROOT = pathlib.Path(__file__).resolve().parent.parent
SRC = (ROOT / 'task' / 'resenje.frag.html').read_text()
MAC = {}
CAPS = {'sw2eth0': ('v7_sw2', 'eth0'), 'sw2eth1': ('v7_sw2', 'eth1'),
        'vpc1': ('v7_vpc1', 'eth0'), 'vpc2': ('v7_vpc2', 'eth0')}
ROUTER_EMU = [
    'ip link add link eth0 name eth0.10 type vlan id 10',
    'ip link add link eth0 name eth0.20 type vlan id 20',
    'ip addr add 10.10.10.1/24 dev eth0.10',
    'ip addr add 10.10.20.1/24 dev eth0.20',
    'ip link set eth0.10 up',
    'ip link set eth0.20 up',
    'sysctl net.ipv4.ip_forward',
    'ip -br addr',
]
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


def run(dev, cmd):
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
    return {m.group(2): int(m.group(1)) for m in re.finditer(r'n_packets=(\d+),.*?(priority=\d+\S*)', out)}


def has_tag(l):
    return '802.1Q' in l


def req(l):
    return 'ICMP echo request' in l


tokens = re.finditer(r'<h3>(.*?)</h3>|<pre data-dev="(\w+)"><code>(.*?)</code></pre>', SRC, re.S)
sections = []
for t in tokens:
    if t.group(1):
        sections.append((html.unescape(t.group(1)), []))
    else:
        sections[-1][1].append((t.group(2), html.unescape(t.group(3))))

MAC['router'] = sh(['docker', 'exec', 'v7_router', 'cat', '/sys/class/net/eth0/address'])[1].strip()
MAC['sw1eth2'] = sh(['docker', 'exec', 'v7_sw1', 'cat', '/sys/class/net/eth2/address'])[1].strip()

for title, blocks in sections:
    print(f'\n##### {title}')
    if title.startswith('Korak 6'):
        f_before = flows_counts(run('sw2', 'ovs-ofctl --names dump-flows br0')[1])
        cap_start()
    sec_out = []
    for dev, code in blocks:
        cmds, cur = [], ''
        for line in code.split('\n'):
            cur = cur + '\n' + line if cur else line
            if not line.endswith('\\'):
                cmds.append(cur); cur = ''
        if dev == 'mikrotik':
            print('[MikroTik] RouterOS komande NISU izvrsene (nema MikroTik slike):')
            for c in cmds:
                print('    ' + c)
            print('[MikroTik] EMULACIJA: Linux ruter v7_router izvrsava ekvivalent:')
            for c in ROUTER_EMU:
                rc, out = sh(['docker', 'exec', 'v7_router', 'sh', '-c', c])
                print(f'[router]# {c}\n{out.rstrip()}\n(rc={rc})')
                sec_out.append(('router', c, rc, out))
            continue
        for c in cmds:
            rc, out = run(dev, c)
            sec_out.append((dev, c, rc, out))
    outputs[title] = sec_out

    def outs(pred):
        return [o for d, c, r, o in sec_out if pred(d, c)]

    if title.startswith('Korak 2'):
        for dev, ip in (('vpc1', '10.10.10.11/24'), ('vpc2', '10.10.20.12/24')):
            s = run(dev, 'show')[1]   # harness-only: MAC adresa za provere snimaka
            m = re.search(r'^VPC\d\s+(\S+)\s+(\S+)\s+([0-9a-f:]{17})', s, re.M)
            check(f'K2 {dev}: IP and gateway set', bool(m) and m.group(1) == ip, m.group(0) if m else s)
            if m:
                MAC[dev] = m.group(3)
        print('MAC:', MAC)
    if title.startswith('Korak 3'):
        s = outs(lambda d, c: c == 'ovs-vsctl show')[-1]
        check('K3 tags/trunks set', 'tag: 10' in s and 'tag: 20' in s and s.count('trunks: [10, 20]') == 3)
        check('K3 patch ports present, no error',
              'peer=br2-br0' in s and 'peer=br0-br2' in s and 'error' not in s.lower())
    if title.startswith('Korak 4'):
        a = outs(lambda d, c: c == 'ip -br addr')[-1]
        check('K4 (EMULATED router) sub-interfaces 10.10.10.1 / 10.10.20.1',
              re.search(r'eth0\.10@eth0\s+UP\s+10\.10\.10\.1/24', a) is not None and
              re.search(r'eth0\.20@eth0\s+UP\s+10\.10\.20\.1/24', a) is not None, a)
        check('K4 (EMULATED router) ip_forward=1', 'ip_forward = 1' in outs(lambda d, c: 'sysctl' in c)[0])
    if title.startswith('Korak 5'):
        check('K5 eth0 in br0', outs(lambda d, c: 'port-to-br' in c)[0].strip() == 'br0')
        p = outs(lambda d, c: c.startswith('ping'))[0]
        check('K5 without SW2 flows (no NORMAL): VPC1->VPC2 fails, gateway not reachable',
              'host (10.10.10.1) not reachable' in p and 'bytes from' not in p, p)
        f = outs(lambda d, c: 'dump-flows' in c)[-1]
        check('K5 exactly 5 flows, no NORMAL', f.count('actions=') == 5 and 'NORMAL' not in f)
    if title.startswith('Korak 6'):
        caps = cap_stop('K6')
        p = outs(lambda d, c: c.startswith('ping'))
        check('K6 VPC1 -> gateway 10.10.10.1', len(re.findall(r'bytes from 10\.10\.10\.1 .*ttl=64', p[0])) == 2, p[0])
        check('K6 VPC1 -> VPC2 ttl=63', len(re.findall(r'bytes from 10\.10\.20\.12 .*ttl=63', p[1])) == 2, p[1])
        check('K6 VPC2 -> gateway 10.10.20.1', len(re.findall(r'bytes from 10\.10\.20\.1 .*ttl=64', p[2])) == 2, p[2])
        check('K6 VPC2 -> VPC1 ttl=63', len(re.findall(r'bytes from 10\.10\.10\.11 .*ttl=63', p[3])) == 2, p[3])
        m1, m2, mr = MAC['vpc1'], MAC['vpc2'], MAC['router']
        for k in ('sw2eth0', 'sw2eth1'):
            t = caps[k]
            check(f'K6 {k}: VPC1->VPC2 request vlan 10 ttl 64 {m1} > router',
                  any(req(l) and 'vlan 10' in l and 'ttl 64' in l and f'{m1} > {mr}' in l and '> 10.10.20.12' in l for l in t))
            check(f'K6 {k}: VPC1->VPC2 request vlan 20 ttl 63 router > {m2}',
                  any(req(l) and 'vlan 20' in l and 'ttl 63' in l and f'{mr} > {m2}' in l and '> 10.10.20.12' in l for l in t))
            check(f'K6 {k}: every frame is 802.1Q tagged (10 or 20)',
                  len(t) > 0 and all(has_tag(l) and re.search(r'vlan (10|20),', l) for l in t))
        for k in ('vpc1', 'vpc2'):
            check(f'K6 {k} cable: no 802.1Q frames', len(caps[k]) > 0 and not any(has_tag(l) for l in caps[k]))
        check('K6 VPC2 cable: request from router MAC ttl 63',
              any(req(l) and f'{mr} > {m2}' in l and 'ttl 63' in l for l in caps['vpc2']))
        f_after = flows_counts(run('sw2', 'ovs-ofctl --names dump-flows br0')[1])
        delta = {k: f_after[k] - f_before.get(k, 0) for k in f_after}
        print('flow deltas K6', delta)
        check('K6 all four forwarding flows used', all(v > 0 for k, v in delta.items() if 'dl_vlan' in k)
              and sum(1 for k in delta if 'dl_vlan' in k) == 4)

# ---- harness-only: VLAN 30 (nije u listi) mora da se odbaci na SW2, u oba smera
print('\n##### NEG: VLAN 30 kroz SW2 (harness-only)')
subprocess.run(['docker', 'rm', '-f', 'v7_inj'], capture_output=True)
subprocess.run(['docker', 'run', '-d', '--name', 'v7_inj', '--privileged', '--net', 'container:v7_sw1',
                'v7-router:local', 'sleep', 'infinity'], capture_output=True, check=True)
for ctr, ifc, ip in (('v7_inj', 'eth2', '10.10.30.2/24'), ('v7_router', 'eth0', '10.10.30.1/24')):
    sh(['docker', 'exec', ctr, 'sh', '-c',
        f'ip link add link {ifc} name {ifc}.30 type vlan id 30 && ip addr add {ip} dev {ifc}.30 && ip link set {ifc}.30 up'])
if os.environ.get('V7_LEAK30'):  # samo-provera harness-a: propusti VLAN 30, NEG provere moraju da padnu
    run('sw2', 'ovs-ofctl add-flow br0 "priority=100,dl_vlan=30,actions=NORMAL"')
f0 = flows_counts(run('sw2', 'ovs-ofctl --names dump-flows br0')[1])
cap_start()
rc1, o1 = sh(['docker', 'exec', 'v7_inj', 'ping', '-c', '3', '-W', '1', '10.10.30.1'])
rc2, o2 = sh(['docker', 'exec', 'v7_router', 'ping', '-c', '3', '-W', '1', '10.10.30.2'])
print(o1, o2)
caps = cap_stop('NEG30')
f1 = flows_counts(run('sw2', 'ovs-ofctl --names dump-flows br0')[1])
drop = [f1[k] - f0.get(k, 0) for k in f1 if k.startswith('priority=0')]
ms, mr = MAC['sw1eth2'], MAC['router']
check('NEG VLAN 30 ping SW1 side -> router fails', rc1 != 0 and ' 0 packets received' in o1)
check('NEG VLAN 30 ping router -> SW1 side fails', rc2 != 0 and ' 0 packets received' in o2)
check('NEG VLAN 30 frames from SW1 reach SW2 eth0 (injection works)',
      any('vlan 30' in l and l.split()[1] == ms for l in caps['sw2eth0']))
check('NEG VLAN 30 frames from router reach SW2 eth1 (injection works)',
      any('vlan 30' in l and l.split()[1] == mr for l in caps['sw2eth1']))
check('NEG no VLAN 30 frame forwarded SW1 -> router (none on eth1 from SW1)',
      not any('vlan 30' in l and l.split()[1] == ms for l in caps['sw2eth1']))
check('NEG no VLAN 30 frame forwarded router -> SW1 (none on eth0 from router)',
      not any('vlan 30' in l and l.split()[1] == mr for l in caps['sw2eth0']))
check('NEG drop flow counter grew', drop and drop[0] >= 6, str(drop))
subprocess.run(['docker', 'rm', '-f', 'v7_inj'], capture_output=True)

print('\n===== SUMMARY')
for n, ok in results:
    print(('PASS ' if ok else 'FAIL ') + n)
bad = [n for n, ok in results if not ok]
print(f'{len(results) - len(bad)}/{len(results)} PASS')
sys.exit(1 if bad else 0)
