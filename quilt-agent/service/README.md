# Running quilt-agent as a service

The agent is one process per host; `run <profile>` blocks until stopped and
resumes any in-flight run from its journal on the next start.

| Platform | File | Notes |
|---|---|---|
| Linux | `quilt-agent.service` | systemd; `Restart=always` |
| macOS | `bio.quilt.agent.plist` | launchd daemon; `KeepAlive` |
| Windows | Scheduled Task (below) | no native Windows service yet |

Windows, from an elevated prompt, running as an account that can read the share:

```
setx /M QUILT_AGENT_API_KEY qk_...
schtasks /Create /TN QuiltAgent /SC ONSTART /RU <account> /RP ^
  /TR "\"C:\Program Files\QuiltAgent\quilt-agent.exe\" run C:\ProgramData\QuiltAgent\profile.yaml"
```

Validate a profile before installing: `quilt-agent check profile.yaml`.
