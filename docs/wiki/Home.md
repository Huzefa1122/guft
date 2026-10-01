# guft

**guft** is a private chat app with no servers and no accounts. Every person runs their own Tor onion service inside the app, and messages go directly between apps, encrypted with post-quantum cryptography.

> **Status: pre-release.** guft has not been independently audited. Please read [[Threat Model and Limits]] before relying on it.

## What you get

- One-to-one chats, files up to 1 MB, and voice notes up to a minute.
- Private rooms of up to 25 people, with no group server.
- **Temporary chats** that live in memory only and vanish when guft locks or closes.
- Everything encrypted at rest behind a passphrase; the app relocks itself when idle.
- No phone number, no email, no directory: people connect with a one-time invite plus a separate code.

## Start here

| I want to... | Page |
|---|---|
| Install guft and verify the download | [[Install]] |
| Create a profile and add my first contact | [[Getting Started]] |
| Understand files, voice notes and message tools | [[Messaging]] |
| Make a group | [[Rooms]] |
| Chat without leaving a trace | [[Temporary Chats]] |
| Know exactly how it protects me | [[Security Model]] |
| Know what it does not protect | [[Threat Model and Limits]] |
| Fix something | [[Troubleshooting]] |
| Build it or contribute | [[Building From Source]], [[Architecture]] |

Source: <https://github.com/Huzefa1122/guft> · Licence: AGPL-3.0-only
