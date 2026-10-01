# Temporary Chats

Some conversations should not exist afterwards. A temporary chat lives **only in memory** on every device taking part.

## Start one

- **With one contact:** open the chat and click the dashed-bubble button in the header. One tap, and the other person's app opens the same chat. Click again later to return to the same one.
- **With a group:** click **New room** and switch on **Temporary room**. Invites and members work as in a normal room.

Temporary chats appear in the list with a dashed avatar and a **TEMP** badge.

## What "temporary" means

- Messages, files, voice notes and the member list are never written to the history database, the outgoing queue or the saved profile. A test reads the profile straight from disk to prove it.
- They are wiped when guft **locks** (manually, on idle, or when you close the window) and when the app exits.
- Ending a one-to-one temporary chat ends it on both sides.
- If the other person locks or restarts, their side is gone. Anything you send after that is ignored by their app, and you may still see it as delivered.
- Unsent temporary messages wait in memory only, so they are lost if you lock before they go out.

## What it does not do

It cannot stop the other person from keeping what they received (screenshots, copying, a modified app), and it does not hide that you and your contact talked over Tor. The ratchet state that keeps your encrypted session working is saved as for any contact.
