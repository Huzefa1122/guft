import { MessageSquareDashed, UsersRound } from "lucide-react";
import { cn } from "@/lib/utils";

/** Group rooms get the people icon; temporary chats a dashed ring, to look as fleeting as they are. */
export function RoomAvatar({ name, temp, className }: { name: string; temp?: boolean; className?: string }) {
  return (
    <div
      className={cn(
        "grid size-12 shrink-0 place-items-center rounded-full",
        temp ? "border-2 border-dashed border-muted-foreground/60 text-muted-foreground" : "bg-primary/15 text-primary",
        className,
      )}
      title={name}
    >
      {temp ? <MessageSquareDashed className="size-5" /> : <UsersRound className="size-5" />}
    </div>
  );
}
