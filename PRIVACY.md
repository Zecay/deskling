# Privacy

Deskling has no telemetry, no accounts and no ads, and it never contacts the internet.

- **Typing along** is off by default. When on, it asks Windows only when the last input happened. It can't see which keys you press and records no text.
- **Local port:** the app listens on `127.0.0.1:47831` so Claude Code can control it. Other computers can't reach it, and connections that don't start with a valid command are closed.
- **Screen:** it reads window positions so buddies can stand on them. It never reads window contents or takes screenshots.
- **Files:** your choices are saved in `%APPDATA%\Deskling\settings.json`. Images you import as hats or glasses are copied to the `wear` folder there. If the app crashes, details go to `crash.log` in the same folder and stay there unless you share them.
