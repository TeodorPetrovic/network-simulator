"""Serijska konzola MikroTik CHR-a (QEMU -serial telnet:127.0.0.1:5000 u kontejneru v8_r1)."""
import os, re, select, subprocess, time

PROMPT = re.compile(rb'\[[\w.-]+@[\w.-]+\] > ')


def clean(b):
    s = re.sub(rb'\xff[\xfb-\xfe].|\xff\xfa.*?\xff\xf0', b'', b, flags=re.S).decode('utf8', 'replace')
    s = s.replace('\r', '')
    s = re.sub(r'\x1b\[[0-9;?]*[A-Za-z]|\x1b[78=>c]', '', s)
    return s


class Ros:
    def __init__(self, ctr='v8_r1', port=5000):
        self.p = subprocess.Popen(['docker', 'exec', '-i', ctr, 'socat', '-', 'TCP:127.0.0.1:%d' % port],
                                  stdin=subprocess.PIPE, stdout=subprocess.PIPE, bufsize=0)

    def read(self, t=5, until=None):
        buf, end = b'', time.time() + t
        while time.time() < end:
            r, _, _ = select.select([self.p.stdout], [], [], 0.2)
            if r:
                d = os.read(self.p.stdout.fileno(), 65536)
                if not d:
                    break
                buf += d
                if until is not None and re.search(until, buf):
                    break
        return buf

    def send(self, s):
        self.p.stdin.write(s.encode())

    def login(self, user='admin', pw='', newpw=None, tries=60):
        for _ in range(tries):  # čekanje da se CHR podigne
            self.send('\r')
            b = self.read(3, rb'Login: |' + PROMPT.pattern)
            if PROMPT.search(b):
                return
            if b'Login:' in b:
                break
        self.send(user + '+cetw200h5000\r'); self.read(5, rb'Password: ')
        self.send(pw + '\r')
        b = self.read(15, rb'\[Y/n\]|new password> |' + PROMPT.pattern + rb'|Login: ')
        if b'[Y/n]' in b:
            self.send('n\r'); b = self.read(10, rb'new password> |' + PROMPT.pattern)
        if b'new password>' in b:
            self.send(newpw + '\r'); self.read(5, rb'repeat new password> ')
            self.send(newpw + '\r'); b = self.read(10, PROMPT.pattern)
        if not PROMPT.search(b):
            raise RuntimeError('login nije uspeo: %r' % clean(b))
        self.read(1)

    def cmd(self, c, t=30):
        self.send(c + '\r')
        b = self.read(t, rb'\n.*' + PROMPT.pattern)
        s = clean(b)
        lines = s.split('\n')
        # prvi red: eho komande, poslednji: prompt
        if lines and c.strip() in lines[0]:
            lines = lines[1:]
        if lines and PROMPT.search(lines[-1].encode()):
            lines = lines[:-1]
        return '\n'.join(l.rstrip() for l in lines).strip('\n')
