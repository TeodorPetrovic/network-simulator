#!/usr/bin/env python3
"""Vezba 9 test harness (za odrzavanje, nije deo vezbe).

Izvrsava blokove iz task/resenje.frag.html tacno kako su zapisani, redom:
- <pre data-netcfg="nagios">: /etc/network/interfaces (GNS3 "Edit network configuration"); isti sadrzaj
  sa svojom adresom za SYSLOG/SERVER1/SERVER2. Cvorovi se pokrecu sa init.sh shimom simulatora
  (src/runtime/resources, BusyBox ifup), kao u GNS3/simulatoru.
- <pre data-dev="nagios|syslog|server1|server2">: ceo blok se nalepi u otvorenu konzolu: NAGIOS kroz
  pomocnu konzolu simulatora (docker exec -i X /gns3/bin/busybox sh), ostali kroz /bin/sh slike (glavna
  konzola). Napomena: BusyBox sh pomocne konzole koristi svoje aplete (npr. logger bez -n) umesto programa slike.
- <pre data-dev="vpc1">: red po red kroz telnet konzolu pravog VPCS-a.
- data-dev="mikrotik|sonic": NE izvrsava se (uredjaja nema).
- data-wait="k3|k4|k7": harness pre bloka ceka (najvise ~4 min) da Nagios dodje u ocekivano stanje.
Dockerfile slike vezba9-linux uzima se iz task/zadatak.frag.html (<pre data-file="Dockerfile">).
Docker-only detalj (nije deo vezbe): Docker IP adresa i default ruta se brisu sa eth0, a TX checksum
offload se iskljucuje (ethtool -K eth0 tx off) jer userspace OVS (netdev) ne racuna delimicne checksume.
"""
import html, json, os, re, select, shutil, subprocess, sys, time, pathlib

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parent
SRC = (ROOT / 'task' / 'resenje.frag.html').read_text()
TASK = (ROOT / 'task' / 'zadatak.frag.html').read_text()
SIM = pathlib.Path('/home/teodor/projects/network-simulator/src/runtime/resources')
STATE = pathlib.Path(os.environ.get('V9_STATE', os.path.join(os.environ.get('TMPDIR', '/tmp'), 'v9-state')))
NODES = {  # cvor: (mreza/kabl, adresa, slika, komanda)
    'nagios': ('v9_n0', '10.9.0.10', 'jasonrivers/nagios:latest', ['/usr/local/bin/start_nagios']),
    'syslog': ('v9_n1', '10.9.0.20', 'vezba9-linux:latest', ['/bin/sh']),
    'server1': ('v9_n2', '10.9.0.31', 'vezba9-linux:latest', ['/bin/sh']),
    'server2': ('v9_n3', '10.9.0.32', 'vezba9-linux:latest', ['/bin/sh']),
}
LAB_HOSTS = ['SYSLOG', 'SERVER1', 'SERVER2', 'VPC1']
results = []


def sh(cmd, inp=None):
    p = subprocess.run(cmd, shell=isinstance(cmd, str), capture_output=True, text=True, input=inp)
    return p.returncode, p.stdout + p.stderr


def check(name, ok, detail=''):
    results.append((name, ok))
    print(f'CHECK {"PASS" if ok else "FAIL"}: {name} {detail if not ok else ""}'.rstrip())


class Con:
    """Konzola koja ostaje otvorena (shell preko docker exec -i)."""
    n = 0

    def __init__(self, ctr, shell):
        self.p = subprocess.Popen(['docker', 'exec', '-i', ctr] + shell,
                                  stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, bufsize=0)

    def run(self, script, timeout=120):
        Con.n += 1
        mark = f'__V9_DONE_{Con.n}__'
        self.p.stdin.write((script + f'\necho "{mark} $?"\n').encode())
        buf, end = b'', time.time() + timeout
        while time.time() < end:
            r, _, _ = select.select([self.p.stdout], [], [], 0.2)
            if r:
                d = os.read(self.p.stdout.fileno(), 65536)
                if not d:
                    break
                buf += d
                m = re.search(mark.encode() + rb' (\d+)\n', buf)
                if m:
                    return int(m.group(1)), buf[:m.start()].decode('utf8', 'replace')
        return -1, buf.decode('utf8', 'replace') + '\n[TIMEOUT]'


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

    def cmd(self, c, timeout=60):
        self.p.stdin.write((c + '\n').encode())
        out = self._read(timeout)
        out = re.sub(rb'\xff[\xfb-\xfe].', b'', out).decode('utf8', 'replace').replace('\r', '')
        lines = out.split('\n')
        if lines and lines[0].strip() == c.strip():
            lines = lines[1:]
        if lines and re.match(r'^[\w-]+> $', lines[-1]):
            lines = lines[:-1]
        return '\n'.join(lines).strip('\n')


CON = {}


def con(dev):
    if dev not in CON:
        # NAGIOS: pomocna (aux) konzola = /gns3/bin/busybox sh; ostali: glavna konzola = /bin/sh slike
        CON[dev] = VCon(f'v9_{dev}') if dev == 'vpc1' else \
            Con(f'v9_{dev}', ['/gns3/bin/busybox', 'sh'] if dev == 'nagios' else ['/bin/sh'])
    return CON[dev]


def start_node(name, netcfg):
    net, ip, image, cmd = NODES[name]
    d = STATE / name
    d.mkdir(parents=True, exist_ok=True)
    (d / 'interfaces').write_text(netcfg.replace('10.9.0.10', ip) + '\n')
    (d / 'expected-eth').write_text('eth0\n')
    args = ['docker', 'run', '-d', '-i', '-t', '--name', f'v9_{name}', '--hostname', name.upper(), '--privileged',
            '--network', net, '--entrypoint', '/gns3/init.sh',
            '-v', f'{STATE}/shim/init.sh:/gns3/init.sh:ro', '-v', f'{STATE}/shim/run-cmd.sh:/gns3/run-cmd.sh:ro',
            '-v', f'{d}/expected-eth:/gns3/expected-eth:ro', '-v', f'{d}/interfaces:/etc/network/interfaces',
            '-v', '/usr/bin/busybox:/gns3/bin/busybox:ro', '-e', 'GNS3_MAX_ETHERNET=eth0', image] + cmd
    rc, out = sh(args)
    assert rc == 0, out
    for _ in range(60):
        rc, a = sh(['docker', 'exec', f'v9_{name}', '/gns3/bin/busybox', 'ip', '-4', '-o', 'addr', 'show', 'eth0'])
        if ip in a:
            break
        time.sleep(0.5)
    check(f'K2 {name}: ifup applied {ip} from network config', ip in a, a)
    # Docker-only: ukloni Docker IPAM adresu i default rutu, iskljuci TX checksum offload
    for m in re.findall(r'inet (172\.31\.\d+\.\d+/\d+)', a):
        sh(['docker', 'exec', f'v9_{name}', '/gns3/bin/busybox', 'ip', 'addr', 'del', m, 'dev', 'eth0'])
    sh(['docker', 'exec', f'v9_{name}', '/gns3/bin/busybox', 'ip', 'route', 'del', 'default'])
    sh(['docker', 'run', '--rm', '--net', f'container:v9_{name}', '--cap-add', 'NET_ADMIN', 'v9-tools:local',
        'ethtool', '-K', 'eth0', 'tx', 'off'])


def status(query):
    rc, out = sh(['docker', 'exec', 'v9_nagios', 'curl', '-s', '-u', 'nagiosadmin:nagios',
                  f'http://localhost/nagios/cgi-bin/statusjson.cgi?{query}'])
    try:
        return json.loads(out)['data']
    except Exception:
        return {}


def wait_for(name, pred, timeout=260):
    t0 = time.time()
    while time.time() - t0 < timeout:
        try:
            if pred():
                print(f'[wait] {name}: ok after {time.time() - t0:.0f}s')
                return True
        except Exception:
            pass
        time.sleep(5)
    print(f'[wait] {name}: TIMEOUT')
    return False


WAITS = {
    'k3': lambda: all(status('query=hostlist').get('hostlist', {}).get(h) == 2 for h in LAB_HOSTS),
    'k4': lambda: (lambda s: len(s) == 2 and all(v.get('HTTP') == 2 and v.get('SNMP uptime') == 2 for v in s.values()))(
        status('query=servicelist&hostgroup=serveri').get('servicelist', {})),
    'k7': lambda: (lambda h: h.get('status') == 4 and h.get('state_type') == 1)(
        status('query=host&hostname=SERVER2').get('host', {})),
}

# ---- slika iz Dockerfile-a u zadatku
dockerfile = html.unescape(re.search(r'<pre data-file="Dockerfile"><code>(.*?)</code></pre>', TASK, re.S).group(1))
rc, out = sh(['docker', 'build', '-q', '-t', 'vezba9-linux:latest', '-'], inp=dockerfile + '\n')
check('K1 vezba9-linux image builds from Dockerfile in task', rc == 0, out)
shutil.rmtree(STATE, ignore_errors=True)
(STATE / 'shim').mkdir(parents=True)
for f in ('init.sh', 'run-cmd.sh'):
    shutil.copy(SIM / f, STATE / 'shim' / f)
    os.chmod(STATE / 'shim' / f, 0o755)

tokens = re.finditer(r'<h3>(.*?)</h3>|<pre data-(dev|netcfg)="([\w ]+)"( data-wait="(\w+)")?><code>(.*?)</code></pre>', SRC, re.S)
sections = []
for t in tokens:
    if t.group(1):
        sections.append((html.unescape(t.group(1)), []))
    else:
        sections[-1][1].append((t.group(2), t.group(3), t.group(5), html.unescape(t.group(6))))

OUT = {}
for title, blocks in sections:
    print(f'\n##### {title}')
    k = title.split(' ')[1]
    if k == '7':
        print('[GNS3] Stop SERVER2  ->  docker stop v9_server2')
        sh(['docker', 'stop', '-t', '2', 'v9_server2'])
        CON.pop('server2', None)
    for kind, devs, wait, code in blocks:
        if kind == 'netcfg':
            print(f'[network config]\n{code}')
            for n in NODES:
                start_node(n, code)
            continue
        if wait:
            check(f'K{k} wait {wait}', wait_for(wait, WAITS[wait]))
        for dev in devs.split():
            if dev in ('mikrotik', 'sonic'):
                print(f'[{dev}] NIJE IZVRSENO (uredjaj nije dostupan):\n' + '\n'.join('    ' + l for l in code.split('\n')))
                continue
            if dev == 'vpc1':
                out = '\n'.join(con(dev).cmd(c) for c in code.split('\n'))
                rc = 0
            else:
                rc, out = con(dev).run(code)
            print(f'[{dev}]$ {code}\n{out.rstrip()}\n(rc={rc})')
            OUT.setdefault(k, []).append((dev, code, rc, out))

    def o(dev, sub=''):
        return '\n'.join(x for d, c, r, x in OUT.get(k, []) if d == dev and sub in c)

    if k == '2':
        check('K2 VPC1 ip set', 'VPC1 : 10.9.0.50 255.255.255.0' in o('vpc1'), o('vpc1'))
        for s in ('server1', 'server2'):
            out = o(s)
            check(f'K2 {s}: httpd and snmpd running', 'httpd -h /var/www' in out and re.search(r'\ssnmpd\s*$', out, re.M), out)
        rc, page = sh(['docker', 'exec', 'v9_nagios', 'curl', '-s', '-m', '5', 'http://10.9.0.32/'])
        check('K2 HTTP from NAGIOS to SERVER2 shows its name', '<h1>SERVER2</h1>' in page, page)
        rc, p = sh(['docker', 'exec', 'v9_nagios', 'ping', '-c', '2', '-W', '2', '10.9.0.50'])
        check('K2 NAGIOS pings VPC1', rc == 0, p)
    if k == '3':
        out = o('nagios', 'lab9.cfg')
        check('K3 config check: 0 errors, reload ok', 'Total Errors:   0' in out and 'ok: run: nagios' in out, out)
        hl = json.loads(re.search(r'\{.*\}', o('nagios', 'hostlist')).group(0))
        check('K3 lab hosts UP (2)', all(hl.get(h) == 2 for h in LAB_HOSTS), str(hl))
        check('K3 R1 and SONiC defined (absent in test -> DOWN 4 or PENDING 1)',
              hl.get('R1') in (1, 4) and hl.get('SONiC') in (1, 4), str(hl))
    if k == '4':
        out = o('nagios', 'nadzor.cfg')
        check('K4 config check: 0 errors, reload ok', 'Total Errors:   0' in out and 'ok: run: nagios' in out, out)
        sl = json.loads(re.search(r'\{.*\}', o('nagios', 'servicelist')).group(0))
        check('K4 HTTP and SNMP uptime OK on SERVER1 and SERVER2',
              all(sl.get(h, {}).get(s) == 2 for h in ('SERVER1', 'SERVER2') for s in ('HTTP', 'SNMP uptime')), str(sl))
        obj = sh(['docker', 'exec', 'v9_nagios', 'curl', '-s', '-u', 'nagiosadmin:nagios',
                  'http://localhost/nagios/cgi-bin/objectjson.cgi?query=contactgroup&contactgroup=admins'])[1]
        check('K4 contact dezurni is member of admins', '"dezurni"' in obj, obj)
        so = status('query=service&hostname=SERVER1&servicedescription=SNMP+uptime').get('service', {})
        check('K4 SNMP uptime plugin output is SNMP OK - Timeticks', so.get('plugin_output', '').startswith('SNMP OK - Timeticks'),
              str(so.get('plugin_output')))
    if k == '5':
        out = o('syslog', 'mreza.conf')
        check('K5 rsyslog config valid and UDP 514 listening', 'End of config validation run' in out and
              re.search(r'udp\s.*0\.0\.0\.0:514', out), out)
        log = o('syslog', 'SERVER1.log')
        check('K5 logger message from SERVER1 in /var/log/mreza/SERVER1.log',
              re.search(r'SERVER1 proba Poruka sa SERVER1', log) is not None, log)
        rc, m = sh(['docker', 'exec', 'v9_syslog', 'sh', '-c', 'cat /var/log/messages 2>/dev/null | grep -c "Poruka sa"'])
        check('K5 remote message not in /var/log/messages', m.strip() == '0', m)
    if k == '6':
        out = o('nagios')
        check('K6 snmpget sysName/sysLocation/sysUpTime', 'sysName.0 = STRING: SERVER1' in out and
              'sysLocation.0 = STRING: Laboratorija 9' in out and 'sysUpTime.0 = Timeticks' in out, out)
        check('K6 snmpwalk ifDescr/ifOperStatus eth0 up', 'ifDescr.2 = STRING: eth0' in out and
              re.search(r'ifOperStatus\.2 = INTEGER: up\(1\)', out) is not None, out)
        check('K6 community public -> Timeout', 'Timeout: No Response from 10.9.0.31' in out, out)
        check('K6 no MIB warnings with -m', 'Cannot find module' not in out, out)
    if k == '7':
        out = o('nagios')
        check('K7 SERVER2 DOWN HARD attempt 2', '"status":4,"state_type":1,"current_attempt":2' in out, out)
        check('K7 SERVER2 HTTP and SNMP uptime CRITICAL (16)', '"HTTP":16' in out and '"SNMP uptime":16' in out, out)
        log = o('syslog')
        check('K7 host notification on SYSLOG', re.search(r'NAGIOS nagios HOST PROBLEM: SERVER2 je DOWN$', log, re.M) is not None, log)
        check('K7 no service notification for SERVER2', 'SERVIS' not in log, log)
        hl = status('query=hostlist').get('hostlist', {})
        check('K7 other lab hosts still UP', all(hl.get(h) == 2 for h in ('SYSLOG', 'SERVER1', 'VPC1')), str(hl))

print('\n===== SUMMARY')
for n, ok in results:
    print(('PASS ' if ok else 'FAIL ') + n)
bad = [n for n, ok in results if not ok]
print(f'{len(results) - len(bad)}/{len(results)} PASS')
sys.exit(1 if bad else 0)
