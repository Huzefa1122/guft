# Troubleshooting

## The window is blank, or the terminal says "Error 71 (Protocol error)" on Wayland
WebKitGTK's GPU buffer path fails on some drivers. guft already sets `WEBKIT_DISABLE_DMABUF_RENDERER=1` by default. If you still see it, launch with `WEBKIT_DISABLE_COMPOSITING_MODE=1 ./guft`.

## "Could not connect to localhost"
You are running a development build that expects the dev server. Use a release build, or build with the default features (`cargo build -p guft-desktop`).

## It says "Connecting" for a long time
The first start has to download Tor network data (often 30 to 90 seconds). Networks that block Tor or reset connections to some relays are slower. You can use guft offline meanwhile: sends wait in a queue. If it never connects, check that your firewall allows outgoing TLS connections.

## "not enough circuits" or a message stays on a clock icon
guft needs at least 5 working Tor circuits to a contact for each send. It retries automatically with increasing delays. Large files and cold starts can take a few minutes. If it persists for one contact, check they are online and unlocked (or use "Stay reachable while locked").

## My contact does not get my messages
- They must be running and unlocked (or have "Stay reachable while locked" on).
- If you removed them or they removed you, the connection no longer works; make a new invite.
- Temporary chats only exist while both apps stay unlocked.

## "wrong passphrase or corrupted data"
The passphrase is wrong or the profile files are damaged. After several wrong tries guft makes you wait longer between attempts. There is no recovery without the passphrase.

## The app exits right away with "could not apply the sandbox"
Your kernel or container forbids a hardening step. Release builds refuse to run unprotected. Check that user namespaces and Landlock are not blocked by your container policy. (Development builds can set `GUFT_NO_SANDBOX=1`; do not do that for real use.)

## Voice notes: "No microphone was found" or "Microphone access was blocked"
Check your system sound settings and that another app is not holding the microphone. Make sure the GStreamer packages in [[Install]] are present. If playback fails, use the small download button on the voice note and play the file in another player.

## Where are my files?
Saved files go to `~/Downloads/guft`. The profile is in `~/.local/share/guft`.
