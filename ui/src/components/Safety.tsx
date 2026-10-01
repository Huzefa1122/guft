import { useEffect, useState } from "react";
import { ShieldAlert, ShieldCheck } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { api, errorText } from "@/lib/api";
import { groupDigits } from "@/lib/format";
import { useApp } from "@/lib/store";

export function SafetyDialog({ open, onOpenChange }: { open: boolean; onOpenChange: (o: boolean) => void }) {
  const { current, refresh, toast } = useApp();
  const [number, setNumber] = useState<string[]>([]);
  const id = current?.kind === "contact" ? current.id : undefined;

  useEffect(() => {
    if (!open || !id) return;
    setNumber([]);
    api.safetyNumber(id).then((n) => setNumber(groupDigits(n))).catch((e) => toast(errorText(e), "error"));
  }, [open, id, toast]);

  if (!current || current.kind !== "contact") return null;
  const contact = current;

  async function toggle() {
    try {
      await api.setVerified(contact.id, !contact.verified);
      await refresh();
      onOpenChange(false);
    } catch (e) {
      toast(errorText(e), "error");
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            {contact.verified ? <ShieldCheck className="size-5 text-primary" /> : <ShieldAlert className="size-5 text-amber-500" />}
            Verify {contact.name}
          </DialogTitle>
          <DialogDescription>
            Compare this number with the one on {contact.name}'s device, in person or over a call you trust. If both match, nobody is sitting between you.
          </DialogDescription>
        </DialogHeader>
        <div className="selectable grid grid-cols-3 gap-x-4 gap-y-3 rounded-xl bg-secondary p-5 text-center font-mono text-lg tracking-wider" aria-label="Safety number">
          {number.length ? number.map((g, i) => <span key={i}>{g}</span>) : <span className="col-span-3 text-sm text-muted-foreground">Calculating…</span>}
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>Close</Button>
          <Button onClick={() => void toggle()} disabled={!number.length}>{contact.verified ? "Mark as not verified" : "Mark as verified"}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
