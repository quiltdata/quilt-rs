# Windows Code Signing — Azure Trusted Signing

Windows Authenticode signing for QuiltSync uses **Azure Trusted Signing**
authenticated via GitHub Actions OIDC. No certificate secrets are stored in
GitHub — the private key never leaves Azure.

## How it works

1. GitHub Actions requests an OIDC token from GitHub's identity provider
2. `azure/login` exchanges that token for an Azure access token (federated
   identity, no client secret)
3. After the Tauri build, `azure/trusted-signing-action` sends the built
   `.exe` and `.msi` artifacts to Azure for signing
4. Azure returns the signed files; the private key is never exposed to the
   runner

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
| `AZURE_SUBSCRIPTION_ID` | Azure subscription ID |
| `AZURE_TRUSTED_SIGNING_ENDPOINT` | Endpoint URL (e.g. `https://eus.codesigning.azure.net`) |
| `AZURE_TRUSTED_SIGNING_ACCOUNT` | Trusted Signing account name |
| `AZURE_TRUSTED_SIGNING_PROFILE` | Certificate profile name |

### Secrets to remove (no longer needed)

- `WINDOWS_PFX_BASE64`
- `WINDOWS_PFX_PASSWORD`

## Verification

After a signed release build:

- Download the installer from GitHub Releases
- Right-click → **Properties → Digital Signatures** — confirm publisher shows
  **Quilt Data, Inc.**
- Or in PowerShell:
  `Get-AuthenticodeSignature .\QuiltSync_x.y.z_x64-setup.exe`
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
