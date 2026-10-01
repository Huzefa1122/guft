# Threat Model and Limits

guft is **pre-release and unaudited**. Do not rely on it against a determined, well-resourced adversary yet.

## What it is designed to resist

- Someone watching or recording the network (including a future quantum computer).
- A server compromise or subpoena: there is no server.
- Someone who steals your disk or a backup while guft is locked.
- Someone who finds your Tor address or an intercepted invite (without the code).
- Malformed or malicious data from the network or a contact.

## What it does not protect

- **Metadata.** Tor hides the network path, not the fact that two onion services exchange cells. Your contacts know your Tor address, and room members learn it too.
- **A compromised computer.** Malware, keyloggers, screen capture, or someone using your unlocked machine can read everything. A memory dump while unlocked exposes keys. Nothing is `mlock`ed, so use encrypted swap.
- **Your contacts.** Anyone can screenshot, copy or leak what they receive. Deleting a message or ending a temporary chat only affects your own device.
- **A weak passphrase.** Someone with a copy of your profile files can guess offline; Argon2id slows them but cannot save "password1".
- **Careless invite sharing.** If the invite and the code travel over the same compromised channel, whoever holds both can connect.
- **Availability.** Hostile contacts can waste your bandwidth, and Tor is slow. Delivery is best-effort.
- **Supply chain.** guft pins `libsignal` to a reviewed tag and runs `cargo audit`, but upstream bugs and compromised dependencies are possible. The `libsignal` and Arti onion-service APIs used are still marked experimental or unstable upstream.

## Known limitations

- A contact can add you to a **room** and you join automatically, which also introduces you to its members. Room invitations will get an accept step.
- Room messages reach a member only if the sender and that member are online at overlapping times; relaying of missed messages is planned.
- No passphrase change, no built-in backup or export, no schema versioning of the saved profile yet.
- Linux only; other platforms would currently run without the sandbox.
- Voice-note playback depends on your system's media decoders; a malformed audio file is handled by the system, not by guft. It is only decoded when you press play.
- Large files take minutes on a cold Tor connection and show no progress yet.

## Reporting a vulnerability

Please report privately to the maintainer through GitHub (a private security advisory on the repository) rather than a public issue. Include the version, what you expected and what happened. Never include real passphrases, invite codes or message contents.
