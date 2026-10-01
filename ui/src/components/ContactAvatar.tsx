import { Avatar, AvatarFallback } from "@/components/ui/avatar";
import { hueOf, initials } from "@/lib/format";
import { cn } from "@/lib/utils";

export function ContactAvatar({ id, name, className }: { id: string; name: string; className?: string }) {
  return (
    <Avatar className={cn("size-12", className)}>
      <AvatarFallback
        className="font-medium text-white"
        style={{ backgroundColor: `oklch(0.58 0.11 ${hueOf(id)})` }}
      >
        {initials(name)}
      </AvatarFallback>
    </Avatar>
  );
}
