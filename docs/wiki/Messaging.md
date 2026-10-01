# Messaging: Chats, Files and Voice Notes

## Messages

- Text up to **8 KiB**. Press Enter to send, Shift+Enter for a new line.
- Ticks show progress: clock = waiting to be sent, one tick = sent, two ticks = delivered to their app.
- If your contact is offline, messages wait and go out when they are reachable.
- Hover a message and use the small arrow: **Copy text**, or **Delete for me**. Deleting an unsent message cancels it. Deleting never removes the message from the other person's device.
- Chat menu: **Verify safety number**, **Rename** (a name only you see), **Clear chat**, **Remove contact**.

## Files

- Use the paperclip. Files must be smaller than **1 MB**.
- Received files are not opened automatically. Use the download button to save one to `~/Downloads/guft`. guft never overwrites an existing file and strips anything dangerous from the name.
- A large file over Tor takes time: about half a minute when connections are warm, a few minutes when they are not. There is no progress bar yet.

## Voice notes

1. Click the **microphone** (it replaces the send button when the message box is empty).
2. Speak. Maximum 60 seconds; the clip is about 180 kB.
3. Click **stop**, listen if you like, then **send** or the bin to discard.

The microphone is released as soon as you stop, cancel, switch chats or lock. Nothing is sent without your second tap.

When you receive a voice note, nothing is decoded until you press **play**. A small download button lets you save the audio and open it elsewhere if playback fails on your system.

Voice notes are ordinary encrypted files named `voice-<time>-<seconds>s.webm`, so they follow the same size limit.

## What guft deliberately does not do

No read receipts, no typing indicators, no link previews, no "last seen", no contact discovery. These would leak information about you.
