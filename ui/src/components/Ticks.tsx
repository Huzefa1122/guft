import { AlertCircle, Check, CheckCheck, Clock } from "lucide-react";
import type { MsgStatus } from "@/lib/api";
import { cn } from "@/lib/utils";

const LABEL: Record<MsgStatus, string> = {
  queued: "Waiting to send",
  sent: "Sent",
  delivered: "Delivered",
  failed: "Failed to send",
};

/** Delivery state: clock (queued), one tick (sent), two ticks (delivered), alert (failed). */
export function Ticks({ status, className }: { status: MsgStatus; className?: string }) {
  const cls = cn("size-3.5 shrink-0", status === "failed" ? "text-destructive" : "text-muted-foreground", className);
  const icon =
    status === "queued" ? <Clock className={cls} /> : status === "sent" ? <Check className={cls} /> : status === "delivered" ? <CheckCheck className={cls} /> : <AlertCircle className={cls} />;
  return (
    <span title={LABEL[status]} aria-label={LABEL[status]} className="inline-flex">
      {icon}
    </span>
  );
}
