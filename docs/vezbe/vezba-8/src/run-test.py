#!/usr/bin/env python3
"""Vezba 8 test harness (za odrzavanje, nije deo vezbe).

Izvrsava tacno komande iz <pre data-dev="..."> blokova u task/resenje.frag.html, redom:
  r1        -> serijska konzola pravog MikroTik CHR-a (RouterOS 7.16.2, QEMU/KVM u v8_r1), red po red
               kao da se kuca (i nastavak reda sa \\)
  sw1, sw2  -> docker-sonic-vs (v8_swN), svaka komanda kroz `docker exec ... bash -c`
  vpc1/vpc2 -> konzola pravog VPCS-a (kao telnet konzola u GNS3)
Zatim provere (PASS/FAIL) i dodatne provere sa servera za nadzor v8_srv (SNMP, syslog) i NAT cvora v8_nat.
Pun izlaz ide na stdout (preusmeriti u ../test-log.md).
"""
import html, os, re, select, subprocess, sys, time, pathlib, json

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from ros import Ros  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parent.parent
SRC = (ROOT / 'task' / 'resenje.frag.html').read_text()
ADMIN_PW = 'Vezba8admin'
results, OUT = [], []


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
R1 = None


def sh(cmd):
    p = subprocess.run(cmd, shell=isinstance(cmd, str), capture_output=True, text=True)
    return p.returncode, p.stdout + p.stderr


def run(dev, cmd):
    global R1
    if dev.startswith('vpc'):
        if dev not in CON:
            CON[dev] = VCon(f'v8_{dev}')
        rc, out = 0, CON[dev].cmd(cmd)
        print(f'[{dev.upper()}> {cmd}')
    elif dev == 'r1':
        if R1 is None:
            R1 = Ros('v8_r1')
            R1.login(newpw=ADMIN_PW)
            print('[R1] prijava admin (prazna lozinka) -> licenca n -> nova lozinka ' + ADMIN_PW)
        parts = cmd.split('\n')
        for p in parts[:-1]:       # nastavak reda sa \ : kuca se red po red
            R1.send(p + '\r'); R1.read(1)
        if cmd == '/file print':
            time.sleep(3)          # backup/export se upisuju u pozadini
        out = R1.cmd(parts[-1])
        rc = 1 if re.search(r'bad command|syntax error|expected |failure:|invalid value|no such item|'
                            r'ambiguous', out) else 0
        print(f'[admin@R1] > {cmd}')
    else:
        rc, out = sh(['docker', 'exec', f'v8_{dev}', 'bash', '-c', cmd])
        print(f'admin@{dev.upper()}:~$ {cmd}')
        m = re.match(r'sudo config hostname (\S+)$', cmd)
        if m:  # zamena za hostcfgd (ne postoji u docker-sonic-vs): primeni ime na sistem
            sh(['docker', 'exec', f'v8_{dev}', 'sh', '-c', f'hostname {m.group(1)}; echo "127.0.0.1 {m.group(1)}" >> /etc/hosts'])
    if out.strip():
        print(out.rstrip())
    print(f'(rc={rc})')
    OUT.append((dev, cmd, rc, out))
    return rc, out


def check(name, ok, detail=''):
    results.append((name, ok))
    print(f'CHECK {"PASS" if ok else "FAIL"}: {name} {detail if not ok else ""}')


def last(pred):
    o = [x for x in OUT if pred(x[0], x[1])]
    return o[-1][3] if o else ''


# wait for CHR boot (konzola)
tokens = re.finditer(r'<h3>(.*?)</h3>|<pre data-dev="(\w+)"><code>(.*?)</code></pre>', SRC, re.S)
sections = []
for t in tokens:
    if t.group(1):
        sections.append((html.unescape(t.group(1)), []))
    else:
        sections[-1][1].append((t.group(2), html.unescape(t.group(3))))

for title, blocks in sections:
    print(f'\n##### {title}')
    for dev, code in blocks:
        cmds, cur = [], ''
        for line in code.split('\n'):
            cur = cur + '\n' + line if cur else line
            if not line.endswith('\\'):
                cmds.append(cur); cur = ''
        for c in cmds:
            rc, out = run(dev, c)
            if dev.startswith('sw') and (rc != 0 or 'Error' in out or 'Traceback' in out):
                check(f'{dev} command ok: {c}', False, out)

    if title.startswith('Korak 3'):
        for sw in ('sw1', 'sw2'):
            v = run(sw, 'show vlan config')[1]
            print(v)
            exp = {'sw1': {('10', 'Ethernet0', 'tagged'), ('99', 'Ethernet0', 'tagged'), ('10', 'Ethernet4', 'tagged'),
                           ('20', 'Ethernet4', 'tagged'), ('99', 'Ethernet4', 'tagged'), ('10', 'Ethernet8', 'untagged')},
                   'sw2': {('20', 'Ethernet0', 'untagged'), ('10', 'Ethernet4', 'tagged'), ('20', 'Ethernet4', 'tagged'),
                           ('99', 'Ethernet4', 'tagged'), ('20', 'Ethernet8', 'untagged'),
                           ('99', 'Ethernet12', 'untagged')}}[sw]
            got = set(re.findall(r'Vlan\d+\s+(\d+)\s+(Ethernet\d+)\s+(tagged|untagged)', v))
            check(f'K3 {sw} VLAN members as in table', got == exp, f'{got ^ exp}')
            ipi = run(sw, 'show ip interfaces')[1]
            ip = '10.10.99.2/24' if sw == 'sw1' else '10.10.99.3/24'
            check(f'K3 {sw} Vlan99 {ip}, no 10.0.0.x on used ports',
                  re.search(r'Vlan99\s+' + re.escape(ip), ipi) is not None and
                  not re.search(r'Ethernet(0|4|8|12)\s+10\.0\.0', ipi), ipi)
            cfg = json.loads(sh(['docker', 'exec', f'v8_{sw}', 'cat', '/etc/sonic/config_db.json'])[1])
            check(f'K3 {sw} saved config_db.json: hostname, VLANs, SNMP community',
                  cfg['DEVICE_METADATA']['localhost']['hostname'] == sw.upper() and
                  {'Vlan10', 'Vlan20', 'Vlan99'} <= set(cfg.get('VLAN', {})) and
                  cfg.get('SNMP_COMMUNITY', {}).get('lab8', {}).get('TYPE') == 'RO')
    if title.startswith('Korak 4'):
        a = last(lambda d, c: c == '/ip address print')
        check('K4 R1 addresses', all(re.search(p, a) for p in (
            r'10\.10\.10\.1/24\s+10\.10\.10\.0\s+vlan10', r'10\.10\.20\.1/24\s+10\.10\.20\.0\s+ether3',
            r'10\.10\.99\.1/24\s+10\.10\.99\.0\s+vlan99', r'D 192\.168\.122\.\d+/24\s+192\.168\.122\.0\s+ether1')), a)
        d = last(lambda d, c: c == '/ip dhcp-client print')
        check('K4 R1 dhcp-client ether1 bound', re.search(r'ether1\s.*bound', d) is not None, d)
        u = run('r1', '/user print')[1]
        check('K4 R1 user tehnicar group read', re.search(r'tehnicar\s+read', u) is not None, u)
        check('K4 R1 identity R1', run('r1', '/system identity print')[1].strip().endswith('name: R1'))
    if title.startswith('Korak 5'):
        check('K5 R1 no command errors', all(rc == 0 for d, c, rc, o in OUT if d == 'r1'))
        f = last(lambda d, c: c == '/file print')
        check('K5 R1-v8.backup and R1-v8.rsc exist', 'R1-v8.backup' in f and 'R1-v8.rsc' in f, f)
        s = last(lambda d, c: c == '/snmp community print')
        check('K5 single community lab8 10.10.99.0/24',
              re.search(r'lab8\s+10\.10\.99\.0/24', s) is not None and 'public' not in s, s)
        rsc = run('r1', '/file print detail where name=R1-v8.rsc')[1]
        print(rsc)
    if title.startswith('Korak 6'):
        ipd = last(lambda d, c: d == 'vpc1' and c == 'ip dhcp')
        m = re.search(r'IP (10\.10\.10\.(\d+))/24 GW 10\.10\.10\.1', ipd)
        check('K6 VPC1 DHCP from pool, GW 10.10.10.1', m is not None and 100 <= int(m.group(2)) <= 199, ipd)
        pings = {(d, c.split()[1]): o for d, c, rc, o in OUT if c.startswith('ping')}
        ok = lambda o, ttl=None: len(re.findall(r'bytes from .*ttl=' + (ttl or r'\d+'), o)) == 2
        check('K6 VPC1 -> VPC2 ttl=63', ok(pings[('vpc1', '10.10.20.12')], '63'))
        check('K6 VPC1 -> 8.8.8.8', ok(pings[('vpc1', '8.8.8.8')]))
        check('K6 VPC1 -> 10.10.99.1 (input chain) ok', ok(pings[('vpc1', '10.10.99.1')], '64'))
        check('K6 VPC1 -> 10.10.99.2 blocked', 'bytes from' not in pings[('vpc1', '10.10.99.2')])
        v1 = [k for k in pings if k[0] == 'vpc2'][0]
        check('K6 VPC2 -> VPC1 ttl=63', m is not None and v1[1] == m.group(1) and ok(pings[v1], '63'))
        n = last(lambda d, c: c == '/ip neighbor print')
        check('K6 R1 LLDP neighbors on ether2 and ether3', re.search(r'ether2\s', n) is not None and
              re.search(r'ether3\s', n) is not None, n)

# ---- dodatne provere (harness-only), sa servera za nadzor / NAT cvora / SONiC-a
print('\n##### dodatne provere')
rc, o = sh('docker exec v8_srv snmpget -v2c -c lab8 -t 2 -r 1 10.10.99.1 SNMPv2-MIB::sysName.0 '
           'SNMPv2-MIB::sysLocation.0 SNMPv2-MIB::sysContact.0')
print(o)
check('X SNMP R1 lab8 from 10.10.99.10: sysName R1, location, contact',
      'STRING: R1' in o and 'Server sala' in o and 'admin@firma.rs' in o, o)
rc, o = sh('docker exec v8_srv snmpget -v2c -c public -t 2 -r 0 10.10.99.1 SNMPv2-MIB::sysName.0')
print(o)
check('X SNMP R1 community public no longer works', 'STRING' not in o)
for sw, ip in (('SW1', '10.10.99.2'), ('SW2', '10.10.99.3')):
    rc, o = sh(f'docker exec v8_srv ping -c 2 -W 2 {ip}')
    check(f'X server -> {sw} {ip} (VLAN 99 over trunk)', rc == 0, o)
    rc, o = sh(f'docker exec v8_srv snmpget -v2c -c lab8 -t 2 -r 1 {ip} SNMPv2-MIB::sysDescr.0')
    print(o)
    check(f'X SNMP {sw} lab8 from 10.10.99.10 (snmpd docker-sonic-vs)', 'SONiC Software Version' in o, o)
run('r1', ':log info "vezba8 syslog test"')
time.sleep(2)
rc, o = sh('docker exec v8_srv cat /tmp/syslog.log')
print(o[-1500:])
check('X syslog from R1 received at 10.10.99.10:514', 'vezba8 syslog test' in o)
# NAT: snimak na NAT cvoru dok VPC1 pinguje 8.8.8.8
sh('docker rm -f v8_cap >/dev/null 2>&1')
sh('docker run -d --name v8_cap --net container:v8_nat v8-tools:local tcpdump -l -n -i eth0 icmp')
time.sleep(1.5)
run('vpc1', 'ping 8.8.8.8 -c 1')
time.sleep(1)
sh('docker kill --signal=INT v8_cap'); sh('docker wait v8_cap')
_, cap = sh('docker logs v8_cap'); sh('docker rm -f v8_cap')
print(cap)
wan = re.search(r'192\.168\.122\.\d+', last(lambda d, c: c == '/ip address print')).group(0)
check('X NAT: echo request at NAT node has source = R1 ether1 address',
      re.search(re.escape(wan) + r' > 8\.8\.8\.8: ICMP echo request', cap) is not None, cap)
for sw in ('v8_sw1', 'v8_sw2'):
    _, o = sh(['docker', 'exec', sw, 'redis-cli', '-n', '0', 'keys', 'LLDP_ENTRY_TABLE:*'])
    print(f'[info] {sw} LLDP_ENTRY_TABLE: {o.split()}')
    for k in o.split():
        _, e = sh(['docker', 'exec', sw, 'redis-cli', '-n', '0', 'hmget', k, 'lldp_rem_sys_name',
                   'lldp_rem_port_id', 'lldp_rem_port_desc'])
        print('   ', k, e.split('\n'))

print('\n===== SUMMARY')
for n, ok in results:
    print(('PASS ' if ok else 'FAIL ') + n)
bad = [n for n, ok in results if not ok]
print(f'{len(results) - len(bad)}/{len(results)} PASS')
sys.exit(1 if bad else 0)
