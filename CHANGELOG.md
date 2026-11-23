# Changelog

## 0.10.0 (2026-11-23)

- Much lighter on the GPU: the overlay now draws and composites only the parts of the desktop that have something on them.
- New Speech page: write your own list of lines, choose whether they mix with the built-in ones or replace them, and set a line for moments like falling asleep, waking up or being thrown.
- Optional robot voice that reads speech bubbles aloud, off by default. Buddies have their own pitch and can talk over each other.
- Higher Sleepiness now makes buddies doze off on their own, even while you use the PC.
- New installs start with bigger buddies (size 9).
- Studio has a Quit button and a little Clawd as its logo.
- Remove all now silences the robot voice instantly.
- Fixed stars, hearts and other effects staying on screen after they ended.
- Negative gravity: buddies land on the top of the screen and walk around upside down.
- Click any physics number to type your own. Values outside the normal range ask for confirmation first.
- Max speed setting.
- Friction setting. At 0% buddies glide like on ice, and below 0% they speed up as they move, on the ground and in the air.
- Setting to turn collisions between buddies off.
- Fixed forced naps waking up instantly, which left their eyes open.
- Fixed buddies falling off screen when dropped over a visible taskbar. Each monitor now has its own floor.
- Deskling Studio now comes above the buddies while you use it, without covering your other apps, and can still be clicked under a screen-sized buddy. Giant buddies also no longer block a visible taskbar.

## 0.9.0 (2026-11-23)

First public version. Deskling is basically Clawd on your desktop, as an unofficial fan project not affiliated with Anthropic.

- New Rust and GPU engine: up to 200 buddies at 60 fps in one small `.exe`, with no install needed.
- Deskling Studio settings window: colors, patterns, hats, glasses, trails, personality, custom phrases, physics and behaviors.
- Two right-click menus, a radial wheel and a dropdown panel, with themes, accent color, size, opacity and animation speed.
- Unleashed mode removes the limits on crew size, buddy size and physics.
- Import your own images as hats and glasses.
- Remove all buddies from the menu, Studio or a hotkey.
- Glow now follows each buddy's shape.
- Pinned favourites, drag-box group commands, live previews and optional sounds.
- System tray icon and global hotkeys.
- Runs without the Visual C++ Redistributable, and falls back to software drawing on PCs without a usable GPU.
- Local command port now refuses cross-site requests. Typing detection no longer checks individual keys.
