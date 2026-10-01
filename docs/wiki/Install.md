# Install

guft currently ships as a **Linux x86_64** binary. Windows and macOS are not supported yet (the process sandbox is Linux-only).

## 1. Download

Get the latest archive from the [Releases page](https://github.com/Huzefa1122/guft/releases). Each release contains:

- `guft-<version>-linux-x86_64.tar.gz` — the app
- `SHA256SUMS` — checksums
- `SHA256SUMS.asc` — a GPG signature over the checksums

## 2. Verify

```sh
sha256sum -c SHA256SUMS --ignore-missing
gpg --verify SHA256SUMS.asc SHA256SUMS
```

The signing key is the one that signs the commits in the repository (`gpg: using EDDSA key 476631EF…`). Compare the fingerprint with the one on the maintainer's GitHub profile before trusting a first download.

## 3. Runtime requirements

guft uses your system's web engine, so install these first:

| Distro | Packages |
|---|---|
| Arch | `webkit2gtk-4.1 gtk3` |
| Debian / Ubuntu | `libwebkit2gtk-4.1-0 libgtk-3-0` |
| Fedora | `webkit2gtk4.1 gtk3` |

For voice notes you also need your system's GStreamer plugins (`gst-plugins-good`, `gst-plugins-base`, and `gst-plugins-bad` for Opus) and a working microphone.

You do **not** need to install Tor: it is built into the app.

## 4. Run

```sh
tar xf guft-<version>-linux-x86_64.tar.gz
cd guft-<version>-linux-x86_64
./guft
```

Optionally copy `guft` to `~/.local/bin`, and `guft.desktop` plus `guft.png` into `~/.local/share/applications` and `~/.local/share/icons` to get a launcher entry.

## Where things live

| What | Where |
|---|---|
| Your profile (encrypted state and history) | `~/.local/share/guft` (override with `GUFT_PROFILE_DIR`) |
| Files you save from chats | `~/Downloads/guft` |
| Window and preference data | `~/.local/share/dev.guft.app` |

To **back up** a profile, close guft and copy the whole profile directory. Do not run the same profile on two machines at once: the encryption ratchets will fall out of sync.

To **uninstall**, delete the binary and those directories.

## Updating

There is no auto-updater yet. Download the new release, verify it, and replace the binary. While guft is pre-1.0 the saved profile has no version number, so a release may require creating a fresh profile. The release notes say when that happens; back up first.
