import { useEffect, useMemo, useState } from "react";
import type { FormEvent } from "react";
import { useNavigate } from "@tanstack/react-router";
import { Boxes, Copy, ExternalLink, Pencil, Plus, RefreshCw, Trash2 } from "lucide-react";
import type { RuntimeTemplate } from "./api";
import { api } from "./api";
import { PagesShell, PageTitle } from "./PagesShell";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";

type RuntimeKey = "docker" | "qemu" | "vpcs" | "dynamips" | "cloud" | "nat" | "ethernet_hub" | "ethernet_switch" | "iou" | "virtualbox" | "vmware";
export type TemplateRecord = RuntimeTemplate & { template_type: RuntimeKey; [key: string]: unknown };
type FieldSpec = { key: string; label: string; section: string; kind?: "text" | "number" | "textarea" | "boolean" | "select"; options?: string[]; active?: boolean; createOnly?: boolean; help?: string };

const runtimeLabels: Record<RuntimeKey, string> = {
  docker: "Docker", qemu: "QEMU", vpcs: "VPCS", dynamips: "Dynamips (IOS routers)", cloud: "Cloud", nat: "NAT", ethernet_hub: "Ethernet hub", ethernet_switch: "Ethernet switch", iou: "IOU", virtualbox: "VirtualBox", vmware: "VMware",
};
const runtimeOrder: RuntimeKey[] = ["docker", "qemu", "vpcs", "dynamips", "cloud", "nat", "ethernet_hub", "ethernet_switch", "iou", "virtualbox", "vmware"];
const unsupported = new Set<RuntimeKey>(["virtualbox", "vmware"]);
const dynamipsPlatforms = ["c1700", "c2600", "c2691", "c3600", "c3725", "c3745", "c7200"];
const dynamipsChassis: Record<string, string[]> = { c1700: ["1720", "1721", "1750", "1751", "1760"], c2600: ["2610", "2611", "2620", "2610XM", "2620XM", "2650XM", "2621", "2611XM", "2621XM", "2651XM"], c3600: ["3620", "3640", "3660"] };
const dynamipsChassisDefault: Record<string, string> = { c1700: "1760", c2600: "2610", c3600: "3640" };
const dynamipsRamDefault: Record<string, number> = { c1700: 160, c2600: 160, c2691: 192, c3600: 192, c3725: 128, c3745: 256, c7200: 512 };
const dynamipsNvramDefault: Record<string, number> = { c1700: 128, c2600: 128, c2691: 256, c3600: 128, c3725: 256, c3745: 256, c7200: 512 };
function dynamipsSlot0(platform: string, chassis: string) {
  if (platform === "c1700") return "C1700-MB-1FE";
  if (platform === "c2600") return chassis === "2611" ? "C2600-MB-2E" : ["2620", "2610XM", "2620XM", "2650XM"].includes(chassis) ? "C2600-MB-1FE" : ["2621", "2611XM", "2621XM", "2651XM"].includes(chassis) ? "C2600-MB-2FE" : "C2600-MB-1E";
  if (platform === "c3600") return chassis === "3660" ? "Leopard-2FE" : "NM-1FE-TX";
  if (["c2691", "c3725", "c3745"].includes(platform)) return "GT96100-FE";
  return "C7200-IO-FE";
}
const field = (key: string, label: string, section: string, active = false, kind: FieldSpec["kind"] = "text", extra: Partial<FieldSpec> = {}): FieldSpec => ({ key, label, section, active, kind, ...extra });

const fieldsByRuntime: Record<RuntimeKey, FieldSpec[]> = {
  docker: [
    field("name", "Template name", "General", true), field("default_name_format", "Default name format", "General"), field("category", "Category", "General"), field("symbol", "Symbol", "General"), field("image", "Image name", "General", false, "text", { createOnly: true }),
    field("start_command", "Start command", "General", true, "textarea"), field("environment", "Environment", "General", true, "textarea", { help: "One KEY=VALUE entry per line." }), field("extra_hosts", "Extra hosts", "General", true, "textarea", { help: "One host:address entry per line." }), field("extra_volumes", "Persistent paths", "General", true, "textarea", { help: "One absolute container path per line." }),
    field("adapters", "Adapters", "Network", true, "number"), field("base_mac", "Base MAC", "Network"), field("console_type", "Console type", "Network", false, "select", { options: ["telnet", "vnc", "http", "none"] }), field("console_resolution", "VNC console resolution", "Network"), field("console_http_port", "HTTP port in the container", "Network", false, "number"), field("console_http_path", "HTTP path", "Network"), field("console_auto_start", "Auto start console", "Network", false, "boolean"),
    field("memory", "Memory limit (MB)", "Advanced", true, "number", { help: "0 means no Docker memory limit." }), field("cpus", "CPU limit", "Advanced", true, "number", { help: "0 means no Docker CPU limit." }),
  ],
  qemu: [
    field("name", "Template name", "General", true), field("default_name_format", "Default name format", "General"), field("symbol", "Symbol", "General"), field("category", "Category", "General"), field("memory", "RAM (MB)", "General", true, "number"), field("cpus", "vCPUs", "General", true, "number"), field("qemu_path", "QEMU binary", "General"), field("boot_priority", "Boot priority", "General", true, "select", { options: ["c", "d", "n", "dc", "cd"] }), field("on_close", "On close", "General", false, "select", { options: ["power_off", "send_key", "stop"] }), field("console_type", "Console type", "General", false, "select", { options: ["telnet", "vnc", "spice", "none"] }), field("console_auto_start", "Auto start console", "General", false, "boolean"),
    field("hda_disk_image", "HDA disk image", "Disks", false, "text", { createOnly: true }), field("hda_disk_interface", "HDA disk interface", "Disks", true, "select", { options: ["ide", "scsi", "virtio", "sd"] }), field("hdb_disk_image", "HDB disk image", "Disks"), field("hdb_disk_interface", "HDB disk interface", "Disks"), field("hdc_disk_image", "HDC disk image", "Disks"), field("hdc_disk_interface", "HDC disk interface", "Disks"), field("hdd_disk_image", "HDD disk image", "Disks"), field("hdd_disk_interface", "HDD disk interface", "Disks"), field("cdrom_image", "CD-ROM image", "Disks"),
    field("adapters", "Adapters", "Network", true, "number"), field("first_port_name", "First port name", "Network"), field("port_name_format", "Name format", "Network"), field("port_segment_size", "Segment size", "Network", false, "number"), field("mac_address", "Base MAC", "Network"), field("adapter_type", "Network adapter type", "Network", true, "select", { options: ["e1000", "virtio-net-pci", "rtl8139", "vmxnet3"] }), field("legacy_networking", "Use legacy networking", "Network", false, "boolean"), field("replicate_network_connection_state", "Replicate network connection state", "Network", false, "boolean"),
    field("initrd", "Initial RAM disk (initrd)", "Boot"), field("kernel_image", "Kernel image", "Boot"), field("kernel_command_line", "Kernel command line", "Boot", false, "textarea"), field("bios_image", "BIOS image", "Boot"),
    field("cpu_throttling", "Activate CPU throttling", "Advanced", false, "boolean"), field("cpu_throttling_percentage", "Percentage of CPU allowed", "Advanced", false, "number"), field("process_priority", "Process priority", "Advanced", false, "select", { options: ["normal", "low", "verylow"] }), field("options", "Options", "Advanced", false, "textarea"), field("linked_clone", "Use as a linked base VM", "Advanced", false, "boolean"), field("tpm", "Enable TPM", "Advanced", false, "boolean"), field("uefi", "Enable UEFI boot mode", "Advanced", false, "boolean"),
  ],
  vpcs: [
    field("name", "Template name", "General", true), field("default_name_format", "Default name format", "General"), field("scriptFile", "Base script file", "General"), field("symbol", "Symbol", "General"), field("category", "Category", "General"), field("console_type", "Console type", "General"), field("console_auto_start", "Auto start console", "General", false, "boolean"), field("startup_script", "Startup script", "General", true, "textarea"),
  ],
  dynamips: [
    field("name", "Template name", "General", true), field("default_name_format", "Default name format", "General"), field("symbol", "Symbol", "General"), field("category", "Category", "General"), field("image", "IOS image path", "General", false, "text", { createOnly: true }), field("platform", "Platform", "General", true, "select", { options: dynamipsPlatforms }), field("chassis", "Chassis", "General", true), field("console_type", "Console type", "General"), field("console_auto_start", "Auto start console", "General", false, "boolean"), field("initial_config", "Initial startup-config", "General", false, "textarea"), field("private_config", "Initial private-config", "General", false, "textarea"),
    field("memory", "RAM size (MB)", "Memory", true, "number"), field("nvram", "NVRAM size (KB)", "Memory", true, "number"), field("iomemory", "I/O memory", "Memory", false, "number"), field("disk0", "PCMCIA disk0", "Memory", false, "number"), field("disk1", "PCMCIA disk1", "Memory", false, "number"), field("auto_delete_disks", "Automatically delete NVRAM and disk files", "Memory", false, "boolean"),
    field("slot0", "Slot 0 adapter", "Adapters", true), field("slot1", "Slot 1 adapter", "Adapters"), field("slot2", "Slot 2 adapter", "Adapters"), field("slot3", "Slot 3 adapter", "Adapters"), field("slot4", "Slot 4 adapter", "Adapters"), field("wic0", "WIC 0", "Adapters"), field("wic1", "WIC 1", "Adapters"), field("wic2", "WIC 2", "Adapters"),
    field("system_id", "System ID", "Advanced"), field("mac_addr", "Base MAC", "Advanced"), field("idlepc", "Idle-PC", "Advanced", true), field("idlemax", "Idlemax", "Advanced", false, "number"), field("idlesleep", "Idlesleep", "Advanced", false, "number"), field("exec_area", "Exec area", "Advanced", false, "number"), field("mmap", "Enable mmap support", "Advanced", false, "boolean"), field("sparsemem", "Enable sparse memory support", "Advanced", false, "boolean"),
  ],
  cloud: [field("name", "Template name", "General", true), field("default_name_format", "Default name format", "General"), field("symbol", "Symbol", "General"), field("category", "Category", "General"), field("console_type", "Console type", "Miscellaneous"), field("remote_console_host", "Console host", "Miscellaneous"), field("remote_console_port", "Console port", "Miscellaneous", false, "number"), field("remote_console_http_path", "Console HTTP path", "Miscellaneous"), field("ethernet_interface", "Ethernet interface", "Ethernet interfaces"), field("tap_interface", "TAP interface", "TAP interfaces"), field("udp_tunnels", "UDP tunnels (name, local port, remote host, remote port)", "UDP tunnels", false, "textarea")],
  nat: [field("name", "Template name", "General", true), field("default_name_format", "Default name format", "General"), field("symbol", "Symbol", "General"), field("category", "Category", "General"), field("console_type", "Console type", "General")],
  ethernet_hub: [field("name", "Template name", "General", true), field("default_name_format", "Default name format", "General"), field("symbol", "Symbol", "General"), field("category", "Category", "General"), field("number_of_ports", "Number of ports", "General", false, "number")],
  ethernet_switch: [field("name", "Template name", "General", true), field("default_name_format", "Default name format", "General"), field("symbol", "Symbol", "General"), field("category", "Category", "General"), field("console_type", "Console type", "General"), field("ports", "Port configuration (port number, VLAN, mode, EtherType)", "Ports", false, "textarea")],
  iou: [field("name", "Template name", "General", true), field("path", "IOU image path", "General", true), field("ram", "RAM (MB)", "Memory", true, "number"), field("nvram", "NVRAM (KB)", "Memory", true, "number"), field("ethernet_adapters", "Ethernet adapters", "Network", true, "number"), field("serial_adapters", "Serial adapters", "Network", true, "number"), field("console_type", "Console type", "General", true), field("console_auto_start", "Auto start console", "General", true, "boolean"), field("startup_config", "Initial startup-config", "Configuration", true, "textarea"), field("private_config", "Initial private-config", "Configuration", true, "textarea")], virtualbox: [], vmware: [],
};

const supported = new Set<RuntimeKey>(["docker", "qemu", "vpcs", "dynamips", "cloud", "nat", "ethernet_hub", "ethernet_switch", "iou"]);
const toText = (value: unknown) => Array.isArray(value) ? value.join("\n") : value === undefined || value === null ? "" : typeof value === "object" ? JSON.stringify(value) : String(value);
const templateDefaults: Partial<Record<RuntimeKey, Record<string, unknown>>> = {
  docker: { adapters: 1, memory: 0, cpus: 0, console_type: "telnet" },
  qemu: { adapters: 1, memory: 256, cpus: 1, boot_priority: "c", hda_disk_interface: "ide", adapter_type: "e1000" },
  vpcs: { console_type: "telnet" },
  dynamips: { platform: "c7200", chassis: "", slot0: "C7200-IO-FE", memory: 512, nvram: 512, mmap: true, sparsemem: true, auto_delete_disks: false, console_auto_start: false, startup_config: "", private_config: "", clock_divisor: 8, idlemax: 500, idlesleep: 30, exec_area: 64, disk0: 0, disk1: 0, mac_addr: "", system_id: "FTX0945W0MY", npe: "npe-400", midplane: "vxr", iomem: 5, ghost_ios: false, console_type: "telnet", aux_type: "none" },
  iou: { ram: 256, nvram: 256, ethernet_adapters: 2, serial_adapters: 0, console_type: "telnet", console_auto_start: false, startup_config: "", private_config: "" },
};
const initialValues = (template?: Partial<TemplateRecord>, runtime?: RuntimeKey) => {
  const kind = runtime ?? (template?.template_type as RuntimeKey) ?? "docker";
  return Object.fromEntries((fieldsByRuntime[kind] ?? []).map((spec) => {
    const stored = template?.[spec.key];
    const fallback = templateDefaults[kind]?.[spec.key] ?? (spec.kind === "select" ? spec.options?.[0] : undefined);
    return [spec.key, toText(stored === undefined || stored === null || (spec.kind === "select" && stored === "") ? fallback : stored)];
  })) as Record<string, string>;
};

function SettingField({ spec, value, onChange, showPending = true }: { spec: FieldSpec; value: string; onChange: (value: string) => void; showPending?: boolean }) {
  const disabled = showPending && !spec.active;
  const id = `pref-${spec.key}`;
  return <label htmlFor={id} className={`grid gap-1.5 ${spec.kind === "boolean" ? "pages-checkbox-row" : ""}`}>
    <span className="flex items-center justify-between gap-2 text-sm font-medium"><span>{spec.label}</span>{disabled && <Badge variant="outline">Pending backend</Badge>}</span>
    {spec.kind === "boolean" ? <input id={id} type="checkbox" checked={value === "true"} disabled={disabled} onChange={(event) => onChange(String(event.target.checked))} /> : spec.kind === "textarea" ? <Textarea id={id} value={value} disabled={disabled} rows={3} onChange={(event) => onChange(event.target.value)} /> : spec.kind === "select" ? <Select value={value || spec.options?.[0] || ""} onValueChange={onChange} disabled={disabled}><SelectTrigger id={id} aria-label={spec.label}><SelectValue placeholder="Choose a value" /></SelectTrigger><SelectContent>{(spec.options ?? []).map((option) => <SelectItem key={option} value={option}>{option}</SelectItem>)}</SelectContent></Select> : <Input id={id} type={spec.kind === "number" ? "number" : "text"} value={value} disabled={disabled} onChange={(event) => onChange(event.target.value)} />}
    {spec.help && <span className="text-xs text-muted-foreground">{spec.help}</span>}
  </label>;
}

export function TemplateEditor({ template, runtime, open, onOpenChange, onSave, creating = false }: { template?: TemplateRecord; runtime: RuntimeKey; open: boolean; onOpenChange: (open: boolean) => void; onSave: (patch: Record<string, unknown>) => Promise<void>; creating?: boolean }) {
  const [values, setValues] = useState<Record<string, string>>({});
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const specs = fieldsByRuntime[runtime] ?? [];
  const sections = [...new Set(specs.map((item) => item.section))];
  useEffect(() => { if (open) { setValues(initialValues(template, runtime)); setError(""); } }, [open, template, runtime]);
  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault(); setSaving(true); setError("");
    const payload: Record<string, unknown> = {};
    for (const spec of specs) {
      if ((!spec.active && !(creating && spec.createOnly)) || (spec.createOnly && !creating)) continue;
      let value: unknown = values[spec.key] ?? "";
      if (spec.kind === "number") value = Number(value);
      if (spec.kind === "boolean") value = value === "true";
      if (spec.key === "extra_volumes") value = String(value).split("\n").map((line) => line.trim()).filter(Boolean);
      payload[spec.key] = value;
    }
    // The controller copies these template fields directly into a new node.
    // Include valid values even when the field is not editable in this form.
    if (runtime === "docker" && !Array.isArray(template?.custom_adapters)) payload.custom_adapters = [];
    if (runtime === "vpcs" && !template?.console_type) payload.console_type = "telnet";
    if (runtime === "dynamips") { Object.assign(payload, { mmap: template?.mmap ?? true, sparsemem: template?.sparsemem ?? true, auto_delete_disks: template?.auto_delete_disks ?? false, ghost_ios: template?.ghost_ios ?? false, private_config: template?.private_config ?? "", clock_divisor: template?.clock_divisor ?? 8, idlemax: template?.idlemax ?? 500, idlesleep: template?.idlesleep ?? 30, exec_area: template?.exec_area ?? 64, disk0: template?.disk0 ?? 0, disk1: template?.disk1 ?? 0, mac_addr: template?.mac_addr ?? "", system_id: template?.system_id ?? "FTX0945W0MY", npe: template?.npe ?? "npe-400", midplane: template?.midplane ?? "vxr", iomem: template?.iomem ?? 5, console_type: template?.console_type ?? "telnet", aux_type: template?.aux_type ?? "none" }); if (!String(payload.slot0 ?? "").trim()) payload.slot0 = dynamipsSlot0(String(payload.platform), String(payload.chassis ?? dynamipsChassisDefault[String(payload.platform)] ?? "")); }
    try { await onSave(payload); onOpenChange(false); }
    catch (reason) { setError(reason instanceof Error ? reason.message : "Template could not be saved"); }
    finally { setSaving(false); }
  }
  return <Dialog open={open} onOpenChange={onOpenChange}><DialogContent className="pages-dialog-wide"><DialogHeader><DialogTitle>{creating ? "Create template" : "Template settings"}</DialogTitle><DialogDescription>{runtimeLabels[runtime]}. Fields marked Pending backend are shown for parity but are not sent to the controller.</DialogDescription></DialogHeader><form onSubmit={(event) => void submit(event)}><Tabs defaultValue={sections[0] ?? "General"}><TabsList>{sections.map((section) => <TabsTrigger value={section} key={section}>{section}</TabsTrigger>)}</TabsList>{sections.map((section) => <TabsContent key={section} value={section} className="grid gap-4 pt-4 sm:grid-cols-2">{specs.filter((spec) => spec.section === section && !(runtime === "dynamips" && spec.key === "chassis" && !dynamipsChassis[values.platform ?? "c7200"])).map((spec) => {
    const liveSpec = runtime === "dynamips" && spec.key === "chassis" ? { ...spec, kind: "select" as const, options: dynamipsChassis[values.platform] } : spec;
    const onChange = (value: string) => setValues((current) => {
      if (runtime === "dynamips" && spec.key === "platform") { const chassis = dynamipsChassisDefault[value] ?? ""; return { ...current, platform: value, chassis, slot0: dynamipsSlot0(value, chassis), memory: String(dynamipsRamDefault[value]), nvram: String(dynamipsNvramDefault[value]) }; }
      if (runtime === "dynamips" && spec.key === "chassis") return { ...current, chassis: value, slot0: dynamipsSlot0(current.platform ?? "c7200", value) };
      return { ...current, [spec.key]: value };
    });
    return <div key={spec.key} className={spec.kind === "textarea" ? "sm:col-span-2" : ""}><SettingField spec={liveSpec} value={values[spec.key] ?? ""} onChange={onChange} showPending={!creating || !spec.createOnly} /></div>;
  })}</TabsContent>)}</Tabs>{error && <p role="alert" className="mt-3 text-sm text-destructive">{error}</p>}<DialogFooter className="mt-4"><Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>Cancel</Button><Button type="submit" disabled={saving}>{saving ? "Saving…" : creating ? "Create template" : "Save template"}</Button></DialogFooter></form></DialogContent></Dialog>;
}

export function PreferencesPage({ fromProject }: { fromProject?: string } = {}) {
  const navigate = useNavigate();
  const [runtime, setRuntime] = useState<RuntimeKey>("docker");
  const [templates, setTemplates] = useState<TemplateRecord[]>([]);
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState("");
  const [editor, setEditor] = useState<TemplateRecord | null>(null);
  const [createOpen, setCreateOpen] = useState(false);
  const [notice, setNotice] = useState("");

  async function refresh() {
    setBusy(true); setError("");
    try { setTemplates(await api.templates() as TemplateRecord[]); }
    catch (reason) { setError(`Template list unavailable: ${reason instanceof Error ? reason.message : "Request failed"}`); }
    finally { setBusy(false); }
  }
  useEffect(() => { void refresh(); }, []);
  const runtimeTemplates = useMemo(() => templates.filter((template) => template.template_type === runtime), [templates, runtime]);

  async function saveTemplate(id: string, patch: Record<string, unknown>) { const updated = await api.updateTemplate(id, patch); setTemplates((current) => current.map((item) => item.template_id === id ? updated as TemplateRecord : item)); }
  async function createTemplate(patch: Record<string, unknown>) {
    const created = await api.createTemplate({ ...patch, template_type: runtime, name: String(patch.name ?? `${runtimeLabels[runtime]} template`) });
    setTemplates((current) => [...current, created as TemplateRecord]); setCreateOpen(false); setNotice(`${runtimeLabels[runtime]} template created.`);
  }
  async function duplicate(template: TemplateRecord) {
    setError("");
    try { const copy = await api.duplicateTemplate(template.template_id); setTemplates((current) => [...current, copy as TemplateRecord]); setNotice(`Duplicated “${template.name}”.`); }
    catch (reason) { setError(`Template duplication unavailable: ${reason instanceof Error ? reason.message : "Request failed"}`); }
  }
  async function remove(template: TemplateRecord) {
    if (!window.confirm(`Delete template “${template.name}”?`)) return;
    setError("");
    try { await api.deleteTemplate(template.template_id); setTemplates((current) => current.filter((item) => item.template_id !== template.template_id)); }
    catch (reason) { setError(`Template deletion unavailable: ${reason instanceof Error ? reason.message : "Request failed"}`); }
  }

  return <PagesShell active="Preferences" fromProject={fromProject}><div className="pages-content">
    <PageTitle eyebrow="CONTROLLER" title="Preferences" description="Manage runtime templates and the defaults used for new nodes." actions={<>{fromProject && <Button variant="ghost" onClick={() => void navigate({ to: "/projects/$projectId" as never, params: { projectId: fromProject } as never })}>Back to project</Button>}<Button variant="outline" onClick={() => void navigate({ to: "/appliances" as never, search: { fromProject } as never })}>Appliances</Button><Button variant="outline" onClick={() => void navigate({ to: "/settings" as never })}>General settings</Button><Button onClick={() => void navigate({ to: "/templates/new" as never })}><Plus size={15} />New template wizard</Button></>} />
    {error && <div role="alert" className="pages-alert">{error}</div>}{notice && <div role="status" className="pages-notice">{notice}</div>}
    <div className="preferences-layout"><Card className="preferences-sidebar"><div className="preferences-sidebar-title"><Boxes size={17} />Template types</div>{runtimeOrder.map((key) => <button key={key} className={`preferences-runtime${runtime === key ? " is-active" : ""}`} onClick={() => setRuntime(key)}>{runtimeLabels[key]}{unsupported.has(key) && <Badge variant="outline">Unsupported</Badge>}</button>)}</Card>
      <section className="min-w-0"><div className="flex items-start justify-between gap-4"><div><h2 className="text-lg font-semibold">{runtimeLabels[runtime]} templates</h2><p className="mt-1 text-sm text-muted-foreground">{unsupported.has(runtime) ? "This runtime is not implemented by the controller." : "Templates are shared across projects and supply defaults when a node is created."}</p></div>{supported.has(runtime) && <div className="flex gap-2"><Button variant="outline" size="sm" onClick={() => void refresh()} disabled={busy}><RefreshCw size={14} />Refresh</Button><Button size="sm" onClick={() => setCreateOpen(true)}><Plus size={14} />Add template</Button></div>}</div>
        {runtime === "iou" && <p className="mt-3 text-sm text-amber-700">Template and node data are saved, but this controller cannot execute IOU without its runtime binary.</p>}
        {unsupported.has(runtime) ? <Card className="pages-empty-state mt-4"><h3>{runtimeLabels[runtime]} is unsupported</h3><p>There is no runtime adapter or template backend for this type in this controller. No template operations are available.</p></Card> : <Card className="mt-4 overflow-hidden"><Table><TableHeader><TableRow><TableHead>Template</TableHead><TableHead>Image / runtime</TableHead><TableHead>Actions</TableHead></TableRow></TableHeader><TableBody>{busy ? <TableRow><TableCell colSpan={3} className="py-10 text-center text-muted-foreground">Loading templates…</TableCell></TableRow> : runtimeTemplates.length ? runtimeTemplates.map((template) => <TableRow key={template.template_id}><TableCell><div className="font-medium">{template.name}</div><div className="text-xs text-muted-foreground">{template.template_id}</div></TableCell><TableCell>{("image" in template && typeof template.image === "string" ? template.image : "path" in template && typeof template.path === "string" ? template.path : "—")}</TableCell><TableCell><div className="flex justify-end gap-1"><Button size="sm" variant="outline" onClick={() => setEditor(template)}><Pencil size={14} />Configure</Button><Button size="icon-sm" variant="ghost" title="Duplicate template" aria-label={`Duplicate ${template.name}`} onClick={() => void duplicate(template)}><Copy size={15} /></Button><Button size="icon-sm" variant="ghost" title="Delete template" aria-label={`Delete ${template.name}`} onClick={() => void remove(template)}><Trash2 size={15} /></Button></div></TableCell></TableRow>) : <TableRow><TableCell colSpan={3} className="py-10 text-center text-muted-foreground">No {runtimeLabels[runtime]} templates are available. Create a manual template or install an appliance definition.</TableCell></TableRow>}</TableBody></Table></Card>}
        {runtime.startsWith("ethernet_") || runtime === "cloud" || runtime === "nat" ? <Card className="mt-4 p-4"><div className="flex items-start justify-between gap-4"><div><h3 className="font-medium">Node editor forms</h3><p className="mt-1 text-sm text-muted-foreground">Edit an existing project node with the matching GNS3 configurator fields.</p></div><Button variant="outline" size="sm" onClick={() => void navigate({ to: "/node-editors/$nodeType" as never, params: { nodeType: runtime } as never })}>Open node editor <ExternalLink size={14} /></Button></div><p className="mt-3 text-xs text-muted-foreground">Built-in runtime support is still being added. Node settings are sent to the controller and any unavailable route is reported.</p></Card> : null}
      </section>
    </div>
    <TemplateEditor template={editor ?? undefined} runtime={runtime} open={Boolean(editor)} onOpenChange={(open) => !open && setEditor(null)} onSave={(patch) => editor ? saveTemplate(editor.template_id, patch) : Promise.resolve()} />
    <TemplateEditor runtime={runtime} open={createOpen} onOpenChange={setCreateOpen} onSave={createTemplate} creating />
  </div></PagesShell>;
}
