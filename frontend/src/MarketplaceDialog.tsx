import { useState } from "react";
import type { FormEvent } from "react";
import { ArrowUpRight, Download, FileBox, HardDrive, Search, Server, X } from "lucide-react";
import { api } from "./api";
import type {
  Gns3Appliance,
  Gns3ApplianceImportResult,
  Gns3ApplianceSearchResult,
  IshareImage,
  IshareDownloadResult,
  RuntimeTemplate,
} from "./api";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Separator } from "@/components/ui/separator";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";

type Catalog = "gns3" | "ishare2";

export function MarketplaceDialog({ onClose }: { onClose: () => void }) {
  const [catalog, setCatalog] = useState<Catalog>("gns3");
  const [query, setQuery] = useState("");
  const [imageType, setImageType] = useState("all");
  const [appliances, setAppliances] = useState<Gns3ApplianceSearchResult[]>([]);
  const [images, setImages] = useState<IshareImage[]>([]);
  const [selectedAppliance, setSelectedAppliance] = useState<Gns3Appliance | null>(null);
  const [localTemplates, setLocalTemplates] = useState<RuntimeTemplate[]>([]);
  const [imageSuggestions, setImageSuggestions] = useState<Record<string, IshareImage[]>>({});
  const [suggestionsLoading, setSuggestionsLoading] = useState(false);
  const [importResult, setImportResult] = useState<Gns3ApplianceImportResult | null>(null);
  const [downloadResult, setDownloadResult] = useState<IshareDownloadResult | null>(null);
  const [downloadedRequirement, setDownloadedRequirement] = useState<string | null>(null);
  const [downloading, setDownloading] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [importing, setImporting] = useState(false);
  const [error, setError] = useState("");

  async function search(event?: FormEvent<HTMLFormElement>) {
    event?.preventDefault();
    setLoading(true);
    setError("");
    setSelectedAppliance(null);
    setLocalTemplates([]);
    setImageSuggestions({});
    setImportResult(null);
    setDownloadResult(null);
    setDownloadedRequirement(null);
    try {
      if (catalog === "gns3") setAppliances(await api.searchGns3Appliances(query.trim()));
      else setImages(await api.searchIshareImages(query.trim(), imageType));
    } catch (reason) {
      setError(errorMessage(reason));
      if (catalog === "gns3") setAppliances([]);
      else setImages([]);
    } finally {
      setLoading(false);
    }
  }

  async function inspect(appliance: Gns3ApplianceSearchResult) {
    setLoading(true);
    setError("");
    setImportResult(null);
    setImageSuggestions({});
    setSuggestionsLoading(true);
    try {
      const detail = await api.gns3Appliance(appliance.file_name);
      setSelectedAppliance(detail);
      const installed = await api.templates().catch(() => [] as RuntimeTemplate[]);
      setLocalTemplates(installed);
      const kind = detail.template_type === "qemu" || detail.template_type === "dynamips"
        ? detail.template_type
        : "all";
      const results = await Promise.all(detail.required_files.map(async (file) => {
        try {
          let matches = await api.searchIshareImages(file.filename, kind);
          if (matches.length === 0) {
            const fallback = [detail.name, file.version].filter(Boolean).join(" ");
            if (fallback) matches = await api.searchIshareImages(fallback, kind);
          }
          return [file.filename, matches] as const;
        } catch {
          return [file.filename, []] as const;
        }
      }));
      setImageSuggestions(Object.fromEntries(results));
    }
    catch (reason) { setError(errorMessage(reason)); }
    finally { setLoading(false); setSuggestionsLoading(false); }
  }

  async function importDefinition() {
    if (!selectedAppliance) return;
    setImporting(true);
    setError("");
    try {
      const result = await api.importGns3Appliance(selectedAppliance.file_name);
      setSelectedAppliance(result.appliance);
      setImportResult(result);
      setAppliances((current) => current.map((item) => item.file_name === result.appliance.file_name ? { ...item, imported: true } : item));
    } catch (reason) { setError(errorMessage(reason)); }
    finally { setImporting(false); }
  }

  async function downloadImage(image: IshareImage, requiredFilename?: string) {
    const key = `${image.image_type}:${image.id}`;
    setDownloading(key);
    setError("");
    setDownloadResult(null);
    setDownloadedRequirement(null);
    try {
      setDownloadResult(await api.downloadIshareImage(image));
      setDownloadedRequirement(requiredFilename ?? null);
      const installed = await api.templates().catch(() => [] as RuntimeTemplate[]);
      setLocalTemplates(installed);
    }
    catch (reason) { setError(errorMessage(reason)); }
    finally { setDownloading(null); }
  }

  function changeCatalog(next: Catalog) {
    setCatalog(next);
    setError("");
    setAppliances([]);
    setImages([]);
    setSelectedAppliance(null);
    setImportResult(null);
    setDownloadResult(null);
    setDownloadedRequirement(null);
  }

  return <Dialog open onOpenChange={(open) => { if (!open) onClose(); }}>
    <DialogContent className="flex h-[88dvh] max-h-[800px] w-[calc(100vw-2rem)] max-w-[1060px] flex-col gap-0 overflow-hidden p-0 sm:max-w-[1060px]">
      <DialogHeader className="px-5 pb-4 pt-5 pr-12">
        <DialogTitle>Image and appliance catalog</DialogTitle>
        <DialogDescription>Import GNS3 appliance definitions or download supported iShare2 images into the global library.</DialogDescription>
      </DialogHeader>
      <Tabs value={catalog} onValueChange={(value) => changeCatalog(value as Catalog)} className="flex min-h-0 flex-1 flex-col">
        <div className="flex flex-col items-stretch gap-3 border-y px-5 py-2.5 sm:flex-row sm:flex-wrap sm:items-center sm:justify-between">
          <TabsList><TabsTrigger value="gns3"><FileBox />GNS3 appliances</TabsTrigger><TabsTrigger value="ishare2"><HardDrive />iShare2 images</TabsTrigger></TabsList>
          <form className="flex w-full min-w-0 flex-1 justify-end gap-2 sm:w-auto" onSubmit={(event) => void search(event)}>
            <div className="relative w-full min-w-0 max-w-md"><Search className="pointer-events-none absolute left-3 top-2.5 size-4 text-muted-foreground" /><Input value={query} onChange={(event) => setQuery(event.target.value)} placeholder={catalog === "gns3" ? "Search appliances, e.g. Alpine or Cisco" : "Search image metadata, e.g. alpine or c7200"} aria-label="Search catalog" className="pl-9" /></div>
            {catalog === "ishare2" && <Select value={imageType} onValueChange={setImageType}><SelectTrigger className="w-32" aria-label="iShare2 image type"><SelectValue /></SelectTrigger><SelectContent><SelectItem value="all">All types</SelectItem><SelectItem value="qemu">QEMU</SelectItem><SelectItem value="dynamips">Dynamips</SelectItem><SelectItem value="iol">IOL</SelectItem></SelectContent></Select>}
            <Button type="submit" disabled={loading}><Search />{loading ? "Searching" : "Search"}</Button>
          </form>
        </div>
        {error && <Card className="mx-5 mt-3 flex items-center justify-between gap-3 border-destructive/40 p-3 text-sm text-destructive" role="alert"><span>{error}</span><Button variant="ghost" size="icon-sm" onClick={() => setError("")} aria-label="Dismiss catalog error"><X /></Button></Card>}
        <TabsContent value="gns3" className="mt-0 min-h-0 flex-1 overflow-hidden">
          <div className="grid h-full min-h-0 grid-cols-1 grid-rows-[minmax(0,0.85fr)_minmax(0,1.15fr)] md:grid-cols-[minmax(0,0.85fr)_minmax(0,1.15fr)] md:grid-rows-1">
            <ScrollArea className="h-full min-h-0 min-w-0 border-b md:border-r md:border-b-0">
              <div className="space-y-1 p-3" aria-label="GNS3 appliance results">
                {appliances.map((item) => <Button key={item.file_name} variant={selectedAppliance?.file_name === item.file_name ? "secondary" : "ghost"} className="h-auto min-h-14 w-full justify-start gap-3 px-2 text-left" onClick={() => void inspect(item)}>
                  <span className="grid size-9 shrink-0 place-items-center rounded-md border text-primary"><FileBox size={17} /></span><span className="grid min-w-0 flex-1 gap-1"><b className="truncate text-xs font-medium">{item.name}</b><small className="truncate text-[10px] text-muted-foreground">{item.file_name}</small></span><Badge variant="outline">{item.imported ? "Imported" : ".gns3a"}</Badge>
                </Button>)}
                {!loading && appliances.length === 0 && <EmptySearch text="Search the GNS3 appliance registry for installable definitions." />}
                {loading && <p className="py-5 text-center text-xs text-muted-foreground">Searching GNS3 catalog…</p>}
              </div>
            </ScrollArea>
            <ScrollArea className="h-full min-h-0 min-w-0"><div className="p-5">
              {selectedAppliance ? <ApplianceDetails appliance={selectedAppliance} localTemplates={localTemplates} suggestions={imageSuggestions} suggestionsLoading={suggestionsLoading} downloading={downloading} downloadResult={downloadResult} downloadedRequirement={downloadedRequirement} importing={importing} importResult={importResult} onDownload={(image, filename) => void downloadImage(image, filename)} onImport={() => void importDefinition()} /> : <EmptySearch text="Choose an appliance to inspect its runtime, image files, and import status." />}
            </div></ScrollArea>
          </div>
        </TabsContent>
        <TabsContent value="ishare2" className="mt-0 min-h-0 flex-1 overflow-hidden">
          <div className="grid h-full min-h-0 grid-cols-1 grid-rows-[minmax(0,0.85fr)_minmax(0,1.15fr)] md:grid-cols-[minmax(0,0.85fr)_minmax(0,1.15fr)] md:grid-rows-1">
            <ScrollArea className="h-full min-h-0 min-w-0 border-b md:border-r md:border-b-0">
              <div className="space-y-1 p-3" aria-label="iShare2 image results">
                {images.map((image) => <Card key={`${image.image_type}:${image.id}`} className="flex items-center gap-3 p-2 shadow-none">
                  <span className="grid size-9 shrink-0 place-items-center rounded-md border text-primary"><HardDrive size={16} /></span><span className="grid min-w-0 flex-1 gap-1"><b className="truncate text-xs font-medium">{image.name}</b><small className="truncate text-[10px] text-muted-foreground" title={image.files?.join(", ")}>{image.image_type.toUpperCase()} · {image.file_count} file{image.file_count === 1 ? "" : "s"} · {image.size}{image.files?.length ? ` · ${image.files.slice(0, 2).join(", ")}` : ""}</small></span>
                  {image.image_type === "qemu" || image.image_type === "dynamips" ? <Button size="sm" variant="outline" disabled={downloading !== null} onClick={() => void downloadImage(image)}><Download />{downloading === `${image.image_type}:${image.id}` ? "Downloading" : "Download"}</Button> : <Badge variant="secondary">Search only</Badge>}
                </Card>)}
                {!loading && images.length === 0 && <EmptySearch text="Search the iShare2 / LabHub index for image metadata." />}
                {loading && <p className="py-5 text-center text-xs text-muted-foreground">Searching iShare2 catalog…</p>}
              </div>
            </ScrollArea>
            <ScrollArea className="h-full min-h-0 min-w-0"><div className="space-y-4 p-5">
              <div><Badge variant="outline">iShare2 / LabHub</Badge><h3 className="mt-3 text-base font-semibold">Global image downloads</h3><p className="mt-2 text-sm leading-relaxed text-muted-foreground">Supported QEMU disks and Dynamips IOS images are downloaded into this controller’s global image library and validated locally.</p></div>
              <Card className="p-3 text-xs leading-relaxed text-muted-foreground shadow-none">Only download images you are licensed to use. IOL search is catalog metadata only; this simulator does not run IOL images. GNS3 appliance definitions remain separate from vendor disk files.</Card>
              {downloadResult && <Card className="p-3 text-xs leading-relaxed" role="status"><b>Download complete: {downloadResult.name}</b><p className="mt-1">Files: {downloadResult.files.join(", ")}</p><p>Saved to {downloadResult.image_directory}. Reopen Add device to use the new image.</p></Card>}
              <a className="inline-flex items-center gap-1 text-xs text-primary underline-offset-4 hover:underline" href="https://github.com/ishare2-org/ishare2-cli" target="_blank" rel="noreferrer">iShare2 CLI documentation <ArrowUpRight size={13} /></a>
            </div></ScrollArea>
          </div>
        </TabsContent>
      </Tabs>
      <Separator />
      <footer className="flex min-h-12 items-center justify-between gap-3 px-5 py-2 text-[11px] text-muted-foreground"><span>GNS3 appliance definitions use <code>.gns3a</code>; <code>.gns3</code> is a project file.</span><Button variant="outline" size="sm" onClick={onClose}>Done</Button></footer>
    </DialogContent>
  </Dialog>;
}

function ApplianceDetails({ appliance, localTemplates, suggestions, suggestionsLoading, downloading, downloadResult, downloadedRequirement, importing, importResult, onDownload, onImport }: { appliance: Gns3Appliance; localTemplates: RuntimeTemplate[]; suggestions: Record<string, IshareImage[]>; suggestionsLoading: boolean; downloading: string | null; downloadResult: IshareDownloadResult | null; downloadedRequirement: string | null; importing: boolean; importResult: Gns3ApplianceImportResult | null; onDownload: (image: IshareImage, filename: string) => void; onImport: () => void }) {
  const supportedRuntime = appliance.template_type === "qemu" || appliance.template_type === "dynamips";
  return <div className="space-y-4">
    <div><Badge variant="outline">GNS3 appliance</Badge><h3 className="mt-2 text-lg font-semibold">{appliance.name}</h3>{appliance.description && <p className="mt-2 text-sm leading-relaxed text-muted-foreground">{appliance.description}</p>}</div>
    <Card className="grid grid-cols-[86px_1fr] gap-x-3 gap-y-2 p-3 text-xs shadow-none"><span className="text-muted-foreground">Runtime</span><b>{appliance.template_type ?? "Registry metadata"}</b><span className="text-muted-foreground">Category</span><b>{appliance.category ?? "Uncategorized"}</b><span className="text-muted-foreground">Definition</span><b className="break-all">{appliance.file_name}</b></Card>
    <div><h4 className="mb-2 text-sm font-medium">Required images and local availability</h4>{appliance.required_files.length ? <div className="space-y-2">{appliance.required_files.map((file) => {
      const local = supportedRuntime ? localTemplates.filter((template) => template.template_type === appliance.template_type && templateImageName(template).some((name) => imageNameMatches(name, file.filename))) : [];
      const matches = suggestions[file.filename] ?? [];
      const downloadedMatch = downloadedRequirement === file.filename && (downloadResult?.source_files ?? []).some((name) => imageNameMatches(name, file.filename));
      const downloadedSuggestion = downloadedRequirement === file.filename && downloadResult !== null;
      return <Card key={`${file.filename}:${file.version ?? ""}`} className="space-y-2 p-3 shadow-none">
        <div className="flex flex-wrap items-start justify-between gap-2"><div className="min-w-0 flex-1"><b className="break-all font-mono text-xs">{file.filename}</b>{file.size != null && <span className="ml-2 text-[11px] text-muted-foreground">{formatBytes(file.size)}</span>}{file.version && <p className="mt-1 text-[11px] text-muted-foreground">Version {file.version}</p>}</div><Badge variant={local.length || downloadedSuggestion ? "secondary" : "outline"}>{local.length ? "Available locally" : downloadedMatch ? "Required file downloaded" : downloadedSuggestion ? "Suggestion downloaded" : "Missing locally"}</Badge></div>
        {local.length > 0 && <p className="break-all text-[11px] text-muted-foreground">Using {local.map((template) => template.name).join(", ")}</p>}
        {downloadedSuggestion && downloadResult && <p className="break-all text-[11px] text-muted-foreground">Saved as {downloadResult.files.join(", ")} in {downloadResult.image_directory}.{local.length ? " The runtime template is ready to use." : downloadedMatch ? " The downloaded package contains the required filename, but no runtime template is available yet." : " Check the downloaded version and filename against this appliance requirement."}</p>}
        {!local.length && supportedRuntime && <div className="space-y-2 border-t pt-2"><p className="text-[11px] font-medium">iShare2 suggestions {suggestionsLoading && <span className="font-normal text-muted-foreground">· searching…</span>}</p>{matches.length ? matches.slice(0, 3).map((image) => {
          const wasDownloaded = downloadedRequirement === file.filename && downloadResult?.id === image.id;
          const matchingFiles = image.files?.filter((name) => imageNameMatches(name, file.filename)) ?? [];
          return <div key={`${image.image_type}:${image.id}`} className="flex flex-wrap items-center gap-2"><span className="grid min-w-0 flex-1 gap-0.5"><b className="truncate text-xs font-normal">{image.name}<span className="ml-1 text-[10px] text-muted-foreground">{image.size}</span></b>{(matchingFiles.length > 0 || (image.files?.length ?? 0) > 0) && <small className="truncate text-[10px] text-muted-foreground" title={image.files?.join(", ")}>{matchingFiles.length ? `Includes required file: ${matchingFiles.join(", ")}` : `Package files: ${image.files?.slice(0, 2).join(", ")}`}</small>}</span><Button size="sm" variant="outline" disabled={downloading !== null || wasDownloaded} onClick={() => onDownload(image, file.filename)}><Download />{downloading === `${image.image_type}:${image.id}` ? "Downloading" : wasDownloaded ? "Downloaded" : "Download"}</Button></div>;
        }) : !suggestionsLoading && <p className="text-[11px] text-muted-foreground">No matching iShare2 entry was found. Search by appliance name in the iShare2 tab or add a local image.</p>}</div>}
        {file.download_url && isWebUrl(file.download_url) && <a className="w-fit text-xs text-primary underline-offset-4 hover:underline" href={file.download_url} target="_blank" rel="noreferrer">Image source <ArrowUpRight className="inline size-3" /></a>}
      </Card>;
    })}</div> : <p className="text-xs text-muted-foreground">The registry does not specify separate image filenames.</p>}</div>
    <Card className="p-3 text-xs leading-relaxed text-muted-foreground shadow-none">The <code>.gns3a</code> file is an appliance definition. Required vendor images are supplied separately and are not downloaded by this import.</Card>
    {importResult && <Card className="p-3 text-xs" role="status">{importResult.message} {importResult.matching_local_templates} local runtime template{importResult.matching_local_templates === 1 ? "" : "s"} matched.</Card>}
    <div className="flex flex-wrap justify-end gap-2">{isWebUrl(appliance.source_url) && <Button asChild variant="outline"><a href={appliance.source_url} target="_blank" rel="noreferrer">Registry source <ArrowUpRight /></a></Button>}<Button onClick={onImport} disabled={importing || appliance.imported}>{importing ? "Importing…" : appliance.imported ? "Definition imported" : "Import .gns3a definition"}</Button></div>
  </div>;
}

function templateImageName(template: RuntimeTemplate): string[] {
  if (template.template_type === "qemu") return [template.hda_disk_image];
  if (template.template_type === "dynamips") return [template.image];
  return [];
}

function imageNameMatches(candidate: string, required: string): boolean {
  const name = candidate.split(/[\\/]/).at(-1)?.toLocaleLowerCase() ?? candidate.toLocaleLowerCase();
  const expected = required.split(/[\\/]/).at(-1)?.toLocaleLowerCase() ?? required.toLocaleLowerCase();
  return name === expected || name.endsWith(`-${expected}`);
}

function EmptySearch({ text }: { text: string }) {
  return <Card className="mx-1 my-2 flex min-h-28 flex-col items-center justify-center gap-2 border-dashed p-4 text-center shadow-none"><Server className="size-5 text-muted-foreground" /><span className="max-w-64 text-xs leading-relaxed text-muted-foreground">{text}</span></Card>;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) { value /= 1024; unit += 1; }
  return `${value.toFixed(value >= 10 ? 0 : 1)} ${units[unit]}`;
}

function isWebUrl(value: string): boolean {
  try { return ["http:", "https:"].includes(new URL(value).protocol); }
  catch { return false; }
}

function errorMessage(reason: unknown): string {
  return reason instanceof Error ? reason.message : "Catalog request failed. Please try again.";
}
