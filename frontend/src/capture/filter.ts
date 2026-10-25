import type { Dissection } from "./dissect";

export type PacketRecord = {
  number: number;
  time: number;
  timestamp: number;
  data: Uint8Array;
  originalLength: number;
  linkType: number;
  decoded: Dissection;
};

type Token = { kind: "word" | "operator" | "left" | "right"; value: string };
type Expression =
  | { kind: "logical"; operator: "and" | "or"; left: Expression; right: Expression }
  | { kind: "not"; expression: Expression }
  | { kind: "comparison"; field: string; operator: string; value: string }
  | { kind: "protocol"; name: string };

export class DisplayFilterError extends Error {}

function tokenize(source: string): Token[] {
  const tokens: Token[] = [];
  const pattern = /\s*(<=|>=|==|!=|&&|\|\||[()!<>]|"(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'|[^\s()!<>=&|]+)/gy;
  let offset = 0;
  while (offset < source.length) {
    pattern.lastIndex = offset;
    const match = pattern.exec(source);
    if (!match) throw new DisplayFilterError(`Invalid filter near “${source.slice(offset, offset + 12)}”.`);
    const value = match[1];
    const kind = value === "(" ? "left" : value === ")" ? "right" :
      /^(==|!=|&&|\|\||!|<|>|<=|>=)$/.test(value) ? "operator" : "word";
    tokens.push({ kind, value });
    offset = pattern.lastIndex;
  }
  return tokens;
}

export function parseDisplayFilter(source: string): Expression {
  const tokens = tokenize(source);
  let position = 0;
  const peek = () => tokens[position];
  const take = () => tokens[position++];
  const isWord = (value: string) => peek()?.value.toLowerCase() === value;

  function atom(): Expression {
    if (peek()?.kind === "left") {
      take();
      const expression = parseOr();
      if (take()?.kind !== "right") throw new DisplayFilterError("Missing closing parenthesis.");
      return expression;
    }
    if (isWord("not") || peek()?.value === "!") {
      take();
      return { kind: "not", expression: atom() };
    }
    const left = take();
    if (!left || left.kind !== "word") throw new DisplayFilterError("Expected a protocol or field name.");
    const operator = peek()?.value.toLowerCase();
    if (operator && ["==", "!=", "<", ">", "<=", ">=", "contains", "matches"].includes(operator)) {
      take();
      const value = take();
      if (!value || value.kind === "right") throw new DisplayFilterError("Expected a value after comparison.");
      return {
        kind: "comparison",
        field: left.value.toLowerCase(),
        operator,
        value: value.value.replace(/^(?:"(.*)"|'(.*)')$/, (_match, doubleQuote, singleQuote) => doubleQuote ?? singleQuote),
      };
    }
    return { kind: "protocol", name: left.value.toLowerCase() };
  }

  function parseAnd(): Expression {
    let expression = atom();
    while (peek() && (peek().value === "&&" || isWord("and"))) {
      take();
      expression = { kind: "logical", operator: "and", left: expression, right: atom() };
    }
    return expression;
  }

  function parseOr(): Expression {
    let expression = parseAnd();
    while (peek() && (peek().value === "||" || isWord("or"))) {
      take();
      expression = { kind: "logical", operator: "or", left: expression, right: parseAnd() };
    }
    return expression;
  }

  if (tokens.length === 0) throw new DisplayFilterError("Enter a display filter.");
  const expression = parseOr();
  if (position !== tokens.length) throw new DisplayFilterError(`Unexpected token “${peek()?.value}”.`);
  return expression;
}

const protocols = new Set([
  "ip", "ipv4", "ipv6", "eth", "ether", "arp", "icmp", "icmpv6", "tcp", "udp", "dns", "dhcp",
  "bootp", "stp", "lldp", "cdp", "vrrp", "ospf", "rip", "vlan",
]);

function layer(packet: PacketRecord, name: string) {
  return packet.decoded.layers.find((item) => item.name.toLowerCase().includes(name.toLowerCase()));
}

function fieldValue(packet: PacketRecord, name: string): string[] | undefined {
  const decoded = packet.decoded;
  const pair = (source: string, destination: string): string[] => [source, destination].filter(Boolean);
  switch (name) {
    case "ip.addr":
      return decoded.src && !decoded.src.includes(":") ? pair(decoded.src, decoded.dst) : undefined;
    case "ip.src":
      return decoded.src && !decoded.src.includes(":") ? [decoded.src] : undefined;
    case "ip.dst":
      return decoded.dst && !decoded.dst.includes(":") ? [decoded.dst] : undefined;
    case "ipv6.addr":
      return decoded.src.includes(":") ? pair(decoded.src, decoded.dst) : undefined;
    case "ipv6.src":
      return decoded.src.includes(":") ? [decoded.src] : undefined;
    case "ipv6.dst":
      return decoded.dst.includes(":") ? [decoded.dst] : undefined;
    case "tcp.port":
    case "tcp.srcport":
    case "tcp.dstport":
    case "udp.port":
    case "udp.srcport":
    case "udp.dstport": {
      const transport = layer(packet, name.startsWith("tcp") ? "Transmission Control Protocol" : "User Datagram Protocol");
      if (!transport) return undefined;
      const source = transport.fields.find((item) => item.label === "Source Port")?.value;
      const destination = transport.fields.find((item) => item.label === "Destination Port")?.value;
      if (name.endsWith("srcport")) return source ? [source] : undefined;
      if (name.endsWith("dstport")) return destination ? [destination] : undefined;
      return [source, destination].filter((value): value is string => Boolean(value));
    }
    case "tcp.flags.syn": case "tcp.flags.ack": case "tcp.flags.fin": case "tcp.flags.rst": {
      const tcp = layer(packet, "Transmission Control Protocol");
      const flag = name.slice("tcp.flags.".length).toUpperCase();
      const value = tcp?.fields.find((item) => item.label === flag)?.value;
      return value ? [value === "Set" ? "1" : "0"] : undefined;
    }
    case "eth.addr": case "eth.src": case "eth.dst": {
      const ethernet = layer(packet, "Ethernet II");
      const source = ethernet?.fields.find((item) => item.label === "Source")?.value;
      const destination = ethernet?.fields.find((item) => item.label === "Destination")?.value;
      if (name === "eth.src") return source ? [source] : undefined;
      if (name === "eth.dst") return destination ? [destination] : undefined;
      return [source, destination].filter((value): value is string => Boolean(value));
    }
    case "frame.len": return [String(packet.originalLength)];
    case "icmp.type": case "icmpv6.type": {
      const icmp = layer(packet, name === "icmp.type" ? "Internet Control Message Protocol" : "Internet Control Message Protocol v6");
      const value = icmp?.fields.find((item) => item.label === "Type")?.value.match(/\((\d+)\)$/)?.[1];
      return value ? [value] : undefined;
    }
    case "dns.qry.name": {
      const dns = layer(packet, "Domain Name System");
      const question = dns?.fields.find((item) => item.label.startsWith("Question "))?.value;
      return question ? [question.split(" ")[0]] : undefined;
    }
    default: return undefined;
  }
}

function isProtocol(name: string, packet: PacketRecord): boolean {
  const decoded = packet.decoded;
  if (name === "ip" || name === "ipv4") return Boolean(decoded.src && !decoded.src.includes(":"));
  if (name === "ipv6") return decoded.src.includes(":");
  if (name === "eth" || name === "ether") return Boolean(layer(packet, "Ethernet II"));
  return decoded.protocol.toLowerCase() === name || decoded.layers.some((item) => item.name.toLowerCase().includes(name));
}

function matchesCidr(address: string, network: string): boolean {
  const [base, prefixText] = network.split("/");
  const prefix = Number(prefixText);
  const parse = (value: string) => value.split(".").reduce((result, part) => (result << 8) | Number(part), 0) >>> 0;
  if (!base || !Number.isInteger(prefix) || prefix < 0 || prefix > 32 || address.includes(":") || base.includes(":")) return false;
  const mask = prefix === 0 ? 0 : (0xffffffff << (32 - prefix)) >>> 0;
  return (parse(address) & mask) === (parse(base) & mask);
}

export function compileDisplayFilter(source: string): (packet: PacketRecord) => boolean {
  const expression = parseDisplayFilter(source);
  const evaluate = (node: Expression, packet: PacketRecord): boolean => {
    if (node.kind === "not") return !evaluate(node.expression, packet);
    if (node.kind === "logical") {
      return node.operator === "and"
        ? evaluate(node.left, packet) && evaluate(node.right, packet)
        : evaluate(node.left, packet) || evaluate(node.right, packet);
    }
    if (node.kind === "protocol") return isProtocol(node.name, packet);
    if (protocols.has(node.field)) return isProtocol(node.field, packet);
    const values = fieldValue(packet, node.field);
    if (!values) return false;
    const expected = node.value.toLowerCase();
    if (node.operator === "contains") return values.some((value) => value.toLowerCase().includes(expected));
    if (node.operator === "matches") {
      try { const pattern = new RegExp(node.value, "i"); return values.some((value) => pattern.test(value)); }
      catch { throw new DisplayFilterError("The matches expression contains an invalid regular expression."); }
    }
    if (node.operator === "==" && node.value.includes("/")) return values.some((value) => matchesCidr(value, node.value));
    if (node.operator === "==") return values.some((value) => value.toLowerCase() === expected);
    if (node.operator === "!=") return values.every((value) => value.toLowerCase() !== expected);
    const target = Number(node.value);
    if (!Number.isFinite(target)) return false;
    return values.some((value) => {
      const number = Number(value);
      if (node.operator === "<") return number < target;
      if (node.operator === ">") return number > target;
      if (node.operator === "<=") return number <= target;
      if (node.operator === ">=") return number >= target;
      return false;
    });
  };
  return (packet) => evaluate(expression, packet);
}
