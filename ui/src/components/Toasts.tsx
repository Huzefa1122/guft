import { AlertCircle, CheckCircle2, X } from "lucide-react";
import { useApp } from "@/lib/store";
import { cn } from "@/lib/utils";

export function Toasts() {
  const { toasts, dismiss } = useApp();
  return (
    <div className="pointer-events-none fixed right-4 bottom-4 z-[100] flex w-80 flex-col gap-2" role="status" aria-live="polite">
      {toasts.map((t) => (
        <div
          key={t.id}
          className={cn(
            "pointer-events-auto flex items-start gap-2 rounded-lg border bg-popover p-3 text-sm shadow-lg animate-in fade-in slide-in-from-bottom-2",
            t.kind === "error" && "border-destructive/40",
          )}
        >
          {t.kind === "error" ? <AlertCircle className="mt-0.5 size-4 shrink-0 text-destructive" /> : <CheckCircle2 className="mt-0.5 size-4 shrink-0 text-primary" />}
          <span className="selectable min-w-0 flex-1 break-words">{t.text}</span>
          <button className="text-muted-foreground hover:text-foreground" onClick={() => dismiss(t.id)} aria-label="Dismiss">
            <X className="size-4" />
          </button>
        </div>
      ))}
    </div>
  );
}
