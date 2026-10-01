# Rooms

A room is a private group of up to **25 people** (you can be in up to 50 rooms). There is no group server: when you write, your app sends the message separately to each member over that member's own post-quantum encrypted connection.

## Create one

Click the **New room** icon, give it a name, and choose whether **members can invite**. Then invite people in either of two ways:

- **Invite someone new** from the room menu: this creates an invite and code exactly like adding a contact, but whoever uses it also joins the room. guft introduces them to every other member automatically, so they become connected to all of them.
- **Add existing contacts** from **Members**: they get an **invitation** and join only if they accept (see below).

## Invitations: nobody is added without saying yes

When a contact adds you to a room, nothing happens yet. You see an invitation at the top of your chat list: who invited you, the room's name and who is in it. **Until you press Join, nobody in the room learns anything about you** (no name, no address, no contact is created). **Decline** tells only the person who invited you, and they stop counting you in.

If you use a room *invite and code* yourself, that already is your yes, so you join straight away, but only that one room. A later invitation from the same person to another room waits for you again.

Details: invitations are kept encrypted on your device for up to 14 days, at most 5 per person and 30 in total. Invitations to a **temporary** room are held in memory only and disappear when guft locks. A one-to-one temporary chat opens without a prompt because nobody else is involved.

## Separate identities: a different you in every room

Switch on **Separate identity** when you create a room, or when you use a room invite, and guft makes a brand-new identity just for that room: **a new name you choose, new encryption keys, and a new Tor address.** Nothing is shared with your main identity or with your other rooms, so nobody can tell it is the same person, not even by comparing Tor addresses or keys.

- It exists only for that room. **Leaving, being removed, or the room ending erases the identity completely** (keys, address, history). Your leave notice is sent first so the others see you go.
- Its people are not in your contact list, and your main contacts cannot be added to it (that would tie them together). Invite people with a room invite and code.
- You can still be in other rooms as yourself, or as other identities, at the same time (up to 8 separate identities at once, each with its own Tor connection).
- It locks and unlocks with the rest of guft. Temporary rooms can use one too.
- If someone you already know invites you into a room, you join as your main identity (they know who you are). To appear as someone else, use a room invite and code.

Honest limits: the person whose invite you used knows you came in with that invite, and anyone who can watch your internet connection could still notice that two Tor connections start at the same time.

## Who can do what

| | Creator | Member |
|---|---|---|
| Send and read | yes | yes |
| Invite | yes | only if "members can invite" is on |
| Rename the room | yes | no |
| Remove a member | yes | no |
| Leave | yes | yes |

Removing someone works because nobody encrypts to them any more; they stop receiving new messages. They keep what they already received.

## Delivery

A message shows **delivered** only when **every** member's app has it. If someone is offline you see **sent**. Members only receive a message while the *sender* is online with them: if you send and then close guft before an offline member comes back, they miss it. (Relaying missed messages between members is planned.)

## Be aware

- Everyone in a room can copy or screenshot anything. A room is as private as its least careful member.
- Joining a room connects you to its other members; they learn your name and Tor address.
- Room names are chosen by the creator. Treat unfamiliar names with the same care as unfamiliar contacts.
