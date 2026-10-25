import { useEffect, useState } from "react";
import type { FormEvent } from "react";
import { ExternalLink } from "lucide-react";
import type { Node } from "./api";
import { dockerHttpConsoleUrl } from "./api";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
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
  const supported = ["qemu", "docker", "vpcs", "dynamips"].includes(kind ?? "");

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
      } : {}),
      ...(kind === "docker" ? {
        start_command: String(node.start_command ?? ""),
        environment: String(node.environment ?? ""),
        extra_hosts: String(node.extra_hosts ?? ""),
        console_type: String(node.console_type ?? "telnet"),
        console_http_port: String(node.console_http_port ?? 80),
        console_http_path: String(node.console_http_path ?? "/"),
        aux_type: String(node.aux_type ?? "none"),
        extra_volumes: Array.isArray(node.extra_volumes)
          ? node.extra_volumes.filter((path): path is string => typeof path === "string").join("\n")
          : "",
      } : {}),
      ...(kind === "vpcs" ? { startup_script: String(node.startup_script ?? "") } : {}),
      ...(kind === "dynamips" ? { startup_config: String(node.startup_config ?? "") } : {}),
    });
    setError("");
  }, [kind, node, open]);

  const field = (name: string, label: string, type = "text", helper?: string, step?: string, disabled = false) => <label className="grid gap-1.5" key={name}>
    <Label htmlFor={`node-${node?.node_id}-${name}`}>{label}</Label>
    <Input id={`node-${node?.node_id}-${name}`} type={type} step={step} min={type === "number" ? "0" : undefined} disabled={disabled} value={draft[name] ?? ""} onChange={(event) => setDraft((current) => ({ ...current, [name]: event.target.value }))} />
    {helper && <span className="text-xs text-muted-foreground">{helper}</span>}
  </label>;
  const selectField = (name: string, label: string, values: string[]) => <label className="grid gap-1.5" key={name}>
    <Label>{label}</Label>
    <Select value={draft[name] ?? values[0]} onValueChange={(value) => setDraft((current) => ({ ...current, [name]: value }))}>
      <SelectTrigger aria-label={label}><SelectValue /></SelectTrigger>
      <SelectContent>{values.map((value) => <SelectItem key={value} value={value}>{value}</SelectItem>)}</SelectContent>
    </Select>
  </label>;
  const textArea = (name: string, label: string, helper: string, rows = 4) => <label className="grid gap-1.5" key={name}>
    <Label htmlFor={`node-${node?.node_id}-${name}`}>{label}</Label>
    <Textarea id={`node-${node?.node_id}-${name}`} value={draft[name] ?? ""} onChange={(event) => setDraft((current) => ({ ...current, [name]: event.target.value }))} rows={rows} />
    <span className="text-xs text-muted-foreground">{helper}</span>
  </label>;

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
    } else if (kind === "docker") {
      patch.memory = Number(draft.memory);
      patch.cpus = Number(draft.cpus);
      patch.adapters = Number(draft.adapters);
      patch.start_command = draft.start_command ?? "";
      patch.environment = draft.environment ?? "";
      patch.extra_hosts = draft.extra_hosts ?? "";
      patch.extra_volumes = (draft.extra_volumes ?? "").split("\n").map((path) => path.trim()).filter(Boolean);
      patch.console_type = draft.console_type ?? "telnet";
      if (patch.console_type !== "none") {
        patch.console_http_port = Number(draft.console_http_port);
        patch.console_http_path = draft.console_http_path || "/";
      }
      patch.aux_type = draft.aux_type ?? "none";
    } else if (kind === "vpcs") {
      patch.startup_script = draft.startup_script ?? "";
    } else if (kind === "dynamips") {
      patch.startup_config = draft.startup_config ?? "";
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
    const host = typeof node.console_host === "string" && node.console_host ? node.console_host : window.location.hostname;
    const url = dockerHttpConsoleUrl(draft.console_type, host, Number(draft.console_http_port), draft.console_http_path || "/");
    if (!url) {
      setError("Enter a valid HTTP console port and path before opening the console.");
      return;
    }
    window.open(url, "_blank", "noopener,noreferrer");
  }

  if (!node || !supported) return null;
  return <Dialog open={open} onOpenChange={onOpenChange}><DialogContent className="template-settings-dialog">
    <DialogHeader><DialogTitle>Configure {node.name}</DialogTitle><DialogDescription>
      Settings apply to this device only. Stop it before saving.{kind === "docker" ? " Docker settings replace its stopped container; configured persistent paths keep their data." : ""}
    </DialogDescription></DialogHeader>
    <form className="template-settings-form" onSubmit={(event) => void save(event)}>
      <div className="template-settings-grid">
        {field("name", "Device name")}
        {(kind === "qemu" || kind === "docker") && field("adapters", "Network adapters", "number", node.status !== "stopped" ? "Stop the device to change the number of adapters" : "The count cannot be reduced below ports already used by links.", undefined, node.status !== "stopped")}
        {(kind === "qemu" || kind === "docker") && field("memory", kind === "qemu" ? "RAM (MB)" : "Memory limit (MB)", "number", kind === "docker" ? "0 means no Docker memory limit." : undefined)}
        {(kind === "qemu" || kind === "docker") && field("cpus", kind === "qemu" ? "vCPUs" : "CPU limit", "number", kind === "docker" ? "0 means no Docker CPU limit." : undefined, kind === "docker" ? "0.25" : "1")}
        {kind === "qemu" && <>
          {selectField("adapter_type", "Network adapter model", ["e1000", "virtio-net-pci", "rtl8139", "vmxnet3"])}
          {selectField("hda_disk_interface", "Primary disk interface", ["ide", "scsi", "virtio", "sd"])}
          {selectField("boot_priority", "Boot priority", ["c", "d", "n", "dc", "cd"])}
        </>}
        {kind === "docker" && <div className="template-settings-wide">
          {selectField("console_type", "Console type", ["telnet", "vnc", "http", "https", "none"])}
          {draft.console_type === "http" || draft.console_type === "https" ? <div className="grid gap-2 rounded-md border p-3 sm:col-span-2">
            <p className="text-sm">The controller stores this console port and path but does not publish or proxy Docker ports. Open in a new tab assumes the service is reachable from your browser.</p>
            <div className="grid gap-3 sm:grid-cols-2">{field("console_http_port", "HTTP port in the container", "number", undefined, "1")}{field("console_http_path", "HTTP path", "text", "Must begin with a slash.")}</div>
            <Button type="button" variant="outline" className="w-fit" onClick={openHttpConsole}><ExternalLink />Open in new tab</Button>
          </div> : null}
          {draft.console_type === "vnc" && <p role="status" className="text-sm text-muted-foreground sm:col-span-2">The controller can start VNC on its TCP console port, but this UI has no browser VNC route.</p>}
          {draft.console_type === "none" && <p role="status" className="text-sm text-muted-foreground sm:col-span-2">The controller does not accept Docker console type “none” yet. Select another type to save.</p>}
          {selectField("aux_type", "Auxiliary console type", ["none", "telnet", "ssh"])}
          {draft.aux_type !== "none" && <p role="status" className="text-sm text-muted-foreground sm:col-span-2">When the device is running, open its context menu and choose “Open auxiliary console”.</p>}
          {textArea("start_command", "Start command", "Arguments passed to the image. Leave blank to use its default command.", 2)}
          {textArea("environment", "Environment variables", "One KEY=VALUE entry per line.")}
          {textArea("extra_hosts", "Extra hosts", "One host:address entry per line.")}
          {textArea("extra_volumes", "Persistent container paths", "One absolute path inside the container per line. Data lives in the project runtime directory.")}
        </div>}
        {kind === "vpcs" && <div className="template-settings-wide">{textArea("startup_script", "Startup script", "VPCS commands run when this device starts.")}</div>}
        {kind === "dynamips" && <div className="template-settings-wide">{textArea("startup_config", "Startup configuration", "IOS configuration written before the router starts.", 8)}</div>}
      </div>
      {node.status !== "stopped" && <p role="status" className="text-sm text-muted-foreground">Stop this device to edit its runtime settings.</p>}
      {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
      <DialogFooter><Button type="button" variant="ghost" disabled={saving} onClick={() => onOpenChange(false)}>Cancel</Button><Button type="submit" disabled={saving || node.status !== "stopped" || (kind === "docker" && draft.console_type === "none")}>{saving ? "Saving…" : "Save device settings"}</Button></DialogFooter>
    </form>
  </DialogContent></Dialog>;
}

