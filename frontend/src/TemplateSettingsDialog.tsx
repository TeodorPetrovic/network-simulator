import { useEffect, useState } from "react";
import type { FormEvent } from "react";
import type { RuntimeTemplate } from "./api";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";

export function TemplateSettingsDialog({ template, open, onOpenChange, onSave }: {
  template: RuntimeTemplate;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSave: (id: string, patch: Record<string, unknown>) => Promise<void>;
}) {
  const [draft, setDraft] = useState<Record<string, string>>({});
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    setDraft({
      name: template.name,
      adapters: String("adapters" in template ? template.adapters : 1),
      memory: String("memory" in template ? template.memory : 0),
      cpus: String("cpus" in template ? template.cpus : 0),
      ...(template.template_type === "qemu" ? {
        adapter_type: template.adapter_type ?? "e1000",
        hda_disk_interface: template.hda_disk_interface ?? "ide",
        boot_priority: template.boot_priority ?? "c",
      } : {}),
      ...(template.template_type === "docker" ? {
        start_command: template.start_command ?? "",
        environment: template.environment ?? "",
        extra_hosts: template.extra_hosts ?? "",
        extra_volumes: (template.extra_volumes ?? []).join("\n"),
      } : {}),
      ...(template.template_type === "dynamips" ? {
        platform: template.platform,
        chassis: template.chassis ?? "",
        slot0: template.slot0 ?? "C7200-IO-FE",
        nvram: String(template.nvram),
        idlepc: template.idlepc,
      } : {}),
      ...(template.template_type === "vpcs" ? { startup_script: template.startup_script ?? "" } : {}),
    });
    setError("");
  }, [open, template]);

  const field = (name: string, label: string, type = "text", helper?: string) => <label className="grid gap-1.5" key={name}>
    <Label htmlFor={`template-${template.template_id}-${name}`}>{label}</Label>
    <Input id={`template-${template.template_id}-${name}`} type={type} value={draft[name] ?? ""} min={type === "number" ? "0" : undefined} onChange={(event) => setDraft((current) => ({ ...current, [name]: event.target.value }))} />
    {helper && <span className="text-xs text-muted-foreground">{helper}</span>}
  </label>;
  const selectField = (name: string, label: string, values: string[]) => <label className="grid gap-1.5" key={name}>
    <Label>{label}</Label>
    <Select value={draft[name] ?? values[0]} onValueChange={(value) => setDraft((current) => ({ ...current, [name]: value }))}>
      <SelectTrigger aria-label={label}><SelectValue /></SelectTrigger>
      <SelectContent position="popper" align="start">{values.map((value) => <SelectItem key={value} value={value}>{value}</SelectItem>)}</SelectContent>
    </Select>
  </label>;
  const textArea = (name: string, label: string, helper: string) => <label className="grid gap-1.5" key={name}>
    <Label htmlFor={`template-${template.template_id}-${name}`}>{label}</Label>
    <Textarea id={`template-${template.template_id}-${name}`} value={draft[name] ?? ""} onChange={(event) => setDraft((current) => ({ ...current, [name]: event.target.value }))} rows={name === "start_command" ? 2 : 4} />
    <span className="text-xs text-muted-foreground">{helper}</span>
  </label>;

  async function save(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setSaving(true);
    setError("");
    const patch: Record<string, unknown> = { name: draft.name?.trim() };
    if (template.template_type === "qemu" || template.template_type === "docker") patch.adapters = Number(draft.adapters);
    if (template.template_type === "qemu" || template.template_type === "docker" || template.template_type === "dynamips") patch.memory = Number(draft.memory);
    if (template.template_type === "qemu") {
      patch.cpus = Number(draft.cpus);
      patch.adapter_type = draft.adapter_type;
      patch.hda_disk_interface = draft.hda_disk_interface;
      patch.boot_priority = draft.boot_priority;
    } else if (template.template_type === "docker") {
      patch.cpus = Number(draft.cpus);
      patch.start_command = draft.start_command ?? "";
      patch.environment = draft.environment ?? "";
      patch.extra_hosts = draft.extra_hosts ?? "";
      patch.extra_volumes = (draft.extra_volumes ?? "").split("\n").map((path) => path.trim()).filter(Boolean);
    } else if (template.template_type === "dynamips") {
      patch.platform = draft.platform;
      patch.chassis = draft.chassis;
      patch.slot0 = draft.slot0;
      patch.nvram = Number(draft.nvram);
      patch.idlepc = draft.idlepc;
    } else if (template.template_type === "vpcs") {
      patch.startup_script = draft.startup_script ?? "";
    }
    try {
      await onSave(template.template_id, patch);
      onOpenChange(false);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "Could not save template settings");
    } finally {
      setSaving(false);
    }
  }

  return <Dialog open={open} onOpenChange={onOpenChange}><DialogContent className="template-settings-dialog">
    <DialogHeader><DialogTitle>Configure template</DialogTitle><DialogDescription>These settings are saved globally and used for new devices based on this template.</DialogDescription></DialogHeader>
    <form className="template-settings-form" onSubmit={(event) => void save(event)}>
      <div className="template-settings-grid">
        {field("name", "Template name")}
        {(template.template_type === "qemu" || template.template_type === "docker") && field("adapters", "Network adapters", "number")}
        {(template.template_type === "qemu" || template.template_type === "docker" || template.template_type === "dynamips") && field("memory", template.template_type === "qemu" ? "RAM (MB)" : template.template_type === "dynamips" ? "RAM (MB)" : "Memory limit (MB)", "number", template.template_type === "docker" ? "0 means no Docker memory limit." : undefined)}
        {(template.template_type === "qemu" || template.template_type === "docker") && field("cpus", template.template_type === "qemu" ? "vCPUs" : "CPU limit", "number", "0 means no Docker CPU limit.")}
        {template.template_type === "qemu" && <>
          {selectField("adapter_type", "Network adapter model", ["e1000", "virtio-net-pci", "rtl8139", "vmxnet3"])}
          {selectField("hda_disk_interface", "Primary disk interface", ["ide", "scsi", "virtio", "sd"])}
          {selectField("boot_priority", "Boot priority", ["c", "d", "n", "dc", "cd"])}
        </>}
        {template.template_type === "docker" && <div className="template-settings-wide">
          {textArea("start_command", "Start command", "Shell-like command arguments passed to Docker; leave blank to use the image default.")}
          {textArea("environment", "Environment variables", "One KEY=VALUE entry per line.")}
          {textArea("extra_hosts", "Extra hosts", "One host:address entry per line.")}
          {textArea("extra_volumes", "Persistent container paths", "One absolute path inside the container per line. Data is stored under the project runtime directory.")}
        </div>}
        {template.template_type === "dynamips" && <>
          {selectField("platform", "Dynamips platform", ["c1700", "c2600", "c2691", "c3600", "c3725", "c3745", "c7200"])}
          {field("chassis", "Chassis", "text", "For c3600, choose a supported chassis such as 3620.")}
          {field("slot0", "Slot 0 adapter")}
          {field("nvram", "NVRAM (KB)", "number")}
          {field("idlepc", "Idle-PC value", "text", "Leave blank to let the router use its normal idle behavior.")}
        </>}
        {template.template_type === "vpcs" && <div className="template-settings-wide">{textArea("startup_script", "Startup script", "VPCS commands executed when a node starts.")}</div>}
      </div>
      {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
      <DialogFooter><Button type="button" variant="ghost" disabled={saving} onClick={() => onOpenChange(false)}>Cancel</Button><Button type="submit" disabled={saving}>{saving ? "Saving…" : "Save template"}</Button></DialogFooter>
    </form>
  </DialogContent></Dialog>;
}
