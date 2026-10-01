# Rooms

A room is a private group of up to **25 people** (you can be in up to 50 rooms). There is no group server: when you write, your app sends the message separately to each member over that member's own post-quantum encrypted connection.

## Create one

Click the **New room** icon, give it a name, and choose whether **members can invite**. Then invite people in either of two ways:

- **Invite someone new** from the room menu: this creates an invite and code exactly like adding a contact, but whoever uses it also joins the room. guft introduces them to every other member automatically, so they become connected to all of them.
- **Add existing contacts** from **Members**: they join immediately.

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
