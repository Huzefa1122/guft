// Everything the UI knows about the backend. All calls go through the Tauri command
// layer; in `pnpm dev` without Tauri a mock backend is used (stripped from release builds).
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type Status = {
  hasProfile: boolean;
  unlocked: boolean;
  name: string | null;
  onion: string | null;
  onlineWhenLocked: boolean;
};
export type Contact = { id: string; name: string; onion: string; verified: boolean };
export type MsgStatus = "queued" | "sent" | "delivered" | "failed";
export type Message = {
  id: number;
  chat: string;
  ts: number;
  outgoing: boolean;
  kind: "text" | "file";
  text: string | null;
  fileName: string | null;
  fileSize: number | null;
  status: MsgStatus;
  /** Contact id of the author, for room messages only. */
  sender: string | null;
};
export type Chat = { contact: Contact; last: Message | null; unread: number };
export type RoomMember = { id: string; name: string; online: boolean; isCreator: boolean };
export type Room = {
  id: string;
  name: string;
  creator: string;
  mine: boolean;
  openInvites: boolean;
  /** Memory only: messages vanish when guft locks or closes. */
  temp: boolean;
  /** A one-to-one temporary chat; `name` is the other person's name. */
  direct: boolean;
  /** The separate identity you appear as in this room (its name), or null for your main one. */
  identity: string | null;
  members: RoomMember[];
  last: Message | null;
  unread: number;
};
export type Invite = { invite: string; code: string };
/** A contact asked you to join a room. Nothing has happened, and nobody there knows you, until you accept. */
export type RoomInvitation = { room: string; from: string; fromName: string; name: string; members: string[]; temp: boolean; ts: number };
export type AppEvent =
  | { type: "unlocked" }
  | { type: "locked" }
  | { type: "networkReady"; onion: string }
  | { type: "networkError"; message: string }
  | { type: "contactAdded"; id: string; name: string }
  | { type: "message"; chat: string; id: number }
  | { type: "delivered"; chat: string; msgId: number }
  | { type: "sendFailed"; chat: string; msgId: number }
  | { type: "roomChanged"; room: string }
  | { type: "roomRemoved"; room: string }
  | { type: "roomInvited"; room: string };

export const MAX_FILE_BYTES = 1_000_000;

export interface Api {
  status(): Promise<Status>;
  createProfile(passphrase: string, name: string): Promise<void>;
  unlock(passphrase: string): Promise<void>;
  lock(): Promise<void>;
  touch(): Promise<void>;
  chats(): Promise<Chat[]>;
  rooms(): Promise<Room[]>;
  messages(contact: string, before?: number): Promise<Message[]>;
  markRead(contact: string): Promise<void>;
  sendText(contact: string, text: string): Promise<number>;
  sendFile(contact: string, name: string, dataB64: string): Promise<number>;
  createRoom(name: string, openInvites: boolean, temp?: boolean, identity?: string): Promise<string>;
  startTempChat(contact: string): Promise<string>;
  roomInvitations(): Promise<RoomInvitation[]>;
  acceptRoomInvite(room: string): Promise<string>;
  declineRoomInvite(room: string): Promise<void>;
  roomInvite(room: string, label: string, ttlMinutes: number): Promise<Invite>;
  addToRoom(room: string, contact: string): Promise<void>;
  leaveRoom(room: string): Promise<void>;
  removeMember(room: string, member: string): Promise<void>;
  renameRoom(room: string, name: string): Promise<void>;
  sendRoomText(room: string, text: string): Promise<number>;
  sendRoomFile(room: string, name: string, dataB64: string): Promise<number>;
  newInvite(label: string, ttlMinutes: number): Promise<Invite>;
  addContact(invite: string, code: string, identity?: string): Promise<string>;
  /** Whether an invite leads into a room (only those can be used with a separate identity). */
  inviteIsRoom(invite: string): Promise<boolean>;
  safetyNumber(contact: string): Promise<string>;
  setVerified(contact: string, verified: boolean): Promise<void>;
  removeContact(contact: string): Promise<void>;
  deleteChat(contact: string): Promise<void>;
  deleteMessage(msgId: number): Promise<void>;
  renameContact(contact: string, name: string): Promise<void>;
  saveFile(msgId: number): Promise<string>;
  /** The file bytes, base64 (used to play voice notes). */
  fileBytes(msgId: number): Promise<string>;
  setOnlineWhenLocked(on: boolean): Promise<void>;
  setIdleMinutes(minutes: number): Promise<void>;
  onEvent(handler: (e: AppEvent) => void): Promise<() => void>;
}

const real: Api = {
  status: () => invoke("status"),
  createProfile: (passphrase, name) => invoke("create_profile", { passphrase, name }),
  unlock: (passphrase) => invoke("unlock", { passphrase }),
  lock: () => invoke("lock"),
  touch: () => invoke("touch"),
  chats: () => invoke("chats"),
  rooms: () => invoke("rooms"),
  messages: (contact, before) => invoke("messages", { contact, before: before ?? null, limit: 50 }),
  markRead: (contact) => invoke("mark_read", { contact }),
  sendText: (contact, text) => invoke("send_text", { contact, text }),
  sendFile: (contact, name, data) => invoke("send_file", { contact, name, data }),
  createRoom: (name, openInvites, temp, identity) => invoke("create_room", { name, openInvites, temp: temp ?? false, identity: identity ?? null }),
  startTempChat: (contact) => invoke("start_temp_chat", { contact }),
  roomInvitations: () => invoke("room_invitations"),
  acceptRoomInvite: (room) => invoke("accept_room_invite", { room }),
  declineRoomInvite: (room) => invoke("decline_room_invite", { room }),
  roomInvite: (room, label, ttlMinutes) => invoke("room_invite", { room, label, ttlMinutes }),
  addToRoom: (room, contact) => invoke("add_to_room", { room, contact }),
  leaveRoom: (room) => invoke("leave_room", { room }),
  removeMember: (room, member) => invoke("remove_member", { room, member }),
  renameRoom: (room, name) => invoke("rename_room", { room, name }),
  sendRoomText: (room, text) => invoke("send_room_text", { room, text }),
  sendRoomFile: (room, name, data) => invoke("send_room_file", { room, name, data }),
  newInvite: (label, ttlMinutes) => invoke("new_invite", { label, ttlMinutes }),
  addContact: (invite, code, identity) => invoke("add_contact", { invite, code, identity: identity ?? null }),
  inviteIsRoom: (invite) => invoke("invite_is_room", { invite }),
  safetyNumber: (contact) => invoke("safety_number", { contact }),
  setVerified: (contact, verified) => invoke("set_verified", { contact, verified }),
  removeContact: (contact) => invoke("remove_contact", { contact }),
  deleteChat: (contact) => invoke("delete_chat", { contact }),
  deleteMessage: (msgId) => invoke("delete_message", { msgId }),
  renameContact: (contact, name) => invoke("rename_contact", { contact, name }),
  saveFile: (msgId) => invoke("save_file", { msgId }),
  fileBytes: (msgId) => invoke("file_bytes", { msgId }),
  setOnlineWhenLocked: (on) => invoke("set_online_when_locked", { on }),
  setIdleMinutes: (minutes) => invoke("set_idle_minutes", { minutes }),
  onEvent: async (handler) => {
    const un = await listen<AppEvent>("guft://event", (e) => handler(e.payload));
    return un;
  },
};

const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

let chosen: Api = real;
if (import.meta.env.DEV && !isTauri) {
  // Dynamic import keeps the mock out of production bundles entirely.
  const { mockApi } = await import("./mock");
  chosen = mockApi;
}

export const api: Api = chosen;

/** Backend errors arrive as plain strings. */
export function errorText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return "Something went wrong";
}
