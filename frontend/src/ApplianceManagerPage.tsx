import { useEffect, useMemo, useRef, useState } from "react";
import type { FormEvent } from "react";
import type { ReactNode } from "react";
import { ArrowLeft, Check, ChevronDown, Download, FileUp, HardDrive, Plus, RefreshCw, Search, Server, Upload, X } from "lucide-react";
import { api } from "./api";
import type { ApplianceImageRequirement, ApplianceVersion, Gns3Appliance, Gns3ApplianceSearchResult, LocalApplianceImage, RuntimeTemplate } from "./api";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Separator } from "@/components/ui/separator";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle, DialogTrigger } from "@/components/ui/dialog";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";

type ManagerTab = "installed" | "registry";
type LibraryEntry = { id: string; name: string; runtime: string; appliance: Gns3Appliance };

export function ApplianceManagerPage({ onBack, onOpenPreferences, onOpenImageCatalog, onTemplatesChanged }: { onBack: () => void; onOpenPreferences?: () => void; onOpenImageCatalog?: () => void; onTemplatesChanged?: (templates: Awaited<ReturnType<typeof api.templates>>) => void }) {
  const [tab, setTab] = useState<ManagerTab>("installed");
  const [appliances, setAppliances] = useState<Gns3Appliance[]>([]);
  const [localImages, setLocalImages] = useState<LocalApplianceImage[]>([]);
  const [customImageOptions, setCustomImageOptions] = useState<RuntimeTemplate[]>([]);
  const [runtimeApplianceTemplates, setRuntimeApplianceTemplates] = useState<Array<{ id: string; name: string; type: string; applianceId: string; version: string }>>([]);
  const [selectedFile, setSelectedFile] = useState("");
  const [selectedVersion, setSelectedVersion] = useState("");
  const [customDialogOpen, setCustomDialogOpen] = useState(false);
  const [applianceFilter, setApplianceFilter] = useState("");
  const [registryQuery, setRegistryQuery] = useState("");
  const [registryResults, setRegistryResults] = useState<Gns3ApplianceSearchResult[]>([]);
  const [registryLoaded, setRegistryLoaded] = useState(false);
  const [openRuntimeGroups, setOpenRuntimeGroups] = useState<Record<string, boolean>>({});
  const [selectedRegistry, setSelectedRegistry] = useState<Gns3Appliance | null>(null);
  const [busy, setBusy] = useState(true);
  const [uploading, setUploading] = useState(false);
  const [imageLibraryOpen, setImageLibraryOpen] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");

  useEffect(() => {
    if (!error && !notice) return;
    const timeout = window.setTimeout(() => {
      setError("");
      setNotice("");
    }, 6000);
    return () => window.clearTimeout(timeout);
  }, [error, notice]);

  const selectedAppliance = appliances.find((appliance) => appliance.file_name === selectedFile) ?? null;
  const versions = selectedAppliance?.versions ?? [];
  const activeVersion = versions.find((version) => version.name === selectedVersion) ?? versions[0] ?? null;
  const activeRuntimeTemplate = selectedAppliance && activeVersion
    ? runtimeApplianceTemplates.find((template) => template.applianceId === selectedAppliance.appliance_id && (template.version === activeVersion.name || (!template.version && activeVersion.name === "Default")))
    : undefined;
  const filteredLibraryGroups = useMemo(() => {
    const query = applianceFilter.trim().toLocaleLowerCase();
    const entries: LibraryEntry[] = appliances.map((appliance): LibraryEntry => ({
        id: appliance.file_name,
        name: appliance.name,
        runtime: appliance.template_type ?? "other",
        appliance,
      })).filter((entry) => !query || `${entry.name} ${entry.runtime} ${entry.appliance.file_name}`.toLocaleLowerCase().includes(query));
    const knownRuntimes = ["dynamips", "qemu", "docker", "vpcs"];
    const groups = new Map<string, LibraryEntry[]>();
    for (const entry of entries) {
      const runtime = knownRuntimes.includes(entry.runtime) ? entry.runtime : "other";
      groups.set(runtime, [...(groups.get(runtime) ?? []), entry]);
    }
    return [...knownRuntimes, "other"].flatMap((runtime) => {
      const items = groups.get(runtime) ?? [];
      return items.length ? [{ runtime, entries: items }] : [];
    });
  }, [applianceFilter, appliances]);
  const readyCount = useMemo(() => appliances.reduce((count, appliance) => count + (appliance.versions ?? []).filter(versionIsReady).length, 0), [appliances]);

  async function refreshManager(selection: "keep" | { fileName?: string; versionName?: string } = "keep") {
    setBusy(true);
    setError("");
    try {
      const [state, templates] = await Promise.all([
        api.applianceManager(),
        api.templates().catch(() => []),
      ]);
      onTemplatesChanged?.(templates);
      const requestedFile = selection === "keep" ? selectedFile : selection.fileName ?? selectedFile;
      const requestedVersion = selection === "keep" ? selectedVersion : selection.versionName ?? selectedVersion;
      const nextAppliance = state.appliances.find((item) => item.file_name === requestedFile) ?? state.appliances[0];
      const nextVersion = nextAppliance?.versions?.some((version) => version.name === requestedVersion)
        ? requestedVersion
        : nextAppliance?.versions?.[0]?.name ?? "";
      setAppliances(state.appliances);
      setLocalImages(state.local_images);
      setCustomImageOptions(state.custom_image_options ?? []);
      setRuntimeApplianceTemplates(templates.flatMap((template) => {
        const applianceTemplate = template as typeof template & { appliance_id?: unknown; appliance_version?: unknown };
        return typeof applianceTemplate.appliance_id === "string" ? [{
          id: template.template_id,
          name: template.name,
          type: template.template_type,
          applianceId: applianceTemplate.appliance_id,
          version: typeof applianceTemplate.appliance_version === "string" ? applianceTemplate.appliance_version : "",
        }] : [];
      }));
      setSelectedFile(nextAppliance?.file_name ?? "");
      setSelectedVersion(nextVersion);
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  }

  useEffect(() => { void refreshManager({ fileName: "", versionName: "" }); }, []);

  useEffect(() => {
    if (tab !== "registry" || registryLoaded) return;
    let active = true;
    setBusy(true);
    setError("");
    api.searchGns3Appliances("")
      .then((results) => {
        if (!active) return;
        setRegistryResults(results);
        setRegistryLoaded(true);
      })
      .catch((reason: unknown) => {
        if (active) setError(errorMessage(reason));
      })
      .finally(() => { if (active) setBusy(false); });
    return () => { active = false; };
  }, [registryLoaded, tab]);

  async function searchRegistry(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setBusy(true);
    setError("");
    setSelectedRegistry(null);
    try {
      setRegistryResults(await api.searchGns3Appliances(registryQuery.trim()));
      setRegistryLoaded(true);
    } catch (reason) {
      setError(errorMessage(reason));
      setRegistryResults([]);
    } finally {
      setBusy(false);
    }
  }

  async function inspectRegistry(item: Gns3ApplianceSearchResult) {
    setBusy(true);
    setError("");
    try { setSelectedRegistry(await api.gns3Appliance(item.file_name)); }
    catch (reason) { setError(errorMessage(reason)); }
    finally { setBusy(false); }
  }

  async function importRegistryDefinition() {
    if (!selectedRegistry) return;
    setBusy(true);
    setError("");
    try {
      await api.importGns3Appliance(selectedRegistry.file_name);
      await refreshManager({ fileName: selectedRegistry.file_name, versionName: selectedRegistry.versions?.[0]?.name });
      setTab("installed");
      setNotice(`${selectedRegistry.name} is now in the appliance library. Assign or verify its local image below.`);
    } catch (reason) { setError(errorMessage(reason)); }
    finally { setBusy(false); }
  }

  async function importLocalDefinition(file?: File) {
    if (!file) return;
    setUploading(true);
    setError("");
    setNotice("");
    try {
      const appliance = await api.importGns3ApplianceFile(file);
      await refreshManager({ fileName: appliance.file_name, versionName: appliance.versions?.[0]?.name });
      setTab("installed");
      setNotice(`${appliance.name} was imported from ${file.name}.`);
    } catch (reason) { setError(errorMessage(reason)); }
    finally { setUploading(false); }
  }

  async function uploadImage(kind: "qemu" | "dynamips", file?: File) {
    if (!file) return;
    setUploading(true);
    setError("");
    setNotice("");
    try {
      if (kind === "qemu") await api.uploadQemuImage(file);
      else await api.uploadDynamipsImage(file);
      await refreshManager();
      setNotice(`${file.name} was validated and added to the global ${kind === "qemu" ? "QEMU" : "Dynamips"} image library.`);
    } catch (reason) { setError(errorMessage(reason)); }
    finally { setUploading(false); }
  }

  async function pullDockerImage(imageRef: string) {
    setBusy(true);
    setError("");
    try {
      await api.pullDockerImage(imageRef);
      await refreshManager();
      setNotice(`${imageRef} is available as a Docker template.`);
    } catch (reason) { setError(errorMessage(reason)); }
    finally { setBusy(false); }
  }

  async function saveBinding(version: ApplianceVersion, requirement: ApplianceImageRequirement, image: LocalApplianceImage, allowMismatch: boolean) {
    if (!selectedAppliance) return;
    setBusy(true);
    setError("");
    try {
      await api.setApplianceImageBinding(selectedAppliance.file_name, {
        version: version.name,
        image_key: requirement.key,
        image_id: image.image_id,
        allow_checksum_mismatch: allowMismatch,
      });
      await refreshManager({ fileName: selectedAppliance.file_name, versionName: version.name });
      setNotice(`${image.name} is assigned to ${selectedAppliance.name} · ${version.name} (${requirement.key}).`);
    } catch (reason) { setError(errorMessage(reason)); }
    finally { setBusy(false); }
  }

  async function clearBinding(version: ApplianceVersion, requirement: ApplianceImageRequirement) {
    if (!selectedAppliance) return;
    setBusy(true);
    setError("");
    try {
      await api.clearApplianceImageBinding(selectedAppliance.file_name, { version: version.name, image_key: requirement.key });
      await refreshManager({ fileName: selectedAppliance.file_name, versionName: version.name });
      setNotice(`The image assignment for ${version.name} · ${requirement.key} was cleared.`);
    } catch (reason) { setError(errorMessage(reason)); }
    finally { setBusy(false); }
  }

  async function customApplianceCreated(appliance: Gns3Appliance) {
    await refreshManager({ fileName: appliance.file_name, versionName: appliance.versions?.[0]?.name });
    setNotice(`${appliance.name} was created in the appliance library and is available to projects when its runtime is ready.`);
  }

  return <main className="appliance-manager-page">
    <header className="appliance-manager-header">
      <div className="flex min-w-0 items-center gap-3">
        <Button variant="outline" size="icon" onClick={onBack} aria-label="Back"><ArrowLeft /></Button>
        <div className="min-w-0"><h1>Appliance library</h1><p>GNS3 imports and appliances you create are the only device definitions shared with projects.</p></div>
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <span className="text-xs text-muted-foreground">{appliances.length} appliances · {readyCount} ready configurations</span>
        {onOpenPreferences && <Button variant="outline" onClick={onOpenPreferences}>Preferences</Button>}
        <Button onClick={() => setCustomDialogOpen(true)}><Plus />Create custom appliance</Button>
        <CreateCustomApplianceDialog open={customDialogOpen} onOpenChange={setCustomDialogOpen} imageOptions={customImageOptions} onCreated={customApplianceCreated} />
        <Dialog open={imageLibraryOpen} onOpenChange={setImageLibraryOpen}>
          <DialogTrigger asChild><Button variant="outline"><HardDrive />Global images<Badge variant="secondary">{localImages.length}</Badge></Button></DialogTrigger>
          <DialogContent className="appliance-image-library-dialog">
            <DialogHeader>
              <DialogTitle>Global image library</DialogTitle>
              <DialogDescription>Images are stored once and shared by all appliances and projects.</DialogDescription>
            </DialogHeader>
            <div className="appliance-image-library-content">
              <div className="grid content-start gap-2">
                <FilePickerButton icon={<Upload />} label={uploading ? "Validating image…" : "Add QEMU image"} accept=".qcow2,.qcow,.raw,.img,.vmdk,.vdi" disabled={uploading} onSelect={(file) => uploadImage("qemu", file)} />
                <FilePickerButton icon={<Upload />} label={uploading ? "Validating image…" : "Add Dynamips IOS image"} accept=".bin,.image,.img,.elf" disabled={uploading} onSelect={(file) => uploadImage("dynamips", file)} />
              </div>
              <ScrollArea className="appliance-local-image-scroll"><div className="appliance-local-image-table">
                {localImages.map((image) => <div key={image.image_id} className="appliance-local-image-row"><span className="grid min-w-0 gap-0.5"><b className="truncate" title={image.name}>{image.name}</b><small className="truncate text-muted-foreground" title={image.path}>{image.path}</small></span><Badge variant="outline">{image.template_type.toUpperCase()}</Badge><small className="text-muted-foreground">{formatBytes(image.size)}</small></div>)}
                {!localImages.length && <EmptyState icon={<HardDrive />} title="No images in the global library" text="Add a QEMU disk or Dynamips IOS image. Appliance definitions will then match files by name, MD5, and size." />}
              </div></ScrollArea>
            </div>
          </DialogContent>
        </Dialog>
        {onOpenImageCatalog && <Button variant="outline" onClick={onOpenImageCatalog}><HardDrive />Image catalog</Button>}
        <Button variant="outline" onClick={() => void refreshManager()} disabled={busy}><RefreshCw className={busy ? "animate-spin" : ""} />Refresh library</Button>
      </div>
    </header>

    {(error || notice) && <div className="appliance-toast-viewport" aria-live="polite" aria-atomic="true">
      <div className={`appliance-toast ${error ? "is-error" : ""}`} role={error ? "alert" : "status"}>
        {error ? <span>{error}</span> : <><Check aria-hidden="true" /><span>{notice}</span></>}
        <Button variant="ghost" size="icon-xs" onClick={() => { setError(""); setNotice(""); }} aria-label="Dismiss notification"><X /></Button>
      </div>
    </div>}

    <Tabs value={tab} onValueChange={(value) => setTab(value as ManagerTab)} className="appliance-manager-tabs">
      <TabsList><TabsTrigger value="installed"><Server />Appliance library</TabsTrigger><TabsTrigger value="registry"><Search />GNS3 registry <Badge variant="secondary" className="ml-1">{registryResults.length}</Badge></TabsTrigger></TabsList>
      <TabsContent value="installed" className="appliance-manager-panel">
        <div className="appliance-manager-grid">
          <Card className="appliance-library-list">
            <div className="appliance-library-heading"><div><h2>Appliances</h2><p>Only imported GNS3 definitions and custom appliances appear here.</p></div><Badge variant="outline">{appliances.length}</Badge></div>
            <div className="appliance-library-filter"><Search aria-hidden="true" /><Input value={applianceFilter} onChange={(event) => setApplianceFilter(event.target.value)} aria-label="Filter installed appliances" placeholder="Filter appliances" /></div>
            <ScrollArea className="appliance-library-scroll"><div className="space-y-2 p-2">
              {filteredLibraryGroups.map(({ runtime, entries }) => <Collapsible key={runtime} open={openRuntimeGroups[runtime] ?? true} onOpenChange={(open) => setOpenRuntimeGroups((current) => ({ ...current, [runtime]: open }))}>
                <CollapsibleTrigger className="appliance-runtime-group-trigger"><span>{applianceRuntimeLabel(runtime)}</span><Badge variant="secondary">{entries.length}</Badge><ChevronDown className="ml-auto size-4" /></CollapsibleTrigger>
                <CollapsibleContent className="space-y-1 pt-1">
                  {entries.map((entry) => <Button key={entry.id} type="button" variant="ghost" className={`appliance-library-item ${selectedFile === entry.appliance.file_name ? "selected" : ""}`} aria-current={selectedFile === entry.appliance.file_name ? "true" : undefined} onClick={() => { setSelectedFile(entry.appliance.file_name); setSelectedVersion(entry.appliance.versions?.[0]?.name ?? ""); setNotice(""); }}>
                    <span className="grid size-9 shrink-0 place-items-center rounded-md border text-primary"><Server size={17} /></span>
                    <span className="grid min-w-0 flex-1 gap-1 text-left"><b className="truncate text-sm font-medium">{entry.appliance.name}</b><small className="truncate text-muted-foreground">{entry.appliance.file_name}</small></span>
                    <Badge variant={entry.appliance.origin === "custom" ? "secondary" : "outline"}>{entry.appliance.origin === "custom" ? "Custom" : "GNS3"}</Badge>
                    <Badge variant={entry.appliance.versions?.some(versionIsReady) ? "secondary" : "outline"}>{entry.appliance.versions?.length ?? 0} config{entry.appliance.versions?.length === 1 ? "" : "s"}</Badge>
                  </Button>)}
                </CollapsibleContent>
              </Collapsible>)}
              {!busy && filteredLibraryGroups.length === 0 && <EmptyState icon={<Server />} title={appliances.length ? "No matching appliances" : "No appliances in the library"} text={appliances.length ? "Try a different name or runtime filter." : "Import a GNS3 appliance or create a custom appliance from an installed image."} />}
            </div></ScrollArea>
            <Separator />
            <div className="grid gap-2 p-3">
              <FilePickerButton icon={<FileUp />} label={uploading ? "Importing definition…" : "Import .gns3a file"} accept=".gns3a,application/json" disabled={uploading} onSelect={importLocalDefinition} />
              <Button variant="outline" onClick={() => setTab("registry")}><Search />Find an appliance in GNS3 registry</Button>
            </div>
          </Card>

          <Card className="appliance-config-panel">
            {selectedAppliance && activeVersion ? <>
              <div className="appliance-panel-heading"><div className="min-w-0"><div className="flex flex-wrap items-center gap-2"><Badge variant="outline">{selectedAppliance.template_type ?? "Runtime not detected"}</Badge><Badge variant={selectedAppliance.origin === "custom" ? "secondary" : "outline"}>{selectedAppliance.origin === "custom" ? "Custom appliance" : "GNS3 import"}</Badge><Badge variant={activeVersion.runtime_supported ? "secondary" : "destructive"}>{activeVersion.runtime_supported ? "Runtime supported" : "Runtime unsupported"}</Badge></div><h2>{selectedAppliance.name}</h2><p>{selectedAppliance.description || "No appliance description is provided."}</p></div></div>
              <div className="appliance-config-meta"><span>Definition</span><b>{selectedAppliance.file_name}</b><span>Configuration</span><Select value={activeVersion.name} onValueChange={setSelectedVersion}><SelectTrigger aria-label="Appliance configuration" className="w-full"><SelectValue /></SelectTrigger><SelectContent position="popper" align="start" className="appliance-select-content">{versions.map((version) => <SelectItem key={version.name} value={version.name}>{version.name}{version.settings ? ` · ${version.settings}` : ""}</SelectItem>)}</SelectContent></Select><span>Runtime settings</span><b>{activeVersion.template_type ?? "Not detected"}{activeVersion.settings ? ` · ${activeVersion.settings}` : ""}</b></div>
              {activeVersion.template_type === "docker" && activeVersion.docker_image && <Card className="appliance-docker-image" role="group" aria-label="Docker image for this appliance configuration">
                <div className="min-w-0"><div className="mb-1 flex flex-wrap items-center gap-2"><Badge variant={activeVersion.docker_image_available ? "secondary" : "outline"}>{activeVersion.docker_image_available ? "Image available" : "Image not installed"}</Badge>{activeVersion.docker_image_id && <code className="text-xs text-muted-foreground">{activeVersion.docker_image_id.slice(0, 19)}</code>}</div><b className="break-all font-mono text-sm">{activeVersion.docker_image}</b><p className="mt-1 text-xs text-muted-foreground">GNS3 Docker appliances use an image reference from the selected template settings. Pulling it creates a runnable template in the device palette.</p></div>
                <Button variant={activeVersion.docker_image_available ? "outline" : "default"} disabled={busy} onClick={() => void pullDockerImage(activeVersion.docker_image!)}><Download />{activeVersion.docker_image_available ? "Pull latest" : "Pull image"}</Button>
              </Card>}
              {!activeVersion.runtime_supported && <Card className="appliance-runtime-warning" role="note">This configuration describes {activeVersion.template_type ?? "an unsupported runtime"}. The current controller cannot create a runnable template for this platform yet. Image assignments are still saved here, but this config will not appear in the device palette until its runtime is implemented.</Card>}
              {activeRuntimeTemplate ? <Card className="appliance-template-ready" role="status"><Badge variant="secondary">Ready in device picker</Badge><div><b>{activeRuntimeTemplate.name}</b><span>{activeRuntimeTemplate.type.toUpperCase()} template · Uses the global image assigned to this configuration.</span><small>Open a project and choose this appliance from Add device.</small></div></Card> : activeVersion.runtime_supported && activeVersion.images.length > 0 && <p className="appliance-template-pending" role="note">This configuration appears in the project device picker after its required image slots are mapped and verified below.</p>}
              <div className="appliance-images-heading"><div><h3>Images required by this configuration</h3><p>GNS3 stores these image slots inside each version. Verify the registry MD5 and size before using a local file.</p></div><Badge variant="outline">{activeVersion.images.length} image{activeVersion.images.length === 1 ? "" : "s"}</Badge></div>
              <ScrollArea className="appliance-image-list"><div className="space-y-3 p-4">
                {activeVersion.images.map((requirement) => <ImageAssignmentCard key={`${activeVersion.name}:${requirement.key}`} version={activeVersion} requirement={requirement} localImages={localImages.filter((image) => image.template_type === activeVersion.template_type)} busy={busy || uploading} onSave={saveBinding} onClear={clearBinding} />)}
                {activeVersion.images.length === 0 && <EmptyState icon={<HardDrive />} title="No separate disk image listed" text="This appliance config does not define a local image slot. Docker images are managed by their configured image tag." />}
              </div></ScrollArea>
            </> : <EmptyState icon={<Server />} title={busy ? "Loading appliance library…" : "Select an appliance"} text="Imported GNS3 definitions and custom appliance configurations appear here." />}
          </Card>
        </div>
      </TabsContent>

      <TabsContent value="registry" className="appliance-manager-panel">
        <div className="appliance-registry-grid">
          <Card className="appliance-library-list">
            <form className="flex gap-2 border-b p-3" onSubmit={(event) => void searchRegistry(event)}><div className="relative min-w-0 flex-1"><Search className="pointer-events-none absolute left-3 top-2.5 size-4 text-muted-foreground" /><Input value={registryQuery} onChange={(event) => setRegistryQuery(event.target.value)} aria-label="Search GNS3 registry" placeholder="Search devices, e.g. Cisco 7200" className="pl-9" /></div><Button type="submit" disabled={busy}><Search />Search</Button></form>
            <ScrollArea className="appliance-library-scroll"><div className="space-y-1 p-2">{registryResults.map((item) => <button key={item.file_name} type="button" className={`appliance-library-item ${selectedRegistry?.file_name === item.file_name ? "selected" : ""}`} onClick={() => void inspectRegistry(item)}><span className="grid size-9 shrink-0 place-items-center rounded-md border text-primary"><Server size={17} /></span><span className="grid min-w-0 flex-1 gap-1 text-left"><b className="truncate text-sm font-medium">{item.name}</b><small className="truncate text-muted-foreground">{item.file_name}</small></span><Badge variant={item.imported ? "secondary" : "outline"}>{item.imported ? "Imported" : "GNS3"}</Badge></button>)}{!busy && registryResults.length === 0 && <EmptyState icon={<Search />} title={registryLoaded ? "No registry entries found" : "Loading the GNS3 registry"} text={registryLoaded ? "Try a different name or runtime keyword." : "The appliance catalog is being loaded."} />}</div></ScrollArea>
          </Card>
          <Card className="appliance-config-panel">{selectedRegistry ? <RegistryDetails appliance={selectedRegistry} busy={busy} onImport={() => void importRegistryDefinition()} /> : <EmptyState icon={<FileUp />} title="GNS3 appliance definition" text="Registry results show the actual .gns3a definition, versions, required files, and hashes." />}</Card>
        </div>
      </TabsContent>
    </Tabs>
  </main>;
}

function CreateCustomApplianceDialog({ open, onOpenChange, imageOptions, onCreated }: {
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

function runtimeOptionLabel(template: RuntimeTemplate): string {
  if (template.template_type === "docker") return template.image;
  if (template.template_type === "qemu") return template.hda_disk_image.split("/").at(-1) ?? template.hda_disk_image;
  if (template.template_type === "dynamips") return template.image.split("/").at(-1) ?? template.image;
  return template.name;
}

function ImageAssignmentCard({ version, requirement, localImages, busy, onSave, onClear }: { version: ApplianceVersion; requirement: ApplianceImageRequirement; localImages: LocalApplianceImage[]; busy: boolean; onSave: (version: ApplianceVersion, requirement: ApplianceImageRequirement, image: LocalApplianceImage, allowMismatch: boolean) => void; onClear: (version: ApplianceVersion, requirement: ApplianceImageRequirement) => void }) {
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

function RegistryDetails({ appliance, busy, onImport }: { appliance: Gns3Appliance; busy: boolean; onImport: () => void }) {
  return <div className="registry-details"><div><Badge variant="outline">GNS3 appliance</Badge><h2>{appliance.name}</h2><p>{appliance.description || "No description is provided."}</p></div><div className="appliance-config-meta"><span>Category</span><b>{appliance.category ?? "Uncategorized"}</b><span>Runtime</span><b>{appliance.template_type ?? "Registry metadata only"}</b><span>Definition</span><b>{appliance.file_name}</b><span>Configs</span><b>{appliance.versions?.length ?? 0}</b></div><h3>Versions and image requirements</h3><ScrollArea className="registry-version-list"><div className="space-y-2 pr-3">{appliance.versions?.map((version) => <Card key={version.name} className="p-3 shadow-none"><div className="mb-2 flex flex-wrap items-center gap-2"><b>{version.name}</b><Badge variant="outline">{version.template_type ?? "Unknown runtime"}</Badge><Badge variant={version.runtime_supported ? "secondary" : "destructive"}>{version.runtime_supported ? "Supported" : "Unsupported"}</Badge></div>{version.docker_image && <div className="registry-requirement"><code>Docker image</code><b className="break-all">{version.docker_image}</b><small>Pull this image after importing the definition to create its project template.</small></div>}{version.images.length ? version.images.map((image) => <div key={image.key} className="registry-requirement"><code>{image.key}</code><b className="break-all">{image.filename}</b><small>MD5 {image.md5sum ?? "not published"}{image.filesize == null ? "" : ` · ${formatBytes(image.filesize)}`}</small></div>) : !version.docker_image && <p className="text-xs text-muted-foreground">No separate local image slots.</p>}</Card>)}{!appliance.versions?.length && <p className="text-sm text-muted-foreground">No version configurations are included in this definition.</p>}</div></ScrollArea><div className="flex justify-end"><Button onClick={onImport} disabled={busy || appliance.imported}>{appliance.imported ? <><Check />Already imported</> : <><FileUp />Import this appliance</>}</Button></div></div>;
}

function EmptyState({ icon, title, text }: { icon: ReactNode; title: string; text: string }) {
  return <div className="appliance-empty-state">{icon}<b>{title}</b><span>{text}</span></div>;
}

function applianceRuntimeLabel(runtime: string): string {
  return ({ dynamips: "Dynamips · Cisco IOS", qemu: "QEMU / KVM", docker: "Docker", vpcs: "VPCS", other: "Other runtimes" } as Record<string, string>)[runtime] ?? runtime;
}

function FilePickerButton({ icon, label, accept, disabled, onSelect }: { icon: ReactNode; label: string; accept: string; disabled: boolean; onSelect: (file: File) => void | Promise<void> }) {
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

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) { value /= 1024; unit += 1; }
  return `${value.toFixed(value >= 10 ? 0 : 1)} ${units[unit]}`;
}

function statusLabel(status: string): { label: string; variant: "default" | "secondary" | "destructive" | "outline" } {
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

function versionIsReady(version: ApplianceVersion): boolean {
  if (!version.runtime_supported) return false;
  if (version.template_type === "vpcs") return true;
  if (version.template_type === "docker") return version.docker_image_available === true;
  return version.images.length > 0 && version.images.every((image) => ["verified", "size-verified", "override", "available-unverified"].includes(image.status));
}

function errorMessage(reason: unknown): string {
  return reason instanceof Error ? reason.message : "Appliance manager request failed. Please try again.";
}
