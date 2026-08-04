# City ops — standing roles (launchd / cron)

rs-agent city offices watch via cron; models act when woken.

## Roles

| Seat | Command | Cadence |
|---|---|---|
| Beadle | `rs-agent role --seat Beadle --once` | every 10–15 min overnight |
| Gargoyle | `rs-agent role --seat Gargoyle --once` | every 30–60 min |
| Drawbridge | `rs-agent role --seat Drawbridge --once` | when CI matters |
| Scryer | `rs-agent role --seat Scryer --once --source PATH` | as needed |
| Marshal | `rs-agent marshal --loop --interval-secs 60` | overnight |

Standing orders live in `brain/roles/<role>.md` (auto-seeded on first run).

## Fish examples

```fish
set -l BIN ~/work/scripts/rs-agent/target/release/rs-agent
cd ~/work/scripts/metal-operators

# Wish intake before bed
$BIN wish "port flashlib Softmax" --auto
$BIN wish "add Metal Softmax tests" --auto

# Fleet + marshal
$BIN -a fleet up --seats Fleet-1,Fleet-2 --budget-minutes 480
$BIN marshal --loop --interval-secs 90 --budget-minutes 480 &

# Beadle unstick (cron-friendly)
$BIN role --seat Beadle --once

# Morning cockpit
$BIN   # TUI: /fleet /mail /beads ready /brain ledger /laurels
```

## launchd (macOS) sketch

`~/Library/LaunchAgents/ai.rs-agent.beadle.plist`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>ai.rs-agent.beadle</string>
  <key>WorkingDirectory</key><string>/Users/YOU/work/scripts/metal-operators</string>
  <key>ProgramArguments</key>
  <array>
    <string>/Users/YOU/work/scripts/rs-agent/target/release/rs-agent</string>
    <string>role</string>
    <string>--seat</string>
    <string>Beadle</string>
    <string>--once</string>
  </array>
  <key>StartInterval</key><integer>900</integer>
  <key>StandardOutPath</key><string>/tmp/rs-agent-beadle.log</string>
  <key>StandardErrorPath</key><string>/tmp/rs-agent-beadle.err</string>
</dict>
</plist>
```

```fish
launchctl load ~/Library/LaunchAgents/ai.rs-agent.beadle.plist
```

## systemd timer sketch

```ini
# /etc/systemd/user/rs-agent-beadle.service
[Service]
Type=oneshot
WorkingDirectory=%h/work/scripts/metal-operators
ExecStart=%h/work/scripts/rs-agent/target/release/rs-agent role --seat Beadle --once
```

```ini
# /etc/systemd/user/rs-agent-beadle.timer
[Timer]
OnUnitActiveSec=15min
Unit=rs-agent-beadle.service
[Install]
WantedBy=timers.target
```

## Recommended ops

- One worktree per fleet seat when editing the same repo concurrently.
- Crew (strong model) for design/review seats; cheaper model on Fleet-* seats via `/seat model`.
- Seneschal TUI for `/mail` when away from the desk.
