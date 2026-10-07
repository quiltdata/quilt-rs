# Running quilt-agent as a service

The agent is one process per host; `run <profile>` blocks until stopped and
resumes any in-flight run from its journal on the next start.

| Platform | File | Notes |
|---|---|---|
| Linux | `quilt-agent.service` | systemd; `Restart=always` |
| macOS | `bio.quilt.agent.plist` | launchd daemon; `KeepAlive` |
| Windows | Scheduled Task (below) | no native Windows service yet |

Windows, from an elevated prompt. The key goes in the service account's own
environment, not the machine's, so other local users cannot read it:

```bat
set EXE="C:\Program Files\QuiltAgent\quilt-agent.exe"
set PROFILE=C:\ProgramData\QuiltAgent\profile.yaml
schtasks /Create /TN QuiltAgent /SC ONSTART /RU <account> /RP ^
  /TR "cmd /c set QUILT_AGENT_API_KEY=qk_... && %EXE% run %PROFILE%"
```

Restrict the task's ACL to administrators and `<account>`; anyone who can read
the task definition can read the key. An OS keystore replaces this with the
installer.

Validate a profile before installing: `quilt-agent check profile.yaml`.
