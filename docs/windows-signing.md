# Windows Code Signing — Azure Trusted Signing

Windows Authenticode signing for QuiltSync uses **Azure Trusted Signing**
authenticated via GitHub Actions OIDC. No certificate secrets are stored in
GitHub — the private key never leaves Azure.

## How it works

Signing happens inside Tauri's own build, so the release keeps the signed
files and the updater signature (`.sig`, `latest.json`) is made over them.

1. The Windows release job passes
   `--config src-tauri/windows-sign.conf.json` to tauri-action. That file
   sets Tauri's `bundle.windows.signCommand`, so only that job signs; local
   builds and the other platforms don't need Azure.
2. Tauri runs `quilt-sync/src-tauri/windows-sign.ps1` for each file it signs:
   the app `.exe` and the `.msi`.
3. The script asks GitHub for a fresh OIDC token, runs `az login` with it
   (federated identity, no client secret), and signs the file with
   `Invoke-ArtifactSigning` from the `ArtifactSigning` PowerShell module. It
   logs in per file because GitHub's token lives only minutes and Tauri
   signs at the end of a long build.
4. Azure signs the file; the private key never reaches the runner. The
   script fails the build if the file's signature isn't valid.
5. Tauri then makes the updater signature and tauri-action uploads the
   signed files to the draft release.

## Azure setup (one-time)

### 1. Create an Azure Trusted Signing account

In the [Azure portal](https://portal.azure.com):

- Search for **Trusted Signing**
- Create a new account (choose a region close to your runners, e.g. East US)
- Note the **endpoint URL** (e.g. `https://eus.codesigning.azure.net`)

Reference: <https://learn.microsoft.com/azure/trusted-signing/quickstart>

### 2. Create a Certificate Profile

Inside the Trusted Signing account:

- Create a **Certificate Profile** of type **Public Trust**
- Set the **Organization** to `Quilt Data, Inc.` (exact casing)
- Note the **profile name**

### 3. Create an App Registration

In **Azure Active Directory (Entra ID)**:

- Create a new **App Registration**
- Note the **Application (client) ID** and **Directory (tenant) ID**

### 4. Add a Federated Credential

On the App Registration → **Certificates & secrets → Federated credentials**:

- Add a credential for **GitHub Actions**
- Organization: `quiltdata`
- Repository: `quilt-rs`
- Entity: **Environment** → `<your environment name>` (matches
  `inputs.environment` in the workflow)
- Note: if you don't use environments, use **Branch** and set `main`

### 5. Assign the signing role

On the **Certificate Profile** resource → **Access control (IAM)**:

- Add role assignment: **Trusted Signing Certificate Profile Signer**
- Assign to the App Registration (service principal) created above

## GitHub configuration

### Variables to add

Settings → Secrets and variables → Actions → Variables:

| Variable | Value |
| --- | --- |
| `AZURE_TENANT_ID` | Directory (tenant) ID from the App Registration |
| `AZURE_CLIENT_ID` | Application (client) ID from the App Registration |
| `AZURE_TRUSTED_SIGNING_ENDPOINT` | Endpoint URL (e.g. `https://eus.codesigning.azure.net`) |
| `AZURE_TRUSTED_SIGNING_ACCOUNT` | Trusted Signing account name |
| `AZURE_TRUSTED_SIGNING_PROFILE` | Certificate profile name |

### Secrets to remove (no longer needed)

- `WINDOWS_PFX_BASE64`
- `WINDOWS_PFX_PASSWORD`

## Verification

After a signed release build:

- Download the `.msi` from the draft GitHub release
- Right-click → **Properties → Digital Signatures** — confirm publisher shows
  **Quilt Data, Inc.**
- Or in PowerShell:
  `Get-AuthenticodeSignature .\QuiltSync_x.y.z_x64_en-US.msi`
- Or on macOS/Linux: `osslsigncode verify -in QuiltSync_x.y.z_x64_en-US.msi`
- Signing history is also visible in the Azure portal under the Certificate
  Profile

## SmartScreen reputation

Azure Trusted Signing issues **Public Trust** certificates (equivalent to EV),
which carry immediate SmartScreen trust — no reputation-building period
required.

To maintain trust:

- Do not change the publisher name or certificate subject
- Renew the certificate profile before expiration using the same organization
  identity

Changing organization identity resets SmartScreen reputation.
