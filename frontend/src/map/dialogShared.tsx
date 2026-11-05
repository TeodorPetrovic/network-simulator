export function Unavailable({ message }: { message: string }) {
  return <div role="alert" className="rounded-md border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive"><b className="block">Feature unavailable</b><span>{message}</span></div>;
}

export function errorMessage(error: unknown) { return error instanceof Error ? error.message : "The controller did not complete this request."; }
export function colorInput(value: string) { return /^#[0-9a-f]{6}$/i.test(value) ? value : "#64748b"; }
