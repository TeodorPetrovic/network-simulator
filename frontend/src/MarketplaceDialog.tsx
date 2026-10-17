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
  const [importResult, setImportResult] = useState<Gns3ApplianceImportResult | null>(null);
  const [downloadResult, setDownloadResult] = useState<IshareDownloadResult | null>(null);
  const [downloading, setDownloading] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [importing, setImporting] = useState(false);
  const [error, setError] = useState("");

  async function search(event?: FormEvent<HTMLFormElement>) {
    event?.preventDefault();
    setLoading(true);
    setError("");
    setSelectedAppliance(null);
    setImportResult(null);
    setDownloadResult(null);
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
    try { setSelectedAppliance(await api.gns3Appliance(appliance.file_name)); }
    catch (reason) { setError(errorMessage(reason)); }
    finally { setLoading(false); }
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

  async function downloadImage(image: IshareImage) {
    const key = `${image.image_type}:${image.id}`;
    setDownloading(key);
    setError("");
    setDownloadResult(null);
    try { setDownloadResult(await api.downloadIshareImage(image)); }
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
  }

  return <Dialog open onOpenChange={(open) => { if (!open) onClose(); }}>
    <DialogContent className="flex max-h-[92dvh] max-w-[1060px] flex-col gap-0 overflow-hidden p-0">
      <DialogHeader className="px-5 pb-4 pt-5 pr-12">
        <DialogTitle>Image and appliance catalog</DialogTitle>
        <DialogDescription>Import GNS3 appliance definitions or download supported iShare2 images into the global library.</DialogDescription>
      </DialogHeader>
      <Tabs value={catalog} onValueChange={(value) => changeCatalog(value as Catalog)} className="flex min-h-0 flex-1 flex-col">
        <div className="flex flex-wrap items-center justify-between gap-3 border-y px-5 py-2.5">
          <TabsList><TabsTrigger value="gns3"><FileBox />GNS3 appliances</TabsTrigger><TabsTrigger value="ishare2"><HardDrive />iShare2 images</TabsTrigger></TabsList>
          <form className="flex min-w-[280px] flex-1 justify-end gap-2" onSubmit={(event) => void search(event)}>
            <div className="relative w-full max-w-md"><Search className="pointer-events-none absolute left-3 top-2.5 size-4 text-muted-foreground" /><Input value={query} onChange={(event) => setQuery(event.target.value)} placeholder={catalog === "gns3" ? "Search appliances, e.g. Alpine or Cisco" : "Search image metadata, e.g. alpine or c7200"} aria-label="Search catalog" className="pl-9" /></div>
            {catalog === "ishare2" && <Select value={imageType} onValueChange={setImageType}><SelectTrigger className="w-32" aria-label="iShare2 image type"><SelectValue /></SelectTrigger><SelectContent><SelectItem value="all">All types</SelectItem><SelectItem value="qemu">QEMU</SelectItem><SelectItem value="dynamips">Dynamips</SelectItem><SelectItem value="iol">IOL</SelectItem></SelectContent></Select>}
            <Button type="submit" disabled={loading}><Search />{loading ? "Searching" : "Search"}</Button>
          </form>
        </div>
        {error && <Card className="mx-5 mt-3 flex items-center justify-between gap-3 border-destructive/40 p-3 text-sm text-destructive" role="alert"><span>{error}</span><Button variant="ghost" size="icon-sm" onClick={() => setError("")} aria-label="Dismiss catalog error"><X /></Button></Card>}
        <TabsContent value="gns3" className="mt-0 min-h-0 flex-1">
          <div className="grid h-full min-h-[330px] grid-cols-[minmax(280px,0.85fr)_minmax(360px,1.15fr)]">
            <ScrollArea className="border-r">
              <div className="space-y-1 p-3" aria-label="GNS3 appliance results">
                {appliances.map((item) => <Button key={item.file_name} variant={selectedAppliance?.file_name === item.file_name ? "secondary" : "ghost"} className="h-auto min-h-14 w-full justify-start gap-3 px-2 text-left" onClick={() => void inspect(item)}>
                  <span className="grid size-9 shrink-0 place-items-center rounded-md border text-primary"><FileBox size={17} /></span><span className="grid min-w-0 flex-1 gap-1"><b className="truncate text-xs font-medium">{item.name}</b><small className="truncate text-[10px] text-muted-foreground">{item.file_name}</small></span><Badge variant="outline">{item.imported ? "Imported" : ".gns3a"}</Badge>
                </Button>)}
                {!loading && appliances.length === 0 && <EmptySearch text="Search the GNS3 appliance registry for installable definitions." />}
                {loading && <p className="py-5 text-center text-xs text-muted-foreground">Searching GNS3 catalog…</p>}
              </div>
            </ScrollArea>
            <ScrollArea><div className="p-5">
              {selectedAppliance ? <ApplianceDetails appliance={selectedAppliance} importing={importing} importResult={importResult} onImport={() => void importDefinition()} /> : <EmptySearch text="Choose an appliance to inspect its runtime, image files, and import status." />}
            </div></ScrollArea>
          </div>
        </TabsContent>
        <TabsContent value="ishare2" className="mt-0 min-h-0 flex-1">
          <div className="grid h-full min-h-[330px] grid-cols-[minmax(280px,0.85fr)_minmax(360px,1.15fr)]">
            <ScrollArea className="border-r">
              <div className="space-y-1 p-3" aria-label="iShare2 image results">
                {images.map((image) => <Card key={`${image.image_type}:${image.id}`} className="flex items-center gap-3 p-2 shadow-none">
                  <span className="grid size-9 shrink-0 place-items-center rounded-md border text-primary"><HardDrive size={16} /></span><span className="grid min-w-0 flex-1 gap-1"><b className="truncate text-xs font-medium">{image.name}</b><small className="truncate text-[10px] text-muted-foreground">{image.image_type.toUpperCase()} · {image.file_count} file{image.file_count === 1 ? "" : "s"} · {image.size}</small></span>
                  {image.image_type === "qemu" || image.image_type === "dynamips" ? <Button size="sm" variant="outline" disabled={downloading !== null} onClick={() => void downloadImage(image)}><Download />{downloading === `${image.image_type}:${image.id}` ? "Downloading" : "Download"}</Button> : <Badge variant="secondary">Search only</Badge>}
                </Card>)}
                {!loading && images.length === 0 && <EmptySearch text="Search the iShare2 / LabHub index for image metadata." />}
                {loading && <p className="py-5 text-center text-xs text-muted-foreground">Searching iShare2 catalog…</p>}
              </div>
            </ScrollArea>
            <ScrollArea><div className="space-y-4 p-5">
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

function ApplianceDetails({ appliance, importing, importResult, onImport }: { appliance: Gns3Appliance; importing: boolean; importResult: Gns3ApplianceImportResult | null; onImport: () => void }) {
  return <div className="space-y-4">
    <div><Badge variant="outline">GNS3 appliance</Badge><h3 className="mt-2 text-lg font-semibold">{appliance.name}</h3>{appliance.description && <p className="mt-2 text-sm leading-relaxed text-muted-foreground">{appliance.description}</p>}</div>
    <Card className="grid grid-cols-[86px_1fr] gap-x-3 gap-y-2 p-3 text-xs shadow-none"><span className="text-muted-foreground">Runtime</span><b>{appliance.template_type ?? "Registry metadata"}</b><span className="text-muted-foreground">Category</span><b>{appliance.category ?? "Uncategorized"}</b><span className="text-muted-foreground">Definition</span><b className="break-all">{appliance.file_name}</b></Card>
    <div><h4 className="mb-2 text-sm font-medium">Required image files</h4>{appliance.required_files.length ? <div className="space-y-2">{appliance.required_files.map((file) => <Card key={`${file.filename}:${file.version ?? ""}`} className="grid grid-cols-[minmax(0,1fr)_auto] gap-1 p-3 shadow-none"><b className="break-all font-mono text-xs">{file.filename}</b>{file.size != null && <span className="text-right text-[11px] text-muted-foreground">{formatBytes(file.size)}</span>}{file.version && <span className="text-[11px] text-muted-foreground">{file.version}</span>}{file.download_url && isWebUrl(file.download_url) && <a className="col-span-2 w-fit text-xs text-primary underline-offset-4 hover:underline" href={file.download_url} target="_blank" rel="noreferrer">Image source <ArrowUpRight className="inline size-3" /></a>}</Card>)}</div> : <p className="text-xs text-muted-foreground">The registry does not specify separate image filenames.</p>}</div>
    <Card className="p-3 text-xs leading-relaxed text-muted-foreground shadow-none">The <code>.gns3a</code> file is an appliance definition. Required vendor images are supplied separately and are not downloaded by this import.</Card>
    {importResult && <Card className="p-3 text-xs" role="status">{importResult.message} {importResult.matching_local_templates} local runtime template{importResult.matching_local_templates === 1 ? "" : "s"} matched.</Card>}
    <div className="flex flex-wrap justify-end gap-2">{isWebUrl(appliance.source_url) && <Button asChild variant="outline"><a href={appliance.source_url} target="_blank" rel="noreferrer">Registry source <ArrowUpRight /></a></Button>}<Button onClick={onImport} disabled={importing || appliance.imported}>{importing ? "Importing…" : appliance.imported ? "Definition imported" : "Import .gns3a definition"}</Button></div>
  </div>;
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
