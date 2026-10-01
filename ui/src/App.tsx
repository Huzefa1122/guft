import { useState } from "react";
import { Lock } from "lucide-react";
import { Button } from "@/components/ui/button";
import { TooltipProvider } from "@/components/ui/tooltip";
import { useApp } from "@/lib/store";
import { AddContactDialog } from "@/components/AddContact";
import { Brand } from "@/components/Brand";
import { ChatView } from "@/components/ChatView";
import { CreateRoomDialog } from "@/components/CreateRoom";
import { LockScreen } from "@/components/LockScreen";
import { SafetyDialog } from "@/components/Safety";
import { SettingsDialog } from "@/components/Settings";
import { Sidebar } from "@/components/Sidebar";
import { Toasts } from "@/components/Toasts";
import { Welcome } from "@/components/Welcome";

function EmptyState({ onAdd, onAddRoom }: { onAdd: () => void; onAddRoom: () => void }) {
  return (
    <div className="chat-wallpaper hidden min-w-0 flex-1 flex-col items-center justify-center gap-4 p-8 text-center md:flex">
      <div className="grid size-20 place-items-center rounded-full bg-card shadow-sm">
        <Lock className="size-9 text-primary" />
      </div>
      <h2 className="text-2xl font-semibold tracking-tight">Private by design</h2>
      <p className="max-w-sm text-sm text-muted-foreground">
        No servers, no accounts, no phone numbers. Pick a conversation, or invite someone to start one. Everything is end-to-end encrypted and travels over Tor.
      </p>
      <div className="flex gap-2">
        <Button onClick={onAdd}>Add a contact</Button>
        <Button variant="outline" onClick={onAddRoom}>New room</Button>
      </div>
    </div>
  );
}

function Main() {
  const { selected } = useApp();
  const [add, setAdd] = useState(false);
  const [addRoom, setAddRoom] = useState(false);
  const [settings, setSettings] = useState(false);
  const [safety, setSafety] = useState(false);
  return (
    <div className="flex h-full">
      <Sidebar onAdd={() => setAdd(true)} onAddRoom={() => setAddRoom(true)} onSettings={() => setSettings(true)} />
      {selected ? <ChatView onSafety={() => setSafety(true)} /> : <EmptyState onAdd={() => setAdd(true)} onAddRoom={() => setAddRoom(true)} />}
      <AddContactDialog open={add} onOpenChange={setAdd} />
      <CreateRoomDialog open={addRoom} onOpenChange={setAddRoom} />
      <SettingsDialog open={settings} onOpenChange={setSettings} />
      <SafetyDialog open={safety} onOpenChange={setSafety} />
    </div>
  );
}

export default function App() {
  const { status } = useApp();
  let screen;
  if (!status) {
    screen = (
      <div className="grid h-full place-items-center">
        <Brand size="lg" />
      </div>
    );
  } else if (!status.hasProfile) {
    screen = <Welcome />;
  } else if (!status.unlocked) {
    screen = <LockScreen />;
  } else {
    screen = <Main />;
  }
  return (
    <TooltipProvider delayDuration={300}>
      {screen}
      <Toasts />
    </TooltipProvider>
  );
}
