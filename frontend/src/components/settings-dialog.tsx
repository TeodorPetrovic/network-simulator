import type { FormEvent, ReactNode } from "react";
import { useRef, useState } from "react";
import { Info, LoaderCircle } from "lucide-react";
import { cn } from "cn";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";

export type SettingsTab = { id: string; label: string; content: ReactNode };

export function SettingsTabs({ tabs, label }: { tabs: SettingsTab[]; label: string }) {
  const [selected, setSelected] = useState(tabs[0]?.id);
  const invalidField = useRef<HTMLInputElement | null>(null);
  const reportingValidity = useRef(false);
  const active = tabs.some((tab) => tab.id === selected) ? selected : tabs[0]?.id;
  return <Tabs value={active} onValueChange={setSelected} className="min-h-0 flex-1 gap-0 overflow-hidden">
    <div className="shrink-0 overflow-x-auto border-b px-4 sm:px-6">
      <TabsList aria-label={label} variant="line" className="min-w-full justify-start gap-3 group-data-horizontal/tabs:h-11">
        {tabs.map((tab) => <TabsTrigger key={tab.id} value={tab.id} className="h-11 flex-none rounded-none border-0 border-b-2 border-transparent px-3 text-muted-foreground shadow-none after:hidden data-[state=active]:border-primary data-[state=active]:bg-transparent data-[state=active]:text-primary data-[state=active]:shadow-none focus-visible:ring-2 focus-visible:ring-inset">{tab.label}</TabsTrigger>)}
      </TabsList>
    </div>
    <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain p-4 sm:p-6">
      {tabs.map((tab) => <TabsContent key={tab.id} value={tab.id} forceMount hidden={active !== tab.id} className="m-0 space-y-6" onInvalidCapture={(event) => {
        if (reportingValidity.current) return;
        // Keep native validation for fields on other tabs, then reveal the field to fix.
        event.preventDefault();
        if (invalidField.current) return;
        const input = event.target as HTMLInputElement;
        invalidField.current = input;
        setSelected(tab.id);
        requestAnimationFrame(() => {
          reportingValidity.current = true;
          input.focus(); input.reportValidity();
          reportingValidity.current = false;
          invalidField.current = null;
        });
      }}>{tab.content}</TabsContent>)}
    </div>
  </Tabs>;
}

/** One bounded surface: context, navigation, scrolling fields, then persistent actions. */
export function SettingsDialog({ open, onOpenChange, title, description, context, tabs, children, onSubmit, saving = false, saveDisabled = false, saveLabel = "Save changes", error, notice }: {
  open: boolean; onOpenChange: (open: boolean) => void; title: ReactNode; description: ReactNode;
  context?: ReactNode; tabs?: SettingsTab[]; children?: ReactNode; onSubmit: (event: FormEvent<HTMLFormElement>) => void;
  saving?: boolean; saveDisabled?: boolean; saveLabel?: string; error?: string; notice?: ReactNode;
}) {
  return <Dialog open={open} onOpenChange={(next) => { if (!saving) onOpenChange(next); }}>
    <DialogContent className="flex max-h-[calc(100dvh-2rem)] w-[calc(100vw-2rem)] flex-col gap-0 overflow-hidden p-0 max-w-4xl [&_[data-slot=select-trigger]]:w-full [&_[data-slot=select-trigger]]:min-w-0" onEscapeKeyDown={(event) => { if (saving) event.preventDefault(); }}>
      <DialogHeader className="shrink-0 border-b px-4 py-5 pr-12 sm:px-6 sm:pr-12">
        {context && <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">{context}</div>}
        <DialogTitle className="break-words text-xl font-semibold leading-tight">{title}</DialogTitle>
        <DialogDescription className="max-w-2xl leading-relaxed">{description}</DialogDescription>
      </DialogHeader>
      <form onSubmit={onSubmit} className="flex min-h-0 flex-1 flex-col" aria-busy={saving}>
        {tabs ? <SettingsTabs tabs={tabs} label="Settings sections" /> : <div className="min-h-0 space-y-6 overflow-y-auto overscroll-contain p-4 sm:p-6">{children}</div>}
        <div className="shrink-0 border-t bg-muted/30">
          {notice && <div className="px-4 pt-3 sm:px-6">{notice}</div>}
          {error && <p role="alert" className="px-4 pt-3 text-sm text-destructive sm:px-6">{error}</p>}
          <DialogFooter className="m-0 border-0 bg-transparent p-4 sm:px-6">
            <Button type="button" variant="outline" disabled={saving} onClick={() => onOpenChange(false)}>Cancel</Button>
            <Button type="submit" disabled={saving || saveDisabled}>{saving && <LoaderCircle className="animate-spin" />}{saving ? "Saving…" : saveLabel}</Button>
          </DialogFooter>
        </div>
      </form>
    </DialogContent>
  </Dialog>;
}

export function SettingsSection({ title, description, children, columns = false }: { title: string; description?: string; children: ReactNode; columns?: boolean }) {
  return <section className="min-w-0 space-y-4">
    <div className="space-y-1"><h3 className="text-sm font-semibold">{title}</h3>{description && <p className="text-xs leading-relaxed text-muted-foreground">{description}</p>}</div>
    <div className={cn("grid min-w-0 items-start gap-4", columns && "sm:grid-cols-2")}>{children}</div>
  </section>;
}

export function SettingsNotice({ children }: { children: ReactNode }) {
  return <div role="status" className="flex items-start gap-2 rounded-lg border bg-muted/40 p-3 text-xs leading-relaxed text-muted-foreground"><Info className="mt-0.5 size-4 shrink-0" /><div>{children}</div></div>;
}
