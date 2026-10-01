# Getting Started

## Create your profile

On first launch guft asks for a **display name** and a **passphrase**.

- The display name is what contacts see when you first connect. You can't change it later yet.
- The passphrase encrypts everything on your disk. **There is no recovery:** if you forget it, the data is gone. Use a long phrase (at least 8 characters is required; 5 or more random words is much better).

guft then starts its built-in Tor client. The first start can take a minute; the sidebar shows **Connecting** and then **Online**. You can read old messages while it connects, and anything you send waits in a queue.

## Add a contact

There is no directory. Two people connect with an **invite** and a **one-time code**, shared over *different* channels.

**The person who invites:**

1. Click the **Add contact** icon, stay on **Invite someone**.
2. Optionally label it (only you see the label) and choose how long it lasts (15 minutes to 7 days).
3. Click **Create invite**. You get a long invite text and a short code like `r4xm-2pqw-8tzk`.
4. Send the **invite** by any messenger or email, and give the **code** by voice or in person.

**The person who joins:**

1. Click **Add contact**, choose **I have an invite**.
2. Paste the invite, type the code, click **Add contact**.

Each invite works **once** and expires. A wrong code never burns it. If the invite leaks without the code, it is useless.

## Verify safety numbers (recommended)

Open the chat, then **Verify safety number**. Compare the number with your contact over a channel you trust (a call, in person). If they match, tap **Mark as verified** and the contact shows a ✓. This protects you against someone who tampered with the invite exchange.

## Lock and unlock

guft **locks** itself after being idle (default 5 minutes; change it in Settings), when you press the lock button, and when you close the window. While locked, nothing can be read and the Tor service is stopped.

Optional: **Stay reachable while locked** (Settings) keeps receiving messages and stores them sealed with a post-quantum key; they are readable only after you unlock. Your Tor address key stays in memory while locked if you turn this on.

## Settings

- Idle lock time, theme (system, light, dark).
- Your Tor address (contacts reach you there; it is not public).
- A summary of how messages are protected.
