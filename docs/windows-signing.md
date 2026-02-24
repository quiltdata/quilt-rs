# Windows Code Signing — Manual Checklist

This document lists the **remaining manual steps** required to enable
Windows SmartScreen–trusted distribution for Quilt Sync.

## 1. Purchase the Certificate

- Buy **DigiCert Standard Code Signing – Organization (OV)**
- Legal name: **Quilt Data, Inc.** (use exact casing)
- Term: **1 year** (upgrade later if needed)

Reference:

- <https://www.digicert.com/code-signing/>

## 2. Complete DigiCert Organization Verification

- Provide Articles of Incorporation or state registry link
- Complete DigiCert email or phone verification
- Optional but recommended: obtain a **D-U-N-S Number**

Reference:

- <https://www.dnb.com/duns.html>

## 3. Export the Certificate

- Export the issued certificate as **PFX (.pfx)**
- Protect with a strong password
- Confirm support for:
  - SHA-256
  - Windows Authenticode
  - RFC3161 timestamping

Reference:

- <https://learn.microsoft.com/windows/win32/seccrypto/code-signing>

## 4. Configure GitHub Repository Secrets

Add the following **Actions secrets**:

- `WINDOWS_PFX_BASE64` — base64-encoded `.pfx`
- `WINDOWS_PFX_PASSWORD` — PFX password

Notes:

- These secrets are required only for Windows builds
- Tauri updater signing secrets remain unchanged

## 5. Run a Signed Release Build

- Push a version tag (e.g. `v0.13.1`)
- Confirm GitHub Actions:
  - Builds Windows installers
  - Signs all `.exe` and `.msi` files
- Verify signatures manually (see step 6)

## 6. One-Time Manual Verification

- Download installer from GitHub Releases
- Right-click → **Properties → Digital Signatures**
- Confirm publisher displays **Quilt Data, Inc.**

Reference:

- <https://learn.microsoft.com/windows/security/operating-system-security/virus-and-threat-protection/microsoft-defender-smartscreen>

## 7. Preserve SmartScreen Reputation

- Do not change:
  - Publisher name
  - Certificate subject
- Renew before expiration using the same organization identity

Changing identities resets SmartScreen reputation.
