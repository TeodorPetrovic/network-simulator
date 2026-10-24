import { useEffect, useMemo, useRef, useState } from "react";
import type { FormEvent, ReactNode } from "react";
import { Check, FileUp, HardDrive } from "lucide-react";
import { api } from "./api";
import type { ApplianceImageRequirement, ApplianceVersion, Gns3Appliance, LocalApplianceImage, RuntimeTemplate } from "./api";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";

export function CreateCustomApplianceDialog({ open, onOpenChange, imageOptions, onCreated }: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  imageOptions: RuntimeTemplate[];
  onCreated: (appliance: Gns3Appliance) => Promise<void>;
}) {
  const [name, setName] = useState("");
  const [runtime, setRuntime] = useState<RuntimeTemplate["template_type"]>("qemu");
  const [sourceTemplateId, setSourceTemplateId] = useState("");
  const [platform, setPlatform] = useState("c7200");
  const [chassis, setChassis] = useState("3620");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const runtimeTypes: RuntimeTemplate["template_type"][] = ["dynamips", "qemu", "docker", "vpcs"];
  const availableRuntimes = useMemo(() => runtimeTypes.filter((type) => imageOptions.some((option) => option.template_type === type)), [imageOptions]);
  const candidates = imageOptions.filter((option) => option.template_type === runtime);
  const selected = candidates.find((option) => option.template_id === sourceTemplateId) ?? candidates[0];

  useEffect(() => {
    if (!open) return;
    const firstRuntime = availableRuntimes[0];
    if (!availableRuntimes.includes(runtime) && firstRuntime) setRuntime(firstRuntime);
    const current = imageOptions.find((option) => option.template_id === sourceTemplateId);
    if (!current || current.template_type !== runtime) {
      const firstOption = imageOptions.find((option) => option.template_type === runtime) ?? (firstRuntime ? imageOptions.find((option) => option.template_type === firstRuntime) : undefined);
      if (firstOption) {
        setRuntime(firstOption.template_type);
        setSourceTemplateId(firstOption.template_id);
      }
    }
  }, [open, availableRuntimes, imageOptions, runtime, sourceTemplateId]);

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!selected) {
      setError("Choose an installed runtime image first. Add one through the global image library or Docker image catalog.");
      return;
    }
    setSaving(true);
    setError("");
    try {
      const appliance = await api.createCustomAppliance({
        name: name.trim(),
        source_template_id: selected.template_id,
        runtime_settings: runtime === "dynamips" ? { platform, ...(platform === "c3600" ? { chassis } : {}) } : {},
      });
      onOpenChange(false);
      setName("");
      await onCreated(appliance);
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setSaving(false);
    }
  }

  return <Dialog open={open} onOpenChange={onOpenChange}><DialogContent className="custom-appliance-dialog">
    <DialogHeader>
      <DialogTitle>Create custom appliance</DialogTitle>
      <DialogDescription>Choose a runtime image to create a reusable appliance definition. The appliance is stored in this library and becomes available in every project.</DialogDescription>
    </DialogHeader>
    <form className="grid gap-4" onSubmit={(event) => void submit(event)}>
      <div className="grid gap-1.5"><Label htmlFor="custom-appliance-name">Appliance name</Label><Input id="custom-appliance-name" required maxLength={100} value={name} onChange={(event) => setName(event.target.value)} placeholder="e.g. MikroTik CHR lab" /></div>
      {availableRuntimes.length > 0 ? <>
        <div className="grid gap-1.5"><Label>Runtime</Label><Select value={runtime} onValueChange={(value) => {
          const nextRuntime = value as RuntimeTemplate["template_type"];
          setRuntime(nextRuntime);
          setSourceTemplateId(imageOptions.find((option) => option.template_type === nextRuntime)?.template_id ?? "");
        }}><SelectTrigger aria-label="Custom appliance runtime"><SelectValue /></SelectTrigger><SelectContent>{availableRuntimes.map((type) => <SelectItem key={type} value={type}>{applianceRuntimeLabel(type)}</SelectItem>)}</SelectContent></Select></div>
        <div className="grid gap-1.5"><Label>Installed image or runtime</Label><Select value={selected?.template_id ?? ""} onValueChange={setSourceTemplateId}><SelectTrigger aria-label="Installed image"><SelectValue placeholder="Select an installed image" /></SelectTrigger><SelectContent className="appliance-select-content">{candidates.map((option) => <SelectItem key={option.template_id} value={option.template_id}>{runtimeOptionLabel(option)}</SelectItem>)}</SelectContent></Select><small className="text-xs text-muted-foreground">This selection is only used to create the appliance. Images do not appear as appliances until you create one.</small></div>
        {runtime === "dynamips" && <>
          <div className="grid gap-1.5"><Label>Dynamips platform</Label><Select value={platform} onValueChange={setPlatform}><SelectTrigger aria-label="Dynamips platform"><SelectValue /></SelectTrigger><SelectContent><SelectItem value="c7200">Cisco 7200</SelectItem><SelectItem value="c3600">Cisco 3600</SelectItem></SelectContent></Select></div>
          {platform === "c3600" && <div className="grid gap-1.5"><Label htmlFor="custom-dynamips-chassis">Chassis</Label><Input id="custom-dynamips-chassis" value={chassis} onChange={(event) => setChassis(event.target.value)} placeholder="3620" /></div>}
        </>}
      </> : <EmptyState icon={<HardDrive />} title="No runtime images available" text="Add a QEMU or Dynamips image, or install a Docker image, then create the appliance from it." />}
      {!error && availableRuntimes.length === 0 && <p className="text-sm text-muted-foreground">Import a GNS3 appliance or add an image/runtime first. Installed images are offered here as inputs; they are not added to the appliance library by themselves.</p>}
      {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
      <DialogFooter><Button type="button" variant="outline" disabled={saving} onClick={() => onOpenChange(false)}>Cancel</Button><Button type="submit" disabled={saving || !selected || !name.trim()}>{saving ? "Creating…" : "Create appliance"}</Button></DialogFooter>
    </form>
  </DialogContent></Dialog>;
}

export function runtimeOptionLabel(template: RuntimeTemplate): string {
  if (template.template_type === "docker") return template.image;
  if (template.template_type === "qemu") return template.hda_disk_image.split("/").at(-1) ?? template.hda_disk_image;
  if (template.template_type === "dynamips") return template.image.split("/").at(-1) ?? template.image;
  return template.name;
}

export function ImageAssignmentCard({ version, requirement, localImages, busy, onSave, onClear }: { version: ApplianceVersion; requirement: ApplianceImageRequirement; localImages: LocalApplianceImage[]; busy: boolean; onSave: (version: ApplianceVersion, requirement: ApplianceImageRequirement, image: LocalApplianceImage, allowMismatch: boolean) => void; onClear: (version: ApplianceVersion, requirement: ApplianceImageRequirement) => void }) {
  const [imageId, setImageId] = useState(requirement.assigned_image_id ?? requirement.suggested_image_id ?? "");
  const [allowMismatch, setAllowMismatch] = useState(requirement.allow_checksum_mismatch);
  const selected = localImages.find((image) => image.image_id === imageId);
  useEffect(() => {
    setImageId(requirement.assigned_image_id ?? requirement.suggested_image_id ?? "");
    setAllowMismatch(requirement.allow_checksum_mismatch);
  }, [requirement.assigned_image_id, requirement.suggested_image_id, requirement.allow_checksum_mismatch]);
  const status = statusLabel(requirement.status);
  const isMismatch = requirement.status === "mismatch" || Boolean(selected && selected.image_id !== requirement.assigned_image_id && selected.image_id !== requirement.suggested_image_id);
  const canOverride = Boolean(selected && (requirement.md5sum || requirement.filesize != null || isMismatch || requirement.status === "override"));

  return <Card className="appliance-image-card">
    <div className="appliance-image-card-heading"><div className="min-w-0"><div className="flex flex-wrap items-center gap-2"><Badge variant="outline">{requirement.key}</Badge><Badge variant={status.variant}>{status.label}</Badge></div><h4 className="break-all font-mono text-sm">{requirement.filename}</h4></div>{requirement.filesize != null && <span className="shrink-0 text-xs text-muted-foreground">{formatBytes(requirement.filesize)}</span>}</div>
    <div className="appliance-hash-grid"><span>Expected MD5</span><code>{requirement.md5sum ?? "Not published"}</code><span>Local MD5</span><code>{requirement.actual_md5sum ?? "Not checked"}</code><span>Local SHA1</span><code>{requirement.actual_sha1sum ?? "Not checked"}</code><span>Local size</span><code>{requirement.actual_size == null ? "Not checked" : formatBytes(requirement.actual_size)}</code></div>
    {(requirement.assigned_image_name || requirement.image_path) && <p className="break-all text-xs text-muted-foreground">Assigned to {requirement.assigned_image_name ?? "missing local file"}{requirement.image_path ? ` · ${requirement.image_path}` : ""}</p>}
    {requirement.assignable ? <div className="grid gap-2 sm:grid-cols-[minmax(0,1fr)_auto]">
      <Select value={imageId} onValueChange={setImageId}>
        <SelectTrigger aria-label={`Local image for ${version.name} ${requirement.key}`} className="w-full"><SelectValue placeholder={localImages.length ? "Choose an image from the global library" : `No ${version.template_type ?? "compatible"} images installed`} /></SelectTrigger>
        <SelectContent position="popper" align="start" className="appliance-select-content">{localImages.map((image) => <SelectItem key={image.image_id} value={image.image_id}>{image.name} · {formatBytes(image.size)}</SelectItem>)}</SelectContent>
      </Select>
      <div className="flex gap-2"><Button variant="outline" disabled={!selected || busy} onClick={() => selected && onSave(version, requirement, selected, allowMismatch)}>{busy ? "Saving…" : "Assign image"}</Button>{requirement.assigned_image_id && <Button variant="ghost" disabled={busy} onClick={() => onClear(version, requirement)} aria-label={`Clear ${requirement.key} image assignment`}>Clear</Button>}</div>
    </div> : <p className="text-xs text-muted-foreground">This configuration lists the image slot, but the current runtime does not attach <code>{requirement.key}</code>. The value is shown for reference and cannot be assigned yet.</p>}
    {(canOverride || allowMismatch || requirement.status === "override") && <div className="appliance-override-toggle"><Checkbox id={`allow-image-override-${requirement.key}`} checked={allowMismatch} onCheckedChange={(checked) => setAllowMismatch(checked === true)} /><div><Label htmlFor={`allow-image-override-${requirement.key}`}>Allow this image despite checksum or size mismatch</Label><p>GNS3 reports the expected and actual values above. This override is saved to this appliance version and image slot.</p></div></div>}
    {requirement.status === "missing" && localImages.length > 0 && <p className="text-xs text-muted-foreground">No exact filename or checksum match was found. Choose a file to verify it, or enable the explicit override if it is a known custom image.</p>}
    {requirement.status === "assigned-image-missing" && <p className="text-xs text-destructive">The assigned path is no longer present in the global library. Select an available image to replace it.</p>}
  </Card>;
}

export function RegistryDetails({ appliance, busy, onImport }: { appliance: Gns3Appliance; busy: boolean; onImport: () => void }) {
  return <div className="registry-details"><div><Badge variant="outline">GNS3 appliance</Badge><h2>{appliance.name}</h2><p>{appliance.description || "No description is provided."}</p></div><div className="appliance-config-meta"><span>Category</span><b>{appliance.category ?? "Uncategorized"}</b><span>Runtime</span><b>{appliance.template_type ?? "Registry metadata only"}</b><span>Definition</span><b>{appliance.file_name}</b><span>Configs</span><b>{appliance.versions?.length ?? 0}</b></div><h3>Versions and image requirements</h3><ScrollArea className="registry-version-list"><div className="space-y-2 pr-3">{appliance.versions?.map((version) => <Card key={version.name} className="p-3 shadow-none"><div className="mb-2 flex flex-wrap items-center gap-2"><b>{version.name}</b><Badge variant="outline">{version.template_type ?? "Unknown runtime"}</Badge><Badge variant={version.runtime_supported ? "secondary" : "destructive"}>{version.runtime_supported ? "Supported" : "Unsupported"}</Badge></div>{version.docker_image && <div className="registry-requirement"><code>Docker image</code><b className="break-all">{version.docker_image}</b><small>Pull this image after importing the definition to create its project template.</small></div>}{version.images.length ? version.images.map((image) => <div key={image.key} className="registry-requirement"><code>{image.key}</code><b className="break-all">{image.filename}</b><small>MD5 {image.md5sum ?? "not published"}{image.filesize == null ? "" : ` · ${formatBytes(image.filesize)}`}</small></div>) : !version.docker_image && <p className="text-xs text-muted-foreground">No separate local image slots.</p>}</Card>)}{!appliance.versions?.length && <p className="text-sm text-muted-foreground">No version configurations are included in this definition.</p>}</div></ScrollArea><div className="flex justify-end"><Button onClick={onImport} disabled={busy || appliance.imported}>{appliance.imported ? <><Check />Already imported</> : <><FileUp />Import this appliance</>}</Button></div></div>;
}

export function EmptyState({ icon, title, text }: { icon: ReactNode; title: string; text: string }) {
  return <div className="appliance-empty-state">{icon}<b>{title}</b><span>{text}</span></div>;
}

export function applianceRuntimeLabel(runtime: string): string {
  return ({ dynamips: "Dynamips · Cisco IOS", qemu: "QEMU / KVM", docker: "Docker", vpcs: "VPCS", other: "Other runtimes" } as Record<string, string>)[runtime] ?? runtime;
}

export function FilePickerButton({ icon, label, accept, disabled, onSelect }: { icon: ReactNode; label: string; accept: string; disabled: boolean; onSelect: (file: File) => void | Promise<void> }) {
  const inputRef = useRef<HTMLInputElement>(null);
  return <>
    <Button type="button" variant="outline" className="w-full justify-start" disabled={disabled} onClick={() => inputRef.current?.click()}>{icon}{label}</Button>
    <Input ref={inputRef} type="file" accept={accept} aria-label={label} className="sr-only" disabled={disabled} onChange={(event) => {
      const file = event.currentTarget.files?.[0];
      event.currentTarget.value = "";
      if (file) void onSelect(file);
    }} />
  </>;
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) { value /= 1024; unit += 1; }
  return `${value.toFixed(value >= 10 ? 0 : 1)} ${units[unit]}`;
}

export function statusLabel(status: string): { label: string; variant: "default" | "secondary" | "destructive" | "outline" } {
  switch (status) {
    case "verified": return { label: "Hash and size verified", variant: "secondary" };
    case "size-verified": return { label: "Size verified · no MD5", variant: "secondary" };
    case "runtime-slot-unsupported": return { label: "Image slot not attachable", variant: "destructive" };
    case "override": return { label: "Explicit override", variant: "default" };
    case "available-unverified": return { label: "Available · no registry hash", variant: "outline" };
    case "mismatch": return { label: "Checksum or size mismatch", variant: "destructive" };
    case "assigned-image-missing": return { label: "Assigned file missing", variant: "destructive" };
    default: return { label: "Missing image", variant: "outline" };
  }
}

export function versionIsReady(version: ApplianceVersion): boolean {
  if (!version.runtime_supported) return false;
  if (version.template_type === "vpcs") return true;
  if (version.template_type === "docker") return version.docker_image_available === true;
  return version.images.length > 0 && version.images.every((image) => ["verified", "size-verified", "override", "available-unverified"].includes(image.status));
}

export function errorMessage(reason: unknown): string {
  return reason instanceof Error ? reason.message : "Appliance manager request failed. Please try again.";
}
