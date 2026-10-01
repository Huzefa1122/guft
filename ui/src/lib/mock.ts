// Development-only stand-in for the Rust backend, so the UI can be built and previewed
// with `pnpm dev` in a plain browser. Passphrase: "demo-passphrase". Never shipped.
import type { Api, AppEvent, Chat, Message, Room, RoomMember, Status } from "./api";

const now = () => Math.floor(Date.now() / 1000);
const handlers = new Set<(e: AppEvent) => void>();
const emit = (e: AppEvent) => handlers.forEach((h) => h(e));

const onion = (c: string) => `${c.repeat(56)}.onion`;
let nextId = 100;
const status: Status = { hasProfile: true, unlocked: false, name: "Huzefa", onion: onion("h"), onlineWhenLocked: false };
let unlocked = false;

const contacts = [
  { id: "a1", name: "Amina Khan", onion: onion("a"), verified: true },
  { id: "d2", name: "Daniel Okafor", onion: onion("d"), verified: false },
  { id: "p3", name: "Priya Raman", onion: onion("p"), verified: true },
  { id: "s4", name: "Sam", onion: onion("s"), verified: false },
];

const m = (id: number, chat: string, ago: number, outgoing: boolean, text: string, st: Message["status"] = "delivered", sender: string | null = null): Message => ({
  id, chat, ts: now() - ago, outgoing, kind: "text", text, fileName: null, fileSize: null, status: st, sender,
});

type MockRoom = Pick<Room, "id" | "name" | "creator" | "mine" | "openInvites"> & { members: RoomMember[]; unread: number; temp?: boolean; direct?: boolean };

const rooms: MockRoom[] = [
  {
    id: "r-8c41d6a9f0b24e13a7c5d2e8b9f04a61",
    name: "Trip planning",
    creator: "a1",
    mine: false,
    openInvites: true,
    unread: 2,
    members: [
      { id: "a1", name: "Amina Khan", online: true, isCreator: true },
      { id: "d2", name: "Daniel Okafor", online: true, isCreator: false },
      { id: "p3", name: "Priya Raman", online: false, isCreator: false },
    ],
  },
  {
    id: "r-1e07b3c5d9a24f68b0e1c2d3a4f50617",
    name: "Weekend crew",
    creator: "me",
    mine: true,
    openInvites: false,
    unread: 0,
    members: [
      { id: "d2", name: "Daniel Okafor", online: true, isCreator: false },
      { id: "s4", name: "Sam", online: false, isCreator: false },
    ],
  },
];

const store: Record<string, Message[]> = {
  a1: [
    m(1, "a1", 90_000, false, "Landed safely. The new app is running, nobody can read this but us 🔒"),
    m(2, "a1", 89_500, true, "Amazing. Did the safety numbers match?"),
    m(3, "a1", 3_700, false, "Yes — verified on the call. Sending the report now."),
    { id: 4, chat: "a1", ts: now() - 3_600, outgoing: false, kind: "file", text: null, fileName: "q3-report.pdf", fileSize: 148_200, status: "delivered", sender: null },
    m(5, "a1", 600, true, "Got it, thanks! Reading it tonight."),
    m(6, "a1", 120, false, "Perfect. Talk tomorrow?", "delivered"),
  ],
  d2: [m(10, "d2", 7_200, true, "Are we still on for Friday?", "delivered"), m(11, "d2", 7_000, false, "Friday works. 6pm?")],
  p3: [m(20, "p3", 200_000, false, "Happy birthday!! 🎉"), m(21, "p3", 199_000, true, "Thank you Priya!", "delivered")],
  s4: [m(30, "s4", 30, true, "Hello?", "queued")],
  [rooms[0].id]: [
    m(40, rooms[0].id, 5_400, true, "Found a lovely place near the lake."),
    m(41, rooms[0].id, 5_200, false, "Send the link! And does it take dogs?", "delivered", "d2"),
    m(42, rooms[0].id, 4_800, true, "Both. Booking it for the long weekend."),
    m(43, rooms[0].id, 200, false, "I'm in. Priya said she'll confirm tomorrow.", "delivered", "a1"),
  ],
  [rooms[1].id]: [m(50, rooms[1].id, 40_000, true, "Cinema on Saturday?", "delivered")],
};
const unread: Record<string, number> = { a1: 1, d2: 0, p3: 0, s4: 0 };

const chatsNow = (): Chat[] =>
  contacts
    .map((c) => ({ contact: c, last: store[c.id]?.at(-1) ?? null, unread: unread[c.id] ?? 0 }))
    .sort((a, b) => (b.last?.ts ?? 0) - (a.last?.ts ?? 0));

const roomsNow = (): Room[] =>
  rooms.map((r) => ({
    id: r.id,
    name: r.name,
    creator: r.creator,
    mine: r.mine,
    openInvites: r.openInvites,
    temp: r.temp ?? false,
    direct: r.direct ?? false,
    members: r.members,
    last: store[r.id]?.at(-1) ?? null,
    unread: r.unread,
  }));

const wait = <T>(v: T, ms = 120) => new Promise<T>((r) => setTimeout(() => r(v), ms));

function send(chat: string, text: string): number {
  const id = nextId++;
  (store[chat] ??= []).push(m(id, chat, 0, true, text, "queued"));
  setTimeout(() => {
    store[chat]!.find((x) => x.id === id)!.status = "delivered";
    emit({ type: "delivered", chat, msgId: id });
  }, 900);
  setTimeout(() => {
    const rid = nextId++;
    const room = rooms.find((r) => r.id === chat);
    const from = room?.members[0]?.id ?? null;
    const reply = room ? "👍 works for me" : "👍 (mock reply)";
    store[chat]!.push(m(rid, chat, 0, false, reply, "delivered", from));
    if (room) room.unread += 1;
    else unread[chat] = (unread[chat] ?? 0) + 1;
    emit({ type: "message", chat, id: rid });
  }, 2200);
  return id;
}

const fileData = new Map<number, string>();

function sendFile(chat: string, name: string, data: string): number {
  const id = nextId++;
  fileData.set(id, data);
  (store[chat] ??= []).push({ id, chat, ts: now(), outgoing: true, kind: "file", text: null, fileName: name, fileSize: Math.floor((data.length * 3) / 4), status: "queued", sender: null });
  setTimeout(() => { store[chat]!.find((x) => x.id === id)!.status = "delivered"; emit({ type: "delivered", chat, msgId: id }); }, 1200);
  return id;
}

export const mockApi: Api = {
  status: () => wait({ ...status, unlocked }),
  createProfile: async (_p, name) => { status.name = name; status.hasProfile = true; unlocked = true; emit({ type: "unlocked" }); },
  unlock: async (p) => {
    await wait(null, 400);
    if (p !== "demo-passphrase") throw "wrong passphrase or corrupted data";
    unlocked = true;
    emit({ type: "unlocked" });
    setTimeout(() => emit({ type: "networkReady", onion: status.onion! }), 900);
  },
  lock: async () => {
    // Temporary rooms live in memory only: locking wipes them.
    for (let i = rooms.length - 1; i >= 0; i--) {
      if (rooms[i]!.temp) {
        delete store[rooms[i]!.id];
        rooms.splice(i, 1);
      }
    }
    unlocked = false;
    emit({ type: "locked" });
  },
  touch: async () => {},
  chats: () => wait(chatsNow()),
  rooms: () => wait(roomsNow()),
  messages: (c, before) => wait([...(store[c] ?? [])].filter((x) => before === undefined || x.id < before).reverse().slice(0, 50)),
  markRead: async (c) => {
    unread[c] = 0;
    const room = rooms.find((r) => r.id === c);
    if (room) room.unread = 0;
  },
  sendText: async (c, text) => send(c, text),
  sendFile: async (c, name, data) => sendFile(c, name, data),
  createRoom: async (name, openInvites, temp) => {
    await wait(null, 300);
    const id = `r-${Math.random().toString(16).slice(2).padEnd(8, "0")}${nextId++}`;
    rooms.push({ id, name, creator: "me", mine: true, openInvites, unread: 0, members: [], temp: temp ?? false });
    store[id] = [];
    emit({ type: "roomChanged", room: id });
    return id;
  },
  startTempChat: async (contact) => {
    const existing = rooms.find((r) => r.direct && r.members[0]?.id === contact);
    if (existing) return existing.id;
    const c = contacts.find((x) => x.id === contact);
    if (!c) throw "unknown contact";
    const id = `r-${Math.random().toString(16).slice(2).padEnd(8, "0")}${nextId++}`;
    rooms.push({ id, name: c.name, creator: "me", mine: true, openInvites: false, unread: 0, temp: true, direct: true, members: [{ id: c.id, name: c.name, online: true, isCreator: false }] });
    store[id] = [];
    emit({ type: "roomChanged", room: id });
    return id;
  },
  roomInvite: async (room, label, ttl) => wait({
    invite: `guft1:${btoa(`${room}:${label}:${ttl}:${"x".repeat(220)}`).replace(/=/g, "")}`,
    code: "r4xm-2pqw-8tzk",
  }),
  addToRoom: async (room, contact) => {
    await wait(null, 200);
    const r = rooms.find((x) => x.id === room);
    const c = contacts.find((x) => x.id === contact);
    if (!r || !c || r.members.some((x) => x.id === contact)) return;
    r.members.push({ id: c.id, name: c.name, online: true, isCreator: false });
    emit({ type: "roomChanged", room });
  },
  leaveRoom: async (room) => {
    const i = rooms.findIndex((r) => r.id === room);
    if (i >= 0) rooms.splice(i, 1);
    delete store[room];
    emit({ type: "roomRemoved", room });
  },
  removeMember: async (room, member) => {
    const r = rooms.find((x) => x.id === room);
    if (r) r.members = r.members.filter((x) => x.id !== member);
    emit({ type: "roomChanged", room });
  },
  renameRoom: async (room, name) => {
    const r = rooms.find((x) => x.id === room);
    if (r) r.name = name;
    emit({ type: "roomChanged", room });
  },
  sendRoomText: async (room, text) => send(room, text),
  sendRoomFile: async (room, name, data) => sendFile(room, name, data),
  newInvite: async (label, ttl) => wait({
    invite: `guft1:${btoa(`${label}:${ttl}:${"x".repeat(220)}`).replace(/=/g, "")}`,
    code: "k3vq-9mzt-bw7x",
  }),
  addContact: async (invite, code) => {
    await wait(null, 500);
    if (!invite.startsWith("guft1:") || code.replace(/\W/g, "").length < 12) throw "invalid input: not a guft invite";
    const id = `n${nextId++}`;
    contacts.push({ id, name: "New contact", onion: onion("n"), verified: false });
    store[id] = [];
    emit({ type: "contactAdded", id, name: "New contact" });
    return id;
  },
  safetyNumber: async () => "30035 44776 92869 39689 28698 76765 45825 75691 62576 84344 09180 79131",
  setVerified: async (c, v) => { const x = contacts.find((k) => k.id === c); if (x) x.verified = v; },
  removeContact: async (c) => { contacts.splice(contacts.findIndex((k) => k.id === c), 1); delete store[c]; },
  deleteChat: async (c) => { store[c] = []; },
  deleteMessage: async (id) => {
    for (const k of Object.keys(store)) store[k] = store[k]!.filter((x) => x.id !== id);
  },
  renameContact: async (c, name) => {
    const x = contacts.find((k) => k.id === c);
    if (!x) throw "unknown contact";
    if (!name.trim()) throw "name is empty";
    x.name = name.trim();
    for (const r of rooms) for (const m of r.members) if (m.id === c) m.name = x.name;
    emit({ type: "contactAdded", id: c, name: x.name });
  },
  saveFile: async (id) => `/home/you/Downloads/guft/file-${id}`,
  fileBytes: async (id) => {
    const d = fileData.get(id);
    if (!d) throw "not a file";
    return d;
  },
  setOnlineWhenLocked: async (on) => { status.onlineWhenLocked = on; },
  setIdleMinutes: async () => {},
  onEvent: async (h) => { handlers.add(h); return () => handlers.delete(h); },
};
