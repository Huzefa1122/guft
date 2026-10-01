import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { api, errorText, type AppEvent, type Message, type RoomInvitation, type RoomMember, type Status } from "./api";

export type Net = "off" | "starting" | "online" | "error";
export type Theme = "system" | "light" | "dark";
export type Toast = { id: number; text: string; kind: "info" | "error" };

/** One row in the sidebar: a one-to-one contact or a room. */
export type Conv = {
  kind: "contact" | "room";
  /** Contact id, or the room chat key (`r-<hex>`). */
  id: string;
  name: string;
  last: Message | null;
  unread: number;
  /** Contacts only. */
  verified: boolean;
  /** Rooms only. */
  mine: boolean;
  openInvites: boolean;
  members: RoomMember[];
  /** Memory-only room or chat: nothing is saved, gone when guft locks or closes. */
  temp: boolean;
  /** A one-to-one temporary chat. */
  direct: boolean;
  /** Rooms with their own identity: the name you appear as there. */
  identity: string | null;
};

type Ctx = {
  status: Status | null;
  convs: Conv[];
  /** Rooms contacts invited you to; they wait for your answer. */
  invitations: RoomInvitation[];
  acceptInvitation: (room: string) => Promise<void>;
  declineInvitation: (room: string) => Promise<void>;
  selected: string | null;
  select: (id: string | null) => void;
  current: Conv | null;
  messages: Message[];
  hasMore: boolean;
  loadOlder: () => Promise<void>;
  net: Net;
  netMessage: string;
  toasts: Toast[];
  toast: (text: string, kind?: Toast["kind"]) => void;
  dismiss: (id: number) => void;
  refresh: () => Promise<void>;
  theme: Theme;
  setTheme: (t: Theme) => void;
  idleMinutes: number;
  setIdleMinutes: (m: number) => void;
  // actions
  unlock: (pass: string) => Promise<void>;
  createProfile: (pass: string, name: string) => Promise<void>;
  lock: () => Promise<void>;
  send: (text: string) => Promise<void>;
  sendFile: (name: string, b64: string) => Promise<void>;
  /** One tap: open (or reuse) a memory-only chat with a contact. */
  startTempChat: (contactId: string) => Promise<void>;
};

const AppCtx = createContext<Ctx | null>(null);

export function useApp(): Ctx {
  const v = useContext(AppCtx);
  if (!v) throw new Error("useApp outside provider");
  return v;
}

function readPref<T extends string | number>(key: string, fallback: T): T {
  try {
    const v = localStorage.getItem(key);
    if (v === null) return fallback;
    return (typeof fallback === "number" ? Number(v) : v) as T;
  } catch {
    return fallback;
  }
}

function writePref(key: string, value: string | number) {
  try {
    localStorage.setItem(key, String(value));
  } catch {
    /* storage may be unavailable; preferences are optional */
  }
}

export function AppProvider({ children }: { children: ReactNode }) {
  const [status, setStatus] = useState<Status | null>(null);
  const [convs, setConvs] = useState<Conv[]>([]);
  const [invitations, setInvitations] = useState<RoomInvitation[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [messages, setMessages] = useState<Message[]>([]);
  const [hasMore, setHasMore] = useState(false);
  const [net, setNet] = useState<Net>("off");
  const [netMessage, setNetMessage] = useState("");
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [theme, setThemeState] = useState<Theme>(() => readPref<Theme>("theme", "system"));
  const [idleMinutes, setIdleState] = useState<number>(() => readPref("idleMinutes", 5));
  const selectedRef = useRef<string | null>(null);
  selectedRef.current = selected;
  const convsRef = useRef<Conv[]>([]);
  convsRef.current = convs;
  const toastId = useRef(1);

  const toast = useCallback((text: string, kind: Toast["kind"] = "info") => {
    const id = toastId.current++;
    setToasts((t) => [...t.slice(-3), { id, text, kind }]);
    setTimeout(() => setToasts((t) => t.filter((x) => x.id !== id)), kind === "error" ? 6000 : 3500);
  }, []);
  const dismiss = useCallback((id: number) => setToasts((t) => t.filter((x) => x.id !== id)), []);

  // Theme: follow the system unless the user chose.
  useEffect(() => {
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const apply = () => document.documentElement.classList.toggle("dark", theme === "dark" || (theme === "system" && mq.matches));
    apply();
    mq.addEventListener("change", apply);
    return () => mq.removeEventListener("change", apply);
  }, [theme]);
  const setTheme = useCallback((t: Theme) => {
    writePref("theme", t);
    setThemeState(t);
  }, []);
  const setIdleMinutes = useCallback((m: number) => {
    writePref("idleMinutes", m);
    setIdleState(m);
    api.setIdleMinutes(m).catch(() => {});
  }, []);

  const loadConvs = useCallback(async () => {
    try {
      const [chats, rooms, invites] = await Promise.all([api.chats(), api.rooms(), api.roomInvitations()]);
      setInvitations(invites);
      const list: Conv[] = [
        ...chats.map(
          ({ contact, last, unread }): Conv => ({
            kind: "contact", id: contact.id, name: contact.name, last, unread,
            verified: contact.verified, mine: false, openInvites: false, members: [], temp: false, direct: false, identity: null,
          }),
        ),
        ...rooms.map(
          (r): Conv => ({
            kind: "room", id: r.id, name: r.name, last: r.last, unread: r.unread,
            verified: false, mine: r.mine, openInvites: r.openInvites, members: r.members, temp: r.temp, direct: r.direct, identity: r.identity,
          }),
        ),
      ];
      list.sort((a, b) => (b.last?.ts ?? 0) - (a.last?.ts ?? 0) || (b.last?.id ?? 0) - (a.last?.id ?? 0));
      setConvs(list);
    } catch {
      /* locked: nothing to show */
    }
  }, []);

  const loadMessages = useCallback(async (id: string) => {
    try {
      const page = await api.messages(id);
      if (selectedRef.current !== id) return;
      setMessages([...page].reverse());
      setHasMore(page.length >= 50);
    } catch {
      /* locked */
    }
  }, []);

  const refresh = useCallback(async () => {
    try {
      setStatus(await api.status());
    } catch {
      /* ignore */
    }
    await loadConvs();
  }, [loadConvs]);

  const select = useCallback(
    (id: string | null) => {
      setSelected(id);
      setMessages([]);
      setHasMore(false);
      if (id) {
        void loadMessages(id).then(() => api.markRead(id).then(loadConvs).catch(() => {}));
      }
    },
    [loadConvs, loadMessages],
  );

  const loadOlder = useCallback(async () => {
    const id = selectedRef.current;
    const oldest = messages[0];
    if (!id || !oldest) return;
    const page = await api.messages(id, oldest.id);
    setMessages((m) => [...[...page].reverse(), ...m]);
    setHasMore(page.length >= 50);
  }, [messages]);

  // Backend events carry ids only; we re-read what changed.
  useEffect(() => {
    let off: (() => void) | undefined;
    let dead = false;
    const handle = (e: AppEvent) => {
      switch (e.type) {
        case "unlocked":
          setNet("starting");
          api.setIdleMinutes(readPref("idleMinutes", 5)).catch(() => {});
          void refresh();
          break;
        case "locked":
          setNet("off");
          setConvs([]);
          setInvitations([]);
          setMessages([]);
          setSelected(null);
          setStatus((s) => (s ? { ...s, unlocked: false } : s));
          break;
        case "networkReady":
          setNet("online");
          setNetMessage("");
          void refresh();
          break;
        case "networkError":
          setNet("error");
          setNetMessage(e.message);
          break;
        case "contactAdded":
          void loadConvs();
          break;
        case "roomChanged":
          void loadConvs();
          break;
        case "roomInvited":
          toast("You have a new room invitation.");
          void loadConvs();
          break;
        case "roomRemoved":
          if (selectedRef.current === e.room) {
            setSelected(null);
            setMessages([]);
            toast("That conversation was closed.");
          }
          void loadConvs();
          break;
        case "message": {
          void loadConvs();
          const id = selectedRef.current;
          if (id === e.chat) void loadMessages(id).then(() => api.markRead(id).then(loadConvs).catch(() => {}));
          break;
        }
        case "delivered":
        case "sendFailed": {
          const next = e.type === "delivered" ? "delivered" : "failed";
          setMessages((ms) => ms.map((m) => (m.id === e.msgId ? { ...m, status: next } : m)));
          void loadConvs();
          break;
        }
      }
    };
    api.onEvent(handle).then((u) => (dead ? u() : (off = u)));
    void api.status().then((s) => {
      setStatus(s);
      if (s.unlocked) {
        setNet("starting");
        void loadConvs();
      }
    });
    return () => {
      dead = true;
      off?.();
    };
  }, [loadConvs, loadMessages, refresh, toast]);

  // User activity keeps the vault open; idle time relocks it (enforced in Rust).
  useEffect(() => {
    let last = 0;
    const ping = () => {
      const t = Date.now();
      if (t - last > 5000) {
        last = t;
        api.touch().catch(() => {});
      }
    };
    const events = ["pointerdown", "keydown", "wheel", "pointermove"] as const;
    events.forEach((n) => window.addEventListener(n, ping, { passive: true }));
    return () => events.forEach((n) => window.removeEventListener(n, ping));
  }, []);

  const unlock = useCallback(async (pass: string) => {
    await api.unlock(pass);
  }, []);
  const createProfile = useCallback(async (pass: string, name: string) => {
    await api.createProfile(pass, name);
  }, []);
  const lock = useCallback(async () => {
    await api.lock();
  }, []);

  const send = useCallback(
    async (text: string) => {
      const id = selectedRef.current;
      if (!id) return;
      const kind = convsRef.current.find((c) => c.id === id)?.kind ?? (id.startsWith("r-") ? "room" : "contact");
      try {
        if (kind === "room") await api.sendRoomText(id, text);
        else await api.sendText(id, text);
        await loadMessages(id);
        void loadConvs();
      } catch (e) {
        toast(errorText(e), "error");
      }
    },
    [loadConvs, loadMessages, toast],
  );
  const sendFile = useCallback(
    async (name: string, b64: string) => {
      const id = selectedRef.current;
      if (!id) return;
      const kind = convsRef.current.find((c) => c.id === id)?.kind ?? (id.startsWith("r-") ? "room" : "contact");
      try {
        if (kind === "room") await api.sendRoomFile(id, name, b64);
        else await api.sendFile(id, name, b64);
        await loadMessages(id);
        void loadConvs();
      } catch (e) {
        toast(errorText(e), "error");
      }
    },
    [loadConvs, loadMessages, toast],
  );

  const startTempChat = useCallback(
    async (contactId: string) => {
      try {
        const id = await api.startTempChat(contactId);
        await loadConvs();
        select(id);
      } catch (e) {
        toast(errorText(e), "error");
      }
    },
    [loadConvs, select, toast],
  );

  const acceptInvitation = useCallback(
    async (room: string) => {
      try {
        const id = await api.acceptRoomInvite(room);
        await loadConvs();
        select(id);
        toast("You joined the room.");
      } catch (e) {
        toast(errorText(e), "error");
        await loadConvs();
      }
    },
    [loadConvs, select, toast],
  );
  const declineInvitation = useCallback(
    async (room: string) => {
      try {
        await api.declineRoomInvite(room);
      } catch (e) {
        toast(errorText(e), "error");
      }
      await loadConvs();
    },
    [loadConvs, toast],
  );

  const current = useMemo(() => convs.find((c) => c.id === selected) ?? null, [convs, selected]);

  const value: Ctx = {
    status, convs, invitations, acceptInvitation, declineInvitation, selected, select, current, messages, hasMore, loadOlder, net, netMessage, toasts, toast, dismiss,
    refresh, theme, setTheme, idleMinutes, setIdleMinutes, unlock, createProfile, lock, send, sendFile, startTempChat,
  };
  return <AppCtx.Provider value={value}>{children}</AppCtx.Provider>;
}
