#!/usr/bin/env python3
"""Provera adresnog plana Vežbe 8 (za održavanje): čita tabele iz task/zadatak.frag.html i komande iz
task/resenje.frag.html i proverava da se podmreže ne preklapaju, da su gateway-i adrese R1 u istoj podmreži,
da je DHCP opseg u VLAN-u 10 bez gateway-a i da komande u rešenju koriste iste adrese i VLAN-ove kao tabele."""
import html, ipaddress as ia, pathlib, re, sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
Z = (ROOT / 'task' / 'zadatak.frag.html').read_text()
R = html.unescape(re.sub(r'<[^>]+>', '', (ROOT / 'task' / 'resenje.frag.html').read_text()))
bad = []


def ok(cond, msg):
    print(('OK   ' if cond else 'FAIL ') + msg)
    if not cond:
        bad.append(msg)


def tables(src):
    for t in re.findall(r'<table[^>]*>(.*?)</table>', src, re.S):
        rows = [[html.unescape(re.sub(r'<[^>]+>', '', c)).strip() for c in re.findall(r'<t[hd][^>]*>(.*?)</t[hd]>', r, re.S)]
                for r in re.findall(r'<tr>(.*?)</tr>', t, re.S)]
        yield rows


T = list(tables(Z))
plan = next(t for t in T if t[0][:3] == ['Uređaj', 'Interfejs', 'VLAN'])[1:]
ports = next(t for t in T if t[0] == ['Port', 'SW1', 'SW2'])[1:]

nets, r1if, hosts = {}, {}, []
for dev, ifc, vlan, ip, gw in plan:
    m = re.match(r'(\d+\.\d+\.\d+\.\d+) / (\d+)$', ip)
    if not m:
        continue
    iface = ia.ip_interface(f'{m.group(1)}/{m.group(2)}')
    if vlan in nets:
        ok(nets[vlan] == iface.network, f'VLAN {vlan}: {dev} {iface} u istoj mreži {nets[vlan]}')
    nets.setdefault(vlan, iface.network)
    if dev == 'R1':
        r1if[vlan] = iface.ip
    hosts.append((dev, ifc, vlan, iface, gw))
for vlan, net in nets.items():
    ok(vlan in r1if, f'VLAN {vlan} ({net}) ima adresu na R1')
wan = ia.ip_network('192.168.122.0/24')   # GNS3 NAT čvor
allnets = list(nets.items()) + [('NAT', wan)]
for i, (a, na) in enumerate(allnets):
    for b, nb in allnets[i + 1:]:
        ok(not na.overlaps(nb), f'{na} (VLAN {a}) i {nb} (VLAN {b}) se ne preklapaju')
seen = set()
for dev, ifc, vlan, iface, gw in hosts:
    ok(iface.ip not in seen, f'{dev} {iface.ip} jedinstvena'); seen.add(iface.ip)
    ok(iface.ip not in (iface.network.network_address, iface.network.broadcast_address), f'{dev} {iface.ip} je adresa hosta')
    if gw not in ('–', ''):
        ok(ia.ip_address(gw) == r1if[vlan], f'{dev} gateway {gw} = R1 u VLAN-u {vlan}')
# DHCP (VPC1)
vpc1 = next(r for r in plan if r[0] == 'VPC1')
lo, hi = map(ia.ip_address, re.findall(r'\d+\.\d+\.\d+\.\d+', vpc1[3]))
ok(lo in nets['10'] and hi in nets['10'] and lo < hi, f'DHCP opseg {lo}-{hi} u {nets["10"]}')
ok(not (lo <= r1if['10'] <= hi), 'gateway VLAN-a 10 nije u DHCP opsegu')
ok(ia.ip_address(vpc1[4]) == r1if['10'], 'VPC1 gateway = R1 vlan10')
srv = ia.ip_address(re.search(r'server za nadzor</text>.*?(\d+\.\d+\.\d+\.\d+)', Z, re.S).group(1))
ok(srv in nets['99'] and srv not in seen, f'server za nadzor {srv} u VLAN-u 99, slobodna adresa')

# rešenje: iste adrese i VLAN-ovi
for dev, ifc, vlan, iface, gw in hosts:
    if dev == 'R1':
        name = ifc.split()[0]
        ok(f'address={iface} interface={name}' in R, f'rešenje: R1 {name} {iface}')
        if name.startswith('vlan'):
            ok(f'name={name} vlan-id={vlan} interface=ether2' in R, f'rešenje: {name} vlan-id={vlan} na ether2')
    elif dev.startswith('SW'):
        ok(f'sudo config interface ip add Vlan{vlan} {iface}' in R, f'rešenje: {dev} Vlan{vlan} {iface}')
    elif dev == 'VPC2':
        ok(f'ip {iface} {gw}' in R, f'rešenje: VPC2 ip {iface} {gw}')
ok(f'ranges={lo}-{hi}' in R and f'gateway={r1if["10"]}' in R, 'rešenje: DHCP opseg i gateway')
ok(f'remote={srv}' in R, 'rešenje: syslog na server za nadzor')
ok('addresses=' + str(nets['99']) in R and f'dst-address={nets["99"]}' in R, 'rešenje: SNMP/firewall mreža VLAN 99')
# portovi svičeva
sec = {'SW1': R[R.index('SW1\nshow ip interfaces'):R.index('SW2\nsudo config')],
       'SW2': R[R.index('SW2\nsudo config'):R.index('show vlan brief')]}
for port, *cols in ports:
    for sw, desc in zip(('SW1', 'SW2'), cols):
        if desc == '–':
            continue
        vl = re.findall(r'\d+', desc.split('(')[0])
        mode = desc.split(',')[0]
        cmds = set(re.findall(r'sudo config vlan member add (-u )?(\d+) (Ethernet\d+)\n', sec[sw] + '\n'))
        exp = {('-u ' if mode == 'access' else '', v, port) for v in vl}
        got = {c for c in cmds if c[2] == port}
        ok(got == exp, f'rešenje: {sw} {port} {desc} -> {sorted(got)}')
        ok(f'interface ip remove {port} ' in sec[sw], f'rešenje: {sw} {port} bez IP adrese')
print(f'\n{"SVE PROVERE PROŠLE" if not bad else str(len(bad)) + " GREŠAKA"}')
sys.exit(1 if bad else 0)
