import { useEffect, useState } from "react";
import type { FormEvent } from "react";
import { ExternalLink } from "lucide-react";
import type { Node } from "./api";
import { dockerHttpConsoleUrl, serverHostname } from "./api";
import { Button } from "@/components/ui/button";
import { SettingsDialog, SettingsNotice, SettingsSection, type SettingsTab } from "@/components/settings-dialog";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";

export function NodeSettingsDialog({ node, open, onOpenChange, onSave }: {
  node: Node | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSave: (id: string, patch: Record<string, unknown>) => Promise<void>;
}) {
  const [draft, setDraft] = useState<Record<string, string>>({});
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const kind = node?.node_type;
  const supported = ["qemu", "docker", "vpcs", "dynamips", "iou", "ethernet_switch", "ethernet_hub", "hub", "cloud", "nat"].includes(kind ?? "");
  const requiresStopped = ["qemu", "docker", "dynamips"].includes(kind ?? "");

  useEffect(() => {
    if (!node) return;
    setDraft({
      name: node.name,
      memory: String(node.memory ?? (kind === "qemu" ? 512 : 0)),
      cpus: String(node.cpus ?? (kind === "qemu" ? 1 : 0)),
      adapters: String(node.adapters ?? 1),
      ...(kind === "qemu" ? {
        adapter_type: String(node.adapter_type ?? "e1000"),
        hda_disk_interface: String(node.hda_disk_interface ?? "ide"),
        boot_priority: String(node.boot_priority ?? "c"),
        hda_disk_image: String(node.hda_disk_image ?? ""), hdb_disk_image: String(node.hdb_disk_image ?? ""),
        hdc_disk_image: String(node.hdc_disk_image ?? ""), hdd_disk_image: String(node.hdd_disk_image ?? ""),
        cdrom_image: String(node.cdrom_image ?? ""), hdb_disk_interface: String(node.hdb_disk_interface ?? "ide"),
        hdc_disk_interface: String(node.hdc_disk_interface ?? "ide"), hdd_disk_interface: String(node.hdd_disk_interface ?? "ide"),
        console_type: String(node.console_type ?? "telnet"), options: String(node.options ?? ""),
        config_export_command: String(node.config_export_command ?? ""), config_export_username: String(node.config_export_username ?? ""), config_export_password: String(node.config_export_password ?? ""), process_priority: String(node.process_priority ?? "normal"), maxcpus: String(node.maxcpus ?? node.cpus ?? 1),
      } : {}),
      ...(kind === "docker" ? {
        image: String(node.image ?? ""),
        start_command: String(node.start_command ?? ""),
        environment: String(node.environment ?? ""),
        extra_hosts: String(node.extra_hosts ?? ""),
        console_type: String(node.console_type ?? "telnet"),
        console_http_port: String(node.console_http_port ?? 80),
        console_http_path: String(node.console_http_path ?? "/"),
        console_vnc_port: node.console_vnc_port ? String(node.console_vnc_port) : "",
        aux_type: String(node.aux_type ?? "none"),
        extra_volumes: Array.isArray(node.extra_volumes)
          ? node.extra_volumes.filter((path): path is string => typeof path === "string").join("\n")
          : "",
      } : {}),
      ...(kind === "vpcs" ? { startup_script: String(node.startup_script ?? ""), console_type: String(node.console_type ?? "telnet") } : {}),
      ...(kind === "dynamips" ? {
        startup_config: String(node.startup_config ?? ""), memory: String(node.ram ?? node.memory ?? 512),
        nvram: String(node.nvram ?? 128), idlepc: String(node.idlepc ?? ""),
        ...Object.fromEntries(Array.from({ length: 8 }, (_, i) => [`slot${i}`, String(node[`slot${i}`] ?? "")])),
        ...Object.fromEntries(Array.from({ length: 4 }, (_, i) => [`wic${i}`, String(node[`wic${i}`] ?? "")])),
      } : {}),
      ...(kind === "iou" ? {
        path: String(node.path ?? ""), ram: String(node.ram ?? 256), nvram: String(node.nvram ?? 256),
        ethernet_adapters: String(node.ethernet_adapters ?? 2), serial_adapters: String(node.serial_adapters ?? 0),
        console_type: String(node.console_type ?? "telnet"), startup_config: String(node.startup_config ?? ""),
        private_config: String(node.private_config ?? ""),
      } : {}),
      ...(["ethernet_switch", "ethernet_hub", "hub"].includes(kind ?? "") ? {
        adapters: String(Array.isArray(node.ports_mapping) ? node.ports_mapping.length : node.ports ?? node.number_of_ports ?? 8),
        port_names: (Array.isArray(node.ports_mapping) ? node.ports_mapping : []).map((item) => String((item as Record<string, unknown>).name ?? "")).join("\n"),
        port_vlans: (Array.isArray(node.ports_mapping) ? node.ports_mapping : []).map((item) => String((item as Record<string, unknown>).vlan ?? 1)).join("\n"),
        port_types: (Array.isArray(node.ports_mapping) ? node.ports_mapping : []).map((item) => String((item as Record<string, unknown>).type ?? "access")).join("\n"),
      } : {}),
      ...(kind === "cloud" ? { interfaces: (Array.isArray(node.ports_mapping) ? node.ports_mapping : []).map((item) => String((item as Record<string, unknown>).interface ?? "")).join("\n") } : {}),
      ...(kind === "nat" ? { allow_host_interfaces: Array.isArray(node.allow_host_interfaces) ? node.allow_host_interfaces.join("\n") : "" } : {}),
    });
    setError("");
  }, [kind, node, open]);

  const field = (name: string, label: string, type = "text", helper?: string, step?: string, disabled = false) => <div className="grid min-w-0 content-start gap-2" key={name}>
    <Label htmlFor={`node-${node?.node_id}-${name}`}>{label}</Label>
    <Input id={`node-${node?.node_id}-${name}`} type={type} step={step} required={name === "name" || type === "number" && name !== "console_vnc_port"} min={type === "number" ? (["adapters", "maxcpus", "console_http_port", "console_vnc_port"].includes(name) || kind === "qemu" && ["cpus", "memory"].includes(name) ? "1" : "0") : undefined} max={name === "adapters" ? 64 : name === "console_http_port" || name === "console_vnc_port" ? 65535 : undefined} disabled={disabled} value={draft[name] ?? ""} onChange={(event) => setDraft((current) => ({ ...current, [name]: event.target.value }))} />
    {helper && <span className="text-xs text-muted-foreground">{helper}</span>}
  </div>;
  const selectField = (name: string, label: string, values: string[]) => <div className="grid min-w-0 content-start gap-2" key={name}>
    <Label>{label}</Label>
    <Select value={draft[name] ?? values[0]} onValueChange={(value) => setDraft((current) => ({ ...current, [name]: value }))}>
      <SelectTrigger aria-label={label}><SelectValue /></SelectTrigger>
      <SelectContent position="popper" align="start">{values.map((value) => <SelectItem key={value} value={value}>{value}</SelectItem>)}</SelectContent>
    </Select>
  </div>;
  const textArea = (name: string, label: string, helper: string, rows = 4) => <div className="grid min-w-0 content-start gap-2" key={name}>
    <Label htmlFor={`node-${node?.node_id}-${name}`}>{label}</Label>
    <Textarea id={`node-${node?.node_id}-${name}`} value={draft[name] ?? ""} onChange={(event) => setDraft((current) => ({ ...current, [name]: event.target.value }))} rows={rows} className="min-h-24 font-mono text-xs leading-relaxed" />
    <span className="text-xs text-muted-foreground">{helper}</span>
  </div>;

  async function save(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!node || !supported) return;
    setSaving(true);
    setError("");
    const patch: Record<string, unknown> = { name: draft.name?.trim() };
    if (kind === "qemu") {
      patch.memory = Number(draft.memory);
      patch.cpus = Number(draft.cpus);
      patch.adapters = Number(draft.adapters);
      patch.adapter_type = draft.adapter_type;
      patch.hda_disk_interface = draft.hda_disk_interface;
      patch.boot_priority = draft.boot_priority;
      for (const key of ["hda_disk_image", "hdb_disk_image", "hdc_disk_image", "hdd_disk_image", "cdrom_image", "options"]) patch[key] = draft[key] ?? "";
      for (const key of ["hdb_disk_interface", "hdc_disk_interface", "hdd_disk_interface", "console_type", "process_priority"]) patch[key] = draft[key];
      patch.maxcpus = Number(draft.maxcpus);
      patch.config_export_command = draft.config_export_command ?? "";
      patch.config_export_username = draft.config_export_username ?? ""; patch.config_export_password = draft.config_export_password ?? "";
    } else if (kind === "docker") {
      patch.memory = Number(draft.memory);
      patch.cpus = Number(draft.cpus);
      patch.adapters = Number(draft.adapters);
      patch.start_command = draft.start_command ?? "";
      patch.environment = draft.environment ?? "";
      patch.extra_hosts = draft.extra_hosts ?? "";
      patch.extra_volumes = (draft.extra_volumes ?? "").split("\n").map((path) => path.trim()).filter(Boolean);
      patch.console_type = draft.console_type ?? "telnet";
      patch.console_vnc_port = patch.console_type === "vnc" && String(draft.console_vnc_port ?? "").trim() ? Number(draft.console_vnc_port) : null;
      if (patch.console_type !== "none") {
        patch.console_http_port = Number(draft.console_http_port);
        patch.console_http_path = draft.console_http_path || "/";
      }
      patch.aux_type = draft.aux_type ?? "none";
      patch.image = draft.image ?? "";
    } else if (kind === "vpcs") {
      patch.startup_script = draft.startup_script ?? "";
      patch.console_type = draft.console_type ?? "telnet";
    } else if (kind === "dynamips") {
      patch.startup_config = draft.startup_config ?? "";
      patch.ram = Number(draft.memory); patch.nvram = Number(draft.nvram); patch.idlepc = draft.idlepc ?? "";
      for (let i = 0; i < 8; i++) patch[`slot${i}`] = draft[`slot${i}`] ?? "";
      for (let i = 0; i < 4; i++) patch[`wic${i}`] = draft[`wic${i}`] ?? "";
    } else if (kind === "iou") {
      patch.path = draft.path ?? "";
      patch.ram = Number(draft.ram);
      patch.nvram = Number(draft.nvram);
      patch.ethernet_adapters = Number(draft.ethernet_adapters);
      patch.serial_adapters = Number(draft.serial_adapters);
      patch.console_type = draft.console_type ?? "telnet";
      patch.startup_config = draft.startup_config ?? "";
      patch.private_config = draft.private_config ?? "";
    } else if (["ethernet_switch", "ethernet_hub", "hub"].includes(kind ?? "")) {
      const count = Math.max(1, Math.min(64, Number(draft.adapters) || 1));
      const names = (draft.port_names ?? "").split("\n");
      const vlans = (draft.port_vlans ?? "").split("\n");
      const types = (draft.port_types ?? "").split("\n");
      patch.ports_mapping = Array.from({ length: count }, (_, index) => ({ ...((Array.isArray(node.ports_mapping) ? node.ports_mapping[index] : {}) as Record<string, unknown>), port_number: index, name: names[index]?.trim() || `Ethernet${index + 1}`, vlan: Number(vlans[index]) || 1, type: types[index]?.trim() || "access", ethertype: types[index]?.trim() === "qinq" ? String((Array.isArray(node.ports_mapping) ? node.ports_mapping[index]?.ethertype : null) ?? "0x8100") : "0x8100" }));
    } else if (kind === "cloud") {
      patch.ports_mapping = (draft.interfaces ?? "").split("\n").map((name, index) => name.trim() ? ({ port_number: index, name: name.trim(), type: "ethernet", interface: name.trim() }) : null).filter(Boolean);
    } else if (kind === "nat") {
      patch.allow_host_interfaces = (draft.allow_host_interfaces ?? "").split("\n").map((name) => name.trim()).filter(Boolean);
    }
    try {
      await onSave(node.node_id, patch);
      onOpenChange(false);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "Could not save device settings");
    } finally {
      setSaving(false);
    }
  }

  function openHttpConsole() {
    if (!node || (draft.console_type !== "http" && draft.console_type !== "https")) return;
    const host = typeof node.console_host === "string" && node.console_host ? node.console_host : serverHostname();
    const url = dockerHttpConsoleUrl(draft.console_type, host, Number(draft.console_http_port), draft.console_http_path || "/");
    if (!url) {
      setError("Enter a valid HTTP console port and path before opening the console.");
      return;
    }
    window.open(url, "_blank", "noopener,noreferrer");
  }

  if (!node || !supported) return null;
  const isSwitch = kind === "ethernet_switch";
  const isHub = kind === "ethernet_hub" || kind === "hub";
  const stoppedRequired = requiresStopped && node.status !== "stopped";
  const portCount = Math.max(1, Math.min(64, Number(draft.adapters) || 1));
  const setPortValue = (key: string, index: number, value: string) => setDraft((current) => {
    const lines = Array.from({ length: portCount }, (_, row) => (current[key] ?? "").split("\n")[row] ?? "");
    lines[index] = value;
    return { ...current, [key]: lines.join("\n") };
  });
  const tabs: SettingsTab[] = [{ id: "general", label: "General", content: <>
    <SettingsSection title="Device identity" description="These settings belong to this device in the current project.">
      {field("name", "Device name")}
      {kind === "docker" && field("image", "Docker image")}
      {kind === "iou" && field("path", "IOU image path")}
    </SettingsSection>
    {(["qemu", "docker", "dynamips", "iou"].includes(kind ?? "")) && <SettingsSection title="Compute resources" columns>
      {field(kind === "iou" ? "ram" : "memory", kind === "docker" ? "Memory limit (MB)" : "RAM (MB)", "number", kind === "docker" ? "0 means no Docker memory limit." : undefined)}
      {(kind === "qemu" || kind === "docker") && field("cpus", kind === "qemu" ? "vCPUs" : "CPU limit", "number", kind === "docker" ? "0 means no Docker CPU limit." : undefined, kind === "docker" ? "0.25" : "1")}
      {(kind === "dynamips" || kind === "iou") && field("nvram", "NVRAM (KB)", "number")}
    </SettingsSection>}
  </> }];

  if (["qemu", "docker", "iou", "ethernet_switch", "ethernet_hub", "hub", "cloud", "nat"].includes(kind ?? "")) tabs.push({ id: "network", label: "Network", content: <>
    <SettingsSection title={isSwitch || isHub ? "Ethernet ports" : "Network interfaces"} description={isSwitch ? "Configure each port on one row. VLAN mode and ID stay alongside the port they belong to." : undefined} columns={!isSwitch && !isHub && kind !== "cloud" && kind !== "nat"}>
      {(kind === "qemu" || kind === "docker") && field("adapters", "Network adapters", "number", node.status !== "stopped" ? "Stop the device to change the number of adapters" : "The count cannot be reduced below ports already used by links.", undefined, node.status !== "stopped")}
      {kind === "qemu" && selectField("adapter_type", "Network adapter model", ["e1000", "virtio-net-pci", "rtl8139", "vmxnet3"])}
      {kind === "iou" && <>{field("ethernet_adapters", "Ethernet adapters", "number")}{field("serial_adapters", "Serial adapters", "number")}</>}
      {(isSwitch || isHub) && <>
        {field("adapters", "Port count", "number", "Between 1 and 64 ports. Connected ports must be kept.")}
        <div className="relative min-w-0 overflow-x-auto rounded-lg border"><table className="w-full text-left text-xs"><thead className="bg-muted/50"><tr><th className="p-3">Port</th><th className="p-3">Name</th>{isSwitch && <><th className="p-3">VLAN</th><th className="p-3">Mode</th></>}</tr></thead><tbody>
          {Array.from({ length: portCount }, (_, index) => <tr key={index} className="border-t">
            <td className="px-3 text-muted-foreground">{index + 1}</td>
            <td className="p-2"><Input aria-label={`Port ${index + 1} name`} className="min-w-32" placeholder={`Ethernet${index + 1}`} value={(draft.port_names ?? "").split("\n")[index] ?? ""} onChange={(event) => setPortValue("port_names", index, event.target.value)} /></td>
            {isSwitch && <><td className="p-2"><Input aria-label={`Port ${index + 1} VLAN`} className="min-w-20" type="number" min={1} max={4094} value={(draft.port_vlans ?? "").split("\n")[index] || "1"} onChange={(event) => setPortValue("port_vlans", index, event.target.value)} /></td>
              <td className="p-2"><Select value={(draft.port_types ?? "").split("\n")[index] || "access"} onValueChange={(value) => setPortValue("port_types", index, value)}><SelectTrigger aria-label={`Port ${index + 1} mode`} className="min-w-28"><SelectValue /></SelectTrigger><SelectContent position="popper" align="start">{["access", "dot1q", "qinq"].map((value) => <SelectItem key={value} value={value}>{value}</SelectItem>)}</SelectContent></Select></td></>}
          </tr>)}
        </tbody></table></div>
      </>}
      {kind === "cloud" && textArea("interfaces", "Ethernet interfaces", "One host interface per line.")}
      {kind === "nat" && textArea("allow_host_interfaces", "Allowed host interfaces", "Explicitly allow these host interfaces for the NAT node, one per line.")}
    </SettingsSection>
    {kind === "docker" && <SettingsSection title="Host entries">{textArea("extra_hosts", "Extra hosts", "One host:address entry per line.")}</SettingsSection>}
  </> });

  if (kind === "qemu") tabs.push({ id: "storage", label: "Storage", content: <>
    <SettingsSection title="Virtual disks" description="Each image is paired with its disk interface. Leave optional disk images blank.">
      {["hda", "hdb", "hdc", "hdd"].map((disk, index) => <div key={disk} className="grid items-start gap-4 rounded-lg border p-4 sm:grid-cols-[minmax(0,2fr)_minmax(0,1fr)]">
        {field(`${disk}_disk_image`, `${disk.toUpperCase()} disk image`)}
        {selectField(`${disk}_disk_interface`, index === 0 ? "Primary disk interface" : `${disk.toUpperCase()} disk interface`, ["ide", "scsi", "virtio", "sd"])}
      </div>)}
    </SettingsSection>
    <SettingsSection title="Boot media" columns>{field("cdrom_image", "CD-ROM image")}{selectField("boot_priority", "Boot priority", ["c", "d", "n", "dc", "cd"])}</SettingsSection>
  </> });

  if (["qemu", "docker", "vpcs", "iou"].includes(kind ?? "")) tabs.push({ id: "console", label: "Console", content: <>
    <SettingsSection title="Console connection" description="Choose how you connect to this device.">
      {selectField("console_type", "Console type", kind === "docker" ? ["telnet", "vnc", "http", "https", "none"] : kind === "qemu" ? ["telnet", "vnc"] : kind === "vpcs" ? ["telnet", "ssh", "none"] : ["telnet", "none"])}
      {kind === "docker" && (draft.console_type === "http" || draft.console_type === "https") && <>
        <div className="grid items-start gap-4 sm:grid-cols-2">{field("console_http_port", "HTTP port in the container", "number", undefined, "1")}{field("console_http_path", "HTTP path", "text", "Must begin with a slash.")}</div>
        <SettingsNotice>The service must be reachable from your browser. The controller does not publish or proxy Docker HTTP ports.</SettingsNotice>
        <Button type="button" variant="outline" className="w-fit" onClick={openHttpConsole}><ExternalLink />Open in new tab</Button>
      </>}
      {kind === "docker" && draft.console_type === "vnc" && <>{field("console_vnc_port", "VNC port in the container (optional)", "number", "Enter the port of an existing VNC server, or leave blank to start Xvfb and x11vnc automatically.", "1")}<SettingsNotice>Open the device console from its context menu to use the browser VNC window.</SettingsNotice></>}
      {kind === "docker" && draft.console_type === "none" && <SettingsNotice>The controller does not accept Docker console type “none” yet. Select another type to save.</SettingsNotice>}
    </SettingsSection>
    {kind === "docker" && <SettingsSection title="Auxiliary console">
      {selectField("aux_type", "Auxiliary console type", ["none", "telnet", "ssh"])}
      {draft.aux_type !== "none" && <SettingsNotice>When the device is running, open its auxiliary console from the device menu.</SettingsNotice>}
    </SettingsSection>}
    {kind === "qemu" && <SettingsSection title="Configuration export" description="Used by Export config in the device menu.">
      {field("config_export_command", "Export config command", "text", "For example, /export on MikroTik RouterOS.")}
      <div className="grid items-start gap-4 sm:grid-cols-2">{field("config_export_username", "Export login user", "text", "MikroTik defaults to admin.")}{field("config_export_password", "Export login password", "password", "Empty for a fresh RouterOS CHR.")}</div>
    </SettingsSection>}
  </> });

  if (["docker", "vpcs", "dynamips", "iou"].includes(kind ?? "")) tabs.push({ id: "startup", label: "Startup", content: <SettingsSection title="Startup configuration" description="Applied when this device starts.">
    {kind === "docker" && <>{textArea("start_command", "Start command", "Leave blank to use the image default.", 3)}{textArea("environment", "Environment variables", "One KEY=VALUE entry per line.", 6)}</>}
    {kind === "vpcs" && textArea("startup_script", "Startup script", "VPCS commands run when this device starts.", 10)}
    {(kind === "dynamips" || kind === "iou") && textArea("startup_config", "Startup configuration", "Configuration written before the router starts.", 10)}
    {kind === "iou" && textArea("private_config", "Private configuration", "Private configuration content stored in the IOU NVRAM.", 6)}
  </SettingsSection> });
  if (kind === "docker") tabs.push({ id: "storage", label: "Storage", content: <SettingsSection title="Persistent storage" description="Preserve selected container data when runtime settings replace the stopped container.">{textArea("extra_volumes", "Persistent container paths", "One absolute path inside the container per line. Data lives in the project runtime directory.", 8)}</SettingsSection> });
  if (kind === "dynamips") tabs.push({ id: "adapters", label: "Adapters", content: <>
    <SettingsSection title="Module slots" columns>{Array.from({ length: 8 }, (_, i) => field(`slot${i}`, `Slot ${i}`))}</SettingsSection>
    <SettingsSection title="WAN interface cards" columns>{Array.from({ length: 4 }, (_, i) => field(`wic${i}`, `WIC ${i}`))}</SettingsSection>
  </> });
  if (kind === "qemu" || kind === "dynamips") tabs.push({ id: "advanced", label: "Advanced", content: <SettingsSection title="Runtime tuning">
    {kind === "qemu" ? <><div className="grid items-start gap-4 sm:grid-cols-2">{field("maxcpus", "Maximum CPUs", "number")}{selectField("process_priority", "Process priority", ["low", "normal", "high"])}</div>{textArea("options", "Additional QEMU options", "Command line options passed to QEMU.", 6)}</> : field("idlepc", "Idle-PC", "text", "Use an Idle-PC value appropriate for the installed IOS image.")}
  </SettingsSection> });

  return <SettingsDialog open={open} onOpenChange={onOpenChange} title={`Configure ${node.name}`} description="Manage hardware, connections, and startup settings for this device."
    context={<><Badge variant="outline" className="uppercase">{kind?.replaceAll("_", " ")}</Badge><span className="capitalize">{node.status ?? "Unknown status"}</span></>}
    tabs={tabs} onSubmit={(event) => void save(event)} saving={saving} saveDisabled={stoppedRequired || (kind === "docker" && draft.console_type === "none") || !draft.name?.trim()} saveLabel="Save device settings" error={error}
    notice={stoppedRequired ? <SettingsNotice>Stop this device to edit its runtime settings.</SettingsNotice> : kind === "docker" && draft.console_type === "none" ? <SettingsNotice>Choose a supported console type in the Console tab before saving.</SettingsNotice> : kind === "iou" && node.runtime_supported === false ? <SettingsNotice>IOU runtime is unavailable here. These settings are still saved with the project node.</SettingsNotice> : undefined} />;
}
