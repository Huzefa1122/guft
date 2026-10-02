import { Fragment, useLayoutEffect, useRef, useState } from "react";
import { ArrowLeft, Lock, MessageSquareDashed, MoreVertical, Pencil, ShieldAlert, ShieldCheck, Trash2, UserMinus, UserPlus, UsersRound } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { api, errorText, type Message } from "@/lib/api";
import { dayLabel, sameDay } from "@/lib/format";
import { useApp } from "@/lib/store";
import { Bubble } from "./Bubble";
import { Composer } from "./Composer";
import { ContactAvatar } from "./ContactAvatar";
import { RenameContactDialog } from "./RenameContact";
import { RoomAvatar } from "./RoomAvatar";
import { RoomInviteDialog } from "./RoomInvite";
import { RoomMembersDialog } from "./RoomMembers";

type Confirm = null | "clear" | "remove" | "leave";

export function ChatView({ onSafety }: { onSafety: () => void }) {
  const { current, messages, hasMore, loadOlder, select, refresh, toast, selected, startTempChat } = useApp();
  const scroller = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  const [confirm, setConfirm] = useState<Confirm>(null);
  const [members, setMembers] = useState(false);
  const [invite, setInvite] = useState(false);
  const [renaming, setRenaming] = useState(false);

  // Stay pinned to the newest message unless the reader scrolled up.
  useLayoutEffect(() => {
    const el = scroller.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [messages]);
  useLayoutEffect(() => {
    stick.current = true;
  }, [selected]);

  if (!current) return null;
  const conv = current;
  const room = current.kind === "room";
  const direct = current.direct;
  /** A room with several people, as opposed to a one-to-one chat (temporary or not). */
  const group = room && !direct;
  const temp = current.temp;
  const name = current.name;

  function senderName(m: Message): string {
    return conv.members.find((x) => x.id === m.sender)?.name ?? "Former member";
  }

  function onScroll() {
    const el = scroller.current;
    if (el) stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 80;
  }

  async function older() {
    const el = scroller.current;
    const before = el?.scrollHeight ?? 0;
    await loadOlder();
    requestAnimationFrame(() => {
      if (el) el.scrollTop += el.scrollHeight - before;
    });
  }

  async function doConfirm() {
    const what = confirm;
    setConfirm(null);
    try {
      if (what === "clear") {
        await api.deleteChat(conv.id);
        toast("Chat cleared");
        select(conv.id);
      } else if (what === "remove") {
        await api.removeContact(conv.id);
        select(null);
        toast(`${name} was removed`);
      } else if (what === "leave") {
        await api.leaveRoom(conv.id);
        select(null);
        toast(`You left ${name}`);
      }
      await refresh();
    } catch (e) {
      toast(errorText(e), "error");
    }
  }

  return (
    <section className="flex h-full min-w-0 flex-1 flex-col">
      <header className="flex h-16 shrink-0 items-center gap-3 border-b bg-card px-4 max-md:gap-2 max-md:px-2">
        <Button variant="ghost" size="icon" className="shrink-0 md:hidden" onClick={() => window.history.back()} aria-label="Back to chats">
          <ArrowLeft className="size-5" />
        </Button>
        {room ? <RoomAvatar name={name} temp={temp} className="size-10" /> : <ContactAvatar id={current.id} name={name} className="size-10" />}
        <div className="min-w-0 flex-1">
          <h2 className="truncate font-medium">{name}</h2>
          {direct ? (
            <span className="flex items-center gap-1 text-xs text-muted-foreground">
              <MessageSquareDashed className="size-3.5" /> Temporary chat
            </span>
          ) : group ? (
            <button onClick={() => setMembers(true)} className="flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground">
              <UsersRound className="size-3.5" /> {current.members.length + 1} member{current.members.length + 1 === 1 ? "" : "s"}
              {temp && " · temporary"}
              {current.identity && ` · you are ${current.identity}`}
            </button>
          ) : (
            <button onClick={onSafety} className="flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground">
              {current.verified ? (
                <>
                  <ShieldCheck className="size-3.5 text-primary" /> Verified
                </>
              ) : (
                <>
                  <ShieldAlert className="size-3.5 text-amber-500" /> Not verified · tap to verify
                </>
              )}
            </button>
          )}
        </div>
        {!room && (
          <Tooltip>
            <TooltipTrigger asChild>
              <Button variant="ghost" size="icon" onClick={() => void startTempChat(current.id)} aria-label="Start a temporary chat">
                <MessageSquareDashed className="size-5" />
              </Button>
            </TooltipTrigger>
            <TooltipContent>Temporary chat: kept in memory only</TooltipContent>
          </Tooltip>
        )}
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="ghost" size="icon" aria-label="Chat options">
              <MoreVertical className="size-5" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            {direct ? (
              <>
                <DropdownMenuItem onSelect={() => setConfirm("clear")}>
                  <Trash2 /> Clear chat
                </DropdownMenuItem>
                <DropdownMenuItem variant="destructive" onSelect={() => setConfirm("leave")}>
                  <UserMinus /> End temporary chat
                </DropdownMenuItem>
              </>
            ) : room ? (
              <>
                {(current.mine || current.openInvites) && (
                  <DropdownMenuItem onSelect={() => setInvite(true)}>
                    <UserPlus /> Invite someone
                  </DropdownMenuItem>
                )}
                <DropdownMenuItem onSelect={() => setMembers(true)}>
                  <UsersRound /> Members
                </DropdownMenuItem>
                <DropdownMenuSeparator />
                <DropdownMenuItem onSelect={() => setConfirm("clear")}>
                  <Trash2 /> Clear chat
                </DropdownMenuItem>
                <DropdownMenuItem variant="destructive" onSelect={() => setConfirm("leave")}>
                  <UserMinus /> Leave room
                </DropdownMenuItem>
              </>
            ) : (
              <>
                <DropdownMenuItem onSelect={onSafety}>
                  <ShieldCheck /> Verify safety number
                </DropdownMenuItem>
                <DropdownMenuItem onSelect={() => setRenaming(true)}>
                  <Pencil /> Rename
                </DropdownMenuItem>
                <DropdownMenuSeparator />
                <DropdownMenuItem onSelect={() => setConfirm("clear")}>
                  <Trash2 /> Clear chat
                </DropdownMenuItem>
                <DropdownMenuItem variant="destructive" onSelect={() => setConfirm("remove")}>
                  <UserMinus /> Remove contact
                </DropdownMenuItem>
              </>
            )}
          </DropdownMenuContent>
        </DropdownMenu>
      </header>

      {temp && (
        <div className="flex shrink-0 items-center justify-center gap-2 border-b bg-secondary px-4 py-1.5 text-xs text-muted-foreground" role="note">
          <MessageSquareDashed className="size-3.5 shrink-0" />
          <span>Temporary: kept in memory only, on every device. Gone when guft locks or closes.</span>
        </div>
      )}
      <div ref={scroller} onScroll={onScroll} className="chat-wallpaper min-h-0 flex-1 overflow-y-auto px-[6%] py-4" aria-live="polite">
        <div className="mx-auto mb-3 flex w-fit max-w-md items-start gap-2 rounded-lg bg-popover/90 px-3 py-2 text-center text-xs text-muted-foreground shadow-sm">
          <Lock className="mt-0.5 size-3.5 shrink-0" />
          <span>
            {direct
              ? `End-to-end encrypted, post-quantum, over Tor. Only you and ${name} can read this, and neither device saves it.`
              : room
              ? `Every message is end-to-end encrypted to each of the ${current.members.length + 1} member${current.members.length + 1 === 1 ? "" : "s"} and sent over Tor.`
              : `Messages are end-to-end encrypted with post-quantum keys and sent over Tor. Only you and ${name} can read them.`}
          </span>
        </div>
        {hasMore && (
          <div className="mb-2 flex justify-center">
            <Button variant="secondary" size="sm" onClick={() => void older()}>
              Load earlier messages
            </Button>
          </div>
        )}
        {messages.map((m, i) => {
          const prev = messages[i - 1];
          const newDay = !prev || !sameDay(prev.ts, m.ts);
          const first = newDay || prev.outgoing !== m.outgoing || (group && prev.sender !== m.sender);
          return (
            <Fragment key={m.id}>
              {newDay && (
                <div className="my-3 flex justify-center">
                  <span className="rounded-lg bg-popover/90 px-3 py-1 text-xs text-muted-foreground shadow-sm">{dayLabel(m.ts)}</span>
                </div>
              )}
              <Bubble m={m} first={first} senderName={group ? senderName(m) : undefined} showSender={group && !m.outgoing && first} />
            </Fragment>
          );
        })}
      </div>

      <Composer />

      <Dialog open={confirm !== null} onOpenChange={(o) => !o && setConfirm(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>
              {confirm === "remove" ? `Remove ${name}?` : confirm === "leave" ? (direct ? "End this temporary chat?" : `Leave ${name}?`) : "Clear this chat?"}
            </DialogTitle>
            <DialogDescription>
              {confirm === "remove"
                ? "They lose access to you and this conversation is deleted from this device. To talk again you'll need a new invite."
                : confirm === "leave"
                  ? direct
                    ? "The chat and its messages disappear from both devices. You can start a new temporary chat any time."
                    : current.identity
                    ? "You stop receiving room messages and this conversation is deleted. The separate identity you used here is erased completely: its keys and Tor address are destroyed."
                    : "You stop receiving room messages and this conversation is deleted from this device. You can only return with a new invite."
                  : "All messages in this chat are deleted from this device. Others keep their own copy."}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setConfirm(null)}>Cancel</Button>
            <Button variant="destructive" onClick={() => void doConfirm()}>
              {confirm === "remove" ? "Remove" : confirm === "leave" ? (direct ? "End chat" : "Leave") : "Clear"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {!room && <RenameContactDialog id={current.id} current={name} open={renaming} onOpenChange={setRenaming} />}

      {group && (
        <>
          <RoomMembersDialog
            room={current}
            open={members}
            onOpenChange={setMembers}
            onInvite={() => {
              setMembers(false);
              setInvite(true);
            }}
          />
          <RoomInviteDialog room={current} open={invite} onOpenChange={setInvite} />
        </>
      )}
    </section>
  );
}
